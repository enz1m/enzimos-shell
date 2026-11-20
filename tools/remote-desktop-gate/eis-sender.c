// Injects a bounded test sequence through an explicitly supplied ConnectToEIS descriptor.
#include <libei.h>
#include <linux/input-event-codes.h>
#include <poll.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

static struct ei_device *rel, *abs_, *kbd;
static int resumed;

static void pump(struct ei *ei, int ms) {
    struct pollfd p = {.fd = ei_get_fd(ei), .events = POLLIN};
    struct timespec start; clock_gettime(CLOCK_MONOTONIC, &start);
    for (;;) {
        struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t);
        int left = ms - (int)((t.tv_sec - start.tv_sec) * 1000 + (t.tv_nsec - start.tv_nsec) / 1000000);
        if (left <= 0) return;
        if (poll(&p, 1, left) <= 0) continue;
        ei_dispatch(ei);
        struct ei_event *e;
        while ((e = ei_get_event(ei))) {
            struct ei_device *d = ei_event_get_device(e);
            switch (ei_event_get_type(e)) {
            case EI_EVENT_CONNECT: printf("connected\n"); break;
            case EI_EVENT_DISCONNECT: printf("disconnected\n"); fflush(stdout); exit(3);
            case EI_EVENT_SEAT_ADDED:
                printf("seat %s\n", ei_seat_get_name(ei_event_get_seat(e)));
                ei_seat_bind_capabilities(ei_event_get_seat(e), EI_DEVICE_CAP_POINTER, EI_DEVICE_CAP_POINTER_ABSOLUTE, EI_DEVICE_CAP_BUTTON,
                                          EI_DEVICE_CAP_SCROLL, EI_DEVICE_CAP_KEYBOARD, EI_DEVICE_CAP_TOUCH, NULL);
                break;
            case EI_EVENT_DEVICE_ADDED:
                printf("device \"%s\" pointer=%d absolute=%d button=%d scroll=%d keyboard=%d touch=%d\n", ei_device_get_name(d),
                       ei_device_has_capability(d, EI_DEVICE_CAP_POINTER), ei_device_has_capability(d, EI_DEVICE_CAP_POINTER_ABSOLUTE),
                       ei_device_has_capability(d, EI_DEVICE_CAP_BUTTON), ei_device_has_capability(d, EI_DEVICE_CAP_SCROLL),
                       ei_device_has_capability(d, EI_DEVICE_CAP_KEYBOARD), ei_device_has_capability(d, EI_DEVICE_CAP_TOUCH));
                if (ei_device_has_capability(d, EI_DEVICE_CAP_POINTER)) rel = ei_device_ref(d);
                if (ei_device_has_capability(d, EI_DEVICE_CAP_POINTER_ABSOLUTE)) {
                    abs_ = ei_device_ref(d);
                    struct ei_region *r = ei_device_get_region(d, 0);
                    if (r) printf("  region %ux%u at %u,%u\n", ei_region_get_width(r), ei_region_get_height(r), ei_region_get_x(r), ei_region_get_y(r));
                }
                if (ei_device_has_capability(d, EI_DEVICE_CAP_KEYBOARD)) {
                    kbd = ei_device_ref(d);
                    printf("  keymap %s\n", ei_device_keyboard_get_keymap(d) ? "yes" : "no");
                }
                break;
            case EI_EVENT_DEVICE_RESUMED: resumed++; break;
            case EI_EVENT_DEVICE_REMOVED: printf("device removed\n"); break;
            default: break;
            }
            ei_event_unref(e);
        }
    }
}

static void frame(struct ei *ei, struct ei_device *d) { ei_device_frame(d, ei_now(ei)); }

int main(int argc, char **argv) {
    struct ei *ei = ei_new_sender(NULL);
    ei_configure_name(ei, "enzui-eis-sender");
    if (argc < 2 || ei_setup_backend_fd(ei, atoi(argv[1])) != 0) { printf("setup failed\n"); return 1; }
    pump(ei, 1500);
    printf("devices resumed %d\n", resumed);
    fflush(stdout);
    if (rel) {
        ei_device_start_emulating(rel, 1);
        ei_device_pointer_motion(rel, 200, 100); frame(ei, rel);
        ei_device_stop_emulating(rel);
        pump(ei, 300); printf("moved relatively\n"); fflush(stdout);
        pump(ei, 2500);
    }
    if (abs_) {
        ei_device_start_emulating(abs_, 2);
        ei_device_pointer_motion_absolute(abs_, 1000, 600); frame(ei, abs_);
        ei_device_stop_emulating(abs_);
        pump(ei, 300); printf("moved absolutely\n"); fflush(stdout);
        pump(ei, 2500);
    }
    if (rel) {
        ei_device_start_emulating(rel, 3);
        ei_device_button_button(rel, BTN_LEFT, true); frame(ei, rel);
        ei_device_button_button(rel, BTN_LEFT, false); frame(ei, rel);
        ei_device_stop_emulating(rel);
        pump(ei, 300); printf("clicked\n"); fflush(stdout);
    }
    if (kbd) {
        ei_device_start_emulating(kbd, 4);
        int keys[] = {KEY_E, KEY_I, KEY_S};
        for (int i = 0; i < 3; i++) {
            ei_device_keyboard_key(kbd, keys[i], true); frame(ei, kbd);
            ei_device_keyboard_key(kbd, keys[i], false); frame(ei, kbd);
        }
        ei_device_stop_emulating(kbd);
        pump(ei, 300); printf("typed eis\n"); fflush(stdout);
    }
    pump(ei, (argc > 2 ? atoi(argv[2]) : 2) * 1000);
    printf("done\n");
    ei_unref(ei);
    return 0;
}
