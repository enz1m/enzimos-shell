#define _GNU_SOURCE
#include "wlr-screencopy-unstable-v1-client-protocol.h"
#include <errno.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <unistd.h>
#include <wayland-client.h>

struct state;
struct output {
  struct state *state;
  struct wl_output *object;
  uint32_t id;
};
struct state {
  struct wl_display *display;
  struct wl_shm *shm;
  struct zwlr_screencopy_manager_v1 *manager;
  struct output outputs[16];
  unsigned count;
  struct wl_output *selected;
  const char *name;
  struct wl_buffer *buffer;
  struct wl_shm_pool *pool;
  void *pixels;
  uint32_t width, height, stride, format, flags;
  uint64_t timestamp;
  bool done, failed;
};
static void die(const char *message) {
  fprintf(stderr, "record-shell-frames: %s\n", message);
  exit(1);
}
static uint64_t number(const char *s) {
  char *end;
  errno = 0;
  unsigned long long n = strtoull(s, &end, 10);
  if (errno || !*s || *s == '-' || *end)
    die("invalid numeric argument");
  return n;
}
static uint64_t process_start(pid_t pid) {
  char path[64], line[4096];
  snprintf(path, sizeof(path), "/proc/%d/stat", pid);
  FILE *file = fopen(path, "r");
  if (!file)
    die("capture process disappeared");
  if (!fgets(line, sizeof(line), file))
    die("cannot read process identity");
  fclose(file);
  char *end = strrchr(line, ')');
  if (!end)
    die("invalid process identity");
  char *save = NULL, *word = strtok_r(end + 2, " ", &save);
  for (int i = 0; i < 19 && word; i++)
    word = strtok_r(NULL, " ", &save);
  if (!word)
    die("missing process start time");
  return number(word);
}
static void geometry(void *d, struct wl_output *o, int32_t x, int32_t y,
                     int32_t pw, int32_t ph, int32_t sub, const char *make,
                     const char *model, int32_t transform) {
  if (transform != 0)
    ((struct output *)d)->state->failed = true;
}
static void mode(void *d, struct wl_output *o, uint32_t flags, int32_t w,
                 int32_t h, int32_t refresh) {}
static void output_done(void *d, struct wl_output *o) {}
static void scale(void *d, struct wl_output *o, int32_t scale) {
  if (scale != 1)
    ((struct output *)d)->state->failed = true;
}
static void name(void *d, struct wl_output *o, const char *name) {
  struct output *out = d;
  if (!strcmp(name, out->state->name)) {
    if (out->state->selected)
      die("ambiguous capture output");
    out->state->selected = o;
  }
}
static void description(void *d, struct wl_output *o, const char *text) {}
static const struct wl_output_listener output_listener = {
    geometry, mode, output_done, scale, name, description};
static void global(void *d, struct wl_registry *registry, uint32_t id,
                   const char *interface, uint32_t version) {
  struct state *s = d;
  if (!strcmp(interface, "wl_shm"))
    s->shm = wl_registry_bind(registry, id, &wl_shm_interface, 1);
  else if (!strcmp(interface, "zwlr_screencopy_manager_v1")) {
    if (version < 3)
      die("screencopy v3 required");
    s->manager = wl_registry_bind(registry, id,
                                  &zwlr_screencopy_manager_v1_interface, 3);
  } else if (!strcmp(interface, "wl_output")) {
    if (s->count == 16 || version < 4)
      die("unsupported output registry");
    struct output *o = &s->outputs[s->count++];
    o->state = s;
    o->id = id;
    o->object = wl_registry_bind(registry, id, &wl_output_interface, 4);
    wl_output_add_listener(o->object, &output_listener, o);
  }
}
static void removed(void *d, struct wl_registry *r, uint32_t id) {
  struct state *s = d;
  for (unsigned i = 0; i < s->count; i++)
    if (s->outputs[i].id == id)
      s->failed = true;
}
static const struct wl_registry_listener registry_listener = {global, removed};
static void buffer(void *d, struct zwlr_screencopy_frame_v1 *frame,
                   uint32_t format, uint32_t width, uint32_t height,
                   uint32_t stride) {
  struct state *s = d;
  if (width != s->width || height != s->height || stride != width * 4)
    die("capture geometry drift");
  if (format != WL_SHM_FORMAT_ARGB8888 && format != WL_SHM_FORMAT_XRGB8888 &&
      format != WL_SHM_FORMAT_ABGR8888 && format != WL_SHM_FORMAT_XBGR8888)
    die("unsupported pixel format");
  if (s->buffer) {
    if (s->format != format)
      die("capture format drift");
  } else {
    s->format = format;
    s->stride = stride;
    size_t bytes = (size_t)stride * height;
    int fd = memfd_create("enzui-shell-capture", MFD_CLOEXEC);
    if (fd < 0 || ftruncate(fd, bytes) < 0)
      die("cannot allocate capture buffer");
    s->pixels = mmap(NULL, bytes, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (s->pixels == MAP_FAILED)
      die("cannot map capture buffer");
    s->pool = wl_shm_create_pool(s->shm, fd, bytes);
    close(fd);
    s->buffer =
        wl_shm_pool_create_buffer(s->pool, 0, width, height, stride, format);
  }
}
static void flags(void *d, struct zwlr_screencopy_frame_v1 *f, uint32_t flags) {
  ((struct state *)d)->flags = flags;
}
static void ready(void *d, struct zwlr_screencopy_frame_v1 *f, uint32_t hi,
                  uint32_t lo, uint32_t ns) {
  struct state *s = d;
  s->timestamp = (((uint64_t)hi << 32) | lo) * 1000000000ULL + ns;
  s->done = true;
}
static void failed(void *d, struct zwlr_screencopy_frame_v1 *f) {
  ((struct state *)d)->failed = true;
}
static void damage(void *d, struct zwlr_screencopy_frame_v1 *f, uint32_t x,
                   uint32_t y, uint32_t w, uint32_t h) {}
static void dmabuf(void *d, struct zwlr_screencopy_frame_v1 *f, uint32_t format,
                   uint32_t w, uint32_t h) {}
static void buffer_done(void *d, struct zwlr_screencopy_frame_v1 *f) {
  struct state *s = d;
  if (!s->buffer)
    die("no SHM capture buffer");
  zwlr_screencopy_frame_v1_copy(f, s->buffer);
}
static const struct zwlr_screencopy_frame_v1_listener frame_listener = {
    buffer, flags, ready, failed, damage, dmabuf, buffer_done};
static void write_all(const void *data, size_t length) {
  const char *p = data;
  while (length) {
    ssize_t n = write(STDOUT_FILENO, p, length);
    if (n < 0 && errno == EINTR)
      continue;
    if (n <= 0)
      die("capture consumer stopped");
    p += n;
    length -= n;
  }
}
int main(int argc, char **argv) {
  if (argc != 8)
    die("usage: record-shell-frames OUTPUT WIDTH HEIGHT FRAMES COMPOSITOR_PID "
        "START_TICKS disposable");
  if (strcmp(argv[7], "disposable") || strncmp(argv[1], "ENZUI-", 6) ||
      !getenv("WAYLAND_DISPLAY"))
    die("explicit disposable named-output target required");
  struct state s = {.name = argv[1]};
  uint64_t width = number(argv[2]), height = number(argv[3]),
           frames = number(argv[4]), pid = number(argv[5]),
           start = number(argv[6]);
  if (width < 240 || height < 240 || width > 4096 || height > 4096 ||
      width * height * 4 > 16 * 1024 * 1024 || frames < 1 || frames > 240 ||
      pid < 2 || pid > INT32_MAX)
    die("capture bounds rejected");
  s.width = width;
  s.height = height;
  if (process_start(pid) != start)
    die("stale capture process");
  s.display = wl_display_connect(NULL);
  if (!s.display)
    die("cannot connect to target");
  struct ucred peer;
  socklen_t length = sizeof(peer);
  if (getsockopt(wl_display_get_fd(s.display), SOL_SOCKET, SO_PEERCRED, &peer,
                 &length) ||
      peer.pid != (pid_t)pid || peer.uid != getuid() ||
      process_start(pid) != start)
    die("wrong capture endpoint");
  alarm(12);
  struct wl_registry *registry = wl_display_get_registry(s.display);
  wl_registry_add_listener(registry, &registry_listener, &s);
  if (wl_display_roundtrip(s.display) < 0 ||
      wl_display_roundtrip(s.display) < 0 || s.failed || !s.selected ||
      !s.shm || !s.manager || s.count != 1)
    die("one untransformed scale-1 named output required");
  printf("READY %u %u\n", s.width, s.height);
  fflush(stdout);
  for (uint64_t i = 0; i < frames; i++) {
    s.done = false;
    s.flags = 0;
    struct zwlr_screencopy_frame_v1 *frame =
        zwlr_screencopy_manager_v1_capture_output(s.manager, 0, s.selected);
    zwlr_screencopy_frame_v1_add_listener(frame, &frame_listener, &s);
    while (!s.done && !s.failed)
      if (wl_display_dispatch(s.display) < 0)
        die("target disconnected");
    if (s.failed)
      die("capture failed or output changed");
    write_all(&s.timestamp, 8);
    write_all(&s.format, 4);
    write_all(&s.flags, 4);
    write_all(s.pixels, (size_t)s.stride * s.height);
    zwlr_screencopy_frame_v1_destroy(frame);
  }
  munmap(s.pixels, (size_t)s.stride * s.height);
  wl_buffer_destroy(s.buffer);
  wl_shm_pool_destroy(s.pool);
  for (unsigned i = 0; i < s.count; i++)
    wl_output_release(s.outputs[i].object);
  zwlr_screencopy_manager_v1_destroy(s.manager);
  wl_shm_destroy(s.shm);
  wl_registry_destroy(registry);
  wl_display_disconnect(s.display);
  return 0;
}
