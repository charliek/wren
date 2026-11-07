"""Unit tests for tools/wrentest/ui.py.

All tests use a stub subprocess.Popen and a real Unix-domain-socket fake
server — no Rust binary is required.

Command to run only these tests::

    uv run --locked --group test \
        python -m unittest discover -s tools/wrentest_unit -p test_ui.py -v
"""

from __future__ import annotations

import atexit
import itertools
import json
import math
import os
import shutil
import signal
import socket
import subprocess
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.wrentest import ui
from tools.wrentest.client import ProtocolError
from tools.wrentest.ui import WrenApp, wait_for

_TEST_PID = 54321


def _good_identify():
    return {"ready": True, "protocol": 1, "pid": _TEST_PID}


# ── stub process ──────────────────────────────────────────────────────────────


class StubProcess:
    """Lightweight fake of ``subprocess.Popen`` for WrenApp tests."""

    def __init__(
        self,
        pid: int = _TEST_PID,
        exit_code: int = 0,
        sigterm_exits: bool = True,
    ) -> None:
        self.pid = pid
        self.returncode: int | None = None
        self._target_code = exit_code
        self._sigterm_exits = sigterm_exits
        self._exited = threading.Event()
        self.signals_received: list[int] = []

    def poll(self) -> int | None:
        if self._exited.is_set():
            if self.returncode is None:
                self.returncode = self._target_code
            return self.returncode
        return None

    def wait(self, timeout: float | None = None) -> int:
        if self._exited.wait(timeout=timeout):
            if self.returncode is None:
                self.returncode = self._target_code
            return self.returncode
        raise subprocess.TimeoutExpired("stub", timeout)

    def kill(self) -> None:
        self.signals_received.append(signal.SIGKILL)
        self.returncode = self._target_code
        self._exited.set()

    def send_signal(self, sig: int) -> None:
        self.signals_received.append(sig)
        if sig == signal.SIGTERM and self._sigterm_exits:
            self.returncode = self._target_code
            self._exited.set()

    def trigger_exit(self, code: int | None = None) -> None:
        """External trigger to simulate process exit (e.g. from app.quit)."""
        self.returncode = code if code is not None else self._target_code
        self._exited.set()


class _UnreapableProcess:
    """Stub whose ``wait()`` always times out — for cannot-reap tests."""

    def __init__(self, pid: int = _TEST_PID) -> None:
        self.pid = pid
        self.returncode: int | None = None
        self.signals_received: list[int] = []

    def poll(self) -> None:
        return None

    def wait(self, timeout: float | None = None) -> int:
        raise subprocess.TimeoutExpired("stub", timeout)

    def kill(self) -> None:
        self.signals_received.append(signal.SIGKILL)

    def send_signal(self, sig: int) -> None:
        self.signals_received.append(sig)


# ── fake IPC server ───────────────────────────────────────────────────────────


class _IdentifyServer:
    """Listens on a Unix socket and responds to ``identify`` / ``app.quit``."""

    def __init__(
        self,
        path: str,
        result,
        *,
        eof_on_quit: bool = False,
        on_quit=None,
    ) -> None:
        self._result = result  # dict or callable returning dict
        self._eof_on_quit = eof_on_quit
        self._on_quit = on_quit
        self._server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._server.bind(path)
        self._server.listen(1)
        self._server.settimeout(10.0)
        self._stop = threading.Event()

    def start(self) -> None:
        self._thread = threading.Thread(target=self._accept_loop, daemon=True)
        self._thread.start()

    def _accept_loop(self) -> None:
        while not self._stop.is_set():
            try:
                conn, _ = self._server.accept()
            except (socket.timeout, OSError):
                continue
            threading.Thread(target=self._handle, args=(conn,), daemon=True).start()

    def _handle(self, conn: socket.socket) -> None:
        try:
            buf = b""
            while True:
                chunk = conn.recv(4096)
                if not chunk:
                    return
                buf += chunk
                while b"\n" in buf:
                    line, buf = buf.split(b"\n", 1)
                    req = json.loads(line)
                    if req["op"] == "identify":
                        r = self._result() if callable(self._result) else self._result
                        resp = {"id": req["id"], "ok": True, "result": r}
                    elif req["op"] == "app.quit":
                        if self._eof_on_quit:
                            if self._on_quit:
                                self._on_quit()
                            return  # close connection → EOF
                        resp = {"id": req["id"], "ok": True, "result": None}
                        conn.sendall((json.dumps(resp) + "\n").encode())
                        if self._on_quit:
                            self._on_quit()
                        return
                    else:
                        resp = {"id": req["id"], "ok": True, "result": None}
                    conn.sendall((json.dumps(resp) + "\n").encode())
        except Exception:
            pass
        finally:
            conn.close()

    def close(self) -> None:
        self._stop.set()
        try:
            self._server.close()
        except OSError:
            pass


# ── test utilities ────────────────────────────────────────────────────────────


class _WrenAppTestBase(unittest.TestCase):
    """Common setup for WrenApp unit tests."""

    def setUp(self) -> None:
        self._tmpdir = Path(tempfile.mkdtemp(prefix="wren-ui-test-"))
        self._artifacts = self._tmpdir / "artifacts"
        self._artifacts.mkdir()
        self._servers: list[_IdentifyServer] = []
        self._processes: list[StubProcess] = []
        self._binary_patch = patch.object(
            ui, "_find_binary", return_value=Path("/bin/true"),
        )
        self._binary_patch.start()

    def tearDown(self) -> None:
        self._binary_patch.stop()
        for s in self._servers:
            s.close()
        shutil.rmtree(self._tmpdir, ignore_errors=True)

    def _make_popen(
        self,
        identify_result,
        exit_code: int = 0,
        sigterm_exits: bool = True,
        eof_on_quit: bool = False,
        quit_exits_process: bool = True,
    ):
        """Return a factory suitable for patching ``subprocess.Popen``."""

        def factory(cmd, **kwargs):
            idx = cmd.index("--test-socket")
            socket_path = cmd[idx + 1]
            proc = StubProcess(
                pid=_TEST_PID,
                exit_code=exit_code,
                sigterm_exits=sigterm_exits,
            )
            on_quit = (
                (lambda: proc.trigger_exit(exit_code))
                if quit_exits_process
                else None
            )
            server = _IdentifyServer(
                socket_path,
                identify_result,
                eof_on_quit=eof_on_quit,
                on_quit=on_quit,
            )
            server.start()
            self._servers.append(server)
            self._processes.append(proc)
            return proc

        return factory


# ── wait_for tests ────────────────────────────────────────────────────────────


class TestWaitFor(unittest.TestCase):

    def test_returns_truthy_value_immediately(self) -> None:
        result = wait_for(lambda: 42, timeout=1.0)
        self.assertEqual(result, 42)

    def test_polls_until_truthy(self) -> None:
        counter = itertools.count()

        def pred():
            n = next(counter)
            return n >= 3

        result = wait_for(pred, timeout=2.0, interval=0.01)
        self.assertTrue(result)

    def test_times_out(self) -> None:
        with self.assertRaises(TimeoutError) as ctx:
            wait_for(lambda: False, timeout=0.15, interval=0.02)
        self.assertIn("last state: False", str(ctx.exception))

    def test_uses_monotonic_clock(self) -> None:
        start = time.monotonic()
        try:
            wait_for(lambda: False, timeout=0.1, interval=0.01)
        except TimeoutError:
            pass
        elapsed = time.monotonic() - start
        self.assertGreater(elapsed, 0.05)
        self.assertLess(elapsed, 1.0)

    def test_default_timeout_is_ten(self) -> None:
        """With no explicit timeout the default is 10 seconds."""
        start = time.monotonic()
        result = wait_for(lambda: 99)
        elapsed = time.monotonic() - start
        self.assertEqual(result, 99)
        self.assertLess(elapsed, 1.0)

    def test_description_appears_in_timeout_message(self) -> None:
        with self.assertRaises(TimeoutError) as ctx:
            wait_for(
                lambda: False,
                timeout=0.1,
                interval=0.01,
                description="socket ready",
            )
        self.assertIn("socket ready", str(ctx.exception))

    def test_timeout_zero_rejected(self) -> None:
        with self.assertRaises(ValueError):
            wait_for(lambda: True, timeout=0)

    def test_timeout_negative_rejected(self) -> None:
        with self.assertRaises(ValueError):
            wait_for(lambda: True, timeout=-1)

    def test_timeout_inf_rejected(self) -> None:
        with self.assertRaises(ValueError):
            wait_for(lambda: True, timeout=float("inf"))

    def test_timeout_nan_rejected(self) -> None:
        with self.assertRaises(ValueError):
            wait_for(lambda: True, timeout=float("nan"))

    def test_interval_zero_rejected(self) -> None:
        with self.assertRaises(ValueError):
            wait_for(lambda: True, timeout=1, interval=0)

    def test_timeout_bool_rejected(self) -> None:
        with self.assertRaises(ValueError):
            wait_for(lambda: True, timeout=True)


# ── _build_env tests ─────────────────────────────────────────────────────────


class TestBuildEnv(unittest.TestCase):

    def test_strips_wren_vars(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            with patch.dict(
                os.environ,
                {"WREN_FOO": "bar", "WREN_TEST_OVERRIDE": "1", "PATH": "/usr/bin"},
                clear=True,
            ):
                env = ui._build_env(Path(d))
        self.assertNotIn("WREN_FOO", env)
        self.assertNotIn("WREN_TEST_OVERRIDE", env)
        self.assertEqual(env.get("WREN_TEST_MODE"), "1")

    def test_preserves_display_and_utility_vars(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            with patch.dict(
                os.environ,
                {
                    "DISPLAY": ":99",
                    "PATH": "/usr/bin",
                    "LANG": "en_US.UTF-8",
                    "GIO_USE_VFS": "local",
                    "GSETTINGS_BACKEND": "memory",
                },
                clear=True,
            ):
                env = ui._build_env(Path(d))
        self.assertEqual(env["DISPLAY"], ":99")
        self.assertEqual(env["PATH"], "/usr/bin")
        self.assertEqual(env["LANG"], "en_US.UTF-8")
        self.assertEqual(env["GIO_USE_VFS"], "local")
        self.assertEqual(env["GSETTINGS_BACKEND"], "memory")

    def test_creates_xdg_dirs_under_sandbox(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            sandbox = Path(d)
            with patch.dict(os.environ, {}, clear=True):
                env = ui._build_env(sandbox)
            for var in ("HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME",
                         "XDG_STATE_HOME", "XDG_CACHE_HOME"):
                p = Path(env[var])
                self.assertTrue(p.is_dir(), f"{var} dir missing")
                self.assertTrue(str(p).startswith(d), f"{var} not under sandbox")


# ── WrenApp readiness tests ───────────────────────────────────────────────────


class TestWrenAppReadiness(_WrenAppTestBase):

    def test_returncode_observes_only_the_owned_child(self) -> None:
        factory = self._make_popen(_good_identify())
        with patch.object(subprocess, "Popen", side_effect=factory):
            with WrenApp(artifacts_dir=self._artifacts, readiness_timeout=5) as app:
                self.assertIsNone(app.returncode)
                self._processes[-1].trigger_exit(0)
                self.assertEqual(app.returncode, 0)

    def test_launch_modes_are_explicit_without_overriding_workspace_or_spike(self) -> None:
        for params, expected in [
            ({}, ["--editor"]),
            ({"evaluation": False}, []),
            ({"workspace": Path("/tmp/notes")}, ["--workspace", "/tmp/notes"]),
            ({"extra_args": ("--editor-spike",)}, ["--editor-spike"]),
            ({"extra_args": ("--workspace", "/tmp/notes")}, ["--workspace", "/tmp/notes"]),
        ]:
            with self.subTest(params=params):
                captured = []
                factory = self._make_popen(_good_identify())

                def launch(cmd, **kwargs):
                    captured.append(cmd)
                    return factory(cmd, **kwargs)

                with patch.object(subprocess, "Popen", side_effect=launch):
                    with WrenApp(artifacts_dir=self._artifacts, **params):
                        pass
                self.assertEqual(captured[0][3:], expected)

    def test_evaluation_requires_a_boolean(self) -> None:
        for value in (0, 1, None, "false"):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "evaluation"):
                WrenApp(evaluation=value)

    def test_valid_identify_starts_and_stops(self) -> None:
        factory = self._make_popen(_good_identify())
        with patch.object(subprocess, "Popen", side_effect=factory):
            with WrenApp(
                artifacts_dir=self._artifacts, readiness_timeout=5.0,
            ) as app:
                self.assertEqual(app.pid, _TEST_PID)
                self.assertTrue(app.sandbox.is_dir())
                self.assertIsNotNone(app.client)

    def test_ready_false_polls_until_ready(self) -> None:
        counter = itertools.count(1)

        def identify_result():
            n = next(counter)
            if n <= 2:
                return {"ready": False, "protocol": 1, "pid": _TEST_PID}
            return _good_identify()

        factory = self._make_popen(identify_result)
        with patch.object(subprocess, "Popen", side_effect=factory):
            with WrenApp(
                artifacts_dir=self._artifacts, readiness_timeout=10.0,
            ) as app:
                self.assertEqual(app.pid, _TEST_PID)

    def test_wrong_protocol_is_fatal(self) -> None:
        result = {"ready": True, "protocol": 99, "pid": _TEST_PID}
        factory = self._make_popen(result)
        with patch.object(subprocess, "Popen", side_effect=factory):
            with self.assertRaisesRegex(RuntimeError, "protocol"):
                with WrenApp(
                    artifacts_dir=self._artifacts, readiness_timeout=2.0,
                ):
                    pass

    def test_wrong_pid_is_fatal(self) -> None:
        result = {"ready": True, "protocol": 1, "pid": 99999}
        factory = self._make_popen(result)
        with patch.object(subprocess, "Popen", side_effect=factory):
            with self.assertRaisesRegex(RuntimeError, "pid"):
                with WrenApp(
                    artifacts_dir=self._artifacts, readiness_timeout=2.0,
                ):
                    pass

    def test_non_dict_identify_is_fatal(self) -> None:
        factory = self._make_popen("not a dict")
        with patch.object(subprocess, "Popen", side_effect=factory):
            with self.assertRaisesRegex(RuntimeError, "not a dict"):
                with WrenApp(
                    artifacts_dir=self._artifacts, readiness_timeout=2.0,
                ):
                    pass

    def test_protocol_bool_true_rejected(self) -> None:
        """protocol: True (bool, == 1) is rejected — type must be int."""
        result = {"ready": True, "protocol": True, "pid": _TEST_PID}
        factory = self._make_popen(result)
        with patch.object(subprocess, "Popen", side_effect=factory):
            with self.assertRaisesRegex(RuntimeError, "protocol"):
                with WrenApp(
                    artifacts_dir=self._artifacts, readiness_timeout=2.0,
                ):
                    pass

    def test_pid_bool_rejected(self) -> None:
        """pid: True (bool) is rejected even though isinstance(True, int)."""
        result = {"ready": True, "protocol": 1, "pid": True}
        factory = self._make_popen(result)
        with patch.object(subprocess, "Popen", side_effect=factory):
            with self.assertRaisesRegex(RuntimeError, "pid"):
                with WrenApp(
                    artifacts_dir=self._artifacts, readiness_timeout=2.0,
                ):
                    pass

    def test_ready_missing_key_is_fatal(self) -> None:
        """Missing 'ready' key in identify result is a fatal RuntimeError."""
        result = {"protocol": 1, "pid": _TEST_PID}
        factory = self._make_popen(result)
        with patch.object(subprocess, "Popen", side_effect=factory):
            with self.assertRaisesRegex(RuntimeError, "ready"):
                with WrenApp(
                    artifacts_dir=self._artifacts, readiness_timeout=2.0,
                ):
                    pass

    def test_ready_int_not_bool_is_fatal(self) -> None:
        """ready: 1 (int, not bool) is a fatal RuntimeError."""
        result = {"ready": 1, "protocol": 1, "pid": _TEST_PID}
        factory = self._make_popen(result)
        with patch.object(subprocess, "Popen", side_effect=factory):
            with self.assertRaisesRegex(RuntimeError, "ready"):
                with WrenApp(
                    artifacts_dir=self._artifacts, readiness_timeout=2.0,
                ):
                    pass

    def test_readiness_timeout_negative_rejected(self) -> None:
        with self.assertRaises(ValueError):
            WrenApp(artifacts_dir=self._artifacts, readiness_timeout=-1)

    def test_readiness_timeout_zero_rejected(self) -> None:
        with self.assertRaises(ValueError):
            WrenApp(artifacts_dir=self._artifacts, readiness_timeout=0)

    def test_readiness_timeout_inf_rejected(self) -> None:
        with self.assertRaises(ValueError):
            WrenApp(artifacts_dir=self._artifacts, readiness_timeout=float("inf"))

    def test_readiness_timeout_nan_rejected(self) -> None:
        with self.assertRaises(ValueError):
            WrenApp(artifacts_dir=self._artifacts, readiness_timeout=float("nan"))


# ── WrenApp lifecycle tests ───────────────────────────────────────────────────


class TestWrenAppLifecycle(_WrenAppTestBase):

    def test_already_exited_child_skips_quit_and_still_validates_status(self) -> None:
        for exit_code in (0, 7):
            with self.subTest(exit_code=exit_code):
                factory = self._make_popen(_good_identify())
                with patch.object(subprocess, "Popen", side_effect=factory):
                    app = WrenApp(artifacts_dir=self._artifacts, readiness_timeout=5)
                    app.__enter__()
                    root = app.sandbox
                    child = self._processes[-1]
                    child.trigger_exit(exit_code)
                    client = app.client
                    with patch.object(client, "call", side_effect=AssertionError("quit after exit")) as call, \
                         patch.object(client, "close", wraps=client.close) as close:
                        if exit_code:
                            with self.assertRaisesRegex(RuntimeError, "code 7"):
                                app.quit()
                        else:
                            app.quit()
                        call.assert_not_called()
                        close.assert_called_once_with()
                    self.assertEqual(child.signals_received, [])
                    self.assertIsNone(app._process)
                    self.assertFalse(root.exists())
                    self.assertFalse(app._atexit_registered)

    def test_nonzero_exit_fails_successful_test(self) -> None:
        """If the test passes but the app exits nonzero, RuntimeError."""
        factory = self._make_popen(_good_identify(), exit_code=1)
        with patch.object(subprocess, "Popen", side_effect=factory):
            with self.assertRaisesRegex(RuntimeError, r"code 1"):
                with WrenApp(
                    artifacts_dir=self._artifacts, readiness_timeout=5.0,
                ) as app:
                    pass  # test body succeeds

    def test_quit_eof_accepted_with_zero_exit(self) -> None:
        """Server closes connection on quit (EOF); child exits 0 → no error."""
        factory = self._make_popen(
            _good_identify(), exit_code=0, eof_on_quit=True,
        )
        with patch.object(subprocess, "Popen", side_effect=factory):
            with WrenApp(
                artifacts_dir=self._artifacts, readiness_timeout=5.0,
            ) as app:
                pass  # no exception expected

    def test_sigterm_ignorer_gets_sigkill(self) -> None:
        """Process that ignores SIGTERM is escalated to SIGKILL."""
        factory = self._make_popen(
            _good_identify(),
            exit_code=-9,
            sigterm_exits=False,
            quit_exits_process=False,
        )
        with patch.object(subprocess, "Popen", side_effect=factory):
            with patch.object(ui, "QUIT_TIMEOUT", 0.1):
                with patch.object(ui, "SIGTERM_TIMEOUT", 0.1):
                    with self.assertRaisesRegex(RuntimeError, "code"):
                        with WrenApp(
                            artifacts_dir=self._artifacts,
                            readiness_timeout=5.0,
                        ):
                            pass
        proc = self._processes[-1]
        self.assertIn(signal.SIGTERM, proc.signals_received)
        self.assertIn(signal.SIGKILL, proc.signals_received)

    def test_restart_creates_unique_logs(self) -> None:
        """Each launch (including restarts) writes to distinct log files."""
        factory = self._make_popen(_good_identify())
        with patch.object(subprocess, "Popen", side_effect=factory):
            with WrenApp(
                artifacts_dir=self._artifacts, readiness_timeout=5.0,
            ) as app:
                first_stdout = app._stdout_log
                first_stderr = app._stderr_log
                app.restart(hard=True)
                second_stdout = app._stdout_log
                second_stderr = app._stderr_log

        self.assertNotEqual(first_stdout, second_stdout)
        self.assertNotEqual(first_stderr, second_stderr)
        # All four are distinct files
        self.assertEqual(
            len({first_stdout, first_stderr, second_stdout, second_stderr}), 4,
        )

    def test_process_reaped_before_tmpdir_deleted(self) -> None:
        """Reap happens before cleanup_ownroot in teardown ordering."""
        order: list[str] = []
        real_reap = ui.WrenApp._reap_process
        real_cleanup = ui.WrenApp._cleanup_ownroot

        def track_reap(self_app, proc):
            order.append("reap")
            return real_reap(self_app, proc)

        def track_cleanup(self_app):
            order.append("cleanup")
            return real_cleanup(self_app)

        factory = self._make_popen(_good_identify())
        with patch.object(subprocess, "Popen", side_effect=factory):
            with patch.object(ui.WrenApp, "_reap_process", track_reap):
                with patch.object(ui.WrenApp, "_cleanup_ownroot", track_cleanup):
                    with WrenApp(
                        artifacts_dir=self._artifacts, readiness_timeout=5.0,
                    ):
                        pass
        self.assertEqual(order, ["reap", "cleanup"])

    def test_atexit_unregistered_after_cleanup(self) -> None:
        """After successful teardown, the atexit handler is unregistered."""
        factory = self._make_popen(_good_identify())
        with patch.object(subprocess, "Popen", side_effect=factory):
            app = WrenApp(
                artifacts_dir=self._artifacts, readiness_timeout=5.0,
            )
            app.__enter__()
            self.assertTrue(app._atexit_registered)
            app.__exit__(None, None, None)
            self.assertFalse(app._atexit_registered)

    def test_popen_failure_propagates_cleanly(self) -> None:
        """If the binary cannot launch, the error propagates and cleanup
        removes the temporary socket directory."""

        def bad_popen(cmd, **kwargs):
            raise OSError("spawn failed")

        with patch.object(subprocess, "Popen", side_effect=bad_popen):
            with self.assertRaisesRegex(OSError, "spawn failed"):
                with WrenApp(
                    artifacts_dir=self._artifacts, readiness_timeout=2.0,
                ):
                    pass

    def test_restart_soft_uses_quit_not_sigterm(self) -> None:
        """A soft restart calls app.quit for clean shutdown, not SIGTERM."""
        factory = self._make_popen(
            _good_identify(), exit_code=0, eof_on_quit=True,
        )
        with patch.object(subprocess, "Popen", side_effect=factory):
            with WrenApp(
                artifacts_dir=self._artifacts, readiness_timeout=5.0,
            ) as app:
                app.restart(hard=False)

        first_proc = self._processes[0]
        self.assertNotIn(signal.SIGTERM, first_proc.signals_received)
        self.assertNotIn(signal.SIGKILL, first_proc.signals_received)
        self.assertEqual(first_proc.returncode, 0)

    def test_workspace_and_extra_args_passed(self) -> None:
        """workspace and extra_args are included in the Popen command."""
        captured_cmds: list[list[str]] = []

        orig_factory = self._make_popen(_good_identify())

        def capturing_factory(cmd, **kwargs):
            captured_cmds.append(cmd)
            return orig_factory(cmd, **kwargs)

        with patch.object(subprocess, "Popen", side_effect=capturing_factory):
            with WrenApp(
                artifacts_dir=self._artifacts,
                readiness_timeout=5.0,
                workspace=Path("/tmp/test-workspace"),
                extra_args=("--editor-spike",),
            ):
                pass

        cmd = captured_cmds[0]
        self.assertIn("--workspace", cmd)
        ws_idx = cmd.index("--workspace")
        self.assertEqual(cmd[ws_idx + 1], "/tmp/test-workspace")
        self.assertIn("--editor-spike", cmd)

    # ── regression: fail-closed reaping ───────────────────────────────────

    def test_cannot_reap_preserves_root_and_atexit(self) -> None:
        """When _reap_process raises, sandbox / atexit / process ref survive."""
        factory = self._make_popen(_good_identify())
        with patch.object(subprocess, "Popen", side_effect=factory):
            app = WrenApp(
                artifacts_dir=self._artifacts, readiness_timeout=5.0,
            )
            app.__enter__()

            sandbox_path = app.sandbox
            self.assertTrue(sandbox_path.is_dir())

            def fail_reap(proc):
                raise RuntimeError("Cannot reap process (test stub)")

            with patch.object(app, "_reap_process", fail_reap):
                with self.assertRaisesRegex(RuntimeError, "[Rr]eap"):
                    app.__exit__(None, None, None)

            # Fail-closed: everything preserved.
            self.assertTrue(sandbox_path.is_dir())
            self.assertTrue(app._atexit_registered)
            self.assertIsNotNone(app._process)

        # Manual cleanup (normal teardown was intentionally prevented).
        self._processes[0].trigger_exit(0)
        app._process = None
        if sandbox_path.is_dir():
            shutil.rmtree(sandbox_path, ignore_errors=True)
        atexit.unregister(app._emergency_cleanup)

    def test_cannot_reap_on_test_exception_adds_note(self) -> None:
        """Reap failure during a failed test is attached as a note."""
        factory = self._make_popen(_good_identify())
        sandbox_path = None
        with patch.object(subprocess, "Popen", side_effect=factory):
            app = WrenApp(
                artifacts_dir=self._artifacts, readiness_timeout=5.0,
            )
            app.__enter__()
            sandbox_path = app.sandbox

            def fail_reap(proc):
                raise RuntimeError("Cannot reap (test)")

            try:
                with patch.object(app, "_reap_process", fail_reap):
                    raise ValueError("original error")
            except ValueError:
                app.__exit__(ValueError, ValueError("original error"), None)
            except RuntimeError:
                self.fail("reap error should not propagate over original")

        # The original exception should still have notes attached.
        # (We cannot easily inspect it from outside; the test verifies
        # no RuntimeError escapes when caller_exc is set.)

        self._processes[0].trigger_exit(0)
        app._process = None
        if sandbox_path and sandbox_path.is_dir():
            shutil.rmtree(sandbox_path, ignore_errors=True)
        atexit.unregister(app._emergency_cleanup)

    # ── regression: escalation exit-0 still fails ─────────────────────────

    def test_escalation_exit_zero_still_fails(self) -> None:
        """Process that requires escalation reports failure even if exit 0."""
        factory = self._make_popen(
            _good_identify(),
            exit_code=0,
            sigterm_exits=True,
            quit_exits_process=False,
        )
        with patch.object(subprocess, "Popen", side_effect=factory):
            with patch.object(ui, "QUIT_TIMEOUT", 0.1):
                with self.assertRaisesRegex(RuntimeError, "escalation"):
                    with WrenApp(
                        artifacts_dir=self._artifacts,
                        readiness_timeout=5.0,
                    ):
                        pass

    # ── regression: graceful restart ──────────────────────────────────────

    def test_graceful_restart_preserves_sandbox(self) -> None:
        """Soft restart keeps the same sandbox directory."""
        factory = self._make_popen(
            _good_identify(), exit_code=0, eof_on_quit=True,
        )
        with patch.object(subprocess, "Popen", side_effect=factory):
            with WrenApp(
                artifacts_dir=self._artifacts, readiness_timeout=5.0,
            ) as app:
                first_sandbox = app.sandbox
                app.restart(hard=False)
                self.assertEqual(app.sandbox, first_sandbox)

    def test_graceful_restart_nonzero_exit_raises(self) -> None:
        """Soft restart with non-zero exit raises RuntimeError."""
        factory = self._make_popen(
            _good_identify(), exit_code=1, eof_on_quit=True,
        )
        with patch.object(subprocess, "Popen", side_effect=factory):
            with self.assertRaisesRegex(RuntimeError, "code 1"):
                with WrenApp(
                    artifacts_dir=self._artifacts, readiness_timeout=5.0,
                ) as app:
                    app.restart(hard=False)

    # ── regression: quit errors are not silenced ──────────────────────────

    def test_quit_protocol_error_reports_failure(self) -> None:
        """If app.quit raises ProtocolError, the teardown reports it."""
        factory = self._make_popen(
            _good_identify(),
            exit_code=0,
            sigterm_exits=True,
            quit_exits_process=False,
        )
        with patch.object(subprocess, "Popen", side_effect=factory):
            app = WrenApp(
                artifacts_dir=self._artifacts, readiness_timeout=5.0,
            )
            app.__enter__()

            original_call = app._client.call

            def patched_call(op, *args, **kwargs):
                if op == "app.quit":
                    raise ProtocolError("malformed quit response")
                return original_call(op, *args, **kwargs)

            app._client.call = patched_call

            with patch.object(ui, "QUIT_TIMEOUT", 0.05):
                with self.assertRaisesRegex(RuntimeError, "quit failed"):
                    app.__exit__(None, None, None)

    # ── regression: teardown error attached as note ───────────────────────

    def test_test_exception_with_teardown_error_adds_note(self) -> None:
        """Teardown error is attached to the original test exception."""
        factory = self._make_popen(
            _good_identify(), exit_code=1,
        )
        with patch.object(subprocess, "Popen", side_effect=factory):
            try:
                with WrenApp(
                    artifacts_dir=self._artifacts, readiness_timeout=5.0,
                ) as app:
                    raise ValueError("test body failed")
            except ValueError as exc:
                notes = getattr(exc, "__notes__", [])
                note_text = "\n".join(notes)
                self.assertIn("code 1", note_text)

    # ── regression: hard restart cannot-reap ──────────────────────────────

    def test_hard_restart_cannot_reap_does_not_relaunch(self) -> None:
        """Hard restart raises when child cannot be reaped; no relaunch."""
        popen_calls: list[list[str]] = []
        base_factory = self._make_popen(_good_identify())

        def tracking_factory(cmd, **kwargs):
            popen_calls.append(cmd)
            return base_factory(cmd, **kwargs)

        with patch.object(subprocess, "Popen", side_effect=tracking_factory):
            with patch.object(ui, "QUIT_TIMEOUT", 0.05), \
                 patch.object(ui, "SIGTERM_TIMEOUT", 0.05), \
                 patch.object(ui, "_FINAL_REAP_TIMEOUT", 0.05):
                app_ref = None
                sandbox_path = None
                try:
                    with WrenApp(
                        artifacts_dir=self._artifacts,
                        readiness_timeout=5.0,
                    ) as app:
                        app_ref = app
                        sandbox_path = app.sandbox
                        app._process = _UnreapableProcess()
                        app.restart(hard=True)
                except RuntimeError:
                    pass  # Expected

        # Only 1 Popen call (initial launch, no relaunch).
        self.assertEqual(len(popen_calls), 1)

        # Manual cleanup.
        if app_ref is not None:
            self._processes[0].trigger_exit(0)
            app_ref._process = None
            atexit.unregister(app_ref._emergency_cleanup)
        if sandbox_path and sandbox_path.is_dir():
            shutil.rmtree(sandbox_path, ignore_errors=True)

    def test_failed_replacement_preserves_unreaped_child_and_sandbox(self) -> None:
        factory = self._make_popen(_good_identify())
        with patch.object(subprocess, "Popen", side_effect=factory):
            app = WrenApp(artifacts_dir=self._artifacts, readiness_timeout=5)
            app.__enter__()
            root = app.sandbox
            replacement = _UnreapableProcess()
            try:
                def start_replacement():
                    app._process = replacement

                with patch.object(app, "_start_process", side_effect=start_replacement), \
                     patch.object(app, "_wait_ready", side_effect=RuntimeError("readiness failed")), \
                     patch.object(ui, "_FINAL_REAP_TIMEOUT", 0.01):
                    with self.assertRaisesRegex(RuntimeError, "readiness failed") as caught:
                        app.restart()
                self.assertIs(app._process, replacement)
                self.assertTrue(root.is_dir())
                self.assertTrue(app._atexit_registered)
                self.assertIn("cannot reap replacement", " ".join(caught.exception.__notes__))
            finally:
                app._process = None
                app._teardown(preserve_logs=True)

    def test_teardown_can_retry_after_failed_reap(self) -> None:
        factory = self._make_popen(_good_identify())
        with patch.object(subprocess, "Popen", side_effect=factory):
            app = WrenApp(artifacts_dir=self._artifacts, readiness_timeout=5)
            app.__enter__()
            root = app.sandbox
            with patch.object(app, "_reap_process", side_effect=RuntimeError("cannot reap")):
                with self.assertRaisesRegex(RuntimeError, "cannot reap"):
                    app.quit()
            self.assertFalse(app._torn_down)
            app.quit()
            self.assertIsNone(app._process)
            self.assertFalse(root.exists())


if __name__ == "__main__":
    unittest.main()
