"""Regression checks for display ownership and smoke-test assertions."""

import io
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import Mock, call, patch
from xml.etree import ElementTree

from tools import run_display
from tools.wrentest import test_setup, ui


class DisplayTests(unittest.TestCase):
    def test_owned_process_is_cleaned_up_after_interruption(self):
        process = Mock()
        with patch.object(run_display.subprocess, "Popen", return_value=process) as launch:
            with patch.object(run_display, "wait_exited", side_effect=KeyboardInterrupt):
                with patch.object(run_display, "stop_owned_group") as stop:
                    with self.assertRaises(KeyboardInterrupt):
                        run_display.run_owned(["example"], {})
                    stop.assert_called_once_with(process)
        launch.assert_called_once_with(["example"], env={}, start_new_session=True)

    def test_cleanup_signals_the_group_before_reaping_its_leader(self):
        process = Mock(pid=12345, returncode=None)
        with patch.object(run_display, "wait_exited", return_value=False) as waited, \
                patch.object(run_display.os, "killpg") as kill:
            run_display.stop_owned_group(process)
        self.assertEqual(kill.call_args_list, [
            call(12345, signal.SIGTERM), call(12345, signal.SIGKILL),
        ])
        waited.assert_called_once_with(process, timeout=1)
        process.wait.assert_called_once_with(timeout=5)

    def test_a_reaped_process_group_is_never_signalled(self):
        process = Mock(pid=12345, returncode=0)
        with patch.object(run_display.os, "killpg") as kill:
            run_display.stop_owned_group(process)
        kill.assert_not_called()
        process.wait.assert_called_once_with(timeout=5)

    def test_a_real_group_is_stopped_and_its_leader_reaped(self):
        process = subprocess.Popen(["sleep", "60"], start_new_session=True)
        run_display.stop_owned_group(process)
        self.assertEqual(process.returncode, -signal.SIGTERM)

    def test_ready_means_listening_not_only_bound(self):
        with tempfile.TemporaryDirectory(prefix="wren-socket-") as root:
            path = Path(root) / "wayland-test"
            with socket.socket(socket.AF_UNIX) as server:
                server.bind(str(path))
                self.assertFalse(run_display.accepts(path))
                server.listen()
                process = Mock(returncode=None)
                run_display.wait_for_socket(path, process, Path(root) / "unused.log")

    def test_a_regular_file_is_not_compositor_readiness(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "not-a-socket"
            path.touch()
            log = Path(root) / "weston.log"
            log.write_text("diagnostic output")
            with patch.object(run_display, "has_exited", return_value=False), \
                    patch.object(run_display.time, "monotonic", side_effect=[0, 16]):
                with self.assertRaisesRegex(RuntimeError, "diagnostic output"):
                    run_display.wait_for_socket(path, Mock(), log)
            self.assertTrue(log.exists())

    def test_compositor_exit_fails_without_waiting_for_timeout(self):
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "weston.log"
            log.write_text("failed to initialize")
            process = Mock(returncode=1)
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


# A stand-in compositor: listens on the socket path it is given until killed.
LISTENER = (
    "import socket, sys, time\n"
    "server = socket.socket(socket.AF_UNIX)\n"
    "server.bind(sys.argv[1])\n"
    "server.listen()\n"
    "time.sleep(60)\n"
)


class CompositorTests(unittest.TestCase):
    """The watcher with real processes: listeners for a Weston that comes up,
    and a process that exits for one that cannot."""

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="wren-c-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.socket = self.root / "wayland-test"
        self.stderr = io.StringIO()
        stderr = patch.object(run_display.sys, "stderr", self.stderr)
        stderr.start()
        self.addCleanup(stderr.stop)

    def compositor(self, *kinds: str) -> run_display.Compositor:
        commands = iter([
            [sys.executable, "-I", "-c", LISTENER, str(self.socket)] if kind == "listens"
            else [sys.executable, "-I", "-c", "raise SystemExit(3)"]
            for kind in kinds
        ])
        compositor = run_display.Compositor(self.root, self.socket, {}, command=lambda: next(commands))
        compositor.POLL = 0.02
        self.addCleanup(compositor.stop)
        return compositor

    def wait(self, predicate, what: str) -> None:
        deadline = time.monotonic() + 20
        while not predicate():
            self.assertLess(time.monotonic(), deadline, f"timed out waiting for {what}")
            time.sleep(0.02)

    def test_a_failed_replacement_is_reported_and_the_next_attempt_runs(self):
        compositor = self.compositor("listens", "exits", "listens")
        compositor.start()
        self.assertTrue(run_display.accepts(self.socket))
        first = compositor.process
        os.kill(first.pid, signal.SIGKILL)
        self.wait(lambda: compositor.attempts == 2 and compositor.process is not None
                  and compositor.process is not first and run_display.accepts(self.socket),
                  "the second attempt listening")
        replacement = compositor.process
        self.assertEqual(first.returncode, -signal.SIGKILL)
        compositor.stop()
        self.assertIsNotNone(replacement.returncode)
        self.assertFalse(compositor.watcher.is_alive())
        self.assertFalse(compositor.lost)
        lines = self.stderr.getvalue().splitlines()
        self.assertIn("Weston exited with signal SIGKILL", lines[0])
        self.assertIn("restart 1 of 3 failed: Weston did not become ready", lines[1])
        self.assertIn("Weston restarted (2 of 3)", lines[2])
        self.assertEqual(len(lines), 3)

    def test_when_restarts_run_out_the_command_is_stopped_and_the_run_lost(self):
        compositor = self.compositor("listens", "exits", "exits", "exits")
        compositor.start()
        command = subprocess.Popen(["sleep", "60"], start_new_session=True)
        self.addCleanup(lambda: command.poll() is None and command.kill())
        compositor.supervise(command)
        os.kill(compositor.process.pid, signal.SIGKILL)
        self.assertEqual(command.wait(timeout=20), -signal.SIGTERM)
        self.wait(lambda: not compositor.watcher.is_alive(), "the watcher to finish")
        self.assertTrue(compositor.lost)
        self.assertEqual(compositor.attempts, 3)
        self.assertIn("stopping the run", self.stderr.getvalue())

    def test_a_compositor_stopped_on_purpose_is_not_restarted(self):
        compositor = self.compositor("listens")
        compositor.start()
        process = compositor.process
        compositor.stop()
        self.assertEqual(process.returncode, -signal.SIGTERM)
        self.assertFalse(compositor.watcher.is_alive())
        self.assertEqual((compositor.attempts, self.stderr.getvalue()), (0, ""))

    def test_a_launch_waits_for_a_compositor_starting_again(self):
        env = {"WAYLAND_DISPLAY": self.socket.name, "XDG_RUNTIME_DIR": str(self.root)}
        with socket.socket(socket.AF_UNIX) as server:
            server.bind(str(self.socket))
            # Bound but not listening, as a restarting Weston's socket can be.
            timer = threading.Timer(0.3, server.listen)
            # Timed from before the timer starts, so a pause in this thread
            # cannot shorten the measured wait below the timer's delay.
            started = time.monotonic()
            timer.start()
            self.addCleanup(timer.cancel)
            ui._wait_for_compositor(env, timeout=10)
            self.assertGreaterEqual(time.monotonic() - started, 0.25)

    def test_a_launch_fails_clearly_when_no_compositor_comes_back(self):
        env = {"WAYLAND_DISPLAY": self.socket.name, "XDG_RUNTIME_DIR": str(self.root)}
        with self.assertRaisesRegex(RuntimeError, "No Wayland compositor accepted"):
            ui._wait_for_compositor(env, timeout=0.3)


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
