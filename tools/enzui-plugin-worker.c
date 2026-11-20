#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <linux/magic.h>
#include <linux/memfd.h>
#include <sched.h>
#include <seccomp.h>
#include <signal.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <sys/statfs.h>
#include <sys/syscall.h>
#include <unistd.h>

static void fail(void) {
  fputs("Plugin isolation could not be established\n", stderr);
  exit(126);
}
static bool hex32(const char *s) {
  if (strlen(s) != 32)
    return false;
  for (size_t i = 0; i < 32; i++)
    if (!((s[i] >= '0' && s[i] <= '9') || (s[i] >= 'a' && s[i] <= 'f')))
      return false;
  return true;
}
static void read_at(int directory, const char *name, char *out,
                    size_t capacity) {
  int fd = openat(directory, name, O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
  if (fd < 0)
    fail();
  ssize_t n = read(fd, out, capacity - 1);
  if (n <= 0 || (size_t)n >= capacity - 1)
    fail();
  out[n] = 0;
  close(fd);
}
static unsigned long long number_at(int fd, const char *name) {
  char buf[80], *end;
  read_at(fd, name, buf, sizeof(buf));
  errno = 0;
  unsigned long long n = strtoull(buf, &end, 10);
  if (errno || end == buf || (*end != '\n' && *end != 0))
    fail();
  return n;
}
static unsigned long long parent_start(pid_t pid) {
  char path[80], buf[4096];
  snprintf(path, sizeof(path), "/proc/%ld/stat", (long)pid);
  int fd = open(path, O_RDONLY | O_CLOEXEC);
  if (fd < 0)
    fail();
  ssize_t n = read(fd, buf, sizeof(buf) - 1);
  close(fd);
  if (n <= 0 || (size_t)n >= sizeof(buf) - 1)
    fail();
  buf[n] = 0;
  char *tail = strrchr(buf, ')');
  if (!tail)
    fail();
  tail++;
  char *save = NULL, *field = strtok_r(tail, " ", &save);
  for (int i = 0; i < 19; i++) {
    if (!field)
      fail();
    field = strtok_r(NULL, " ", &save);
  }
  if (!field)
    fail();
  char *end;
  errno = 0;
  unsigned long long start = strtoull(field, &end, 10);
  if (errno || end == field || *end)
    fail();
  return start;
}
static int worker_cgroup(const char *requested, pid_t parent) {
  char current[PATH_MAX] = {0}, line[PATH_MAX + 64];
  FILE *file = fopen("/proc/self/cgroup", "re");
  if (!file)
    fail();
  while (fgets(line, sizeof(line), file))
    if (strncmp(line, "0::", 3) == 0) {
      if (current[0])
        fail();
      size_t len = strcspn(line + 3, "\n");
      if (len >= sizeof(current))
        fail();
      memcpy(current, line + 3, len);
      current[len] = 0;
    }
  fclose(file);
  size_t len = strlen(current);
  const char suffix[] = "/supervisor";
  if (len < sizeof(suffix) ||
      strcmp(current + len - (sizeof(suffix) - 1), suffix))
    fail();
  current[len - (sizeof(suffix) - 1)] = 0;
  char expected[PATH_MAX], resolved[PATH_MAX];
  int count =
      snprintf(expected, sizeof(expected), "/sys/fs/cgroup%s/plugins-%ld-%llu-",
               current, (long)parent, parent_start(parent));
  if (count < 0 || (size_t)count >= sizeof(expected) ||
      !realpath(requested, resolved) || strcmp(requested, resolved) ||
      strncmp(resolved, expected, (size_t)count))
    fail();
  char remainder[96];
  if (strlen(resolved + count) >= sizeof(remainder))
    fail();
  strcpy(remainder, resolved + count);
  char *slash = strchr(remainder, '/');
  if (!slash)
    fail();
  *slash = 0;
  if (!hex32(remainder) || strncmp(slash + 1, "w-", 2) || !hex32(slash + 3))
    fail();
  int fd = open(resolved, O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW);
  if (fd < 0)
    fail();
  struct stat st;
  struct statfs fs;
  if (fstat(fd, &st) || fstatfs(fd, &fs) || st.st_uid != getuid() ||
      fs.f_type != CGROUP2_SUPER_MAGIC)
    fail();
  unsigned long long memory = number_at(fd, "memory.max"),
                     pids = number_at(fd, "pids.max");
  if (memory < 16 * 1024 * 1024 || memory > 96 * 1024 * 1024 || pids < 4 ||
      pids > 32 || number_at(fd, "memory.swap.max") ||
      number_at(fd, "memory.oom.group") != 1)
    fail();
  char cpu[80], extra;
  unsigned long long quota, period;
  read_at(fd, "cpu.max", cpu, sizeof(cpu));
  if (sscanf(cpu, "%llu %llu %c", &quota, &period, &extra) != 2 ||
      period < 1000 || period > 1000000 || quota == 0 || quota > period / 4)
    fail();
  int procs = openat(fd, "cgroup.procs", O_WRONLY | O_CLOEXEC | O_NOFOLLOW);
  if (procs < 0 || write(procs, "0\n", 2) != 2)
    fail();
  close(procs);
  return fd;
}
static void read_exact(int fd, unsigned char *out, size_t size) {
  while (size) {
    ssize_t n = read(fd, out, size);
    if (n < 0 && errno == EINTR)
      continue;
    if (n <= 0)
      fail();
    out += n;
    size -= (size_t)n;
  }
}
static int program_source(void) {
  unsigned char header[12], buffer[8192];
  alarm(5);
  read_exact(STDIN_FILENO, header, sizeof(header));
  uint32_t size = ((uint32_t)header[8] << 24) | ((uint32_t)header[9] << 16) |
                  ((uint32_t)header[10] << 8) | header[11];
  if (memcmp(header, "ENZPY002", 8) || !size || size > 512 * 1024)
    fail();
  int fd = (int)syscall(SYS_memfd_create, "enzui-plugin-source", MFD_ALLOW_SEALING);
  if (fd < 0)
    fail();
  while (size) {
    size_t take = size < sizeof(buffer) ? size : sizeof(buffer);
    read_exact(STDIN_FILENO, buffer, take);
    size_t offset = 0;
    while (offset < take) {
      ssize_t n = write(fd, buffer + offset, take - offset);
      if (n < 0 && errno == EINTR)
        continue;
      if (n <= 0)
        fail();
      offset += (size_t)n;
    }
    size -= (uint32_t)take;
  }
  if (fchmod(fd, 0400) || lseek(fd, 0, SEEK_SET) < 0 ||
      fcntl(fd, F_ADD_SEALS,
            F_SEAL_WRITE | F_SEAL_GROW | F_SEAL_SHRINK | F_SEAL_SEAL) < 0)
    fail();
  alarm(0);
  return fd;
}
static int sandbox_filter(void) {
  scmp_filter_ctx ctx = seccomp_init(SCMP_ACT_ALLOW);
  if (!ctx)
    fail();
  const char *denied[] = {"ptrace",
                          "process_vm_readv",
                          "process_vm_writev",
                          "kcmp",
                          "kexec_load",
                          "kexec_file_load",
                          "init_module",
                          "finit_module",
                          "delete_module",
                          "open_by_handle_at",
                          "name_to_handle_at",
                          "perf_event_open",
                          "bpf",
                          "userfaultfd",
                          "keyctl",
                          "add_key",
                          "request_key",
                          "mount",
                          "umount2",
                          "move_mount",
                          "open_tree",
                          "fsopen",
                          "fsmount",
                          "fsconfig",
                          "fspick",
                          "pivot_root",
                          "swapon",
                          "swapoff",
                          "reboot",
                          "setns",
                          "unshare"};
  for (size_t i = 0; i < sizeof(denied) / sizeof(denied[0]); i++) {
    int nr = seccomp_syscall_resolve_name(denied[i]);
    if (nr != __NR_SCMP_ERROR &&
        seccomp_rule_add(ctx, SCMP_ACT_ERRNO(EPERM), nr, 0) < 0)
      fail();
  }
  if (seccomp_rule_add(ctx, SCMP_ACT_ERRNO(ENOSYS), SCMP_SYS(clone3), 0) < 0)
    fail();
  const uint64_t flags[] = {CLONE_NEWUSER,  CLONE_NEWNET, CLONE_NEWNS,
                            CLONE_NEWPID,   CLONE_NEWIPC, CLONE_NEWUTS,
                            CLONE_NEWCGROUP};
  for (size_t i = 0; i < sizeof(flags) / sizeof(flags[0]); i++)
    if (seccomp_rule_add(ctx, SCMP_ACT_ERRNO(EPERM), SCMP_SYS(clone), 1,
                         SCMP_A0(SCMP_CMP_MASKED_EQ, flags[i], flags[i])) < 0)
      fail();
  int fd =
      (int)syscall(SYS_memfd_create, "enzui-plugin-filter", MFD_ALLOW_SEALING);
  if (fd < 0)
    fail();
  if (seccomp_export_bpf(ctx, fd) < 0 || lseek(fd, 0, SEEK_SET) < 0 ||
      fcntl(fd, F_ADD_SEALS,
            F_SEAL_WRITE | F_SEAL_GROW | F_SEAL_SHRINK | F_SEAL_SEAL) < 0)
    fail();
  seccomp_release(ctx);
  return fd;
}
int main(int argc, char **argv) {
  if (argc == 2 && !strcmp(argv[1], "--version")) {
    puts("enzui-plugin-worker 2");
    return 0;
  }
  if (argc != 3 || strcmp(argv[2], "--pipe-source-v2"))
    fail();
  if (close_range(3, UINT_MAX, 0))
    fail();
  pid_t parent = getppid();
  if (parent <= 1 || prctl(PR_SET_PDEATHSIG, SIGKILL) || getppid() != parent)
    fail();
  int group = worker_cgroup(argv[1], parent);
  struct rlimit core = {0, 0}, files = {64, 64},
                file_size = {4 * 1024 * 1024, 4 * 1024 * 1024};
  if (setrlimit(RLIMIT_CORE, &core) || setrlimit(RLIMIT_NOFILE, &files) ||
      setrlimit(RLIMIT_FSIZE, &file_size) ||
      prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0))
    fail();
  int program = program_source();
  int filter = sandbox_filter();
  char filter_text[32], program_text[32];
  snprintf(filter_text, sizeof(filter_text), "%d", filter);
  snprintf(program_text, sizeof(program_text), "%d", program);
  close(group);
  if (clearenv())
    fail();
  execl("/usr/bin/bwrap", "bwrap", "--unshare-all", "--die-with-parent",
        "--new-session", "--cap-drop", "ALL", "--hostname", "enzui-plugin",
        "--ro-bind", "/usr", "/usr", "--symlink", "usr/lib", "/lib",
        "--symlink", "usr/lib", "/lib64", "--proc", "/proc", "--dev", "/dev",
        "--tmpfs", "/tmp", "--dir", "/plugin", "--perms", "0400",
        "--ro-bind-data", program_text, "/plugin/main.py", "--chdir",
        "/plugin", "--clearenv", "--setenv", "PATH", "/usr/bin", "--setenv",
        "LANG", "C.UTF-8", "--seccomp", filter_text, "--", "/usr/bin/python3",
        "-I", "-S", "-B", "/plugin/main.py", (char *)NULL);
  fail();
}
