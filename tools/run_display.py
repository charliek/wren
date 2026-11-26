"""Run a command with a private Linux display, session bus, and runtime directory."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from xml.sax.saxutils import escape

ROOT = Path(__file__).resolve().parents[1]


def artifact_directory(mode: str, env: dict[str, str]) -> Path:
    override = env.get("WREN_TEST_ARTIFACT_DIR")
    directory = Path(override) if override else (
        ROOT / "target" / "wrentest" / f"{mode}-{env.get('ICED_BACKEND', 'default')}"
    )
    if not directory.is_absolute():
        raise ValueError("WREN_TEST_ARTIFACT_DIR must be absolute")
    directory.mkdir(parents=True, exist_ok=True)
    return directory


def has_exited(process: subprocess.Popen) -> bool:
    """Whether `process` has exited, without reaping it: until it is reaped
    its process group ID cannot be reused, so the group is still ours to
    signal."""
    if process.returncode is not None:
        return True
    try:
        return os.waitid(os.P_PID, process.pid,
                         os.WEXITED | os.WNOHANG | os.WNOWAIT) is not None
    except ChildProcessError:
        return True


def wait_exited(process: subprocess.Popen, timeout: float | None = None) -> bool:
    """Wait, without reaping, until `process` exits or `timeout` passes."""
    deadline = None if timeout is None else time.monotonic() + timeout
    while not has_exited(process):
        if deadline is not None and time.monotonic() >= deadline:
            return False
        time.sleep(0.02)
    return True


def signal_group(process: subprocess.Popen, number: int) -> None:
    # Once reaped, its group ID may belong to someone else.
    if process.returncode is None:
        try:
            os.killpg(process.pid, number)
        except ProcessLookupError:
            pass


def stop_owned_group(process: subprocess.Popen) -> None:
    # Helpers started by D-Bus or the compositor may outlive their direct
    # parent, so the group is signalled again before the leader is reaped.
    signal_group(process, signal.SIGTERM)
    wait_exited(process, timeout=1)
    signal_group(process, signal.SIGKILL)
    process.wait(timeout=5)


def run_owned(command: list[str], env: dict[str, str],
              started=lambda process: None) -> int:
    process = subprocess.Popen(command, env=env, start_new_session=True)
    try:
        started(process)
        wait_exited(process)
    finally:
        stop_owned_group(process)
    return process.returncode


def accepts(path: Path) -> bool:
    """Whether something listens at the socket `path`. A socket bound but not
    yet listening, or one whose compositor died, refuses the connection."""
    with socket.socket(socket.AF_UNIX) as probe:
        probe.settimeout(1)
        try:
            probe.connect(str(path))
        except OSError:
            return False
    return True


def wait_for_socket(path: Path, process: subprocess.Popen, log: Path,
                    timeout: float = 15) -> None:
    deadline = time.monotonic() + timeout
    while not accepts(path):
        if has_exited(process) or time.monotonic() >= deadline:
            diagnostics = log.read_text(errors="replace")[-16000:]
            raise RuntimeError(f"Weston did not become ready; log: {log}\n{diagnostics}")
        time.sleep(0.05)


def describe_exit(info: os.waitid_result | None) -> str:
    if info is None:
        return "an unknown status"
    if info.si_code == os.CLD_EXITED:
        return f"code {info.si_status}"
    try:
        return f"signal {signal.Signals(info.si_status).name}"
    except ValueError:
        return f"signal {info.si_status}"


def start_time(pid: int) -> str | None:
    try:
        stat = Path(f"/proc/{pid}/stat").read_text()
    except OSError:
        return None
    # Field 22, counted after the command name, which may hold spaces.
    return stat.rsplit(")", 1)[1].split()[19]


def children(pid: int) -> dict[int, str]:
    """The children of `pid` with their start times, so a later signal can
    skip a process ID reused since."""
    try:
        listed = Path(f"/proc/{pid}/task/{pid}/children").read_text().split()
    except OSError:
        return {}
    found = {}
    for child in map(int, listed):
        started = start_time(child)
        if started is not None:
            found[child] = started
    return found


def weston_command(socket_name: str) -> list[str]:
    return [
        "weston", "--backend=headless", "--renderer=pixman", "--no-config",
        "--idle-time=0", "--width=1440", "--height=900", f"--socket={socket_name}",
    ]


class Compositor:
    """Headless Weston for the whole run, started again on the same socket if
    it dies, so one crash fails the test it interrupts rather than every test
    after it. Weston 13 logs no fatal signal, so the notice names it. If it
    cannot be kept running, the command is stopped and the run fails.

    Weston starts its desktop-shell and keyboard helpers in sessions of their
    own, out of reach of its group, so the watcher keeps a list of its
    children while it runs and stops them after it."""

    ATTEMPTS = 3
    POLL = 0.25

    def __init__(self, artifacts: Path, socket_path: Path, env: dict[str, str],
                 command=None) -> None:
        self.artifacts = artifacts
        self.socket = socket_path
        self.env = dict(env)
        self.command = command or (lambda: weston_command(socket_path.name))
        self.process: subprocess.Popen | None = None
        self.helpers: dict[int, str] = {}
        self.log: Path | None = None
        self.attempts = 0
        self.stopping = False
        self.lost = False
        self.supervised: subprocess.Popen | None = None
        self.lock = threading.Lock()
        self.watcher: threading.Thread | None = None

    def report(self, message: str) -> None:
        print(f"run_display: {message}", file=sys.stderr, flush=True)

    def launch(self) -> None:
        self.log = self.artifacts / f"weston-{uuid.uuid4().hex}.log"
        self.helpers = {}
        with self.log.open("xb") as output:
            self.process = subprocess.Popen(
                self.command(), env=self.env, stdout=output, stderr=subprocess.STDOUT,
                start_new_session=True)
        wait_for_socket(self.socket, self.process, self.log)

    def discard(self) -> None:
        """Stop the current Weston, its group and the helpers it started."""
        if self.process is not None:
            self.helpers.update(children(self.process.pid))
            stop_owned_group(self.process)
            self.process = None
        for pid, started in self.helpers.items():
            if start_time(pid) == started:
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
        self.helpers = {}

    def start(self) -> None:
        self.launch()
        watcher = threading.Thread(target=self.watch, name="compositor", daemon=True)
        watcher.start()
        self.watcher = watcher

    def supervise(self, command: subprocess.Popen) -> None:
        """Run `command` only while a compositor can be kept running."""
        with self.lock:
            self.supervised = command
            if self.lost:
                signal_group(command, signal.SIGTERM)

    def watch(self) -> None:
        while True:
            process = self.process
            assert process is not None
            info = None
            while info is None:
                try:
                    info = os.waitid(os.P_PID, process.pid,
                                     os.WEXITED | os.WNOHANG | os.WNOWAIT)
                except ChildProcessError:
                    break
                if info is None:
                    if self.stopping:
                        return
                    self.helpers.update(children(process.pid))
                    time.sleep(self.POLL)
            with self.lock:
                if self.stopping:
                    return
                log = self.log
                self.discard()
                self.report(f"Weston exited with {describe_exit(info)}; log: {log}")
                if not self.restart():
                    self.lost = True
                    self.report(f"Weston is down after {self.attempts} restart attempts; "
                                "stopping the run")
                    if self.supervised is not None:
                        signal_group(self.supervised, signal.SIGTERM)
                    return

    def restart(self) -> bool:
        while self.attempts < self.ATTEMPTS and not self.stopping:
            self.attempts += 1
            # A stale socket refuses connections, but the path must be free.
            self.socket.unlink(missing_ok=True)
            try:
                self.launch()
            except (OSError, RuntimeError) as error:
                log = self.log
                self.discard()
                self.report(f"restart {self.attempts} of {self.ATTEMPTS} failed: "
                            f"{str(error).splitlines()[0]}; log: {log}")
                continue
            self.report(f"Weston restarted ({self.attempts} of {self.ATTEMPTS}); log: {self.log}")
            return True
        return False

    def stop(self) -> None:
        self.stopping = True
        with self.lock:
            self.discard()
        if self.watcher is not None:
            self.watcher.join(timeout=10)


def session_bus_command(runtime: Path, command: list[str]) -> list[str]:
    # Default session services can create FUSE mounts or contact a real desktop.
    config = runtime / "dbus.conf"
    config.write_text(
        '<busconfig><type>session</type>'
        f'<listen>unix:tmpdir={escape(str(runtime))}</listen><auth>EXTERNAL</auth>'
        '<policy context="default"><allow user="*"/><allow own="*"/>'
        '<allow send_destination="*"/><allow receive_sender="*"/></policy>'
        '</busconfig>', encoding="utf-8",
    )
    return ["dbus-run-session", f"--config-file={config}", "--", *command]


def run_display(mode: str, command: list[str]) -> int:
    env = os.environ.copy()
    artifacts = artifact_directory(mode, env)
    env["WREN_TEST_ARTIFACT_DIR"] = str(artifacts)
    for name in (
        "DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "DBUS_SESSION_BUS_ADDRESS",
        "DBUS_STARTER_ADDRESS", "DBUS_STARTER_BUS_TYPE",
    ):
        env.pop(name, None)

    with tempfile.TemporaryDirectory(prefix="wren-display-") as runtime:
        env["XDG_RUNTIME_DIR"] = runtime
        for variable, name in (
            ("HOME", "home"), ("XDG_CONFIG_HOME", "config"),
            ("XDG_DATA_HOME", "data"), ("XDG_STATE_HOME", "state"),
            ("XDG_CACHE_HOME", "cache"),
        ):
            path = Path(runtime) / name
            path.mkdir()
            env[variable] = str(path)

        env["GIO_USE_VFS"] = "local"
        env["GSETTINGS_BACKEND"] = "memory"
        token = uuid.uuid4().hex
        bus_command = session_bus_command(Path(runtime), command)
        if mode == "x11":
            return run_owned([
                "xvfb-run", "-a",
                f"--server-args=-screen 0 {env.get('WREN_XVFB_SCREEN', '1920x1200')}x24",
                f"--error-file={artifacts / f'xvfb-{token}.log'}", *bus_command,
            ], env)

        socket_name = "wayland-wren"
        compositor = Compositor(artifacts, Path(runtime) / socket_name, env)
        try:
            compositor.start()
            env["WAYLAND_DISPLAY"] = socket_name
            code = run_owned(bus_command, env, started=compositor.supervise)
        finally:
            compositor.stop()
        if compositor.lost:
            raise RuntimeError("the run failed: Weston could not be kept running")
        return code


def terminate(_signum: int, _frame: object) -> None:
    raise SystemExit(128 + _signum)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("x11", "wayland"))
    argv = sys.argv[1:] if argv is None else argv
    args = parser.parse_args(argv[:1])
    if len(argv) < 3 or argv[1] != "--":
        parser.error("a command must follow --")
    signal.signal(signal.SIGTERM, terminate)
    try:
        result = run_display(args.mode, argv[2:])
        return result if result >= 0 else 128 - result
    except KeyboardInterrupt:
        return 130
    except (OSError, RuntimeError, ValueError) as error:
        print(f"run_display: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
