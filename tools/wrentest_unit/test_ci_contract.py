"""Keep the small CI workflow and its fail-closed aggregate in agreement."""

import json
from pathlib import Path
import re
import subprocess
import sys
import unittest

from tools.ci_gate import all_succeeded

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/ci.yml"


def job_blocks(source: str) -> dict[str, str]:
    # This workflow deliberately uses two-space job IDs and inline needs lists.
    source = source.split("\njobs:\n", 1)[1]
    matches = list(re.finditer(r"^  ([A-Za-z_][A-Za-z0-9_-]*):$", source, re.MULTILINE))
    return {
        match[1]: source[match.end():matches[i + 1].start() if i + 1 < len(matches) else None]
        for i, match in enumerate(matches)
    }


class WorkflowContractTests(unittest.TestCase):
    def test_parser_includes_all_valid_job_ids(self):
        self.assertEqual(set(job_blocks("\njobs:\n  _Native_check-1:\n  ci-success:\n")),
                         {"_Native_check-1", "ci-success"})

    def test_aggregate_requires_every_job_unconditionally(self):
        jobs = job_blocks(WORKFLOW.read_text())
        gate = jobs.pop("ci-success")
        needs = re.search(r"^    needs: \[([^\]]+)\]$", gate, re.MULTILINE)
        self.assertIsNotNone(needs)
        self.assertEqual({name.strip() for name in needs[1].split(",")}, set(jobs))
        self.assertIn("    if: always()\n", gate)
        self.assertIn("run: python3 tools/ci_gate.py", gate)
        self.assertIn("NEEDS: ${{ toJSON(needs) }}", gate)
        for name, block in jobs.items():
            self.assertNotRegex(block, r"(?m)^    if:", name)
            self.assertNotIn("continue-on-error:", block, name)

    def test_jobs_use_existing_shared_make_targets_and_tests(self):
        source = WORKFLOW.read_text()
        jobs = job_blocks(source)
        self.assertIn("run: make build fmt-check clippy test check-patches", jobs["rust"])
        self.assertIn("run: make test-python", jobs["harness-unit"])
        self.assertIn('run: make setup "smoke-${DISPLAY_BACKEND}"', jobs["linux-smoke"])
        makefile = (ROOT / "Makefile").read_text()
        for target in ("setup", "build", "fmt-check", "clippy", "test", "test-python",
                       "smoke-x11", "smoke-wayland"):
            self.assertRegex(makefile, rf"(?m)^{target}:")
        for path in set(re.findall(r"\btools/[\w./-]+", source + makefile)):
            self.assertTrue((ROOT / path).exists(), path)
        for suite in ("wrentest", "wrentest_unit"):
            self.assertTrue(list((ROOT / "tools" / suite).glob("test_*.py")), suite)

    def test_native_bundle_rebuilds_without_smoke_features_and_is_preserved(self):
        rust = job_blocks(WORKFLOW.read_text())["rust"]
        self.assertIn("run: make package-debug", rust)
        self.assertIn("uses: astral-sh/setup-uv@v7", rust)
        self.assertLess(rust.index("uses: astral-sh/setup-uv@v7"), rust.index("run: make package-debug"))
        self.assertIn('UV_PYTHON: "3.12"', WORKFLOW.read_text())
        self.assertIn("name: wren-debug-${{ matrix.os }}", rust)
        self.assertIn("run: tar -czf target/wren-debug.tar.gz -C target/package .", rust)
        self.assertIn("path: target/wren-debug.tar.gz", rust)
        self.assertIn("if-no-files-found: error", rust)
        makefile = (ROOT / "Makefile").read_text()
        self.assertIn("PACKAGE_HOST := $(shell rustc --print host-tuple)", makefile)
        for target, profile, build in (
            ("package", "release", "cargo build --locked --release -p wren-iced"),
            ("package-debug", "debug", "cargo build --locked -p wren-iced"),
        ):
            block = re.search(rf"(?m)^{target}:\n((?:\t[^\n]*\n)+)", makefile)[1]
            self.assertIn(build + ' --target-dir target/package-build --target "$(PACKAGE_HOST)"\n', block)
            self.assertIn("cargo fetch --locked\n", block)
            self.assertIn(f'uv run --locked --group test python tools/package.py --binary "target/package-build/$(PACKAGE_HOST)/{profile}/wren"', block)
            self.assertNotIn("--features", block)
            self.assertNotIn("rm ", block)
            self.assertIn("--output", block)

    def test_platform_matrix_and_failure_artifacts(self):
        source = WORKFLOW.read_text()
        self.assertIn("os: [ubuntu-24.04, macos-15]", source)
        self.assertIn("display: [x11, wayland]", source)
        self.assertIn("renderer: [wgpu, tiny-skia]", source)
        smoke = job_blocks(source)["linux-smoke"]
        self.assertIn("ICED_BACKEND: ${{ matrix.renderer }}", smoke)
        self.assertIn("WGPU_BACKEND: vulkan", smoke)
        # Every display and renderer lane runs the editor suite on both engines.
        self.assertIn("WRENTEST_ENGINE: classic,advanced", smoke)
        self.assertRegex(smoke, r"if: always\(\)\n +uses: actions/upload-artifact@")
        self.assertIn("name: wrentest-${{ matrix.display }}-${{ matrix.renderer }}", smoke)
        self.assertIn("path: target/wrentest/", smoke)
        self.assertNotIn("paths:", source)
        self.assertNotIn("paths-ignore:", source)


class PerformanceWorkflowTests(unittest.TestCase):
    def test_scheduled_lane_runs_the_real_release_benchmark(self):
        source = (ROOT / ".github/workflows/performance.yml").read_text()
        self.assertIn("  schedule:\n", source)
        self.assertRegex(source, r'cron: "[^"\n]+"')
        self.assertIn("  workflow_dispatch:\n", source)
        jobs = job_blocks(source)
        self.assertEqual(set(jobs), {"search-performance"})
        self.assertNotRegex(jobs["search-performance"], r"(?m)^    if:")
        self.assertNotIn("continue-on-error:", source)
        self.assertIn("make perf-search", source)
        self.assertIn('exit "$status"', source)
        makefile = (ROOT / "Makefile").read_text()
        self.assertIn("cargo test -p wren-core --lib --release --locked", makefile)
        self.assertIn("index::tests::benchmark_search_5000_notes_25mb_under_30ms", makefile)
        self.assertIn("cargo test -p wren-core --test navigation --release --locked", makefile)
        self.assertIn("benchmark_filesystem_5000_notes_25mb_load_reconcile_and_edits", makefile)
        self.assertIn("-- --ignored --exact --nocapture", makefile)
        self.assertIn("if: always()", source)
        self.assertIn("path: target/performance/", source)
        self.assertIn("if-no-files-found: error", source)


class GateTests(unittest.TestCase):
    def test_invalid_or_empty_needs_is_rejected(self):
        for raw in (None, "", " ", "{", "{}", "[]", "null", '"success"',
                    '{"rust": {}}', '{"rust": null}', '{"rust": "success"}'):
            with self.subTest(raw=raw):
                self.assertFalse(all_succeeded(raw))

    def test_every_result_must_be_success(self):
        for status in (None, "failure", "cancelled", "skipped", "abandoned", "", "SUCCESS", 1):
            with self.subTest(status=status):
                self.assertFalse(all_succeeded(json.dumps({
                    "rust": {"result": "success"}, "harness-unit": {"result": status},
                })))

    def test_all_success(self):
        self.assertTrue(all_succeeded(json.dumps({
            name: {"result": "success", "outputs": {}}
            for name in ("rust", "harness-unit", "linux-smoke")
        })))

    def test_cli_reads_environment_and_preserves_failure_exit(self):
        for env, code in (({}, 1), ({"NEEDS": '{"rust":{"result":"success"}}'}, 0),
                          ({"NEEDS": '{"rust":{"result":"failure"}}'}, 1)):
            with self.subTest(env=env):
                result = subprocess.run([sys.executable, str(ROOT / "tools/ci_gate.py")],
                                        env=env, capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, code, result.stderr)


if __name__ == "__main__":
    unittest.main()
