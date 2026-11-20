// Public background-app protocol fixture.
#define GLIB_VERSION_MIN_REQUIRED GLIB_VERSION_2_66
#define GLIB_VERSION_MAX_ALLOWED GLIB_VERSION_2_66
#include <gio/gio.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

static GMainLoop *loop;
static gboolean finishing;
static const char *app = "org.enzui.BackgroundQA";
static const char *service = "org.enzimos.Background1";
static const char *xml =
    "<node><interface name='org.enzimos.ApplicationLifecycle1'>"
    "<method name='QueryStop'><arg type='s' direction='out'/>"
    "<arg type='a(sss)' direction='out'/></method>"
    "<method name='Stop'><arg type='s' direction='in'/><arg type='s' direction='in'/>"
    "<arg type='s' direction='out'/></method></interface></node>";

static gboolean quit(gpointer unused) {
    (void)unused;
    g_main_loop_quit(loop);
    return G_SOURCE_REMOVE;
}

static gboolean allowed(GDBusConnection *bus, const char *sender) {
    GVariant *reply = g_dbus_connection_call_sync(bus, "org.freedesktop.DBus",
        "/org/freedesktop/DBus", "org.freedesktop.DBus", "GetNameOwner",
        g_variant_new("(s)", service), G_VARIANT_TYPE("(s)"), G_DBUS_CALL_FLAGS_NONE,
        2000, NULL, NULL);
    if (!reply) return FALSE;
    const char *owner;
    g_variant_get(reply, "(&s)", &owner);
    gboolean accepted = g_strcmp0(owner, sender) == 0;
    g_variant_unref(reply);
    return accepted;
}

static void method(GDBusConnection *bus, const char *sender, const char *path,
                    const char *interface, const char *name, GVariant *parameters,
                    GDBusMethodInvocation *invocation, gpointer unused) {
    (void)path; (void)interface; (void)unused;
    if (!allowed(bus, sender)) {
        g_dbus_method_invocation_return_dbus_error(invocation,
            "org.freedesktop.DBus.Error.AccessDenied", "Only the current background service may control this public fixture");
        return;
    }
    const char *revision = finishing ? "public-finishing" : "public-running";
    if (strcmp(name, "QueryStop") == 0) {
        GVariantBuilder rows;
        g_variant_builder_init(&rows, G_VARIANT_TYPE("a(sss)"));
        g_variant_builder_add(&rows, "(sss)", "public-timer", "Public timer fixture",
            finishing ? "Finishing; no user files are involved" : "Finish waits five seconds; cancel exits promptly");
        g_dbus_method_invocation_return_value(invocation, g_variant_new("(sa(sss))", revision, &rows));
    } else if (strcmp(name, "Stop") == 0) {
        const char *choice, *reviewed;
        g_variant_get(parameters, "(&s&s)", &choice, &reviewed);
        const char *result = "refused";
        if (strcmp(reviewed, revision) != 0) result = "stale-revision";
        else if (!finishing && (strcmp(choice, "finish-then-exit") == 0 || strcmp(choice, "cancel-and-exit") == 0)) {
            finishing = TRUE;
            result = "accepted";
            g_timeout_add(strcmp(choice, "finish-then-exit") == 0 ? 5000 : 500, quit, NULL);
            printf("PUBLIC_STOP_ACCEPTED=%s\n", choice);
        }
        g_dbus_method_invocation_return_value(invocation, g_variant_new("(s)", result));
        g_dbus_connection_flush_sync(bus, NULL, NULL);
    } else {
        g_dbus_method_invocation_return_dbus_error(invocation,
            "org.freedesktop.DBus.Error.UnknownMethod", "Unknown public fixture method");
    }
}

static void acquired(GDBusConnection *bus, const char *name, gpointer unused) {
    (void)bus; (void)name; (void)unused;
    puts("BACKGROUND_QA_READY");
}

static void lost(GDBusConnection *bus, const char *name, gpointer unused) {
    (void)bus; (void)name; (void)unused;
    g_main_loop_quit(loop);
}

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IOLBF, 0);
    if (geteuid() < 1000 || g_strcmp0(g_getenv("ENZUI_BACKGROUND_QA"), "owned-disposable-guest") != 0) return 2;
    if (g_file_test("/.flatpak-info", G_FILE_TEST_EXISTS)) return 2;
    if (argc == 2 && strcmp(argv[1], "--idle-control") == 0) {
        loop = g_main_loop_new(NULL, FALSE);
        g_timeout_add_seconds(1200, quit, NULL);
        g_main_loop_run(loop);
        g_main_loop_unref(loop);
        return 0;
    }
    if (argc != 1) return 2;
    GError *error = NULL;
    GDBusConnection *bus = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, &error);
    if (!bus) { g_clear_error(&error); return 1; }
    GDBusNodeInfo *info = g_dbus_node_info_new_for_xml(xml, &error);
    if (!info) { g_clear_error(&error); g_object_unref(bus); return 1; }
    static const GDBusInterfaceVTable table = { .method_call = method };
    guint object = g_dbus_connection_register_object(bus, "/org/enzui/BackgroundQA", info->interfaces[0], &table, NULL, NULL, &error);
    if (!object) { g_clear_error(&error); g_dbus_node_info_unref(info); g_object_unref(bus); return 1; }
    loop = g_main_loop_new(NULL, FALSE);
    guint owner = g_bus_own_name_on_connection(bus, app, G_BUS_NAME_OWNER_FLAGS_DO_NOT_QUEUE,
        acquired, lost, NULL, NULL);
    g_timeout_add_seconds(1200, quit, NULL);
    g_main_loop_run(loop);
    g_bus_unown_name(owner);
    g_dbus_connection_unregister_object(bus, object);
    g_dbus_connection_flush_sync(bus, NULL, NULL);
    g_main_loop_unref(loop); g_dbus_node_info_unref(info); g_object_unref(bus);
    return 0;
}
