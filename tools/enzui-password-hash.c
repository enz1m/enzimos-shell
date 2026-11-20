#define _GNU_SOURCE
#include <crypt.h>
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/resource.h>
#include <unistd.h>

static int write_all(int fd, const char *bytes, size_t length) {
    while (length) {
        ssize_t written = write(fd, bytes, length);
        if (written < 0 && errno == EINTR) continue;
        if (written <= 0) return -1;
        bytes += written;
        length -= (size_t)written;
    }
    return 0;
}

int main(int argc, char **argv) {
    (void)argv;
    char password[CRYPT_MAX_PASSPHRASE_SIZE] = {0};
    char setting[CRYPT_GENSALT_OUTPUT_SIZE] = {0};
    struct crypt_data state = {0};
    struct rlimit no_core = {0, 0};
    size_t length = 0;
    int status = 1;
    (void)setrlimit(RLIMIT_CORE, &no_core);
    (void)prctl(PR_SET_DUMPABLE, 0, 0, 0, 0);
    if (argc != 1) goto done;
    for (;;) {
        ssize_t count = read(STDIN_FILENO, password + length, sizeof(password) - length);
        if (count < 0 && errno == EINTR) continue;
        if (count < 0) goto done;
        if (count == 0) break;
        length += (size_t)count;
        if (length >= sizeof(password)) goto done;
    }
    if (!length || memchr(password, 0, length)) goto done;
    /* libxcrypt chooses its supported default algorithm and obtains fresh OS entropy. */
    if (!crypt_gensalt_rn(NULL, 0, NULL, 0, setting, sizeof(setting))) goto done;
    const char *hash = crypt_r(password, setting, &state);
    if (!hash || hash[0] != '$') goto done;
    if (write_all(STDOUT_FILENO, hash, strlen(hash)) || write_all(STDOUT_FILENO, "\n", 1)) goto done;
    status = 0;
done:
    explicit_bzero(password, sizeof(password));
    explicit_bzero(setting, sizeof(setting));
    explicit_bzero(&state, sizeof(state));
    if (status) fputs("Password hashing unavailable.\n", stderr);
    return status;
}
