"""Run a command with a private Linux display, session bus, and runtime directory."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
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
                "xvfb-run", "-a", "--server-args=-screen 0 1920x1200x24",
                f"--error-file={artifacts / f'xvfb-{token}.log'}", *bus_command,
            ], env)

        log = artifacts / f"weston-{token}.log"
        socket_name = "wayland-wren"
        with log.open("xb") as output:
            compositor = subprocess.Popen([
                "weston", "--backend=headless", "--renderer=pixman", "--no-config",
                "--idle-time=0", "--width=1440", "--height=900",
                f"--socket={socket_name}",
            ], env=env, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                wait_for_socket(Path(runtime) / socket_name, compositor, log)
                env["WAYLAND_DISPLAY"] = socket_name
                return run_owned(bus_command, env)
            finally:
                stop_owned_group(compositor)


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
