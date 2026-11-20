// Compositor-glass fixture; pattern and ticker modes provide opaque test backdrops.
#define _GNU_SOURCE
#include <errno.h>
#include <math.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>
#include <wayland-client.h>
#include "enzui-material-v1-client-protocol.h"
#include "xdg-shell-client-protocol.h"

struct tokens {
    float blur_sigma, refraction, bevel_width, index_of_refraction, dispersion, color_diffusion, color_reach, optical_depth, tint_opacity, saturation,
        highlight, brightness, light_angle, profile_curvature, contrast_protection, protected_inset;
};

static struct tokens base(void) {
    return (struct tokens){0.0f, 0.69f, 40.0f, 1.5f, 0.05f, 0.0f, 24.0f, 40.0f, 0.0f, 1.0f, 0.05f, 0.0f, 90.0f, 2.0f, 0.0f, 12.0f};
}
static struct tokens style(const char* name) {
    struct tokens t = base();
    struct tokens adaptive = t;
    adaptive.contrast_protection = 0.85f;
    if (!strcmp(name, "regular")) {
        t = adaptive;
    } else if (!strcmp(name, "clear")) {
        t.dispersion = 0.0f; t.highlight = 0.0f; t.contrast_protection = 0.85f;
    } else if (!strcmp(name, "lens")) {
        t = adaptive; t.blur_sigma = 3.3f; t.refraction = 1.2f; t.bevel_width = 60.0f; t.dispersion = 0.2f; t.brightness = -0.2f;
    } else if (!strcmp(name, "subtle")) {
        t = adaptive; t.blur_sigma = 3.3f; t.brightness = -0.1f;
    } else if (!strcmp(name, "frosted")) {
        t.blur_sigma = 5.5f; t.brightness = -0.3f; t.contrast_protection = 1.0f;
    } else if (!strcmp(name, "optical")) {
        t.blur_sigma = 0.0f; t.refraction = 0.8f; t.bevel_width = 32.0f; t.dispersion = 0.0f; t.color_diffusion = 0.0f; t.color_reach = 32.0f;
        t.tint_opacity = 0.0f; t.saturation = 1.0f; t.highlight = 0.0f; t.brightness = 0.0f; t.contrast_protection = 0.0f;
    }
    return t;
}

struct box { float x, y, w, h, r; };

static const int WIDTH = 520, HEIGHT = 640;
static const struct box SIDEBAR = {16, 16, 220, 608, 22};
static const struct box TOOLBAR = {252, 16, 252, 52, 26};
static const struct box BUTTON_A = {252, 572, 52, 52, 26};
static const struct box BUTTON_B = {316, 572, 52, 52, 26};

static struct wl_compositor* compositor;
static struct wl_shm* shm;
static struct xdg_wm_base* wm;
static struct enzui_material_manager_v1* manager;
static bool configured, closed;

static void registry_global(void* data, struct wl_registry* registry, uint32_t name, const char* interface, uint32_t version) {
    (void)data;
    if (!strcmp(interface, wl_compositor_interface.name)) compositor = wl_registry_bind(registry, name, &wl_compositor_interface, 4);
    else if (!strcmp(interface, wl_shm_interface.name)) shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
    else if (!strcmp(interface, xdg_wm_base_interface.name)) wm = wl_registry_bind(registry, name, &xdg_wm_base_interface, 1);
    else if (!strcmp(interface, enzui_material_manager_v1_interface.name)) manager = wl_registry_bind(registry, name, &enzui_material_manager_v1_interface, 1);
    (void)version;
}
static void registry_remove(void* data, struct wl_registry* registry, uint32_t name) { (void)data; (void)registry; (void)name; }
static const struct wl_registry_listener registry_listener = {registry_global, registry_remove};

static void ping(void* data, struct xdg_wm_base* base, uint32_t serial) { (void)data; xdg_wm_base_pong(base, serial); }
static const struct xdg_wm_base_listener wm_listener = {ping};
static void surface_configure(void* data, struct xdg_surface* surface, uint32_t serial) { (void)data; xdg_surface_ack_configure(surface, serial); configured = true; }
static const struct xdg_surface_listener surface_listener = {surface_configure};
static void top_configure(void* d, struct xdg_toplevel* t, int32_t w, int32_t h, struct wl_array* s) { (void)d; (void)t; (void)w; (void)h; (void)s; }
static void top_close(void* d, struct xdg_toplevel* t) { (void)d; (void)t; closed = true; }
static void top_bounds(void* d, struct xdg_toplevel* t, int32_t w, int32_t h) { (void)d; (void)t; (void)w; (void)h; }
static void top_capabilities(void* d, struct xdg_toplevel* t, struct wl_array* c) { (void)d; (void)t; (void)c; }
static const struct xdg_toplevel_listener top_listener = {top_configure, top_close, top_bounds, top_capabilities};

static float coverage(struct box b, float px, float py) {
    float cx = b.x + b.w / 2, cy = b.y + b.h / 2;
    float r = fminf(b.r, fminf(b.w, b.h) / 2);
    float qx = fabsf(px - cx) - b.w / 2 + r, qy = fabsf(py - cy) - b.h / 2 + r;
    float ox = fmaxf(qx, 0), oy = fmaxf(qy, 0);
    float d = sqrtf(ox * ox + oy * oy) + fminf(fmaxf(qx, qy), 0) - r;
    return fminf(fmaxf(0.5f - d, 0), 1);
}

static float ink_level = 1.0f;

static void ink(uint32_t* p, float a) {
    if (a <= 0) return;
    uint32_t old = *p;
    float oa = (old >> 24) / 255.0f, oc = (old & 0xff) / 255.0f;
    float na = a + oa * (1 - a), nc = a * ink_level + oc * (1 - a);
    uint32_t c = (uint32_t)(nc * 255 + 0.5f), A = (uint32_t)(na * 255 + 0.5f);
    *p = (A << 24) | (c << 16) | (c << 8) | c;
}

static void draw_pattern(uint32_t* pixels, int shift) {
    for (int y = 0; y < HEIGHT; y++)
        for (int x = 0; x < WIDTH; x++) {
            float fx = x / (float)WIDTH, fy = y / (float)HEIGHT;
            uint32_t r = (uint32_t)(40 + 200 * fx), g = (uint32_t)(60 + 150 * (1 - fy)), b = 170;
            if (((x / 14) + (y / 14)) % 2 == 0) { r = r * 3 / 4; g = g * 3 / 4; }
            if (((x + shift) % 48) < 3) { r = g = b = 250; }
            float dx = x - 360.0f, dy = y - 200.0f;
            if (dx * dx + dy * dy < 90 * 90) { r = 250; g = 200; b = 40; }
            if (y > 420 && y < 460) { r = 230; g = 60; b = 60; }
            pixels[y * WIDTH + x] = 0xff000000u | (r << 16) | (g << 8) | b;
        }
}

static void draw(uint32_t* pixels, bool light) {
    memset(pixels, 0, (size_t)WIDTH * HEIGHT * 4);
    float alpha = 0.92f;
    ink_level = light ? 0.08f : 1.0f;
    for (int y = 0; y < HEIGHT; y++)
        for (int x = 0; x < WIDTH; x++) {
            float px = x + 0.5f, py = y + 0.5f;
            uint32_t* p = &pixels[y * WIDTH + x];
            for (int row = 0; row < 9; row++) {
                float top = 48 + row * 56;
                struct box dot = {40, top, 20, 20, 10}, label = {72, top + 5, (row % 3 == 0) ? 120 : 92, 10, 5};
                float selected = row == 1 ? 1.0f : 0.0f;
                if (selected > 0) {
                    struct box pill = {28, top - 12, 196, 44, 14};
                    ink(p, coverage(pill, px, py) * 0.14f);
                }
                ink(p, coverage(dot, px, py) * alpha);
                ink(p, coverage(label, px, py) * alpha * (row == 1 ? 1.0f : 0.72f));
            }
            for (int i = 0; i < 3; i++) {
                struct box icon = {272 + i * 40, 32, 20, 20, 6};
                ink(p, coverage(icon, px, py) * alpha);
            }
            struct box title = {400, 37, 84, 10, 5};
            ink(p, coverage(title, px, py) * alpha * 0.8f);
            float ax = px - (BUTTON_A.x + 26), ay = py - (BUTTON_A.y + 26);
            if (ax > -7 && ax < 9 && fabsf(ay) < (9 - ax) * 0.55f) ink(p, alpha);
            float bx = px - (BUTTON_B.x + 26), by = py - (BUTTON_B.y + 26);
            if ((fabsf(bx) < 9 && fabsf(by) < 2) || (fabsf(by) < 9 && fabsf(bx) < 2)) ink(p, alpha);
        }
}

static void put(float* out, struct box b, struct tokens t, bool light) {
    float tint[4] = {0x3A / 255.0f, 0x37 / 255.0f, 0x3E / 255.0f, 1.0f};
    if (light) { tint[0] = 0xF9 / 255.0f; tint[1] = 0xFA / 255.0f; tint[2] = 0xFC / 255.0f; }
    float head[5] = {b.x, b.y, b.w, b.h, b.r};
    memcpy(out, head, sizeof head);
    memcpy(out + 5, &t, sizeof t);
    memcpy(out + 21, tint, sizeof tint);
}

int main(int argc, char** argv) {
    const char* name = argc > 1 ? argv[1] : "regular";
    bool ticker = !strcmp(name, "ticker");
    bool pattern = ticker || !strcmp(name, "pattern");
    bool light = argc > 2 && !strcmp(argv[2], "light");
    struct wl_display* display = wl_display_connect(NULL);
    if (!display) { fprintf(stderr, "no Wayland display\n"); return 1; }
    struct wl_registry* registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, NULL);
    wl_display_roundtrip(display);
    if (!compositor || !shm || !wm) { fprintf(stderr, "missing core globals\n"); return 1; }
    if (!manager) fprintf(stderr, "the compositor offers no enzui_material_manager_v1: no glass\n");
    xdg_wm_base_add_listener(wm, &wm_listener, NULL);

    struct wl_surface* surface = wl_compositor_create_surface(compositor);
    struct xdg_surface* xdg = xdg_wm_base_get_xdg_surface(wm, surface);
    xdg_surface_add_listener(xdg, &surface_listener, NULL);
    struct xdg_toplevel* top = xdg_surface_get_toplevel(xdg);
    xdg_toplevel_add_listener(top, &top_listener, NULL);
    xdg_toplevel_set_title(top, pattern ? "Glass backdrop" : "Glass demo");
    xdg_toplevel_set_app_id(top, pattern ? "dev.enzui.GlassBackdrop" : "dev.enzui.GlassDemo");
    wl_surface_commit(surface);
    while (!configured && wl_display_dispatch(display) != -1) {}

    // Keep the transparent no-glass case as the negative control.
    if (manager && !pattern && strcmp(name, "none")) {
        struct enzui_surface_material_v1* material = enzui_material_manager_v1_get_surface_material(manager, surface);
        struct tokens t = style(name);
        float regions[4 * 25];
        put(regions + 0, SIDEBAR, t, light);
        put(regions + 25, TOOLBAR, t, light);
        put(regions + 50, BUTTON_A, t, light);
        put(regions + 75, BUTTON_B, t, light);
        struct wl_array array = {.size = sizeof regions, .alloc = sizeof regions, .data = regions};
        enzui_surface_material_v1_set_regions(material, &array);
        enzui_surface_material_v1_set_shadow(material, wl_fixed_from_double(17), wl_fixed_from_double(4), wl_fixed_from_double(0.45));
    }

    int stride = WIDTH * 4, size = stride * HEIGHT;
    int fd = memfd_create("glass-demo", MFD_CLOEXEC);
    if (fd < 0 || ftruncate(fd, size * 2) < 0) { perror("memfd"); return 1; }
    uint32_t* pixels = mmap(NULL, size * 2, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    struct wl_shm_pool* pool = wl_shm_create_pool(shm, fd, size * 2);
    struct wl_buffer* buffers[2] = {
        wl_shm_pool_create_buffer(pool, 0, WIDTH, HEIGHT, stride, WL_SHM_FORMAT_ARGB8888),
        wl_shm_pool_create_buffer(pool, size, WIDTH, HEIGHT, stride, WL_SHM_FORMAT_ARGB8888),
    };
    if (pattern) draw_pattern(pixels, 0); else draw(pixels, light);
    wl_surface_attach(surface, buffers[0], 0, 0);
    wl_surface_damage_buffer(surface, 0, 0, WIDTH, HEIGHT);
    wl_surface_commit(surface);
    for (int frame = 1; !closed; frame++) {
        if (!ticker) {
            if (wl_display_dispatch(display) == -1) break;
            continue;
        }
        usleep(50000);
        if (wl_display_dispatch_pending(display) == -1) break;
        uint32_t* target = pixels + (frame % 2) * (WIDTH * HEIGHT);
        draw_pattern(target, frame * 3);
        wl_surface_attach(surface, buffers[frame % 2], 0, 0);
        wl_surface_damage_buffer(surface, 0, 0, WIDTH, HEIGHT);
        wl_surface_commit(surface);
        if (wl_display_flush(display) < 0) break;
    }
    return 0;
}
