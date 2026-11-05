.DEFAULT_GOAL := help

.PHONY: help setup build run test test-python fmt fmt-check clippy check build-smoke smoke-x11 smoke-wayland

help:
	@printf '%s\n' 'Wren development commands:' '  make setup          Fetch locked Rust/Python dependencies.' '  make run            Launch the development scaffold (not a notes editor yet).' '  make check          Run formatting, Clippy, Rust, and Python unit tests.' '  make smoke-x11      Capture both themes in a real X11 window under Xvfb.' '  make smoke-wayland  Capture both themes in a real headless Wayland window.'

setup:
	cargo fetch --locked
	uv sync --locked --group test

build:
	cargo build --workspace --locked

run:
	cargo run --locked -p wren-iced

test:
	cargo test --workspace --locked
	cargo test --workspace --locked --features smoke-test

test-python:
	uv run --locked --group test python -m unittest discover -s tools/wrentest_unit -v

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
	uv run --locked --group test python tools/run_display.py wayland -- uv run --locked --group test pytest tools/wrentest --junitxml=target/wrentest/wayland-$(or $(ICED_BACKEND),default)/junit.xml
