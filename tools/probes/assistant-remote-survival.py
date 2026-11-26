"""Hold a consented RemoteDesktop stream and verify frame and EIS continuity on demand.

Run in a disposable managed desktop with a focused text fixture. Enter a Linux
keycode on stdin after each Assistant revoke to require a fresh live frame and
send a press/release through the original EIS descriptor. The text fixture
provides the independent assertion that the key reached the application.
"""
import ctypes
import hashlib
import re
import secrets
import select
import subprocess
import sys
import time

import dbus
import gi
from dbus.mainloop.glib import DBusGMainLoop

gi.require_version("Gst", "1.0")
from gi.repository import GLib, Gst

DESKTOP = "org.freedesktop.portal.Desktop"
BASE = "/org/freedesktop/portal/desktop"
OPTIONS = dbus.Dictionary({}, signature="sv")
DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()
portal = bus.get_object(DESKTOP, BASE)
remote = dbus.Interface(portal, "org.freedesktop.portal.RemoteDesktop")
cast = dbus.Interface(portal, "org.freedesktop.portal.ScreenCast")
main = GLib.MainContext.default()


def request(method, *args, **options):
    token = "survival" + secrets.token_hex(8)
    sender = bus.get_unique_name()[1:].replace(".", "_")
    path = f"{BASE}/request/{sender}/{token}"
    result = []
    loop = GLib.MainLoop()
    signal = bus.add_signal_receiver(lambda *reply: (result.extend(reply), loop.quit()), signal_name="Response", path=path)
    timer = GLib.timeout_add_seconds(45, lambda: (loop.quit(), False)[1])
    try:
        assert method(*args, dbus.Dictionary({"handle_token": token, **options}, signature="sv")) == path
        if not result:
            loop.run()
        assert result and result[0] == 0, (method, result)
        return result[1]
    finally:
        signal.remove()
        if main.find_source_by_id(timer):
            GLib.source_remove(timer)


lib = ctypes.CDLL("libei.so.1")
pointer = ctypes.c_void_p
integer = ctypes.c_int
uint = ctypes.c_uint32


def function(name, output, *args):
    call = getattr(lib, "ei_" + name)
    call.restype = output
    call.argtypes = args
    return call


new = function("new_sender", pointer, pointer)
setup = function("setup_backend_fd", integer, pointer, integer)
get_fd = function("get_fd", integer, pointer)
dispatch = function("dispatch", None, pointer)
get_event = function("get_event", pointer, pointer)
kind = function("event_get_type", integer, pointer)
get_seat = function("event_get_seat", pointer, pointer)
bind = function("seat_bind_capabilities", None, pointer)
get_device = function("event_get_device", pointer, pointer)
device_ref = function("device_ref", pointer, pointer)
device_unref = function("device_unref", pointer, pointer)
unref_event = function("event_unref", pointer, pointer)
unref = function("unref", pointer, pointer)
start = function("device_start_emulating", None, pointer, uint)
key = function("device_keyboard_key", None, pointer, uint, ctypes.c_bool)
frame = function("device_frame", None, pointer, ctypes.c_uint64)
now = function("now", ctypes.c_uint64, pointer)

session = request(remote.CreateSession, session_handle_token="remote" + secrets.token_hex(8))["session_handle"]
closed = []
bus.add_signal_receiver(lambda *args: closed.append(args), signal_name="Closed", path=session)
session_control = dbus.Interface(bus.get_object(DESKTOP, session), "org.freedesktop.portal.Session")
context = None
keyboard = None
pipeline = None
try:
    request(cast.SelectSources, session, types=dbus.UInt32(1), multiple=dbus.Boolean(False), cursor_mode=dbus.UInt32(1))
    request(remote.SelectDevices, session, types=dbus.UInt32(1), persist_mode=dbus.UInt32(0))
    result = request(remote.Start, session, "")
    assert int(result["devices"]) & 1
    node = int(result["streams"][0][0])
    remote_fd = cast.OpenPipeWireRemote(session, OPTIONS).take()
    context = new(None)
    assert setup(context, remote.ConnectToEIS(session, OPTIONS).take()) == 0
    fd = get_fd(context)

    def drain():
        global keyboard
        dispatch(context)
        while event := get_event(context):
            try:
                event_type = kind(event)
                assert event_type not in (2, 6), f"EIS closed or lost its device: {event_type}"
                if event_type == 3:
                    bind(get_seat(event), integer(4), pointer())
                if event_type == 8 and not keyboard:
                    keyboard = device_ref(get_device(event))
            finally:
                unref_event(event)

    deadline = time.monotonic() + 5
    while not keyboard and time.monotonic() < deadline:
        select.select([fd], [], [], max(0, deadline - time.monotonic()))
        drain()
    assert keyboard, "remote EIS keyboard was never resumed"
    start(keyboard, 1)
    dispatch(context)

    Gst.init(None)
    info = subprocess.check_output(["pw-cli", "info", str(node)], text=True)
    serial = re.search(r'object\.serial = "(\d+)"', info)[1]
    pipeline = Gst.parse_launch(
        f"pipewiresrc fd={remote_fd} target-object={serial} on-disconnect=error "
        "stream-properties=properties,media.type=Video,media.category=Capture "
        "! video/x-raw ! videoconvert ! video/x-raw,format=RGB ! appsink name=sink max-buffers=1 drop=true sync=false"
    )
    sink = pipeline.get_by_name("sink")
    if pipeline.set_state(Gst.State.PLAYING) == Gst.StateChangeReturn.FAILURE:
        message = pipeline.get_bus().pop_filtered(Gst.MessageType.ERROR)
        raise AssertionError(message.parse_error() if message else "PipeWire sink failed to start")

    def sample(timeout):
        value = sink.emit("try-pull-sample", timeout * Gst.SECOND)
        assert value is not None, "original PipeWire node stopped delivering"
        buffer = value.get_buffer()
        return (buffer.pts, hashlib.sha256(buffer.extract_dup(0, buffer.get_size())).hexdigest())

    def check_alive():
        while main.pending():
            main.iteration(False)
        assert not closed, "unrelated RemoteDesktop session closed"
        drain()
        assert not pipeline.get_bus().pop_filtered(Gst.MessageType.EOS | Gst.MessageType.ERROR)

    print(f"ready session={session} node={node} frame={sample(8)}", flush=True)
    for line in sys.stdin:
        command = line.strip()
        if command == "quit":
            break
        check_alive()
        before = sample(5)
        for pressed in (True, False):
            key(keyboard, int(command), pressed)
            frame(keyboard, now(context))
            dispatch(context)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            after = sample(1)
            if after[0] > before[0] and after[1] != before[1]:
                break
        else:
            raise AssertionError(f"no fresh changed frame on PipeWire node {node}")
        check_alive()
        print(f"survived node={node} key={command} before={before} after={after}", flush=True)
finally:
    if pipeline:
        pipeline.set_state(Gst.State.NULL)
    if keyboard:
        device_unref(keyboard)
    if context:
        unref(context)
    if not closed:
        session_control.Close(timeout=3)
