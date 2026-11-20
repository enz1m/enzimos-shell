#define main glass_demo_main
#include "glass-demo.c"
#undef main
#include <poll.h>
#include <time.h>

static int received;
static void luminance(void* data, struct enzui_surface_material_v1* material, uint32_t region, wl_fixed_t value) {
    (void)data;
    (void)material;
    received++;
    printf("LUMINANCE region=%u value=%f\n", region, wl_fixed_to_double(value));
    fflush(stdout);
}
static const struct enzui_surface_material_v1_listener material_listener = {.luminance = luminance};
static double milliseconds(void) {
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    return now.tv_sec * 1000.0 + now.tv_nsec / 1e6;
}
static void pump(struct wl_display* display, int duration) {
    const double end = milliseconds() + duration;
    while (milliseconds() < end) {
        while (wl_display_prepare_read(display) != 0)
            wl_display_dispatch_pending(display);
        wl_display_flush(display);
        struct pollfd fd = {.fd = wl_display_get_fd(display), .events = POLLIN};
        if (poll(&fd, 1, 10) > 0) {
            if (wl_display_read_events(display) < 0) return;
            wl_display_dispatch_pending(display);
        } else {
            wl_display_cancel_read(display);
        }
    }
}
static struct enzui_surface_material_v1* regions(struct wl_surface* surface) {
    struct enzui_surface_material_v1* material = enzui_material_manager_v1_get_surface_material(manager, surface);
    enzui_surface_material_v1_add_listener(material, &material_listener, NULL);
    struct tokens tokens = style("regular");
    float record[25];
    put(record, SIDEBAR, tokens, false);
    struct wl_array array = {.size = sizeof record, .alloc = sizeof record, .data = record};
    enzui_surface_material_v1_set_regions(material, &array);
    enzui_surface_material_v1_set_measure_only(material, 1);
    return material;
}
static void version_global(void* data, struct wl_registry* reg, uint32_t name, const char* interface, uint32_t version) {
    (void)data;
    if (!strcmp(interface, enzui_material_manager_v1_interface.name) && version >= 2)
        manager = wl_registry_bind(reg, name, &enzui_material_manager_v1_interface, 2);
}

int main(int argc, char** argv) {
    struct wl_display* display = wl_display_connect(NULL);
    if (!display) return 1;
    struct wl_registry* registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, NULL);
    wl_display_roundtrip(display);
    if (manager) enzui_material_manager_v1_destroy(manager);
    manager = NULL;
    struct wl_registry* version_registry = wl_display_get_registry(display);
    const struct wl_registry_listener version_listener = {.global = version_global, .global_remove = registry_remove};
    wl_registry_add_listener(version_registry, &version_listener, NULL);
    wl_display_roundtrip(display);
    if (!manager || !compositor || !shm || !wm) return 2;
    xdg_wm_base_add_listener(wm, &wm_listener, NULL);
    struct wl_surface* surface = wl_compositor_create_surface(compositor);
    struct xdg_surface* xdg = xdg_wm_base_get_xdg_surface(wm, surface);
    xdg_surface_add_listener(xdg, &surface_listener, NULL);
    struct xdg_toplevel* top = xdg_surface_get_toplevel(xdg);
    xdg_toplevel_add_listener(top, &top_listener, NULL);
    xdg_toplevel_set_app_id(top, "dev.enzui.ReadbackProbe");
    xdg_toplevel_set_title(top, "Readback probe");
    wl_surface_commit(surface);
    while (!configured && wl_display_dispatch(display) != -1) {}
    int stride = WIDTH * 4, bytes = stride * HEIGHT;
    int fd = memfd_create("readback-probe", MFD_CLOEXEC);
    if (fd < 0 || ftruncate(fd, bytes) < 0) return 3;
    uint32_t* pixels = mmap(NULL, bytes, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    draw(pixels, false);
    struct wl_shm_pool* pool = wl_shm_create_pool(shm, fd, bytes);
    struct wl_buffer* buffer = wl_shm_pool_create_buffer(pool, 0, WIDTH, HEIGHT, stride, WL_SHM_FORMAT_ARGB8888);
    struct enzui_surface_material_v1* material = regions(surface);
    wl_surface_attach(surface, buffer, 0, 0);
    wl_surface_damage_buffer(surface, 0, 0, WIDTH, HEIGHT);
    wl_surface_commit(surface);
    pump(display, 150);
    if (argc > 1 && !strcmp(argv[1], "replace")) {
        float record[25];
        put(record, TOOLBAR, style("regular"), false);
        struct wl_array array = {.size = sizeof record, .alloc = sizeof record, .data = record};
        enzui_surface_material_v1_set_regions(material, &array);
        wl_surface_damage_buffer(surface, 0, 0, WIDTH, HEIGHT);
        wl_surface_commit(surface);
    } else if (argc > 1 && !strcmp(argv[1], "destroy")) {
        enzui_surface_material_v1_destroy(material);
        material = regions(surface);
        wl_surface_damage_buffer(surface, 0, 0, WIDTH, HEIGHT);
        wl_surface_commit(surface);
    } else if (argc > 1 && !strcmp(argv[1], "resize")) {
        wl_surface_set_buffer_scale(surface, 2);
        wl_surface_attach(surface, buffer, 0, 0);
        wl_surface_damage_buffer(surface, 0, 0, WIDTH, HEIGHT);
        wl_surface_commit(surface);
    }
    pump(display, 900);
    printf("TOTAL %d\n", received);
    fflush(stdout);
    pump(display, 6000);
    wl_display_disconnect(display);
    return 0;
}
