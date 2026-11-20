"""portal-session.py [HOLD]: the RemoteDesktop flow of an RDP server
through xdg-desktop-portal: CreateSession, SelectDevices (keyboard and
pointer), ScreenCast SelectSources (a monitor), Start, then ConnectToEIS,
whose fd goes to eis-sender (beside this script, built with
`cc -o eis-sender eis-sender.c $(pkg-config --cflags --libs libei-1.0)`).
Also checks that a second ConnectToEIS and Notify calls are refused once
EIS is connected. Run in the user's session; answer the picker and the
consent dialog on screen. HOLD: seconds the sender stays connected after
its input (use a long one to lock the screen meanwhile)."""
import sys, time, subprocess, os
from gi.repository import Gio, GLib
bus = Gio.bus_get_sync(Gio.BusType.SESSION)
name = bus.get_unique_name()[1:].replace(".", "_")
loop = GLib.MainLoop()
count = [0]
def log(*a):
    print(time.strftime("%H:%M:%S"), *a, flush=True)
def call(iface, method, sig, args):
    return bus.call_sync("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", iface, method,
                         GLib.Variant(sig, args), None, 0, 600000, None)
def request(iface, method, sig, build):
    count[0] += 1
    token = f"t{count[0]}"
    path = f"/org/freedesktop/portal/desktop/request/{name}/{token}"
    out = {}
    def got(conn, sender, obj, i, sig_, params):
        out["code"], out["results"] = params.unpack()
        loop.quit()
    sub = bus.signal_subscribe(None, "org.freedesktop.portal.Request", "Response", path, None, 0, got)
    call(iface, method, sig, build(token))
    loop.run()
    bus.signal_unsubscribe(sub)
    log(method, out["code"], out["results"])
    return out["code"], out["results"]
RD = "org.freedesktop.portal.RemoteDesktop"
SC = "org.freedesktop.portal.ScreenCast"
code, res = request(RD, "CreateSession", "(a{sv})", lambda t: ({"handle_token": GLib.Variant("s", t), "session_handle_token": GLib.Variant("s", "s" + t)},))
session = res["session_handle"]
request(RD, "SelectDevices", "(oa{sv})", lambda t: (session, {"handle_token": GLib.Variant("s", t), "types": GLib.Variant("u", 3)}))
request(SC, "SelectSources", "(oa{sv})", lambda t: (session, {"handle_token": GLib.Variant("s", t), "types": GLib.Variant("u", 1)}))
code, res = request(RD, "Start", "(osa{sv})", lambda t: (session, "", {"handle_token": GLib.Variant("s", t)}))
if code != 0:
    sys.exit(1)
v = bus.call_sync("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.DBus.Properties", "Get",
                  GLib.Variant("(ss)", (RD, "version")), None, 0, 5000, None).unpack()[0]
log("frontend RemoteDesktop version", v)
def connect():
    reply, fds = bus.call_with_unix_fd_list_sync("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", RD, "ConnectToEIS",
                                                 GLib.Variant("(oa{sv})", (session, {})), GLib.VariantType("(h)"), 0, 10000, None, None)
    return fds.get(reply.unpack()[0])
fd = connect()
log("ConnectToEIS fd", fd)
try:
    connect()
    log("second ConnectToEIS was accepted")
except GLib.Error as e:
    log("second ConnectToEIS refused:", e.message)
try:
    call(RD, "NotifyPointerMotion", "(oa{sv}dd)", (session, {}, 20.0, 10.0))
    log("Notify after EIS returned without error")
except GLib.Error as e:
    log("Notify after EIS refused:", e.message)
hold = sys.argv[1] if len(sys.argv) > 1 else "2"
out = subprocess.run([os.path.join(os.path.dirname(os.path.abspath(__file__)), "eis-sender"), str(fd), hold], pass_fds=[fd], capture_output=True, text=True, timeout=120)
for line in out.stdout.splitlines():
    log("eis:", line)
log("eistest exit", out.returncode)
bus.call_sync("org.freedesktop.portal.Desktop", session, "org.freedesktop.portal.Session", "Close", None, None, 0, 5000, None)
log("closed")
