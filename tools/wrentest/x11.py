"""Native X11 operations, restricted to a window of the owned test child."""

from __future__ import annotations

import ctypes as c
from ctypes.util import find_library
from dataclasses import dataclass
import os
from pathlib import Path
import subprocess
import threading


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


# Offsets are signed 16-bit fields in GetImage; test captures are intentionally
# much smaller than the protocol's unsigned 16-bit dimensions.
MAX_READ_DIMENSION = 4096
MAX_READ_PIXELS = 4 * 1024 * 1024
MAX_READ_BYTES = 16 * 1024 * 1024
_READ_LOCK = threading.Lock()


@dataclass(frozen=True)
class RGBImage:
    """Owned, tightly packed RGB8 bytes, independent of XImage storage."""

    width: int
    height: int
    data: bytes

    def pixel(self, x: int, y: int) -> tuple[int, int, int]:
        if type(x) is not int or type(y) is not int or not (
            0 <= x < self.width and 0 <= y < self.height
        ):
            raise IndexError("RGB pixel is outside the captured rectangle")
        offset = (y * self.width + x) * 3
        return tuple(self.data[offset:offset + 3])


class _Visual(c.Structure):
    _fields_ = [("ext_data", c.c_void_p), ("visualid", c.c_ulong), ("class_", c.c_int),
                ("red_mask", c.c_ulong), ("green_mask", c.c_ulong), ("blue_mask", c.c_ulong),
                ("bits_per_rgb", c.c_int), ("map_entries", c.c_int)]


class _WindowAttributes(c.Structure):
    _fields_ = [("x", c.c_int), ("y", c.c_int), ("width", c.c_int), ("height", c.c_int),
                ("border_width", c.c_int), ("depth", c.c_int), ("visual", c.POINTER(_Visual)),
                ("root", c.c_ulong), ("class_", c.c_int), ("bit_gravity", c.c_int),
                ("win_gravity", c.c_int), ("backing_store", c.c_int),
                ("backing_planes", c.c_ulong), ("backing_pixel", c.c_ulong),
                ("save_under", c.c_int), ("colormap", c.c_ulong), ("map_installed", c.c_int),
                ("map_state", c.c_int), ("all_event_masks", c.c_long),
                ("your_event_mask", c.c_long), ("do_not_propagate_mask", c.c_long),
                ("override_redirect", c.c_int), ("screen", c.c_void_p)]


class _XImage(c.Structure):
    # Xlib.h's prefix through obdata, followed by its six function pointers.
    # In particular, data is a pointer, not c_char_p (which truncates at NUL).
    _fields_ = [("width", c.c_int), ("height", c.c_int), ("xoffset", c.c_int),
                ("format", c.c_int), ("data", c.c_void_p), ("byte_order", c.c_int),
                ("bitmap_unit", c.c_int), ("bitmap_bit_order", c.c_int),
                ("bitmap_pad", c.c_int), ("depth", c.c_int), ("bytes_per_line", c.c_int),
                ("bits_per_pixel", c.c_int), ("red_mask", c.c_ulong),
                ("green_mask", c.c_ulong), ("blue_mask", c.c_ulong),
                ("obdata", c.c_void_p), ("functions", c.c_void_p * 6)]


class _XError(c.Structure):
    _fields_ = [("type", c.c_int), ("display", c.c_void_p), ("resourceid", c.c_ulong),
                ("serial", c.c_ulong), ("error_code", c.c_ubyte),
                ("request_code", c.c_ubyte), ("minor_code", c.c_ubyte)]


_ErrorHandler = c.CFUNCTYPE(c.c_int, c.c_void_p, c.POINTER(_XError))


def _decode_image(image: _XImage, attributes: _WindowAttributes,
                  width: int, height: int) -> RGBImage:
    if (image.width, image.height, image.xoffset) != (width, height, 0):
        raise RuntimeError("XGetImage returned unexpected dimensions or xoffset")
    if (image.format != 2 or image.byte_order not in (0, 1)
            or image.depth not in (24, 32) or image.depth != attributes.depth
            or image.bits_per_pixel not in (24, 32) or image.depth > image.bits_per_pixel
            or image.bitmap_unit not in (8, 16, 32) or image.bitmap_pad not in (8, 16, 32)
            or image.bitmap_bit_order not in (0, 1)):
        raise RuntimeError("Unsupported XImage format; require RGB8 ZPixmap at 24/32 bpp")
    bytes_per_pixel = image.bits_per_pixel // 8
    alignment = image.bitmap_pad // 8
    minimum_stride = ((width * bytes_per_pixel + alignment - 1) // alignment) * alignment
    stride = image.bytes_per_line
    if stride < minimum_stride or stride % alignment:
        raise RuntimeError("Invalid XImage row stride")
    if stride * height > MAX_READ_BYTES:
        raise RuntimeError(f"XImage exceeds the {MAX_READ_BYTES}-byte readback limit")
    if not image.data:
        raise RuntimeError("XImage has no pixel data")
    masks = (image.red_mask, image.green_mask, image.blue_mask)
    visual = attributes.visual.contents
    if masks != (visual.red_mask, visual.green_mask, visual.blue_mask):
        raise RuntimeError("XImage masks do not match the TrueColor visual")
    shifts = []
    used = 0
    for mask in masks:
        shift = (mask & -mask).bit_length() - 1
        if (mask == 0 or mask != 255 << shift
                or mask >> image.bits_per_pixel or mask & used):
            raise RuntimeError("Unsupported XImage masks; require non-overlapping RGB8 channels")
        used |= mask
        shifts.append(shift)
    raw = c.string_at(image.data, stride * height)
    output = bytearray(width * height * 3)
    byte_order = "little" if image.byte_order == 0 else "big"
    destination = 0
    for y in range(height):
        for x in range(width):
            start = y * stride + x * bytes_per_pixel
            pixel = int.from_bytes(raw[start:start + bytes_per_pixel], byte_order)
            for shift in shifts:
                output[destination] = (pixel >> shift) & 255
                destination += 1
    return RGBImage(width, height, bytes(output))


def read_pixels(pid: int, window: str, x: int, y: int, width: int, height: int) -> RGBImage:
    """Read actual client-window pixels, never an IPC/offscreen screenshot.

    Only private ``tools/run_display.py x11`` fixtures may call this helper.
    The positive child PID must match getwindowpid before loading Xlib. Crops
    must fit the client, with 0..32767 integer offsets, dimensions 1..4096,
    at most 4 Mi pixels and 16 MiB of native storage. Only TrueColor RGB8
    ZPixmap images (24/32-bit depth/storage) are supported; alpha is discarded.
    """
    if type(pid) is not int or pid <= 0:
        raise ValueError("An owned positive child pid is required")
    if type(window) is not str:
        raise ValueError("Invalid X11 window ID")
    try:
        number = int(window, 0)
    except ValueError as error:
        raise ValueError("Invalid X11 window ID") from error
    if not 0 < number <= 0xffffffff:
        raise ValueError("Invalid X11 window ID; require a 32-bit protocol XID")
    if (any(type(value) is not int for value in (x, y, width, height))
            or not (0 <= x <= 32767 and 0 <= y <= 32767)
            or not (1 <= width <= MAX_READ_DIMENSION and 1 <= height <= MAX_READ_DIMENSION)
            or width * height > MAX_READ_PIXELS):
        raise ValueError("Invalid or oversized integer X11 readback rectangle")
    if (not os.environ.get("DISPLAY") or os.environ.get("WAYLAND_DISPLAY")
            or not Path(os.environ.get("XDG_RUNTIME_DIR", "")).name.startswith("wren-display-")):
        raise RuntimeError("Pixel readback requires the private X11 test display")
    owner = subprocess.run(["xdotool", "getwindowpid", str(number)], check=True,
                           capture_output=True, text=True, timeout=5)
    try:
        owned = int(owner.stdout.strip()) == pid
    except ValueError:
        owned = False
    if not owned:
        raise RuntimeError("Refusing to read a window not owned by the test child")
    library = find_library("X11")
    if library is None:
        raise RuntimeError("Native X11 tests require libX11")
    x11 = c.CDLL(library)
    x11.XOpenDisplay.argtypes = [c.c_char_p]
    x11.XOpenDisplay.restype = c.c_void_p
    x11.XGetWindowAttributes.argtypes = [c.c_void_p, c.c_ulong, c.POINTER(_WindowAttributes)]
    x11.XGetWindowAttributes.restype = c.c_int
    x11.XGetImage.argtypes = [c.c_void_p, c.c_ulong, c.c_int, c.c_int,
                             c.c_uint, c.c_uint, c.c_ulong, c.c_int]
    x11.XGetImage.restype = c.POINTER(_XImage)
    x11.XDestroyImage.argtypes = [c.POINTER(_XImage)]
    x11.XDestroyImage.restype = c.c_int
    x11.XGrabServer.argtypes = [c.c_void_p]
    x11.XUngrabServer.argtypes = [c.c_void_p]
    x11.XSync.argtypes = [c.c_void_p, c.c_int]
    x11.XSetErrorHandler.argtypes = [c.c_void_p]
    x11.XSetErrorHandler.restype = c.c_void_p
    x11.XCloseDisplay.argtypes = [c.c_void_p]
    # XSetErrorHandler is process-global; serialize our scoped replacements.
    with _READ_LOCK:
        display = x11.XOpenDisplay(None)
        if not display:
            raise RuntimeError("Cannot open the private X11 test display")
        errors = []

        @_ErrorHandler
        def record_error(_display, event):
            error = event.contents
            errors.append((error.error_code, error.request_code, error.resourceid))
            return 0

        image = None
        previous = None
        handler_installed = False
        try:
            previous = x11.XSetErrorHandler(c.cast(record_error, c.c_void_p))
            handler_installed = True
            # Keep a concurrent resize/unmap from invalidating the checked crop.
            # The synchronous XGetImage finishes before the server is released;
            # Python decoding never holds this grab.
            x11.XGrabServer(display)
            try:
                attributes = _WindowAttributes()
                if not x11.XGetWindowAttributes(display, number, c.byref(attributes)) or errors:
                    raise RuntimeError(f"Cannot inspect the owned X11 client window: {errors}")
                if attributes.class_ != 1 or attributes.map_state != 2:
                    raise RuntimeError("X11 readback requires a viewable InputOutput client window")
                if (attributes.depth not in (24, 32) or not attributes.visual
                        or attributes.visual.contents.class_ != 4):
                    raise RuntimeError("Unsupported X11 visual; require 24/32-bit TrueColor")
                if x + width > attributes.width or y + height > attributes.height:
                    raise ValueError("X11 readback rectangle exceeds the window client geometry")
                image = x11.XGetImage(display, number, x, y, width, height,
                                     c.c_ulong(-1).value, 2)  # AllPlanes, ZPixmap.
            finally:
                x11.XUngrabServer(display)
                x11.XSync(display, 0)
            if errors:
                raise RuntimeError(f"Native X11 pixel readback failed: {errors}")
            if not image:
                raise RuntimeError("XGetImage returned no image for the owned window")
            return _decode_image(image.contents, attributes, width, height)
        finally:
            try:
                if image:
                    x11.XDestroyImage(image)
            finally:
                try:
                    x11.XSync(display, 0)
                finally:
                    try:
                        if handler_installed:
                            x11.XSetErrorHandler(previous)
                    finally:
                        x11.XCloseDisplay(display)
