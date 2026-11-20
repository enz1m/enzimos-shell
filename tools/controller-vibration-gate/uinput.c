// Public virtual controllers; never opens another input device.
#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include <fcntl.h>
#include <linux/input.h>
#include <linux/uinput.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <time.h>
#include <unistd.h>

static volatile sig_atomic_t stopping, releasing;
static struct uinput_ff_upload held;
static int held_fd = -1;
static void release(int signum) { (void)signum; releasing = 1; }
static void stop(int signum) { (void)signum; stopping = 1; }
static int create(const char *name, unsigned product) {
    int fd = open("/dev/uinput", O_RDWR | O_NONBLOCK | O_CLOEXEC);
    if (fd < 0) return -1;
    if (ioctl(fd, UI_SET_EVBIT, EV_KEY) || ioctl(fd, UI_SET_KEYBIT, BTN_SOUTH) || ioctl(fd, UI_SET_KEYBIT, BTN_EAST)
        || ioctl(fd, UI_SET_EVBIT, EV_ABS) || ioctl(fd, UI_SET_ABSBIT, ABS_X) || ioctl(fd, UI_SET_ABSBIT, ABS_Y)
        || ioctl(fd, UI_SET_EVBIT, EV_FF) || ioctl(fd, UI_SET_FFBIT, FF_RUMBLE)) goto fail;
    for (unsigned code = ABS_X; code <= ABS_Y; ++code) {
        struct uinput_abs_setup axis = { .code = code, .absinfo = { .minimum = -127, .maximum = 127 } };
        if (ioctl(fd, UI_ABS_SETUP, &axis)) goto fail;
    }
    struct uinput_setup setup = { .id = { .bustype = BUS_USB, .vendor = 0x1209, .product = product, .version = 1 }, .ff_effects_max = 8 };
    snprintf(setup.name, sizeof(setup.name), "%s", name);
    if (ioctl(fd, UI_DEV_SETUP, &setup) || ioctl(fd, UI_DEV_CREATE)) goto fail;
    return fd;
fail:
    close(fd); return -1;
}
static void initial(int fd) {
    struct input_event events[] = { {.type=EV_ABS,.code=ABS_X,.value=0}, {.type=EV_ABS,.code=ABS_Y,.value=0}, {.type=EV_SYN,.code=SYN_REPORT,.value=0} };
    if (write(fd, events, sizeof(events)) != sizeof(events)) stopping = 1;
}
static int handle(int fd, const char *role, int hold, int reject) {
    struct input_event event;
    ssize_t count = read(fd, &event, sizeof(event));
    if (count < 0 && (errno == EAGAIN || errno == EINTR)) return 0;
    if (count != sizeof(event)) return -1;
    if (event.type == EV_UINPUT && event.code == UI_FF_UPLOAD) {
        struct uinput_ff_upload upload = {.request_id = event.value};
        if (ioctl(fd, UI_BEGIN_FF_UPLOAD, &upload)) return -1;
        printf("QA_UPLOAD role=%s id=%d type=%u length=%u strong=%u weak=%u\n", role, upload.effect.id, upload.effect.type,
            upload.effect.replay.length, upload.effect.u.rumble.strong_magnitude, upload.effect.u.rumble.weak_magnitude);
        if (strcmp(role, "control") == 0) return -1;
        upload.retval = reject ? -ENOSPC : 0;
        if (upload.effect.id < 0 || upload.effect.id >= 8 || upload.effect.type != FF_RUMBLE || upload.effect.replay.length > 200 || upload.effect.replay.delay
            || upload.effect.u.rumble.strong_magnitude > 0x3000 || upload.effect.u.rumble.weak_magnitude > 0x3000) upload.retval = -EINVAL;
        if (hold && upload.retval != -EINVAL) { held = upload; held_fd = fd; puts("QA_UPLOAD_HELD"); return 0; }
        if (ioctl(fd, UI_END_FF_UPLOAD, &upload)) return -1;
    } else if (event.type == EV_UINPUT && event.code == UI_FF_ERASE) {
        struct uinput_ff_erase erase = {.request_id = event.value};
        if (ioctl(fd, UI_BEGIN_FF_ERASE, &erase)) return -1;
        printf("QA_ERASE role=%s id=%u\n", role, erase.effect_id);
        erase.retval = 0;
        if (ioctl(fd, UI_END_FF_ERASE, &erase)) return -1;
    } else if (event.type == EV_FF) {
        if (event.code >= 8 || (event.value != 0 && event.value != 1)) return -1;
        printf("QA_%s role=%s id=%u count=%d\n", event.value ? "PLAY" : "STOP", role, event.code, event.value);
        if (strcmp(role, "control") == 0) return -1;
    }
    return 0;
}
int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IOLBF, 0);
    if (geteuid() < 1000 || !getenv("ENZUI_CONTROLLER_QA") || strcmp(getenv("ENZUI_CONTROLLER_QA"), "owned-disposable-guest")) return 2;
    int hold = argc == 2 && !strcmp(argv[1], "--hold-upload");
    int reject = argc == 2 && !strcmp(argv[1], "--reject-upload");
    if (argc != 1 && !hold && !reject) return 2;
    struct sigaction stopping_action = {.sa_handler = stop}, release_action = {.sa_handler = release};
    sigemptyset(&stopping_action.sa_mask); sigemptyset(&release_action.sa_mask);
    if (sigaction(SIGTERM, &stopping_action, NULL) || sigaction(SIGINT, &stopping_action, NULL)
        || sigaction(SIGUSR1, &release_action, NULL)) return 2;
    int target = create("enzUI Vibration QA", 0xe881), control = create("enzUI Vibration Control QA", 0xe882);
    if (target < 0 || control < 0) { if (target >= 0) close(target); if (control >= 0) close(control); return 1; }
    char name[80] = {0};
    if (ioctl(target, UI_GET_SYSNAME(sizeof(name)), name) < 0) return 1;
    printf("QA_DEVICE role=target sysname=%s\n", name);
    if (ioctl(control, UI_GET_SYSNAME(sizeof(name)), name) < 0) return 1;
    printf("QA_DEVICE role=control sysname=%s\n", name);
    initial(target); initial(control);
    struct timespec now; clock_gettime(CLOCK_MONOTONIC, &now); time_t deadline = now.tv_sec + 300;
    int status = 0;
    while (!stopping) {
        if (releasing && held_fd >= 0) {
            held.retval = 0;
            printf("QA_UPLOAD_RELEASE result=%d\n", ioctl(held_fd, UI_END_FF_UPLOAD, &held));
            held_fd = -1; releasing = 0;
        }
        clock_gettime(CLOCK_MONOTONIC, &now); if (now.tv_sec >= deadline) break;
        struct pollfd fds[] = {{.fd=target,.events=POLLIN}, {.fd=control,.events=POLLIN}};
        if (poll(fds, 2, 50) < 0 && errno != EINTR) { status=1; break; }
        if (fds[0].revents & POLLIN && handle(target, "target", hold, reject)) { status=1; break; }
        if (fds[1].revents & POLLIN && handle(control, "control", 0, 0)) { status=1; break; }
    }
    ioctl(target, UI_DEV_DESTROY); ioctl(control, UI_DEV_DESTROY); close(target); close(control);
    puts("QA_DEVICES_DESTROYED");
    return status;
}
