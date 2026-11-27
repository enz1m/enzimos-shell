#!/usr/bin/env python3
"""Private logind fixture. Pipe EOF observes the lifetime of each inhibitor."""

import os
import select
from gi.repository import Gio, GLib

LOGIN = "org.freedesktop.login1"
PATH = "/org/freedesktop/login1"
SESSION = PATH + "/session/qa"
MANAGER = LOGIN + ".Manager"
SESSION_INTERFACE = LOGIN + ".Session"
SAVER = "org.freedesktop.ScreenSaver"
SAVER_PATH = "/org/freedesktop/ScreenSaver"
DBUS = "org.freedesktop.DBus"
VAR = GLib.Variant

node = Gio.DBusNodeInfo.new_for_xml(f"""
<node>
 <interface name="{MANAGER}">
  <method name="GetSession"><arg type="s" direction="in"/><arg type="o" direction="out"/></method>
  <method name="Inhibit">
   <arg type="s" direction="in"/><arg type="s" direction="in"/>
   <arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="h" direction="out"/>
  </method>
  <method name="SetLocked"><arg type="b" direction="in"/></method>
  <method name="Prepare"><arg type="b" direction="in"/></method>
  <method name="AssertSleepLock"><arg type="b" direction="in"/></method>
  <signal name="PrepareForShutdown"><arg type="b"/></signal>
 </interface>
 <interface name="{SESSION_INTERFACE}">
  <property name="LockedHint" type="b" access="read"/>
 </interface>
 <interface name="{SAVER}">
  <method name="Inhibit"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="u" direction="out"/></method>
  <property name="Inhibited" type="b" access="read"/>
 </interface>
</node>
""")
bus = Gio.bus_get_sync(Gio.BusType.SESSION)
locks = []
locked = False
idle_owners = set()


def call(_bus, _sender, _path, _interface, method, args, invocation):
    global locked
    if method == "GetSession":
        invocation.return_value(VAR("(o)", (SESSION,)))
        return
    if method == "Inhibit":
        what, who, reason, mode = args.unpack()
        print("Inhibit", what, who, reason, mode, flush=True)
        read, write = os.pipe()
        locks.append((read, what, mode))
        descriptors = Gio.UnixFDList.new()
        descriptors.append(write)
        os.close(write)
        invocation.return_value_with_unix_fd_list(VAR("(h)", (0,)), descriptors)
        return
    if method == "SetLocked":
        locked, = args.unpack()
        bus.emit_signal(None, SESSION, "org.freedesktop.DBus.Properties", "PropertiesChanged",
                        VAR("(sa{sv}as)", (SESSION_INTERFACE, {"LockedHint": VAR("b", locked)}, [])))
    if method == "Prepare":
        bus.emit_signal(None, PATH, MANAGER, "PrepareForShutdown", args)
    if method == "AssertSleepLock":
        expected, = args.unpack()
        held = any(what == "sleep" and mode == "block" and not select.select([fd], [], [], 0)[0]
                   for fd, what, mode in locks)
        if held != expected:
            invocation.return_dbus_error("org.enzui.Test.UnexpectedLease", f"sleep lock held: {held}")
            return
    invocation.return_value(None)


def inhibit_idle(_bus, sender, _path, _interface, _method, args, invocation):
    app, reason = args.unpack()
    if not app or reason != "Portal integration":
        invocation.return_dbus_error("org.enzui.Test.InvalidInhibitor", f"{app}: {reason}")
        return
    idle_owners.add(sender)
    invocation.return_value(VAR("(u)", (1,)))


def owner_changed(_bus, _sender, _path, _interface, _member, args):
    name, _, new = args.unpack()
    if not new:
        idle_owners.discard(name)


bus.register_object(PATH, node.interfaces[0], call, None, None)
bus.register_object(SESSION, node.interfaces[1], None, lambda *_: VAR("b", locked), None)
bus.register_object(SAVER_PATH, node.interfaces[2], inhibit_idle, lambda *_: VAR("b", bool(idle_owners)), None)
bus.signal_subscribe(DBUS, DBUS, "NameOwnerChanged", "/org/freedesktop/DBus", None,
                     Gio.DBusSignalFlags.NONE, owner_changed)
for service in (LOGIN, SAVER):
    Gio.bus_own_name_on_connection(bus, service, Gio.BusNameOwnerFlags.NONE, None, None)
GLib.MainLoop().run()
