#include <wayland-client.h>
#include <wayland-client-protocol.h>
#include "wlr-virtual-pointer-client.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static struct zwlr_virtual_pointer_manager_v1 *manager;
static void global(void *data, struct wl_registry *registry, uint32_t id, const char *name, uint32_t version) {
    (void)data;
    if (!strcmp(name, zwlr_virtual_pointer_manager_v1_interface.name))
        manager = wl_registry_bind(registry, id, &zwlr_virtual_pointer_manager_v1_interface, version < 2 ? version : 2);
}
static void gone(void *data, struct wl_registry *registry, uint32_t id) {
    (void)data; (void)registry; (void)id;
}
int main(void) {
    struct wl_display *display = wl_display_connect(NULL);
    if (!display) return 1;
    struct wl_registry *registry = wl_display_get_registry(display);
    struct wl_registry_listener listener = {global, gone};
    wl_registry_add_listener(registry, &listener, NULL);
    wl_display_roundtrip(display);
    if (!manager) { fputs("nested compositor does not expose virtual pointer\n", stderr); return 2; }
    struct zwlr_virtual_pointer_v1 *pointer = zwlr_virtual_pointer_manager_v1_create_virtual_pointer(manager, NULL);
    if (!pointer) return 3;
    char command[128];
    while (fgets(command, sizeof(command), stdin)) {
        int x, y, button;
        if (sscanf(command, "m %d %d", &x, &y) == 2 && x >= 0 && y >= 0 && x < 1440 && y < 900)
            zwlr_virtual_pointer_v1_motion_absolute(pointer, 0, x, y, 1440, 900);
        else if (sscanf(command, "b %d", &button) == 1 && (button == 0 || button == 1))
            zwlr_virtual_pointer_v1_button(pointer, 0, 272, button ? WL_POINTER_BUTTON_STATE_PRESSED : WL_POINTER_BUTTON_STATE_RELEASED);
        else { fputs("invalid QA pointer command\n", stderr); return 4; }
        zwlr_virtual_pointer_v1_frame(pointer);
        if (wl_display_roundtrip(display) < 0) return 5;
        puts("ok"); fflush(stdout);
    }
    zwlr_virtual_pointer_v1_destroy(pointer);
    zwlr_virtual_pointer_manager_v1_destroy(manager);
    wl_registry_destroy(registry);
    wl_display_disconnect(display);
    return 0;
}
