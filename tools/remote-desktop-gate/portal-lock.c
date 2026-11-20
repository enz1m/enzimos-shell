// Exercise approved RemoteDesktop capture/EIS revocation without injecting input.
// Build with gio-unix-2.0 and libei-1.0; run as the graphical session's user.
#include <gio/gio.h>
#include <gio/gunixfdlist.h>
#include <libei.h>
#include <poll.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

static GDBusConnection *bus;
static GMainLoop *loop;
static GVariant *response;
static guint sequence;
static gboolean timed_out;
static const char *destination = "org.freedesktop.portal.Desktop";
static const char *object = "/org/freedesktop/portal/desktop";
static const char *remote = "org.freedesktop.portal.RemoteDesktop";
static const char *cast = "org.freedesktop.portal.ScreenCast";

static void fail(const char *operation, GError *error) {
    fprintf(stderr, "%s: %s\n", operation, error ? error->message : "invalid response");
    exit(1);
}

static gboolean timeout(gpointer unused) {
    (void)unused;
    timed_out = TRUE;
    g_main_loop_quit(loop);
    return G_SOURCE_REMOVE;
}

static void responded(GDBusConnection *connection, const gchar *sender,
                      const gchar *path, const gchar *interface,
                      const gchar *signal, GVariant *parameters, gpointer unused) {
    (void)connection; (void)sender; (void)path; (void)interface;
    (void)signal; (void)unused;
    response = g_variant_ref(parameters);
    g_main_loop_quit(loop);
}

static GVariant *request(const char *interface, const char *method,
                         const char *session, guint types) {
    char token[32];
    snprintf(token, sizeof(token), "lockprobe%u", ++sequence);
    char *sender = g_strdup(g_dbus_connection_get_unique_name(bus) + 1);
    for (char *p = sender; *p; ++p) if (*p == '.') *p = '_';
    char *path = g_strdup_printf("/org/freedesktop/portal/desktop/request/%s/%s", sender, token);
    guint subscription = g_dbus_connection_signal_subscribe(bus, destination,
        "org.freedesktop.portal.Request", "Response", path, NULL,
        G_DBUS_SIGNAL_FLAGS_NONE, responded, NULL, NULL);
    GVariantBuilder options;
    g_variant_builder_init(&options, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&options, "{sv}", "handle_token", g_variant_new_string(token));
    if (!session)
        g_variant_builder_add(&options, "{sv}", "session_handle_token", g_variant_new_string("locksession"));
    if (types)
        g_variant_builder_add(&options, "{sv}", "types", g_variant_new_uint32(types));
    GVariant *args = !session ? g_variant_new("(a{sv})", &options) :
        types ? g_variant_new("(oa{sv})", session, &options) :
                g_variant_new("(osa{sv})", session, "", &options);
    GError *error = NULL;
    GVariant *reply = g_dbus_connection_call_sync(bus, destination, object,
        interface, method, args, NULL, G_DBUS_CALL_FLAGS_NONE, 10000, NULL, &error);
    if (!reply) fail(method, error);
    g_variant_unref(reply);
    guint deadline = g_timeout_add_seconds(90, timeout, NULL);
    if (!response) g_main_loop_run(loop);
    if (timed_out) fail("consent/request timeout", NULL);
    g_source_remove(deadline);
    g_dbus_connection_signal_unsubscribe(bus, subscription);
    guint code;
    GVariant *results;
    g_variant_get(response, "(u@a{sv})", &code, &results);
    g_variant_unref(response);
    response = NULL;
    g_free(sender);
    g_free(path);
    printf("%s response=%u\n", method, code);
    if (code) fail(method, NULL);
    return results;
}

int main(void) {
    setvbuf(stdout, NULL, _IOLBF, 0);
    GError *error = NULL;
    bus = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, &error);
    if (!bus) fail("session bus", error);
    loop = g_main_loop_new(NULL, FALSE);
    GVariant *results = request(remote, "CreateSession", NULL, 0);
    char *session = NULL;
    if (!g_variant_lookup(results, "session_handle", "s", &session) ||
        !g_variant_is_object_path(session)) fail("session handle", NULL);
    g_variant_unref(results);
    g_variant_unref(request(remote, "SelectDevices", session, 3));
    g_variant_unref(request(cast, "SelectSources", session, 1));
    g_variant_unref(request(remote, "Start", session, 0));
    GUnixFDList *fds = NULL;
    GVariant *reply = g_dbus_connection_call_with_unix_fd_list_sync(bus,
        destination, object, remote, "ConnectToEIS",
        g_variant_new("(oa{sv})", session, NULL), G_VARIANT_TYPE("(h)"),
        G_DBUS_CALL_FLAGS_NONE, 10000, NULL, &fds, NULL, &error);
    if (!reply) fail("ConnectToEIS", error);
    gint index;
    g_variant_get(reply, "(h)", &index);
    int fd = g_unix_fd_list_get(fds, index, &error);
    if (fd < 0) fail("EIS descriptor", error);
    g_variant_unref(reply);
    g_object_unref(fds);
    struct ei *ei = ei_new_sender(NULL);
    ei_configure_name(ei, "enzui-portal-lock-probe");
    if (ei_setup_backend_fd(ei, fd)) fail("EIS setup", NULL);
    gint64 end = g_get_monotonic_time() + 90 * G_TIME_SPAN_SECOND;
    gboolean connected = FALSE, ready = FALSE, revoked = FALSE;
    while (g_get_monotonic_time() < end && !revoked) {
        struct pollfd pollfd = {.fd = ei_get_fd(ei), .events = POLLIN};
        if (poll(&pollfd, 1, 250) < 0) fail("EIS poll", NULL);
        ei_dispatch(ei);
        struct ei_event *event;
        while ((event = ei_get_event(ei))) {
            enum ei_event_type type = ei_event_get_type(event);
            if (type == EI_EVENT_CONNECT) connected = TRUE;
            if (type == EI_EVENT_SEAT_ADDED)
                ei_seat_bind_capabilities(ei_event_get_seat(event),
                    EI_DEVICE_CAP_POINTER, EI_DEVICE_CAP_POINTER_ABSOLUTE,
                    EI_DEVICE_CAP_BUTTON, EI_DEVICE_CAP_SCROLL, EI_DEVICE_CAP_KEYBOARD, NULL);
            if (type == EI_EVENT_DEVICE_RESUMED && connected && !ready) {
                ready = TRUE;
                puts("READY_FOR_LOCK: approved capture and EIS device, no input sent");
            }
            if (type == EI_EVENT_DISCONNECT) revoked = TRUE;
            ei_event_unref(event);
        }
    }
    ei_unref(ei);
    g_free(session);
    g_main_loop_unref(loop);
    g_object_unref(bus);
    if (!ready || !revoked) fail("expected ready device followed by lock revocation", NULL);
    puts("PASS: approved EIS connection revoked");
    return 0;
}
