"""Validate native close messages and bounded pixels without a real display."""

from contextlib import contextmanager
import ctypes
import os
from types import SimpleNamespace
import unittest
from unittest.mock import MagicMock, patch

from tools.wrentest import x11


class CloseMessageTests(unittest.TestCase):
    def test_foreign_window_is_rejected_before_loading_xlib(self):
        with patch.object(x11.subprocess, "run", return_value=SimpleNamespace(stdout="999\n")), \
             patch.object(x11.c, "CDLL") as load:
            with self.assertRaisesRegex(RuntimeError, "not owned"):
                x11.request_close(123, "456")
            load.assert_not_called()

    def test_invalid_ids_are_rejected_before_native_calls(self):
        for pid, window in [(True, "1"), (0, "1"), (123, "0"), (123, "-1"), (123, "no-window")]:
            with self.subTest(pid=pid, window=window), patch.object(x11.subprocess, "run") as run:
                with self.assertRaises(ValueError):
                    x11.request_close(pid, window)
                run.assert_not_called()

    def test_constructs_wm_delete_not_destroy_and_closes_display(self):
        library = MagicMock()
        library.XOpenDisplay.return_value = 42
        library.XInternAtom.side_effect = [10, 11]
        seen = []

        def send(display, window, propagate, mask, pointer):
            event = ctypes.cast(pointer, ctypes.POINTER(x11._Event)).contents.client
            seen.append((display, window, propagate, mask, event.type, event.window,
                         event.message_type, event.format, event.data.longs[0]))
            return 1

        library.XSendEvent.side_effect = send
        with patch.object(x11.subprocess, "run", return_value=SimpleNamespace(stdout="123\n")), \
             patch.object(x11, "find_library", return_value="libX11"), \
             patch.object(x11.c, "CDLL", return_value=library):
            x11.request_close(123, "456")
        self.assertEqual(seen, [(42, 456, 0, 0, 33, 456, 10, 32, 11)])
        library.XCloseDisplay.assert_called_once_with(42)
        library.XFlush.assert_called_once_with(42)

    def test_send_failure_still_closes_owned_display_connection(self):
        library = MagicMock()
        library.XOpenDisplay.return_value = 42
        library.XInternAtom.side_effect = [10, 11]
        library.XSendEvent.return_value = 0
        with patch.object(x11.subprocess, "run", return_value=SimpleNamespace(stdout="123\n")), \
             patch.object(x11, "find_library", return_value="libX11"), \
             patch.object(x11.c, "CDLL", return_value=library):
            with self.assertRaisesRegex(RuntimeError, "not sent"):
                x11.request_close(123, "456")
        library.XCloseDisplay.assert_called_once_with(42)


class FakeXlib:
    """Real ctypes records/backing bytes, fake Xlib calls and no display access."""

    def __init__(self, *, raw=b"\x33\x22\x11\xff", width=1, height=1,
                 bits_per_pixel=32, byte_order=0, stride=None,
                 masks=(0xff0000, 0xff00, 0xff)):
        self.visual = x11._Visual(class_=4, red_mask=masks[0], green_mask=masks[1],
                                  blue_mask=masks[2], bits_per_rgb=8, map_entries=256)
        self.attributes = x11._WindowAttributes(
            width=1280, height=800, depth=24, class_=1, map_state=2,
            visual=ctypes.pointer(self.visual),
        )
        self.buffer = ctypes.create_string_buffer(raw)
        self.image = x11._XImage(
            width=width, height=height, xoffset=0, format=2,
            data=ctypes.cast(self.buffer, ctypes.c_void_p), byte_order=byte_order,
            bitmap_unit=32, bitmap_bit_order=0, bitmap_pad=32, depth=24,
            bytes_per_line=stride or ((width * bits_per_pixel + 31) // 32) * 4,
            bits_per_pixel=bits_per_pixel, red_mask=masks[0], green_mask=masks[1],
            blue_mask=masks[2],
        )
        self.pointer = ctypes.pointer(self.image)
        self.handler = None
        self.events = []
        self.XOpenDisplay = self.function("open", lambda _name: 42)
        self.XGetWindowAttributes = self.function("attributes", self.get_attributes)
        self.XGetImage = self.function("image", lambda *_args: self.pointer)
        self.XDestroyImage = self.function("destroy", lambda _image: 1)
        self.XGrabServer = self.function("grab", lambda _display: 1)
        self.XUngrabServer = self.function("ungrab", lambda _display: 1)
        self.XSync = self.function("sync", lambda *_args: 1)
        self.XSetErrorHandler = self.function("handler", self.set_handler)
        self.XCloseDisplay = self.function("close", lambda _display: 1)

    def function(self, name, action):
        def invoke(*args):
            self.events.append(name)
            return action(*args)
        return MagicMock(side_effect=invoke)

    def get_attributes(self, _display, _window, pointer):
        ctypes.memmove(pointer, ctypes.byref(self.attributes), ctypes.sizeof(self.attributes))
        return 1

    def set_handler(self, pointer):
        self.handler = pointer
        return 19

    def protocol_error(self, *, code=8, request=73):
        error = x11._XError(display=42, resourceid=456, error_code=code, request_code=request)
        ctypes.cast(self.handler, x11._ErrorHandler)(42, ctypes.byref(error))


_PRIVATE_DISPLAY = {"DISPLAY": ":99", "XDG_RUNTIME_DIR": "/tmp/wren-display-owned"}


@contextmanager
def fake_display(library=None, *, owner="123\n"):
    library = library or FakeXlib()
    with patch.dict(os.environ, _PRIVATE_DISPLAY, clear=True), \
         patch.object(x11.subprocess, "run", return_value=SimpleNamespace(stdout=owner)) as run, \
         patch.object(x11, "find_library", return_value="libX11") as find, \
         patch.object(x11.c, "CDLL", return_value=library) as load:
        yield library, run, find, load


class PixelReadbackTests(unittest.TestCase):
    def test_bgra_is_decoded_to_owned_rgb_and_resources_are_released(self):
        with fake_display() as (library, run, _find, _load):
            image = x11.read_pixels(123, "0x1c8", 20, 30, 1, 1)
        self.assertEqual(image, x11.RGBImage(1, 1, b"\x11\x22\x33"))
        self.assertEqual(image.pixel(0, 0), (0x11, 0x22, 0x33))
        run.assert_called_once_with(["xdotool", "getwindowpid", "456"], check=True,
                                    capture_output=True, text=True, timeout=5)
        library.XGetImage.assert_called_once_with(
            42, 456, 20, 30, 1, 1, ctypes.c_ulong(-1).value, 2,
        )
        library.XDestroyImage.assert_called_once_with(library.pointer)
        library.XCloseDisplay.assert_called_once_with(42)
        self.assertEqual(library.events, [
            "open", "handler", "grab", "attributes", "image", "ungrab", "sync",
            "destroy", "sync", "handler", "close",
        ])
        self.assertEqual(library.handler, 19, "The process-global X error handler was not restored")
        library.buffer[0] = b"\x00"
        self.assertEqual(image.pixel(0, 0), (0x11, 0x22, 0x33), "RGB aliases destroyed XImage memory")
        for point in ((-1, 0), (1, 0), (0, 1), (True, 0), (0.5, 0)):
            with self.subTest(point=point), self.assertRaises(IndexError):
                image.pixel(*point)

    def test_masks_endianness_and_padded_rows_are_not_assumed_rgba(self):
        for byte_order, bits, masks, raw, stride in (
            (0, 32, (0xff, 0xff00, 0xff0000),
             b"\x11\x22\x33\xeePAD!\x44\x55\x66\xddPAD!", 8),
            (1, 32, (0xff0000, 0xff00, 0xff),
             b"\xee\x11\x22\x33\xdd\x44\x55\x66", 4),
            (0, 24, (0xff0000, 0xff00, 0xff),
             b"\x33\x22\x11\xee\x66\x55\x44\xdd", 4),
            (1, 24, (0xff0000, 0xff00, 0xff),
             b"\x11\x22\x33\xee\x44\x55\x66\xdd", 4),
            (1, 32, (0xff000000, 0xff0000, 0xff00),
             b"\x11\x22\x33\xee\x44\x55\x66\xdd", 4),
        ):
            with self.subTest(order=byte_order, bits=bits, masks=masks):
                library = FakeXlib(raw=raw, height=2, bits_per_pixel=bits,
                                   byte_order=byte_order, stride=stride, masks=masks)
                with fake_display(library):
                    image = x11.read_pixels(123, "456", 0, 0, 1, 2)
                self.assertEqual(image.data, b"\x11\x22\x33\x44\x55\x66")

    def test_truecolor_depth_32_and_unaligned_rgb8_masks_are_decoded(self):
        masks = (255 << 17, 255 << 9, 255 << 1)
        pixel = (0x11 << 17) | (0x22 << 9) | (0x33 << 1)
        library = FakeXlib(raw=pixel.to_bytes(4, "little"), masks=masks)
        library.attributes.depth = library.image.depth = 32
        with fake_display(library):
            self.assertEqual(x11.read_pixels(123, "456", 0, 0, 1, 1).data, b"\x11\x22\x33")

    def test_ximage_prefix_matches_xlib_h_alignment(self):
        pointer_size = ctypes.sizeof(ctypes.c_void_p)
        self.assertEqual(x11._XImage.data.offset, 16)
        self.assertEqual(x11._XImage.byte_order.offset, 16 + pointer_size)
        red_offset = 56 if pointer_size == 8 else 48
        self.assertEqual(x11._XImage.red_mask.offset, red_offset)
        self.assertEqual(x11._XImage.blue_mask.offset, red_offset + 2 * pointer_size)
        self.assertIs(x11._XImage._fields_[4][1], ctypes.c_void_p)

    def test_invalid_identity_is_rejected_before_process_or_xlib_calls(self):
        for pid, window in (
            (True, "1"), (0, "1"), (-1, "1"), (1.0, "1"), (123, "0"),
            (123, "-1"), (123, "not-a-window"), (123, "0x100000000"),
            (123, 456), (123, None),
        ):
            with self.subTest(pid=pid, window=window), fake_display() as (_lib, run, find, load):
                with self.assertRaises(ValueError):
                    x11.read_pixels(pid, window, 0, 0, 1, 1)
                run.assert_not_called()
                find.assert_not_called()
                load.assert_not_called()

    def test_invalid_or_oversized_rectangles_are_rejected_before_lookup(self):
        for rectangle in (
            (-1, 0, 1, 1), (0, -1, 1, 1), (0, 0, 0, 1), (0, 0, 1, -1),
            (True, 0, 1, 1), (0, 0, False, 1), (0.0, 0, 1, 1),
            (0, 0, 1.5, 1), (0, 0, float("inf"), 1), (float("nan"), 0, 1, 1),
            (0, None, 1, 1), (0, 0, "1", 1), (32768, 0, 1, 1),
            (0, 32768, 1, 1), (0, 0, 4097, 1), (0, 0, 1, 4097),
            (0, 0, 4096, 4096),
        ):
            with self.subTest(rectangle=rectangle), fake_display() as (_lib, run, find, load):
                with self.assertRaisesRegex(ValueError, "rectangle"):
                    x11.read_pixels(123, "456", *rectangle)
                run.assert_not_called()
                find.assert_not_called()
                load.assert_not_called()

    def test_foreign_and_unverifiable_owners_are_rejected_before_xlib(self):
        for owner in ("999\n", "", "unknown\n", "123\n999\n"):
            with self.subTest(owner=owner), fake_display(owner=owner) as (_lib, _run, find, load):
                with self.assertRaisesRegex(RuntimeError, "not owned"):
                    x11.read_pixels(123, "456", 0, 0, 1, 1)
                find.assert_not_called()
                load.assert_not_called()

    def test_failed_ownership_lookup_never_loads_xlib(self):
        with fake_display() as (_lib, run, find, load):
            run.side_effect = x11.subprocess.TimeoutExpired("xdotool", 5)
            with self.assertRaises(x11.subprocess.TimeoutExpired):
                x11.read_pixels(123, "456", 0, 0, 1, 1)
            find.assert_not_called()
            load.assert_not_called()

    def test_user_desktop_and_wayland_are_rejected_before_lookup(self):
        for environment in ({}, {"DISPLAY": ":0"},
                            {**_PRIVATE_DISPLAY, "WAYLAND_DISPLAY": "wayland-0"},
                            {**_PRIVATE_DISPLAY, "XDG_RUNTIME_DIR": "/run/user/1000"}):
            with self.subTest(environment=environment), fake_display() as (_lib, run, find, load):
                with patch.dict(os.environ, environment, clear=True):
                    with self.assertRaisesRegex(RuntimeError, "private X11"):
                        x11.read_pixels(123, "456", 0, 0, 1, 1)
                run.assert_not_called()
                find.assert_not_called()
                load.assert_not_called()

    def test_missing_library_and_unopened_display_fail_without_cleanup_of_unowned_handles(self):
        with fake_display() as (library, _run, find, load):
            find.return_value = None
            with self.assertRaisesRegex(RuntimeError, "libX11"):
                x11.read_pixels(123, "456", 0, 0, 1, 1)
            load.assert_not_called()
            library.XCloseDisplay.assert_not_called()
        with fake_display() as (library, _run, _find, _load):
            library.XOpenDisplay.side_effect = None
            library.XOpenDisplay.return_value = None
            with self.assertRaisesRegex(RuntimeError, "Cannot open"):
                x11.read_pixels(123, "456", 0, 0, 1, 1)
            library.XSetErrorHandler.assert_not_called()
            library.XCloseDisplay.assert_not_called()

    def test_geometry_is_checked_before_get_image_and_grab_is_released(self):
        for rectangle in ((1279, 0, 2, 1), (0, 799, 1, 2), (1280, 0, 1, 1)):
            with self.subTest(rectangle=rectangle), fake_display() as (library, *_rest):
                with self.assertRaisesRegex(ValueError, "client geometry"):
                    x11.read_pixels(123, "456", *rectangle)
                library.XGetImage.assert_not_called()
                library.XDestroyImage.assert_not_called()
                library.XUngrabServer.assert_called_once_with(42)
                library.XCloseDisplay.assert_called_once_with(42)
                self.assertEqual(library.handler, 19)
        with fake_display() as (library, *_rest):
            self.assertEqual(x11.read_pixels(123, "456", 1279, 799, 1, 1).data, b"\x11\x22\x33")
            library.XGetImage.assert_called_once()

    def test_unviewable_input_only_and_unsupported_visuals_never_get_image(self):
        for field, value in (("map_state", 0), ("map_state", 1), ("class_", 2),
                             ("depth", 16), ("depth", 30), ("visual", None)):
            with self.subTest(field=field, value=value), fake_display() as (library, *_rest):
                setattr(library.attributes, field, value)
                with self.assertRaises(RuntimeError):
                    x11.read_pixels(123, "456", 0, 0, 1, 1)
                library.XGetImage.assert_not_called()
                library.XCloseDisplay.assert_called_once_with(42)
        for visual_class in (0, 1, 2, 3, 5):
            with self.subTest(visual_class=visual_class), fake_display() as (library, *_rest):
                library.visual.class_ = visual_class
                with self.assertRaisesRegex(RuntimeError, "TrueColor"):
                    x11.read_pixels(123, "456", 0, 0, 1, 1)
                library.XGetImage.assert_not_called()
                library.XCloseDisplay.assert_called_once_with(42)

    def test_failed_geometry_and_bad_window_are_nonfatal_and_close_display(self):
        for protocol_error in (False, True):
            with self.subTest(protocol_error=protocol_error), fake_display() as (library, *_rest):
                def failed_attributes(*_args):
                    if protocol_error:
                        library.protocol_error(code=3, request=3)
                    return 0
                library.XGetWindowAttributes.side_effect = failed_attributes
                with self.assertRaisesRegex(RuntimeError, "Cannot inspect"):
                    x11.read_pixels(123, "456", 0, 0, 1, 1)
                library.XGetImage.assert_not_called()
                library.XDestroyImage.assert_not_called()
                library.XUngrabServer.assert_called_once_with(42)
                library.XCloseDisplay.assert_called_once_with(42)
                self.assertEqual(library.handler, 19)

    def test_null_image_and_bad_match_are_nonfatal_and_always_release_resources(self):
        for protocol_error, returned_image in ((False, False), (True, False), (True, True)):
            with self.subTest(error=protocol_error, image=returned_image), fake_display() as (library, *_rest):
                def get_image(*_args):
                    if protocol_error:
                        library.protocol_error()
                    return library.pointer if returned_image else ctypes.POINTER(x11._XImage)()
                library.XGetImage.side_effect = get_image
                with self.assertRaisesRegex(RuntimeError, "failed|no image"):
                    x11.read_pixels(123, "456", 0, 0, 1, 1)
                self.assertEqual(library.XDestroyImage.call_count, int(returned_image))
                library.XUngrabServer.assert_called_once_with(42)
                library.XCloseDisplay.assert_called_once_with(42)
                self.assertEqual(library.handler, 19)

    def test_invalid_image_layout_is_rejected_before_reading_data(self):
        for field, value in (
            ("width", 2), ("height", 2), ("xoffset", 1), ("format", 1),
            ("byte_order", 2), ("depth", 16), ("depth", 30), ("depth", 32),
            ("bits_per_pixel", 16), ("bitmap_unit", 64), ("bitmap_bit_order", 2),
            ("bitmap_pad", 64), ("bytes_per_line", 0), ("bytes_per_line", 3),
            ("bytes_per_line", -4), ("bytes_per_line", x11.MAX_READ_BYTES + 4),
            ("data", None), ("red_mask", 0xff),
        ):
            with self.subTest(field=field, value=value), fake_display() as (library, *_rest):
                setattr(library.image, field, value)
                with patch.object(x11.c, "string_at") as read:
                    with self.assertRaises(RuntimeError):
                        x11.read_pixels(123, "456", 0, 0, 1, 1)
                    read.assert_not_called()
                library.XDestroyImage.assert_called_once_with(library.pointer)
                library.XCloseDisplay.assert_called_once_with(42)

    def test_invalid_masks_are_rejected_without_guessing_colours(self):
        for masks in ((0, 0xff00, 0xff), (0x1f0000, 0xff00, 0xff),
                      (0xff0000, 0xff0000, 0xff), (0xff00000000, 0xff00, 0xff),
                      (0xa50000, 0xff00, 0xff)):
            with self.subTest(masks=masks):
                library = FakeXlib(masks=masks)
                with fake_display(library), patch.object(x11.c, "string_at") as read:
                    with self.assertRaisesRegex(RuntimeError, "masks"):
                        x11.read_pixels(123, "456", 0, 0, 1, 1)
                    read.assert_not_called()
                library.XDestroyImage.assert_called_once_with(library.pointer)
                library.XCloseDisplay.assert_called_once_with(42)

    def test_default_null_error_handler_is_restored(self):
        with fake_display() as (library, *_rest):
            library.XSetErrorHandler.side_effect = [None, None]
            x11.read_pixels(123, "456", 0, 0, 1, 1)
            self.assertEqual(library.XSetErrorHandler.call_count, 2)
            self.assertEqual(library.XSetErrorHandler.call_args.args, (None,))
            library.XDestroyImage.assert_called_once_with(library.pointer)
            library.XCloseDisplay.assert_called_once_with(42)

    def test_error_handler_setup_failure_still_closes_display(self):
        with fake_display() as (library, *_rest):
            library.XSetErrorHandler.side_effect = RuntimeError("handler setup failed")
            with self.assertRaisesRegex(RuntimeError, "handler setup failed"):
                x11.read_pixels(123, "456", 0, 0, 1, 1)
            library.XSetErrorHandler.assert_called_once()
            library.XGrabServer.assert_not_called()
            library.XGetImage.assert_not_called()
            library.XDestroyImage.assert_not_called()
            library.XCloseDisplay.assert_called_once_with(42)

    def test_native_and_decode_exceptions_still_close_the_display(self):
        with fake_display() as (library, *_rest):
            library.XGetImage.side_effect = RuntimeError("native read failure")
            with self.assertRaisesRegex(RuntimeError, "native read failure"):
                x11.read_pixels(123, "456", 0, 0, 1, 1)
            library.XDestroyImage.assert_not_called()
            library.XUngrabServer.assert_called_once_with(42)
            library.XCloseDisplay.assert_called_once_with(42)
            self.assertEqual(library.handler, 19)
        with fake_display() as (library, *_rest), \
             patch.object(x11, "_decode_image", side_effect=RuntimeError("decode failure")):
            with self.assertRaisesRegex(RuntimeError, "decode failure"):
                x11.read_pixels(123, "456", 0, 0, 1, 1)
            library.XDestroyImage.assert_called_once_with(library.pointer)
            library.XCloseDisplay.assert_called_once_with(42)
            self.assertEqual(library.handler, 19)


if __name__ == "__main__":
    unittest.main()
