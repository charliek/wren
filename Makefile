.DEFAULT_GOAL := help
PACKAGE_HOST := $(shell rustc --print host-tuple)

.PHONY: help setup build run test test-python fmt fmt-check clippy check build-smoke smoke-x11 smoke-wayland perf-search package package-debug

help:
	@printf '%s\n' 'Wren development commands:' '  make setup          Fetch locked Rust/Python dependencies.' '  make run            Restore the last workspace, or show onboarding.' '  make run ARGS="--workspace /ABS/PATH"  Browse and autosave Markdown notes.' '  make run ARGS="--editor"  Launch the unsaved editor evaluation.' '  make check          Run formatting, Clippy, Rust, and Python unit tests.' '  make smoke-x11      Run real-window X11 acceptance under Xvfb.' '  make smoke-wayland  Run headless Wayland acceptance.' '  make perf-search    Measure release search and filesystem workloads.' '  make package        Build a normal release executable and uninstalled native bundle.' '  make package-debug  Build the same bundle with a normal debug executable.' '  PACKAGE_OUTPUT=/new/path overrides the bundle output; existing outputs are never replaced.'

setup:
	cargo fetch --locked
	uv sync --locked --group test

build:
	cargo build --workspace --locked

run:
	cargo run --locked -p wren-iced -- $(ARGS)

test:
	cargo test --workspace --locked
	cargo test --workspace --locked --features smoke-test

test-python:
	uv run --locked --group test python -m unittest discover -s tools/wrentest_unit -v

perf-search:
	cargo test -p wren-core --lib --release --locked index::tests::benchmark_search_5000_notes_25mb_under_30ms -- --ignored --exact --nocapture
	cargo test -p wren-core --test navigation --release --locked benchmark_filesystem_5000_notes_25mb_load_reconcile_and_edits -- --ignored --exact --nocapture

package:
	cargo build --locked --release -p wren-iced --target-dir target/package-build --target "$(PACKAGE_HOST)"
	cargo fetch --locked
	uv run --locked --group test python tools/package.py --binary "target/package-build/$(PACKAGE_HOST)/release/wren" $(if $(PACKAGE_OUTPUT),--output "$(PACKAGE_OUTPUT)")

package-debug:
	cargo build --locked -p wren-iced --target-dir target/package-build --target "$(PACKAGE_HOST)"
	cargo fetch --locked
	uv run --locked --group test python tools/package.py --binary "target/package-build/$(PACKAGE_HOST)/debug/wren" $(if $(PACKAGE_OUTPUT),--output "$(PACKAGE_OUTPUT)")

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

clippy:
	cargo clippy --workspace --all-targets --locked -- -D warnings
	cargo clippy --workspace --all-targets --locked --features smoke-test -- -D warnings

check: fmt-check clippy test test-python

build-smoke:
	cargo build --locked -p wren-iced --features smoke-test

smoke-x11: build-smoke
	uv run --locked --group test python tools/run_display.py x11 -- uv run --locked --group test pytest tools/wrentest --junitxml=target/wrentest/x11-$(or $(ICED_BACKEND),default)/junit.xml

smoke-wayland: build-smoke
	uv run --locked --group test python tools/run_display.py wayland -- uv run --locked --group test pytest tools/wrentest -m 'not native_x11' --junitxml=target/wrentest/wayland-$(or $(ICED_BACKEND),default)/junit.xml
