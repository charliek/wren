"""Native X11 close requests, restricted to a window of the owned child."""

from __future__ import annotations

import ctypes as c
from ctypes.util import find_library
import subprocess


class _Data(c.Union):
    _fields_ = [("bytes", c.c_char * 20), ("shorts", c.c_short * 10), ("longs", c.c_long * 5)]


class _ClientMessage(c.Structure):
    _fields_ = [("type", c.c_int), ("serial", c.c_ulong), ("send_event", c.c_int),
                ("display", c.c_void_p), ("window", c.c_ulong), ("message_type", c.c_ulong),
                ("format", c.c_int), ("data", _Data)]


class _Event(c.Union):
    _fields_ = [("client", _ClientMessage), ("padding", c.c_long * 24)]


def request_close(pid: int, window: str) -> None:
    """Send WM_DELETE_WINDOW, not XDestroyWindow or a process-kill request."""
    if type(pid) is not int or pid <= 0:
        raise ValueError("An owned positive child pid is required")
    number = int(window, 0)
    if not 0 < number <= (1 << (c.sizeof(c.c_ulong) * 8)) - 1:
        raise ValueError("Invalid X11 window ID")
    owner = subprocess.run(["xdotool", "getwindowpid", str(number)], check=True,
                           capture_output=True, text=True, timeout=5)
    if int(owner.stdout.strip()) != pid:
        raise RuntimeError("Refusing to close a window not owned by the test child")
    library = find_library("X11")
    if library is None:
        raise RuntimeError("Native X11 tests require libX11")
    x11 = c.CDLL(library)
    x11.XOpenDisplay.argtypes = [c.c_char_p]
    x11.XOpenDisplay.restype = c.c_void_p
    x11.XInternAtom.argtypes = [c.c_void_p, c.c_char_p, c.c_int]
    x11.XInternAtom.restype = c.c_ulong
    x11.XSendEvent.argtypes = [c.c_void_p, c.c_ulong, c.c_int, c.c_long, c.POINTER(_Event)]
    x11.XSendEvent.restype = c.c_int
    x11.XFlush.argtypes = [c.c_void_p]
    x11.XCloseDisplay.argtypes = [c.c_void_p]
    display = x11.XOpenDisplay(None)
    if not display:
        raise RuntimeError("Cannot open the private X11 test display")
    try:
        event = _Event()
        event.client.type = 33  # Xlib ClientMessage, per the X11 wire protocol.
        event.client.send_event = 1
        event.client.display = display
        event.client.window = number
        event.client.message_type = x11.XInternAtom(display, b"WM_PROTOCOLS", 0)
        event.client.format = 32
        event.client.data.longs[0] = x11.XInternAtom(display, b"WM_DELETE_WINDOW", 0)
        if not x11.XSendEvent(display, number, 0, 0, c.byref(event)):
            raise RuntimeError("WM_DELETE_WINDOW was not sent")
        x11.XFlush(display)
    finally:
        x11.XCloseDisplay(display)
