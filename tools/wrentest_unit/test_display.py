"""Regression checks for display ownership and smoke-test assertions."""

import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, call, patch
from xml.etree import ElementTree

from tools import run_display
from tools.wrentest import test_setup


class DisplayTests(unittest.TestCase):
    def test_owned_process_is_cleaned_up_after_interruption(self):
        process = Mock()
        process.wait.side_effect = KeyboardInterrupt
        with patch.object(run_display.subprocess, "Popen", return_value=process) as launch:
            with patch.object(run_display, "stop_owned_group") as stop:
                with self.assertRaises(KeyboardInterrupt):
                    run_display.run_owned(["example"], {})
                stop.assert_called_once_with(process)
        launch.assert_called_once_with(["example"], env={}, start_new_session=True)

    def test_cleanup_escalates_only_the_owned_process_group(self):
        process = Mock(pid=12345)
        process.wait.side_effect = [subprocess.TimeoutExpired("example", 1), 0]
        with patch.object(run_display.os, "killpg") as kill:
            run_display.stop_owned_group(process)
        self.assertEqual(kill.call_args_list, [
            call(12345, signal.SIGTERM), call(12345, signal.SIGKILL),
        ])
        self.assertEqual(process.wait.call_args_list, [call(timeout=1), call(timeout=5)])

    def test_cleanup_reaps_an_already_exited_process(self):
        process = Mock(pid=12345)
        with patch.object(run_display.os, "killpg", side_effect=ProcessLookupError):
            run_display.stop_owned_group(process)
        self.assertEqual(process.wait.call_count, 2)

    def test_ready_socket_is_a_real_unix_socket(self):
        with tempfile.TemporaryDirectory(prefix="wren-socket-") as root:
            path = Path(root) / "wayland-test"
            with socket.socket(socket.AF_UNIX) as server:
                server.bind(str(path))
                process = Mock()
                run_display.wait_for_socket(path, process, Path(root) / "unused.log")
                process.poll.assert_not_called()

    def test_a_regular_file_is_not_compositor_readiness(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "not-a-socket"
            path.touch()
            log = Path(root) / "weston.log"
            log.write_text("diagnostic output")
            process = Mock()
            process.poll.return_value = None
            with patch.object(run_display.time, "monotonic", side_effect=[0, 16]):
                with self.assertRaisesRegex(RuntimeError, "diagnostic output"):
                    run_display.wait_for_socket(path, process, log)
            self.assertTrue(log.exists())

    def test_compositor_exit_fails_without_waiting_for_timeout(self):
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "weston.log"
            log.write_text("failed to initialize")
            process = Mock()
            process.poll.return_value = 1
            with self.assertRaisesRegex(RuntimeError, "failed to initialize"):
                run_display.wait_for_socket(Path(root) / "missing", process, log)

    def test_private_bus_has_no_desktop_service_activation(self):
        with tempfile.TemporaryDirectory() as root:
            runtime = Path(root) / "runtime&test"
            runtime.mkdir()
            command = run_display.session_bus_command(runtime, ["example"])
            self.assertEqual(command, [
                "dbus-run-session", f"--config-file={runtime / 'dbus.conf'}", "--", "example",
            ])
            config = ElementTree.parse(runtime / "dbus.conf").getroot()
            self.assertEqual(config.findtext("listen"), f"unix:tmpdir={runtime}")
            self.assertIsNone(config.find("standard_session_servicedirs"))
            self.assertEqual(config.findall("servicedir"), [])

    def test_relative_artifact_directory_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "must be absolute"):
            run_display.artifact_directory("x11", {"WREN_TEST_ARTIFACT_DIR": "relative"})

    def test_child_exit_codes_are_preserved(self):
        for child, expected in ((0, 0), (42, 42), (-signal.SIGTERM, 143)):
            with self.subTest(child=child):
                with patch.object(run_display, "run_display", return_value=child):
                    with patch.object(run_display.signal, "signal"):
                        self.assertEqual(run_display.main(["x11", "--", "example"]), expected)


class SmokeAssertionTests(unittest.TestCase):
    def test_missing_or_unknown_renderer_cannot_pass(self):
        with patch.dict(os.environ, {"ICED_BACKEND": "wgpu"}, clear=True):
            for output in ("", "WREN_SMOKE_BACKEND=", "WREN_SMOKE_BACKEND=unknown"):
                with self.subTest(output=output), self.assertRaises(AssertionError):
                    test_setup.check_backend(output)

    def test_renderer_identity_is_checked_not_just_requested(self):
        for requested, reported in (("wgpu", "Vulkan"), ("tiny-skia", "tiny-skia")):
            with patch.dict(os.environ, {"ICED_BACKEND": requested}, clear=True):
                test_setup.check_backend(f"WREN_SMOKE_BACKEND={reported}")
                other = "tiny-skia" if requested == "wgpu" else "Vulkan"
                with self.assertRaises(AssertionError):
                    test_setup.check_backend(f"WREN_SMOKE_BACKEND={other}")

    def test_requested_vulkan_cannot_silently_fall_back_to_gl(self):
        with patch.dict(os.environ, {"ICED_BACKEND": "wgpu", "WGPU_BACKEND": "vulkan"}, clear=True):
            with self.assertRaises(AssertionError):
                test_setup.check_backend("WREN_SMOKE_BACKEND=Gl")

    def test_app_environment_cannot_inherit_wren_paths_or_user_state(self):
        with tempfile.TemporaryDirectory() as root:
            with patch.dict(os.environ, {
                "HOME": "/user/home", "WREN_CONFIG": "/user/config", "DISPLAY": ":test",
            }, clear=True):
                env = test_setup.isolated_environment(Path(root))
            self.assertNotIn("WREN_CONFIG", env)
            self.assertEqual(env["WREN_TEST_MODE"], "1")
            self.assertEqual(env["DISPLAY"], ":test")
            for variable in ("HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"):
                self.assertTrue(Path(env[variable]).is_relative_to(root))
                self.assertTrue(Path(env[variable]).is_dir())

    def test_blank_capture_does_not_pass_as_rendered_text(self):
        background = test_setup.BACKGROUNDS["dark"]
        image = test_setup.png.PNG(640, 400, bytes((*background, 255)) * (640 * 400))
        with self.assertRaisesRegex(AssertionError, "blank"):
            test_setup.check_pixels(image, background)


if __name__ == "__main__":
    unittest.main()
