#!/usr/bin/env python3
"""Exercise the real desktop portal on a disposable session bus."""

import argparse
import json
import os
from pathlib import Path
import time
from xml.etree import ElementTree

from gi.repository import Gio, GLib

FRONTEND = "org.freedesktop.portal.Desktop"
BACKEND = "org.freedesktop.impl.portal.desktop.enzui"
PATH = "/org/freedesktop/portal/desktop"
PUBLIC = "org.freedesktop.portal."
IMPL = "org.freedesktop.impl.portal."
PROPS = "org.freedesktop.DBus.Properties"
LOGIN = "org.freedesktop.login1"
LOGIN_PATH = "/org/freedesktop/login1"
SAVER = "org.freedesktop.ScreenSaver"
APPEARANCE = "org.freedesktop.appearance"
INTERFACE = "org.gnome.desktop.interface"
VAR = GLib.Variant


def connect(address):
    return Gio.DBusConnection.new_for_address_sync(
        address, Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT
        | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION, None, None
    )


class Probe:
    def __init__(self):
        self.bus = connect(os.environ["DBUS_SESSION_BUS_ADDRESS"])
        self.context = GLib.MainContext.default()
        self.sender = self.bus.get_unique_name().lstrip(":").replace(".", "_")

    def call(self, interface, method, arguments=None, destination=FRONTEND, path=PATH):
        return self.bus.call_sync(destination, path, interface, method, arguments,
                                  None, Gio.DBusCallFlags.NONE, 5000, None)

    def signal(self, destination, interface, member, path=PATH):
        values = []
        subscription = self.bus.signal_subscribe(
            destination, interface, member, path, None, Gio.DBusSignalFlags.NONE,
            lambda _bus, _sender, _path, _interface, _member, args: values.append(args.unpack())
        )
        return values, subscription

    def until(self, condition, description, timeout=5):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            while self.context.pending():
                self.context.iteration(False)
            if result := condition():
                return result
            time.sleep(0.01)
        raise AssertionError(f"Timed out waiting for {description}")

    def request(self, interface, method, token, arguments):
        path = f"{PATH}/request/{self.sender}/{token}"
        responses, subscription = self.signal(FRONTEND, PUBLIC + "Request", "Response", path)
        assert self.call(PUBLIC + interface, method, arguments).unpack() == (path,)
        response, detail = self.until(lambda: responses and responses[0], method)
        self.bus.signal_unsubscribe(subscription)
        assert response == 0, (method, response, detail)
        return path, detail

    def interfaces(self, destination):
        xml, = self.call("org.freedesktop.DBus.Introspectable", "Introspect", destination=destination).unpack()
        return {entry.attrib["name"] for entry in ElementTree.fromstring(xml).findall("interface")}

    def settings(self, merged):
        shell_backends = ["FileChooser", "Access", "Settings", "Background", "Notification", "Inhibit", "Lockdown"]
        exported_portals = ["FileChooser", "Settings", "Background", "Notification", "Inhibit"]
        if merged:
            shell_backends += ["Print", "AppChooser", "DynamicLauncher", "Account", "Email"]
            exported_portals += ["Print", "OpenURI", "DynamicLauncher", "Account", "Email"]

        interfaces = self.interfaces(BACKEND)
        for name in shell_backends:
            assert IMPL + name in interfaces, (name, interfaces)
        interfaces = self.interfaces(FRONTEND)
        for name in exported_portals:
            assert PUBLIC + name in interfaces, (name, interfaces)

        try:
            self.call(IMPL + "Notification", "AddNotification",
                      VAR("(ssa{sv})", ("org.example.App", "forged", {})), BACKEND)
        except GLib.Error as error:
            assert "AccessDenied" in str(error), error
        else:
            raise AssertionError("A caller bypassed the public Notification portal")

        def read(namespace, key):
            return self.call(PUBLIC + "Settings", "ReadOne", VAR("(ss)", (namespace, key)))

        assert read(INTERFACE, "gtk-enable-primary-paste").unpack() == (True,)
        assert read(APPEARANCE, "accent-color").get_child_value(0).get_variant().get_type_string() == "(ddd)"
        legacy = self.call(PUBLIC + "Settings", "Read", VAR("(ss)", (APPEARANCE, "color-scheme")))
        assert legacy.get_child_value(0).get_variant().get_type_string() == "v"
        namespaces, = self.call(PUBLIC + "Settings", "ReadAll", VAR("(as)", (["org.gnome.desktop.a11y.*"],))).unpack()
        assert set(namespaces) == {"org.gnome.desktop.a11y.interface", "org.gnome.desktop.a11y.applications"}
        try:
            read(INTERFACE, "unknown-key")
        except GLib.Error as error:
            assert "NotFound" in str(error), error
        else:
            raise AssertionError("Unknown Settings key unexpectedly returned a value")

        changes, subscription = self.signal(FRONTEND, PUBLIC + "Settings", "SettingChanged")
        settings = Path(os.environ["ENZUI_SHELL_SETTINGS"])
        document = json.loads(settings.read_text()) if settings.exists() else {}
        new_size = 48 if read(INTERFACE, "cursor-size").unpack() != (48,) else 24
        document.setdefault("compositor", {})["cursor_size"] = new_size
        write_json(settings, document)
        self.until(lambda: (INTERFACE, "cursor-size", new_size) in changes, "Settings.SettingChanged")
        assert read(INTERFACE, "cursor-size").unpack() == (new_size,)
        theme = Path(os.environ["XDG_CONFIG_HOME"]) / "enzui/theme.toml"
        write_text(theme, 'version = 1\n[motion]\nreduced_motion = true\n[window]\nbuttons = "all"\n')
        self.until(lambda: (APPEARANCE, "reduced-motion", 1) in changes, "Appearance.SettingChanged")
        assert read(INTERFACE, "enable-animations").unpack() == (False,)
        assert read("org.gnome.desktop.wm.preferences", "button-layout").unpack() == (":minimize,maximize,close",)
        self.bus.signal_unsubscribe(subscription)

        lockdown = self.call(PROPS, "GetAll", VAR("(s)", (IMPL + "Lockdown",)), BACKEND).unpack()[0]
        assert len(lockdown) == 7 and lockdown["disable-location"] is True
        assert not any(value for key, value in lockdown.items() if key != "disable-location")
        changes, subscription = self.signal(BACKEND, PROPS, "PropertiesChanged")
        write_json(Path(os.environ["XDG_CONFIG_HOME"]) / "enzui-shell/location.json", {"mode": "approximate"})
        self.until(lambda: any(iface == IMPL + "Lockdown" and values == {"disable-location": False}
                               for iface, values, _ in changes), "Lockdown.PropertiesChanged")
        self.bus.signal_unsubscribe(subscription)
        print("Settings values, wire types, filters and changes passed")
        print("Lockdown properties and live location policy passed")

    def inhibit(self):
        def idle():
            return self.call(PROPS, "Get", VAR("(ss)", (SAVER, "Inhibited")), SAVER,
                             "/org/freedesktop/ScreenSaver").unpack()[0]

        token = "services_inhibit"
        path, _ = self.request("Inhibit", "Inhibit", token, VAR("(sua{sv})", ("", 15, {
            "handle_token": VAR("s", token), "reason": VAR("s", "Portal integration"),
        })))
        assert idle()
        self.login("AssertSleepLock", VAR("(b)", (True,)))
        try:
            self.call(IMPL + "Request", "Close", destination=BACKEND, path=path)
        except GLib.Error as error:
            assert "AccessDenied" in str(error), error
        else:
            raise AssertionError("A different caller closed the backend request")
        assert idle()
        self.call(PUBLIC + "Request", "Close", path=path)
        self.until(lambda: not idle(), "idle inhibitor release")
        self.login("AssertSleepLock", VAR("(b)", (False,)))

        changes, subscription = self.signal(FRONTEND, PUBLIC + "Inhibit", "StateChanged")
        token = "services_monitor"
        _, result = self.request("Inhibit", "CreateMonitor", token, VAR("(sa{sv})", ("", {
            "handle_token": VAR("s", token), "session_handle_token": VAR("s", token),
        })))
        session = result["session_handle"]
        self.until(lambda: (session, {"screensaver-active": False, "session-state": 1}) in changes,
                   "initial monitor state")
        self.login("SetLocked", VAR("(b)", (True,)))
        self.until(lambda: (session, {"screensaver-active": True, "session-state": 1}) in changes,
                   "locked monitor state")
        self.login("Prepare", VAR("(b)", (True,)))
        self.until(lambda: (session, {"screensaver-active": True, "session-state": 2}) in changes,
                   "query end")
        self.call(PUBLIC + "Inhibit", "QueryEndResponse", VAR("(o)", (session,)))
        self.until(lambda: (session, {"screensaver-active": True, "session-state": 3}) in changes,
                   "ending")
        self.login("Prepare", VAR("(b)", (False,)))
        self.until(lambda: changes[-1] == (session, {"screensaver-active": True, "session-state": 1}),
                   "running after cancelled shutdown")
        self.call(PUBLIC + "Session", "Close", path=session)
        self.bus.signal_unsubscribe(subscription)
        print("Inhibit request ownership, idle and sleep release, and session transitions passed")

    def login(self, method, args):
        return self.call(LOGIN + ".Manager", method, args, LOGIN, LOGIN_PATH)

    def notification(self, ready):
        self.call("org.freedesktop.host.portal.Registry", "Register",
                  VAR("(sa{sv})", ("org.enzui.PortalServicesQA", {})))
        assert self.call(PROPS, "Get", VAR("(ss)", (PUBLIC + "Notification", "version"))).unpack() == (2,)
        actions, _ = self.signal(FRONTEND, PUBLIC + "Notification", "ActionInvoked")

        def add(title):
            self.call(PUBLIC + "Notification", "AddNotification", VAR("(sa{sv})", ("portal-services-qa", {
                "title": VAR("s", title),
                "markup-body": VAR("s", "<b>Portal notification</b> in the Shell"),
                "priority": VAR("s", "urgent"),
                "icon": VAR("(sv)", ("themed", VAR("as", ["dialog-information"]))),
                "default-action": VAR("s", "default"),
                "buttons": VAR("aa{sv}", [{"label": VAR("s", "Confirm"), "action": VAR("s", "confirm"),
                                          "target": VAR("s", "portal-action-target")}]),
                "sound": VAR("s", "silent"),
            })))

        add("Portal integration check")
        ready.touch()
        id_, action, parameters = self.until(lambda: actions and actions[0], "notification action", 60)
        assert (id_, action, parameters[0]) == ("portal-services-qa", "confirm", "portal-action-target")
        assert parameters[-1].get("activation-token"), parameters
        add("Portal notification replacement")
        self.call(PUBLIC + "Notification", "RemoveNotification", VAR("(s)", ("portal-services-qa",)))
        print("Notification v2 action and target passed")


def write_json(path, value):
    write_text(path, json.dumps(value))


def write_text(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(".new")
    temporary.write_text(value)
    temporary.replace(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--notification-ready", type=Path)
    parser.add_argument("--merged", action="store_true")
    args = parser.parse_args()
    probe = Probe()
    if args.notification_ready:
        probe.notification(args.notification_ready)
    else:
        probe.settings(args.merged)
        probe.inhibit()


if __name__ == "__main__":
    main()
