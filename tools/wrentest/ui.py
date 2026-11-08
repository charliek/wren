"""Python app harness for Wren IPC-based editor tests.

Launches the Wren binary under ``--test-socket`` control, waits for readiness
(confirmed by a strict ``identify`` reply), and tears down cleanly.

Usage::

    from tools.wrentest.ui import WrenApp

    with WrenApp() as app:
        result = app.client.call("identify")
        assert result["ready"] is True
"""

from __future__ import annotations

import atexit
import math
import os
import shutil
import signal
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Any

from tools.wrentest.client import Client, ProtocolError, RemoteError, TransportError

__all__ = ["WrenApp", "wait_for"]

ROOT = Path(__file__).resolve().parents[2]

READINESS_TIMEOUT: float = 30.0
READINESS_POLL: float = 0.15
QUIT_TIMEOUT: float = 10.0
SIGTERM_TIMEOUT: float = 5.0
_FINAL_REAP_TIMEOUT: float = 5.0

_PRESERVE_VARS = frozenset({
    "DISPLAY", "WAYLAND_DISPLAY", "XDG_RUNTIME_DIR",
    "XAUTHORITY", "LIBGL_ALWAYS_SOFTWARE", "MESA_LOADER_DRIVER_OVERRIDE",
    "WGPU_BACKEND", "ICED_BACKEND",
    "DBUS_SESSION_BUS_ADDRESS",
    "GIO_USE_VFS", "GSETTINGS_BACKEND",
    "PATH", "LANG",
})

_XDG_DIRS = {
    "HOME": "home",
    "XDG_CONFIG_HOME": "config",
    "XDG_DATA_HOME": "data",
    "XDG_STATE_HOME": "state",
    "XDG_CACHE_HOME": "cache",
}


def _validate_positive_finite(value: float, name: str) -> None:
    """Raise ``ValueError`` unless *value* is a finite positive number."""
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be finite and positive")
    if not math.isfinite(value) or value <= 0:
        raise ValueError(f"{name} must be finite and positive")


def wait_for(
    predicate,
    *,
    timeout: float = 10,
    interval: float = 0.1,
    description: str = "condition",
):
    """Poll *predicate* until truthy or *timeout* seconds elapse.

    Returns the last truthy value.  Raises ``TimeoutError`` with the last
    falsy state if the deadline expires.
    """
    _validate_positive_finite(timeout, "timeout")
    _validate_positive_finite(interval, "interval")
    deadline = time.monotonic() + timeout
    last = predicate()
    while not last:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError(
                f"wait_for({description}) timed out after {timeout}s; "
                f"last state: {last!r}"
            )
        time.sleep(min(interval, remaining))
        last = predicate()
    return last


def _find_binary() -> Path:
    override = os.environ.get("WREN_TEST_BIN")
    binary = Path(override) if override else ROOT / "target" / "debug" / "wren"
    if not binary.is_absolute():
        raise RuntimeError(
            f"WREN_TEST_BIN must be an absolute path, got {binary}"
        )
    if not binary.is_file():
        raise RuntimeError(
            f"Wren binary not found at {binary}; build with "
            "`cargo build --features smoke-test` first"
        )
    return binary


def _build_env(sandbox: Path) -> dict[str, str]:
    """Build an isolated child environment: strip WREN_*, sandbox XDG."""
    env: dict[str, str] = {}
    for key, value in os.environ.items():
        if key.startswith("WREN_"):
            continue
        if key in _PRESERVE_VARS:
            env[key] = value
    for var, subdir in _XDG_DIRS.items():
        path = sandbox / subdir
        path.mkdir(mode=0o700, exist_ok=True)
        env[var] = str(path)
    env["WREN_TEST_MODE"] = "1"
    return env


class WrenApp:
    """Owns a running Wren process under test-IPC control.

    Parameters
    ----------
    artifacts_dir:
        Directory for log files.  Defaults to ``WREN_TEST_ARTIFACT_DIR`` or
        ``ROOT/target/wrentest/ipc/``.
    keep_logs:
        When ``True`` (default), log files are preserved even on success.
    readiness_timeout:
        Maximum seconds to wait for the ``identify`` reply after launch.
    workspace:
        Optional workspace directory passed via ``--workspace``.
    extra_args:
        Additional CLI arguments appended to the command.
    evaluation:
        Use the explicit unsaved editor when no workspace is supplied. Set to
        ``False`` to exercise normal registry restoration and onboarding.
    settings:
        Text for the sandbox's ``config.conf``, written before the first
        launch. Restarts keep whatever the app has written there since.
    """

    def __init__(
        self,
        artifacts_dir: Path | None = None,
        keep_logs: bool = True,
        readiness_timeout: float = READINESS_TIMEOUT,
        workspace: Path | None = None,
        extra_args: tuple[str, ...] = (),
        evaluation: bool = True,
        settings: str | None = None,
    ) -> None:
        _validate_positive_finite(readiness_timeout, "readiness_timeout")
        if type(evaluation) is not bool:
            raise ValueError("evaluation must be bool")
        if settings is not None and not isinstance(settings, str):
            raise ValueError("settings must be str or None")
        self._binary = _find_binary()
        env_artifacts = os.environ.get("WREN_TEST_ARTIFACT_DIR")
        self._artifacts_base = (
            artifacts_dir
            or (Path(env_artifacts) if env_artifacts else None)
            or (ROOT / "target" / "wrentest" / "ipc")
        )
        self._keep_logs = keep_logs
        self._readiness_timeout = readiness_timeout
        self._workspace = workspace
        self._extra_args = extra_args
        self._evaluation = evaluation
        self._settings = settings

        self._ownroot: Path | None = None
        self._socket_path: Path | None = None
        self._process: subprocess.Popen[bytes] | None = None
        self._client: Client | None = None
        self._stdout_log: Path | None = None
        self._stderr_log: Path | None = None
        self._launch_counter = 0
        self._atexit_registered: bool = False
        self._torn_down: bool = False

    # ── context manager ───────────────────────────────────────────────────

    def __enter__(self) -> WrenApp:
        self._launch()
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_val: BaseException | None,
        tb: object,
    ) -> bool:
        failed = exc_type is not None
        try:
            self._teardown(
                preserve_logs=failed or self._keep_logs,
                caller_exc=exc_val,
            )
        except BaseException as teardown_err:
            if exc_val is not None:
                if not hasattr(exc_val, "__notes__"):
                    exc_val.__notes__ = []
                exc_val.__notes__.append(f"During teardown: {teardown_err}")
                return False
            raise
        return False

    # ── public API ────────────────────────────────────────────────────────

    @property
    def client(self) -> Client:
        if self._client is None:
            raise RuntimeError("WrenApp is not running")
        return self._client

    @property
    def pid(self) -> int:
        if self._process is None:
            raise RuntimeError("WrenApp is not running")
        return self._process.pid

    @property
    def returncode(self) -> int | None:
        """Observe only this harness-owned child's exit status."""
        if self._process is None:
            raise RuntimeError("WrenApp is not running")
        return self._process.poll()

    @property
    def sandbox(self) -> Path:
        if self._ownroot is None:
            raise RuntimeError("WrenApp is not running")
        return self._ownroot

    @property
    def settings_path(self) -> Path:
        """The sandboxed ``config.conf`` the app reads (it may not exist)."""
        return self.sandbox / "home_root" / _XDG_DIRS["XDG_CONFIG_HOME"] / "wren" / "config.conf"

    def quit(self) -> None:
        """Request a clean exit and tear down."""
        self._teardown(preserve_logs=self._keep_logs)

    def restart(self, hard: bool = True) -> None:
        """Kill (hard) or graceful quit (soft) the running process, then relaunch.

        When ``hard=False`` the running process is asked to quit via
        ``app.quit`` and must exit with code 0.  ``hard=True`` sends
        SIGKILL.  In both cases the child is verified reaped before the
        stale socket file is removed.  The same sandbox is reused.
        """
        if self._process is None:
            raise RuntimeError("no running process to restart")

        if hard:
            if self._client is not None:
                self._client.close()
                self._client = None

            proc = self._process
            try:
                proc.kill()
            except ProcessLookupError:
                pass

            try:
                proc.wait(timeout=_FINAL_REAP_TIMEOUT)
            except subprocess.TimeoutExpired:
                raise RuntimeError(
                    f"Cannot reap process {proc.pid} after SIGKILL "
                    f"during hard restart"
                )
            self._process = None
        else:
            quit_error: BaseException | None = None
            if self._client is not None:
                try:
                    self._client.call("app.quit", timeout=5.0)
                except TransportError as e:
                    if not e.eof:
                        quit_error = e
                except Exception as e:
                    quit_error = e
                finally:
                    self._client.close()
                    self._client = None

            if quit_error is not None:
                raise RuntimeError(
                    f"app.quit failed during soft restart: {quit_error}"
                )

            proc = self._process
            try:
                proc.wait(timeout=QUIT_TIMEOUT)
            except subprocess.TimeoutExpired:
                raise RuntimeError(
                    f"Process did not exit after app.quit within "
                    f"{QUIT_TIMEOUT}s during soft restart"
                )

            if proc.returncode != 0:
                self._process = None
                raise RuntimeError(
                    f"Process exited with code {proc.returncode} after "
                    f"quit (expected 0 for clean restart)\n"
                    f"{self._read_logs()}"
                )
            self._process = None

        # Stale socket removed only after verified reap.
        if self._socket_path is not None and self._socket_path.exists():
            self._socket_path.unlink()

        self._torn_down = False
        try:
            self._start_process()
            self._wait_ready()
        except BaseException as error:
            if self._process is not None:
                try:
                    try:
                        self._process.kill()
                    except ProcessLookupError:
                        pass
                    self._process.wait(timeout=_FINAL_REAP_TIMEOUT)
                except BaseException as cleanup_error:
                    error.add_note(f"During restart cleanup: cannot reap replacement: {cleanup_error}")
                else:
                    self._process = None
            raise

    # ── internals ─────────────────────────────────────────────────────────

    def _launch(self) -> None:
        self._artifacts_base.mkdir(parents=True, exist_ok=True)

        self._ownroot = Path(tempfile.mkdtemp(prefix="wren-t-", dir="/tmp"))
        self._ownroot.chmod(0o700)
        self._socket_path = self._ownroot / "s.sock"

        atexit.register(self._emergency_cleanup)
        self._atexit_registered = True

        try:
            if self._settings is not None:
                self._write_settings(self._settings)
            self._start_process()
            self._wait_ready()
        except BaseException as exc:
            try:
                self._teardown(preserve_logs=True, caller_exc=exc)
            except BaseException as cleanup_err:
                if not hasattr(exc, "__notes__"):
                    exc.__notes__ = []
                exc.__notes__.append(
                    f"During startup cleanup: {cleanup_err}"
                )
            raise exc

    def _write_settings(self, text: str) -> None:
        path = self.settings_path
        directory = self.sandbox
        for part in path.relative_to(self.sandbox).parts[:-1]:
            directory /= part
            directory.mkdir(mode=0o700, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def _start_process(self) -> None:
        assert self._socket_path is not None
        assert self._ownroot is not None

        self._launch_counter += 1

        home_root = self._ownroot / "home_root"
        home_root.mkdir(mode=0o700, exist_ok=True)
        env = _build_env(home_root)

        self._artifacts_base.mkdir(parents=True, exist_ok=True)
        stamp = f"{time.monotonic_ns()}-{self._launch_counter}"
        self._stdout_log = self._artifacts_base / f"process-{stamp}.stdout"
        self._stderr_log = self._artifacts_base / f"process-{stamp}.stderr"

        cmd = [str(self._binary), "--test-socket", str(self._socket_path)]
        if self._workspace is not None:
            cmd.extend(["--workspace", str(self._workspace)])
        elif self._evaluation and not any(
            flag in self._extra_args for flag in ("--workspace", "--editor", "--editor-spike")
        ):
            cmd.append("--editor")
        cmd.extend(self._extra_args)

        with open(self._stdout_log, "wb") as stdout_fh, \
             open(self._stderr_log, "wb") as stderr_fh:
            self._process = subprocess.Popen(
                cmd, env=env, stdout=stdout_fh, stderr=stderr_fh,
            )

    def _wait_ready(self) -> None:
        assert self._socket_path is not None
        assert self._process is not None

        deadline = time.monotonic() + self._readiness_timeout
        last_error: Exception | None = None

        while time.monotonic() < deadline:
            ret = self._process.poll()
            if ret is not None:
                raise RuntimeError(
                    f"Wren process exited with code {ret} before becoming "
                    f"ready.\n{self._read_logs()}"
                )

            remaining = deadline - time.monotonic()
            if remaining <= 0:
                break

            if self._client is None:
                self._client = Client(
                    self._socket_path,
                    default_timeout=min(remaining, 5.0),
                )

            try:
                if not self._client.connected:
                    self._client.connect()
                result = self._client.call(
                    "identify", timeout=min(remaining, 5.0),
                )
                if self._validate_identify(result):
                    return
                self._client.close()
                self._client = None
                time.sleep(min(READINESS_POLL, max(0, deadline - time.monotonic())))
            except (TransportError, ProtocolError, OSError) as exc:
                last_error = exc
                if self._client is not None:
                    self._client.close()
                    self._client = None
                time.sleep(min(READINESS_POLL, max(0, deadline - time.monotonic())))

        raise RuntimeError(
            f"Wren app not ready after {self._readiness_timeout}s. "
            f"Last error: {last_error}\n{self._read_logs()}"
        )

    def _validate_identify(self, result: Any) -> bool:
        """Return True when ready.  Raise RuntimeError for fatal mismatches."""
        if not isinstance(result, dict):
            raise RuntimeError(f"identify result is not a dict: {result!r}")

        protocol = result.get("protocol")
        if type(protocol) is not int or protocol != 1:
            raise RuntimeError(
                f"identify: unexpected protocol {protocol!r}, expected 1"
            )

        expected_pid = self._process.pid if self._process else None
        actual_pid = result.get("pid")
        if type(actual_pid) is not int or actual_pid != expected_pid:
            raise RuntimeError(
                f"identify: pid mismatch (expected {expected_pid}, "
                f"got {actual_pid!r})"
            )

        if "ready" not in result:
            raise RuntimeError(
                f"identify: missing required 'ready' key in {result!r}"
            )
        ready = result["ready"]
        if type(ready) is not bool:
            raise RuntimeError(
                f"identify: 'ready' must be bool, got "
                f"{type(ready).__name__}: {ready!r}"
            )

        return ready

    def _teardown(
        self,
        preserve_logs: bool,
        caller_exc: BaseException | None = None,
    ) -> None:
        if self._torn_down:
            return
        self._torn_down = True

        teardown_error: BaseException | None = None
        reaped = self._process is None

        # Native window close may already have exited the owned child. Still
        # reap it and validate its status below, without writing to a dead socket.
        exited = self._process is not None and self._process.poll() is not None
        if self._client is not None:
            try:
                if not exited:
                    self._client.call("app.quit", timeout=5.0)
            except TransportError as e:
                if not e.eof:
                    teardown_error = e
            except (ProtocolError, RemoteError) as e:
                teardown_error = e
            except Exception as e:
                teardown_error = e
            finally:
                self._client.close()
                self._client = None

        # Phase 2: reap the child process.
        if self._process is not None:
            try:
                exit_code, escalated = self._reap_process(self._process)
                reaped = True
                self._process = None

                if teardown_error is not None:
                    teardown_error = RuntimeError(
                        f"app.quit failed ({teardown_error}); "
                        f"process exited with code {exit_code}\n"
                        f"{self._read_logs()}"
                    )
                elif escalated:
                    teardown_error = RuntimeError(
                        f"Process required signal escalation "
                        f"(not clean shutdown), exit code {exit_code}\n"
                        f"{self._read_logs()}"
                    )
                elif exit_code != 0:
                    teardown_error = RuntimeError(
                        f"Wren process exited with code {exit_code}\n"
                        f"{self._read_logs()}"
                    )
            except BaseException as reap_err:
                self._torn_down = False
                # Cannot verify child reaped — fail-closed: preserve
                # sandbox, atexit handler, and process reference.
                if caller_exc is not None:
                    if not hasattr(caller_exc, "__notes__"):
                        caller_exc.__notes__ = []
                    caller_exc.__notes__.append(
                        f"During teardown: cannot reap process: {reap_err}"
                    )
                    return
                raise

        # Phase 3: clean sandbox and atexit (only when reaped).
        if reaped:
            self._cleanup_ownroot()
            if not preserve_logs and teardown_error is None:
                self._remove_logs()
            if self._atexit_registered:
                atexit.unregister(self._emergency_cleanup)
                self._atexit_registered = False

        # Phase 4: report teardown problems.
        if teardown_error is not None:
            if caller_exc is None:
                raise teardown_error
            if not hasattr(caller_exc, "__notes__"):
                caller_exc.__notes__ = []
            caller_exc.__notes__.append(
                f"During teardown: {teardown_error}"
            )

    def _reap_process(
        self, proc: subprocess.Popen[bytes],
    ) -> tuple[int, bool]:
        """Wait for *proc* to exit, escalating signals.

        Returns ``(exit_code, escalated)`` where *escalated* is ``True``
        when SIGTERM or SIGKILL was needed.  Raises ``RuntimeError`` if
        the process cannot be verified reaped.
        """
        try:
            proc.wait(timeout=QUIT_TIMEOUT)
            return proc.returncode, False
        except subprocess.TimeoutExpired:
            pass

        try:
            proc.send_signal(signal.SIGTERM)
        except ProcessLookupError:
            try:
                proc.wait(timeout=_FINAL_REAP_TIMEOUT)
                return proc.returncode, True
            except subprocess.TimeoutExpired:
                raise RuntimeError(
                    f"Process {proc.pid} vanished but cannot be reaped"
                )

        try:
            proc.wait(timeout=SIGTERM_TIMEOUT)
            return proc.returncode, True
        except subprocess.TimeoutExpired:
            pass

        try:
            proc.kill()
        except ProcessLookupError:
            pass
        try:
            proc.wait(timeout=_FINAL_REAP_TIMEOUT)
            return proc.returncode, True
        except subprocess.TimeoutExpired:
            raise RuntimeError(
                f"Cannot reap process {proc.pid} after SIGKILL"
            )

    def _cleanup_ownroot(self) -> None:
        if self._ownroot is not None and self._ownroot.exists():
            shutil.rmtree(self._ownroot)
        self._ownroot = None
        self._socket_path = None

    def _remove_logs(self) -> None:
        for log in (self._stdout_log, self._stderr_log):
            if log is not None:
                try:
                    log.unlink(missing_ok=True)
                except OSError:
                    pass

    def _read_logs(self) -> str:
        parts: list[str] = []
        for label, path in (
            ("stdout", self._stdout_log),
            ("stderr", self._stderr_log),
        ):
            if path is not None and path.is_file():
                try:
                    content = (
                        path.read_bytes()
                        .decode("utf-8", errors="replace")
                        .strip()
                    )
                    if content:
                        parts.append(f"--- {label} ---\n{content}")
                except OSError:
                    pass
        return "\n".join(parts) if parts else "(no log output)"

    def _emergency_cleanup(self) -> None:
        proc = self._process
        reaped = proc is None
        if proc is not None:
            try:
                proc.kill()
            except ProcessLookupError:
                pass
            try:
                proc.wait(timeout=3.0)
                reaped = True
                self._process = None
            except Exception:
                pass
        if reaped:
            self._cleanup_ownroot()
