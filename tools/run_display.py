"""Run a command with a private Linux display, session bus, and runtime directory."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import signal
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


def stop_owned_group(process: subprocess.Popen) -> None:
    # Helpers started by D-Bus or the compositor may outlive their direct parent.
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        process.wait(timeout=1)
    except subprocess.TimeoutExpired:
        pass
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait(timeout=5)


def run_owned(command: list[str], env: dict[str, str]) -> int:
    process = subprocess.Popen(command, env=env, start_new_session=True)
    try:
        return process.wait()
    finally:
        stop_owned_group(process)


def wait_for_socket(path: Path, process: subprocess.Popen, log: Path) -> None:
    deadline = time.monotonic() + 15
    while not path.is_socket():
        if process.poll() is not None or time.monotonic() >= deadline:
            diagnostics = log.read_text(errors="replace")[-16000:]
            raise RuntimeError(f"Weston did not become ready; log: {log}\n{diagnostics}")
        time.sleep(0.05)


def describe_exit(code: int) -> str:
    if code < 0:
        try:
            return f"signal {signal.Signals(-code).name}"
        except ValueError:
            return f"signal {-code}"
    return f"code {code}"


class Compositor:
    """Headless Weston for the whole run, started again on the same socket if
    it dies, so one crash fails the test it interrupts rather than every test
    after it. Weston 13 logs no fatal signal, so the notice names it."""

    RESTARTS = 3

    def __init__(self, artifacts: Path, socket: Path, env: dict[str, str]) -> None:
        self.artifacts = artifacts
        self.socket = socket
        self.env = dict(env)
        self.process: subprocess.Popen | None = None
        self.log: Path | None = None
        self.restarts = 0
        self.stopping = False
        self.lock = threading.Lock()
        self.watcher: threading.Thread | None = None

    def launch(self) -> None:
        self.log = self.artifacts / f"weston-{uuid.uuid4().hex}.log"
        with self.log.open("xb") as output:
            self.process = subprocess.Popen([
                "weston", "--backend=headless", "--renderer=pixman", "--no-config",
                "--idle-time=0", "--width=1440", "--height=900",
                f"--socket={self.socket.name}",
            ], env=self.env, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
        wait_for_socket(self.socket, self.process, self.log)

    def start(self) -> None:
        try:
            self.launch()
        except BaseException:
            if self.process is not None:
                stop_owned_group(self.process)
            raise
        self.watcher = threading.Thread(target=self.watch, name="compositor", daemon=True)
        self.watcher.start()

    def watch(self) -> None:
        while True:
            process = self.process
            assert process is not None
            code = process.wait()
            with self.lock:
                if self.stopping:
                    return
                # Its helpers share its process group and may outlive it.
                stop_owned_group(process)
                notice = f"run_display: Weston exited with {describe_exit(code)}; log: {self.log}"
                if self.restarts >= self.RESTARTS:
                    print(f"{notice}; not restarting after {self.restarts} restarts",
                          file=sys.stderr, flush=True)
                    return
                self.restarts += 1
                # A stale socket would pass the readiness check at once.
                self.socket.unlink(missing_ok=True)
                try:
                    self.launch()
                except (OSError, RuntimeError) as error:
                    print(f"{notice}; restart failed: {error}", file=sys.stderr, flush=True)
                    return
                print(f"{notice}; restarted ({self.restarts} of {self.RESTARTS}), log: {self.log}",
                      file=sys.stderr, flush=True)

    def stop(self) -> None:
        with self.lock:
            self.stopping = True
            if self.process is not None:
                stop_owned_group(self.process)
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
        compositor.start()
        try:
            env["WAYLAND_DISPLAY"] = socket_name
            return run_owned(bus_command, env)
        finally:
            compositor.stop()


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
