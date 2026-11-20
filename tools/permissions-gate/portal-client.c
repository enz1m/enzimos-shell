// Public sandbox fixture for real portal-grant and background-lifetime checks.
#define GLIB_VERSION_MIN_REQUIRED GLIB_VERSION_2_66
#define GLIB_VERSION_MAX_ALLOWED GLIB_VERSION_2_66
#include <gio/gio.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

static GMainLoop *loop;
static GVariant *response;
static gboolean timed_out;
static const char *destination = "org.freedesktop.portal.Desktop";
static const char *object = "/org/freedesktop/portal/desktop";

static gboolean timeout(gpointer unused) {
    (void)unused;
    timed_out = TRUE;
    g_main_loop_quit(loop);
    return G_SOURCE_REMOVE;
}

static void replied(GDBusConnection *connection, const gchar *sender,
                    const gchar *path, const gchar *interface,
                    const gchar *signal, GVariant *parameters, gpointer unused) {
    (void)connection; (void)sender; (void)path; (void)interface;
    (void)signal; (void)unused;
    response = g_variant_ref(parameters);
    g_main_loop_quit(loop);
}

static GVariant *call(GDBusConnection *bus, const char *interface,
                       const char *method, GVariant *args) {
    GError *error = NULL;
    GVariant *result = g_dbus_connection_call_sync(bus, destination, object,
        interface, method, args, NULL, G_DBUS_CALL_FLAGS_NONE, 10000, NULL, &error);
    if (!result) {
        fprintf(stderr, "%s: %s\n", method, error ? error->message : "failed");
        g_clear_error(&error);
    }
    return result;
}

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IOLBF, 0);
    GKeyFile *info = g_key_file_new();
    if (!g_key_file_load_from_file(info, "/.flatpak-info", G_KEY_FILE_NONE, NULL)) return 2;
    char *id = g_key_file_get_string(info, "Application", "name", NULL);
    gboolean owned = g_strcmp0(id, "org.enzui.PermissionsQA") == 0;
    g_free(id); g_key_file_unref(info);
    if (!owned || argc < 2) return 2;
    GError *error = NULL;
    GDBusConnection *bus = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, &error);
    if (!bus) { g_clear_error(&error); return 1; }
    loop = g_main_loop_new(NULL, FALSE);
    int status = 0;
    if (strcmp(argv[1], "notify") == 0) {
        const char *note = argc > 2 ? argv[2] : "permissionsqa";
        if (!*note || strlen(note) > 32 || strspn(note, "abcdefghijklmnopqrstuvwxyz0123456789-_") != strlen(note)) return 2;
        GVariantBuilder options;
        g_variant_builder_init(&options, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&options, "{sv}", "title", g_variant_new_string("enzUI Permissions QA"));
        g_variant_builder_add(&options, "{sv}", "body", g_variant_new_string(note));
        GVariant *result = call(bus, "org.freedesktop.portal.Notification", "AddNotification",
            g_variant_new("(sa{sv})", note, &options));
        if (!result) status = 1;
        else { g_variant_unref(result); puts("NOTIFICATION_REQUEST_RETURNED"); }
    } else if (strcmp(argv[1], "background") == 0) {
        char *sender = g_strdup(g_dbus_connection_get_unique_name(bus) + 1);
        for (char *p = sender; *p; ++p) if (*p == '.') *p = '_';
        char token[32]; snprintf(token, sizeof(token), "permissionqa%u", (unsigned)getpid());
        char *path = g_strdup_printf("/org/freedesktop/portal/desktop/request/%s/%s", sender, token);
        guint subscription = g_dbus_connection_signal_subscribe(bus, destination,
            "org.freedesktop.portal.Request", "Response", path, NULL,
            G_DBUS_SIGNAL_FLAGS_NONE, replied, NULL, NULL);
        GVariantBuilder options;
        g_variant_builder_init(&options, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&options, "{sv}", "handle_token", g_variant_new_string(token));
        g_variant_builder_add(&options, "{sv}", "reason", g_variant_new_string("Owned public sandbox permission test"));
        g_variant_builder_add(&options, "{sv}", "autostart", g_variant_new_boolean(FALSE));
        GVariant *result = call(bus, "org.freedesktop.portal.Background", "RequestBackground",
            g_variant_new("(sa{sv})", "", &options));
        if (!result) status = 1;
        else {
            g_variant_unref(result);
            guint deadline = g_timeout_add_seconds(90, timeout, NULL);
            if (!response) g_main_loop_run(loop);
            if (timed_out || !response) status = 1;
            else {
                g_source_remove(deadline);
                guint code; gboolean allowed = FALSE; GVariant *values;
                g_variant_get(response, "(u@a{sv})", &code, &values);
                g_variant_lookup(values, "background", "b", &allowed);
                printf("BACKGROUND_RESPONSE=%u ALLOWED=%u\n", code, allowed);
                g_variant_unref(values); g_variant_unref(response);
                if (code || !allowed) status = 3;
                else {
                    puts("READY_FOR_BACKGROUND_QA");
                    g_timeout_add_seconds(120, timeout, NULL);
                    g_main_loop_run(loop);
                }
            }
        }
        g_dbus_connection_signal_unsubscribe(bus, subscription);
        g_free(path); g_free(sender);
    } else status = 2;
    g_main_loop_unref(loop); g_object_unref(bus);
    return status;
}
