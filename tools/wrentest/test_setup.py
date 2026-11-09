"""Real-window rendering checks for the development scaffold, not editor acceptance."""

from __future__ import annotations

import os
import re
from pathlib import Path
import subprocess
import tempfile

import pytest

from tools.wrentest import png

ROOT = Path(__file__).resolve().parents[2]
BACKGROUNDS = {"dark": (0x14, 0x16, 0x1B), "light": (0xF7, 0xF8, 0xFA)}
# editor-max-width's default: the editor centres a column this wide, gutter
# included, in a wider pane, as focus mode always does.
EDITOR_COLUMN = 760


def editor_column(navigation: dict, root_x: float = 0) -> tuple[float, float]:
    """The left edge and width of the editor's column of gutter and text."""
    browser = 0 if navigation["focus"] else sum(navigation["effective_widths"].values())
    pane = navigation["viewport"]["width"] - browser
    width = min(pane, EDITOR_COLUMN)
    return root_x + browser + (pane - width) // 2, width


def binary_path() -> Path:
    override = os.environ.get("WREN_TEST_BIN")
    binary = Path(override) if override else ROOT / "target" / "debug" / "wren"
    if not binary.is_absolute() or not binary.is_file():
        pytest.fail(f"Expected an absolute path to a built smoke-test binary, got {binary}")
    return binary


def isolated_environment(root: Path, *, test_mode: bool = True) -> dict[str, str]:
    env = {key: value for key, value in os.environ.items() if not key.startswith("WREN_")}
    for variable, name in (
        ("HOME", "home"), ("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"),
        ("XDG_STATE_HOME", "state"), ("XDG_CACHE_HOME", "cache"),
    ):
        path = root / name
        path.mkdir()
        env[variable] = str(path)
    if test_mode:
        env["WREN_TEST_MODE"] = "1"
    return env


def artifact_directory(case: str) -> Path:
    override = os.environ.get("WREN_TEST_ARTIFACT_DIR")
    display = "wayland" if os.environ.get("WAYLAND_DISPLAY") else "x11"
    base = Path(override) if override else (
        ROOT / "target" / "wrentest" / f"{display}-{os.environ.get('ICED_BACKEND', 'default')}"
    )
    if not base.is_absolute():
        pytest.fail("WREN_TEST_ARTIFACT_DIR must be absolute")
    base.mkdir(parents=True, exist_ok=True)
    # Artifact uploads reject characters such as ':' found in test ids.
    safe = re.sub(r"[^A-Za-z0-9._-]+", "_", case)
    return Path(tempfile.mkdtemp(prefix=f"{safe}-", dir=base))


def run_binary(args: list[str], env: dict[str, str], artifacts: Path, cwd: Path):
    def save_log(stdout, stderr):
        def decoded(value):
            return value.decode(errors="replace") if isinstance(value, bytes) else value or ""
        (artifacts / "process.log").write_text(
            f"stdout:\n{decoded(stdout)}\nstderr:\n{decoded(stderr)}", encoding="utf-8"
        )

    try:
        result = subprocess.run(
            [str(binary_path()), *args], env=env, cwd=cwd, capture_output=True,
            text=True, encoding="utf-8", errors="replace", timeout=60,
        )
    except subprocess.TimeoutExpired as error:
        save_log(error.stdout, error.stderr)
        raise
    save_log(result.stdout, result.stderr)
    return result


def check_backend(stderr: str) -> None:
    values = [line.split("=", 1)[1].strip().lower() for line in stderr.splitlines()
              if line.startswith("WREN_SMOKE_BACKEND=")]
    assert len(values) == 1, f"Missing or ambiguous renderer information:\n{stderr}"
    requested = os.environ.get("ICED_BACKEND", "wgpu")
    if requested == "tiny-skia":
        assert values[0] == "tiny-skia", stderr
    else:
        assert requested == "wgpu", f"Unsupported smoke renderer: {requested}"
        assert values[0] in {"vulkan", "gl", "metal", "dx12", "dx11"}, stderr
        if os.environ.get("WGPU_BACKEND") == "vulkan":
            assert values[0] == "vulkan", stderr


def check_pixels(image: png.PNG, background: tuple[int, int, int]) -> None:
    width, height = image.width, image.height
    assert width >= 640 and height >= 400
    for x, y in ((0, 0), (width - 1, 0), (0, height - 1),
                 (width - 1, height - 1), (width // 2, height - 1)):
        pixel = image.pixel(x, y)
        assert pixel[3] == 255
        assert all(abs(actual - expected) <= 2 for actual, expected in zip(pixel[:3], background))

    colors = [image.pixel(x, y)[:3]
              for y in range(height // 4, 3 * height // 4)
              for x in range(width // 4, 3 * width // 4)]
    non_background = sum(
        any(abs(actual - expected) > 10 for actual, expected in zip(pixel, background))
        for pixel in colors
    )
    assert non_background >= len(colors) * 0.01, "The captured content region is blank"
    assert len(set(colors)) >= 5, "The captured content region has no rendered text detail"


@pytest.mark.parametrize("theme", ["dark", "light"])
def test_smoke_capture(theme: str, tmp_path: Path) -> None:
    artifacts = artifact_directory(theme)
    screenshot = artifacts / "window.png"
    result = run_binary(
        ["--smoke-test", theme, str(screenshot)], isolated_environment(tmp_path),
        artifacts, tmp_path,
    )
    assert result.returncode == 0, result.stderr
    assert screenshot.is_file(), result.stderr
    check_backend(result.stderr)
    check_pixels(png.read_png(screenshot.read_bytes()), BACKGROUNDS[theme])


@pytest.mark.parametrize("case,theme,relative,test_mode,error", [
    ("no-opt-in", "dark", False, False, "requires WREN_TEST_MODE=1"),
    ("unknown-theme", "neon", False, True, "unknown smoke theme"),
    ("relative-path", "dark", True, True, "output path must be absolute"),
])
def test_invalid_capture_is_rejected_before_gui(
    case: str, theme: str, relative: bool, test_mode: bool, error: str, tmp_path: Path,
) -> None:
    env = isolated_environment(tmp_path, test_mode=test_mode)
    env.pop("DISPLAY", None)
    env.pop("WAYLAND_DISPLAY", None)
    output = tmp_path / "invalid.png"
    result = run_binary(
        ["--smoke-test", theme, output.name if relative else str(output)],
        env, artifact_directory(case), tmp_path,
    )
    assert result.returncode != 0
    assert error in result.stderr, result.stderr
    assert not output.exists()


def test_existing_output_is_not_overwritten(tmp_path: Path) -> None:
    output = tmp_path / "existing.png"
    original = b"existing contents must survive"
    output.write_bytes(original)
    result = run_binary(
        ["--smoke-test", "dark", str(output)], isolated_environment(tmp_path),
        artifact_directory("existing-output"), tmp_path,
    )
    assert result.returncode != 0
    assert "renderer smoke capture did not complete successfully" in result.stderr
    assert output.read_bytes() == original
