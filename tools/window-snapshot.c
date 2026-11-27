#define _GNU_SOURCE
#include "hyprland-toplevel-export-v1-client-protocol.h"
#include <endian.h>
#include <errno.h>
#include <ctype.h>
#include <fcntl.h>
#include <limits.h>
#include <poll.h>
#include <png.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>

struct snapshot {
    struct wl_display *display;
    struct wl_registry *registry;
    struct wl_callback *sync;
    struct wl_shm *shm;
    struct hyprland_toplevel_export_manager_v1 *manager;
    struct hyprland_toplevel_export_frame_v1 *frame;
    struct wl_buffer *buffer;
    void *pixels;
    size_t bytes;
    uint32_t width, height, stride, format, flags;
    bool done, failed;
};

static int64_t now_ms(void) {
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    return (int64_t)now.tv_sec * 1000 + now.tv_nsec / 1000000;
}

static bool dispatch_until(struct snapshot *s, int64_t deadline) {
    while (!s->done && !s->failed) {
        while (wl_display_prepare_read(s->display) != 0)
            if (wl_display_dispatch_pending(s->display) < 0) return false;
        int flushed = wl_display_flush(s->display);
        if (flushed < 0 && errno != EAGAIN) { wl_display_cancel_read(s->display); return false; }
        int64_t remaining = deadline - now_ms();
        if (remaining <= 0) { wl_display_cancel_read(s->display); return false; }
        struct pollfd fd = { .fd = wl_display_get_fd(s->display), .events = POLLIN | (flushed < 0 ? POLLOUT : 0) };
        int result = poll(&fd, 1, (int)remaining);
        if (result > 0 && (fd.revents & POLLIN)) {
            if (wl_display_read_events(s->display) < 0) return false;
        } else {
            wl_display_cancel_read(s->display);
            if (result < 0 && errno == EINTR) continue;
            if (result <= 0 || (fd.revents & (POLLERR | POLLHUP | POLLNVAL))) return false;
        }
        if (wl_display_dispatch_pending(s->display) < 0) return false;
    }
    return s->done && !s->failed;
}

static void synced(void *data, struct wl_callback *callback, uint32_t serial) {
    (void)serial;
    struct snapshot *s = data;
    s->done = true;
    s->sync = NULL;
    wl_callback_destroy(callback);
}
static const struct wl_callback_listener sync_listener = { .done = synced };

static void global(void *data, struct wl_registry *registry, uint32_t name, const char *interface, uint32_t version) {
    struct snapshot *s = data;
    if (!strcmp(interface, wl_shm_interface.name)) s->shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
    else if (!strcmp(interface, hyprland_toplevel_export_manager_v1_interface.name) && version >= 1)
        s->manager = wl_registry_bind(registry, name, &hyprland_toplevel_export_manager_v1_interface, 1);
}
static void removed(void *data, struct wl_registry *registry, uint32_t name) { (void)data; (void)registry; (void)name; }
static const struct wl_registry_listener registry_listener = { .global = global, .global_remove = removed };

static void frame_buffer(void *data, struct hyprland_toplevel_export_frame_v1 *frame,
                         uint32_t format, uint32_t width, uint32_t height, uint32_t stride) {
    (void)frame;
    struct snapshot *s = data;
    if (!width || !height || width > 8192 || height > 8192 || stride < width * 4 ||
        (uint64_t)stride * height > 128 * 1024 * 1024 ||
        (format != WL_SHM_FORMAT_ARGB8888 && format != WL_SHM_FORMAT_XRGB8888)) { s->failed = true; return; }
    s->width = width; s->height = height; s->stride = stride; s->format = format;
}
static void damage(void *data, struct hyprland_toplevel_export_frame_v1 *frame, uint32_t x, uint32_t y, uint32_t w, uint32_t h) {
    (void)data; (void)frame; (void)x; (void)y; (void)w; (void)h;
}
static void flags(void *data, struct hyprland_toplevel_export_frame_v1 *frame, uint32_t flags_) {
    (void)frame; ((struct snapshot *)data)->flags = flags_;
}
static void ready(void *data, struct hyprland_toplevel_export_frame_v1 *frame, uint32_t hi, uint32_t lo, uint32_t ns) {
    (void)frame; (void)hi; (void)lo; (void)ns; ((struct snapshot *)data)->done = true;
}
static void failed(void *data, struct hyprland_toplevel_export_frame_v1 *frame) {
    (void)frame; ((struct snapshot *)data)->failed = true;
}
static void dmabuf(void *data, struct hyprland_toplevel_export_frame_v1 *frame, uint32_t format, uint32_t width, uint32_t height) {
    (void)data; (void)frame; (void)format; (void)width; (void)height;
}
static void buffer_done(void *data, struct hyprland_toplevel_export_frame_v1 *frame) {
    struct snapshot *s = data;
    if (!s->width || s->failed || s->pixels || s->buffer) { s->failed = true; return; }
    s->bytes = (size_t)s->stride * s->height;
    int fd = memfd_create("enzui-window-snapshot", MFD_CLOEXEC);
    if (fd < 0) { s->failed = true; return; }
    if (ftruncate(fd, (off_t)s->bytes) < 0) { close(fd); s->failed = true; return; }
    s->pixels = mmap(NULL, s->bytes, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (s->pixels == MAP_FAILED) { s->pixels = NULL; close(fd); s->failed = true; return; }
    struct wl_shm_pool *pool = wl_shm_create_pool(s->shm, fd, (int32_t)s->bytes);
    close(fd);
    if (!pool) { s->failed = true; return; }
    s->buffer = wl_shm_pool_create_buffer(pool, 0, (int32_t)s->width, (int32_t)s->height, (int32_t)s->stride, s->format);
    wl_shm_pool_destroy(pool);
    if (!s->buffer) { s->failed = true; return; }
    hyprland_toplevel_export_frame_v1_copy(frame, s->buffer, 1);
}
static const struct hyprland_toplevel_export_frame_v1_listener frame_listener = {
    .buffer = frame_buffer, .damage = damage, .flags = flags, .ready = ready, .failed = failed,
    .linux_dmabuf = dmabuf, .buffer_done = buffer_done,
};

// The size ENZUI_SNAPSHOT_FIT ("WxH") asks the picture to fit inside, never
// enlarged; without it, the window's own size.
static void fitted(const struct snapshot *s, uint32_t *width, uint32_t *height) {
    *width = s->width; *height = s->height;
    const char *fit = getenv("ENZUI_SNAPSHOT_FIT");
    unsigned long w = 0, h = 0;
    char *end;
    if (!fit) return;
    errno = 0; w = strtoul(fit, &end, 10);
    if (errno || *end != 'x' || !w || w > 8192) return;
    h = strtoul(end + 1, &end, 10);
    if (errno || *end || !h || h > 8192) return;
    double scale = (double)w / s->width;
    if ((double)h / s->height < scale) scale = (double)h / s->height;
    if (scale >= 1.0) return;
    *width = (uint32_t)(s->width * scale + 0.5); *height = (uint32_t)(s->height * scale + 0.5);
    if (!*width) *width = 1;
    if (!*height) *height = 1;
}

static bool write_picture(const struct snapshot *s, const char *path, bool raw) {
    uint32_t width, height;
    fitted(s, &width, &height);
    size_t bytes = (size_t)width * height * 4;
    uint8_t *pixels = malloc(bytes);
    if (!pixels) return false;
    // Each picture pixel averages the window pixels it covers, still
    // premultiplied as the compositor wrote them.
    for (uint32_t y = 0; y < height && (width != s->width || height != s->height); ++y) {
        uint32_t y0 = (uint32_t)((uint64_t)y * s->height / height);
        uint32_t y1 = (uint32_t)((uint64_t)(y + 1) * s->height / height);
        if (y1 <= y0) y1 = y0 + 1;
        for (uint32_t x = 0; x < width; ++x) {
            uint32_t x0 = (uint32_t)((uint64_t)x * s->width / width);
            uint32_t x1 = (uint32_t)((uint64_t)(x + 1) * s->width / width);
            if (x1 <= x0) x1 = x0 + 1;
            uint64_t sum[4] = {0, 0, 0, 0};
            for (uint32_t sy = y0; sy < y1; ++sy) {
                uint32_t source_y = (s->flags & 1) ? s->height - 1 - sy : sy;
                const uint8_t *row = (const uint8_t *)s->pixels + (size_t)source_y * s->stride;
                for (uint32_t sx = x0; sx < x1; ++sx) {
                    uint32_t value; memcpy(&value, row + sx * 4, sizeof(value));
                    sum[0] += (value >> 16) & 255; sum[1] += (value >> 8) & 255; sum[2] += value & 255;
                    sum[3] += s->format == WL_SHM_FORMAT_XRGB8888 ? 255 : value >> 24;
                }
            }
            uint64_t count = (uint64_t)(x1 - x0) * (y1 - y0);
            uint8_t *p = pixels + ((size_t)y * width + x) * 4;
            for (unsigned i = 0; i < 4; ++i) p[i] = (uint8_t)((sum[i] + count / 2) / count);
            if (p[3] && p[3] != 255) for (unsigned i = 0; i < 3; ++i) {
                unsigned value_ = ((unsigned)p[i] * 255 + p[3] / 2) / p[3];
                p[i] = (uint8_t)(value_ > 255 ? 255 : value_);
            }
        }
    }
    // At the window's own size each picture pixel is one window pixel.
    for (uint32_t y = 0; y < height && width == s->width && height == s->height; ++y) {
        uint32_t source_y = (s->flags & 1) ? s->height - 1 - y : y;
        const uint8_t *row = (const uint8_t *)s->pixels + (size_t)source_y * s->stride;
        for (uint32_t x = 0; x < width; ++x) {
            uint32_t value; memcpy(&value, row + x * 4, sizeof(value));
            uint8_t *p = pixels + ((size_t)y * width + x) * 4;
            p[0] = (uint8_t)(value >> 16); p[1] = (uint8_t)(value >> 8); p[2] = (uint8_t)value;
            p[3] = s->format == WL_SHM_FORMAT_XRGB8888 ? 255 : (uint8_t)(value >> 24);
            if (p[3] && p[3] != 255) for (unsigned i = 0; i < 3; ++i) {
                unsigned value_ = ((unsigned)p[i] * 255 + p[3] / 2) / p[3];
                p[i] = (uint8_t)(value_ > 255 ? 255 : value_);
            }
        }
    }
    int fd = open(path, O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW, 0600);
    FILE *file = fd < 0 ? NULL : fdopen(fd, "wb");
    bool ok = false;
    if (file && raw) {
        uint32_t size[] = { htole32(width), htole32(height) };
        ok = fwrite(size, sizeof(size), 1, file) == 1 && fwrite(pixels, bytes, 1, file) == 1;
    } else if (file) {
        png_image image = { .version = PNG_IMAGE_VERSION, .width = width, .height = height,
                            .format = PNG_FORMAT_RGBA, .flags = PNG_IMAGE_FLAG_FAST };
        ok = png_image_write_to_stdio(&image, file, 0, pixels, 0, NULL) != 0;
        png_image_free(&image);
    }
    if (file) { if (fclose(file) != 0) ok = false; }
    else if (fd >= 0) close(fd);
    if (!ok && fd >= 0) unlink(path);
    free(pixels);
    return ok;
}

int main(int argc, char **argv) {
    if (argc != 3) { fprintf(stderr, "usage: window-snapshot WINDOW_ADDRESS OUTPUT.png\n"); return 2; }
    const char *hex = !strncmp(argv[1], "0x", 2) ? argv[1] + 2 : argv[1];
    if (!*hex) return 2;
    for (const char *p = hex; *p; ++p) if (!isxdigit((unsigned char)*p)) return 2;
    char *end; errno = 0; unsigned long long address = strtoull(hex, &end, 16);
    if (errno || *end || !address) return 2;
    struct snapshot s = {0};
    bool ok = false;
    s.display = wl_display_connect(NULL);
    if (!s.display) goto cleanup;
    s.registry = wl_display_get_registry(s.display);
    wl_registry_add_listener(s.registry, &registry_listener, &s);
    s.sync = wl_display_sync(s.display);
    wl_callback_add_listener(s.sync, &sync_listener, &s);
    if (!dispatch_until(&s, now_ms() + 2000) || !s.shm || !s.manager) goto cleanup;
    s.done = false;
    s.frame = hyprland_toplevel_export_manager_v1_capture_toplevel(s.manager, 0, (uint32_t)address);
    hyprland_toplevel_export_frame_v1_add_listener(s.frame, &frame_listener, &s);
    if (!dispatch_until(&s, now_ms() + 3000)) goto cleanup;
    const char *raw = getenv("ENZUI_SNAPSHOT_RGBA");
    ok = write_picture(&s, argv[2], raw && !strcmp(raw, "1"));
    if (ok) printf("%u %u\n", s.width, s.height);
cleanup:
    if (s.sync) wl_callback_destroy(s.sync);
    if (s.frame) hyprland_toplevel_export_frame_v1_destroy(s.frame);
    if (s.buffer) wl_buffer_destroy(s.buffer);
    if (s.pixels) munmap(s.pixels, s.bytes);
    if (s.manager) hyprland_toplevel_export_manager_v1_destroy(s.manager);
    if (s.shm) wl_shm_destroy(s.shm);
    if (s.registry) wl_registry_destroy(s.registry);
    if (s.display) wl_display_disconnect(s.display);
    if (!ok) fprintf(stderr, "window-snapshot: capture or picture write failed\n");
    return ok ? 0 : 1;
}
