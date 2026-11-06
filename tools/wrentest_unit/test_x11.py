"""Validate native close-message construction without accessing a real display."""

import ctypes
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


if __name__ == "__main__":
    unittest.main()
