// Passive protocol admission checks for an owned disposable compositor.
#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <sys/un.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>
#include "ext-session-lock-v1-client-protocol.h"

static struct wl_display *display;
static struct ext_session_lock_manager_v1 *lock_manager;
static int capture, input, locked, refused;

static void found(void *data, struct wl_registry *registry, uint32_t name, const char *interface, uint32_t version) {
    (void)data; (void)version;
    const char *captures[]={"zwlr_screencopy_manager_v1","hyprland_toplevel_export_manager_v1","ext_image_copy_capture_manager_v1",
                            "ext_output_image_capture_source_manager_v1","ext_foreign_toplevel_image_capture_source_manager_v1"};
    for(size_t i=0;i<sizeof(captures)/sizeof(captures[0]);i++) if(!strcmp(interface,captures[i])) capture=1;
    if (!strcmp(interface, "zwp_virtual_keyboard_manager_v1") || !strcmp(interface,"zwlr_virtual_pointer_manager_v1")) input = 1;
    if (!strcmp(interface, "ext_session_lock_manager_v1"))
        lock_manager = wl_registry_bind(registry, name, &ext_session_lock_manager_v1_interface, 1);
}
static void removed(void *data, struct wl_registry *registry, uint32_t name) { (void)data; (void)registry; (void)name; }
static const struct wl_registry_listener registry_listener = {.global=found, .global_remove=removed};
static void got_lock(void *data, struct ext_session_lock_v1 *lock) { (void)data; (void)lock; locked=1; }
static void denied_lock(void *data, struct ext_session_lock_v1 *lock) { (void)data; (void)lock; refused=1; }
static const struct ext_session_lock_v1_listener lock_listener = {.locked=got_lock, .finished=denied_lock};

static int open_control(void) {
    const char *runtime = getenv("XDG_RUNTIME_DIR"), *instance = getenv("HYPRLAND_INSTANCE_SIGNATURE");
    if (!runtime || !instance) return -1;
    struct sockaddr_un address = {.sun_family=AF_UNIX};
    int size = snprintf(address.sun_path, sizeof(address.sun_path), "%s/hypr/%s/.socket.sock", runtime, instance);
    if (size<0 || (size_t)size>=sizeof(address.sun_path)) return -1;
    int fd=socket(AF_UNIX, SOCK_STREAM|SOCK_CLOEXEC, 0);
    if (fd<0) return -1;
    struct timeval deadline={.tv_sec=3};
    if (setsockopt(fd,SOL_SOCKET,SO_RCVTIMEO,&deadline,sizeof(deadline)) || setsockopt(fd,SOL_SOCKET,SO_SNDTIMEO,&deadline,sizeof(deadline)) ||
        connect(fd,(struct sockaddr*)&address,sizeof(address))) { close(fd); return -1; }
    return fd;
}
static int control_fd(int fd, const char *body) {
    if(fd<0 || strlen(body)>2048) { if(fd>=0) close(fd);return -1; }
    char request[2100], reply[512];
    int size=snprintf(request,sizeof(request),"/enzuiadmit %s",body);
    if (size<0 || (size_t)size>=sizeof(request) || write(fd,request,(size_t)size)!=size) { close(fd); return -1; }
    ssize_t count=read(fd,reply,sizeof(reply)-1);close(fd);
    if(count<=0) return -1;
    reply[count]=0;printf("CONTROL %s\n",reply);
    return !strcmp(reply,"ok") ? 0 : 1;
}

static int control(const char *body) { return control_fd(open_control(),body); }

static int hold(void) {
    for (;;) {
        if (wl_display_flush(display)<0 && errno!=EAGAIN) return 1;
        struct pollfd fds[]={{.fd=wl_display_get_fd(display),.events=POLLIN},{.fd=STDIN_FILENO,.events=POLLIN}};
        if(poll(fds,2,-1)<0) { if(errno==EINTR) continue;return 1; }
        if(fds[0].revents && wl_display_dispatch(display)<0) { puts("DISCONNECTED");return 0; }
        if(fds[1].revents) { char command[16];if(read(STDIN_FILENO,command,sizeof(command))<=0 || command[0]=='q') return 0; }
    }
}

int main(int argc, char **argv) {
    if(getuid()<1000 || !getenv("ENZUI_ADMISSION_QA") || strcmp(getenv("ENZUI_ADMISSION_QA"),"owned-disposable-compositor")) return 2;
    setvbuf(stdout,NULL,_IOLBF,0);alarm(45);
    const char *mode=argc>1?argv[1]:"report";
    display=wl_display_connect(NULL);if(!display) return 3;
    struct wl_registry *registry=wl_display_get_registry(display);
    wl_registry_add_listener(registry,&registry_listener,NULL);
    if(wl_display_roundtrip(display)<0) return 4;
    printf("REPORT pid=%ld capture=%d input=%d\n",(long)getpid(),capture,input);
    int status=0;
    if(!strcmp(mode,"hold")) status=hold();
    else if(!strcmp(mode,"inherited") || (!strcmp(mode,"inherited-control") && argc==3)) {
        int inherited_control=!strcmp(mode,"inherited-control")?open_control():-1;
        if(!strcmp(mode,"inherited-control") && inherited_control<0) return 5;
        pid_t child=fork();if(child<0) return 5;
        if(child>0) { printf("INHERITED_CHILD pid=%ld\n",(long)child);_exit(0); }
        struct timespec delay={.tv_nsec=200000000};nanosleep(&delay,NULL);
        if(inherited_control>=0) {
            int denied=control_fd(inherited_control,argv[2])==1;
            printf("DEAD_CONTROL_REFUSED %d\n",denied);status=denied?0:1;
        } else {
            int disconnected=wl_display_roundtrip(display)<0;
            printf("DEAD_PEER_DISCONNECTED %d\n",disconnected);status=disconnected?0:1;
        }
    } else if(!strcmp(mode,"control") && argc==3) status=control(argv[2]);
    else if(!strcmp(mode,"intent") && argc==6) {
        char body[2100];int ttl=atoi(argv[3]), delay=atoi(argv[4]), count=atoi(argv[5]);
        if(strlen(argv[2])>1900 || ttl<100 || ttl>10000 || delay<0 || delay>1000 || count<1 || count>2) return 2;
        snprintf(body,sizeof(body),"intent capture %s %d",argv[2],ttl);
        status=control(body);
        if(!status) {
            struct timespec wait={.tv_sec=delay/1000,.tv_nsec=(delay%1000)*1000000L};nanosleep(&wait,NULL);
            for(int i=0;i<count;i++) {
                pid_t child=fork();if(child<0) return 5;
                if(!child) { execl(argv[2],argv[2],"report",NULL);_exit(127); }
                int result=0;if(waitpid(child,&result,0)<0 || !WIFEXITED(result) || WEXITSTATUS(result)) { status=1;break; }
            }
        }
    } else if(!strcmp(mode,"lock")) {
        if(!lock_manager) return 6;
        struct ext_session_lock_v1 *lock=ext_session_lock_manager_v1_lock(lock_manager);
        ext_session_lock_v1_add_listener(lock,&lock_listener,NULL);
        while(!locked && !refused) if(wl_display_dispatch(display)<0) return 7;
        if(refused) { ext_session_lock_v1_destroy(lock);return 8; }
        puts("LOCKED_PROTOCOL_ONLY");
        char command[16];if(read(STDIN_FILENO,command,sizeof(command))<=0 || command[0]!='u') return 9;
        ext_session_lock_v1_unlock_and_destroy(lock);
        if(wl_display_roundtrip(display)<0) return 10;
        puts("UNLOCKED");
    } else if(strcmp(mode,"report")) return 2;
    if(lock_manager) ext_session_lock_manager_v1_destroy(lock_manager);
    wl_registry_destroy(registry);wl_display_disconnect(display);return status<0?1:status;
}
