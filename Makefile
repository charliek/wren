.DEFAULT_GOAL := help
PACKAGE_HOST := $(shell rustc --print host-tuple)
# Release artifacts (make deb / app / dmg). RELEASE_VERSION defaults to the
# [workspace.package] version; release.yml passes the tag's (X.Y.Z or
# X.Y.Z-pre), and tools/package.py refuses one that is not the Cargo version.
CARGO_VERSION := $(shell sed -n '/^\[workspace\.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)
RELEASE_VERSION ?= $(CARGO_VERSION)
RELEASE_BINARY := target/package-build/$(PACKAGE_HOST)/release/wren
DEB_HOST_ARCH := $(shell dpkg --print-architecture 2>/dev/null)
ARCH ?= $(DEB_HOST_ARCH)
NFPM ?= nfpm
# The Sparkle feed and key every Wren.app built here carries. Installed copies
# poll this URL forever, so the appcast must never move (RELEASING.md).
SPARKLE_FEED_URL ?= https://raw.githubusercontent.com/charliek/wren/main/packaging/macos/appcast.xml
SPARKLE_PUBLIC_KEY ?= packaging/macos/sparkle-ed-public-key.txt
# What the smoke targets hand pytest: a folder, files or a path glob, such as
# only the board suite. A glob, not -k, so "board" does not match "keyboard".
SMOKE_TESTS ?= tools/wrentest

.PHONY: help setup build run test test-python fmt fmt-check clippy check check-patches build-smoke smoke-x11 smoke-wayland perf-search fuzz-board perf-board package package-debug release-binary deb app dmg icons

help:
	@printf '%s\n' 'Wren development commands:' '  make setup          Fetch locked Rust/Python dependencies.' '  make run            Restore the last workspace, or show onboarding.' '  make run ARGS="--workspace /ABS/PATH"  Browse and autosave Markdown notes.' '  make run ARGS="--editor"  Launch the unsaved editor evaluation.' '  make check          Run formatting, Clippy, Rust, and Python unit tests.' '  make smoke-x11      Run real-window X11 acceptance under Xvfb.' '  make smoke-wayland  Run headless Wayland acceptance.' '  make perf-search    Measure release search and filesystem workloads.' '  make fuzz-board     Run the board format property tests at full count, in release.' '  make perf-board     Time board parse, write and scene build, in release.' '  make smoke-x11 SMOKE_TESTS="PATHS"  Run only those test files.' '  make package        Build a normal release executable and uninstalled native bundle.' '  make package-debug  Build the same bundle with a normal debug executable.' '  PACKAGE_OUTPUT=/new/path overrides the bundle output; existing outputs are never replaced.' '  make deb            Linux: build target/dist/wren_<version>_<arch>.deb with nfpm (native ARCH only).' '  make app            macOS: build and sign target/dist/macos/Wren.app (Developer ID if WREN_DEVELOPER_ID_IDENTITY is set).' '  make dmg            macOS: package that Wren.app as target/dist/Wren-<version>.dmg.' '  RELEASE_VERSION=X.Y.Z[-pre] names the release; it must equal the Cargo version.' '  make icons          Regenerate committed app icons from assets/icon/wren-icon.svg.' '  make icons ICON_COLORS="eyes=#FFAA00"  Recolor a role in the SVG, then regenerate.'

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

# Full counts of the ignored wren-board tests; the default run keeps small
# counts so `make test` stays fast. Names start fuzz_ or perf_ to be picked here.
fuzz-board:
	cargo test -p wren-board --lib --release --locked fuzz_ -- --ignored --nocapture

perf-board:
	cargo test -p wren-board --lib --release --locked perf_ -- --ignored --nocapture --test-threads=1

package:
	cargo build --locked --release -p wren-iced --target-dir target/package-build --target "$(PACKAGE_HOST)"
	cargo fetch --locked
	uv run --locked --group test python tools/package.py --binary "target/package-build/$(PACKAGE_HOST)/release/wren" $(if $(PACKAGE_OUTPUT),--output "$(PACKAGE_OUTPUT)")

package-debug:
	cargo build --locked -p wren-iced --target-dir target/package-build --target "$(PACKAGE_HOST)"
	cargo fetch --locked
	uv run --locked --group test python tools/package.py --binary "target/package-build/$(PACKAGE_HOST)/debug/wren" $(if $(PACKAGE_OUTPUT),--output "$(PACKAGE_OUTPUT)")

# Release artifacts. Unlike `make package`, whose outputs are never replaced,
# these rebuild fixed paths under target/: each run is one artifact for one
# RELEASE_VERSION, and .github/workflows/release.yml calls exactly these.
release-binary:
	cargo build --locked --release -p wren-iced --target-dir target/package-build --target "$(PACKAGE_HOST)"
	cargo fetch --locked

# nfpm packages the /usr tree tools/package.py stages (packaging/nfpm.yaml), so
# the bundle layout keeps one source. Built natively: ARCH must be this host's.
deb:
	@[ -n "$(DEB_HOST_ARCH)" ] || { echo "make deb needs dpkg (a Debian/Ubuntu build host)" >&2; exit 1; }
	@[ "$(ARCH)" = "$(DEB_HOST_ARCH)" ] || { echo "make deb builds natively: ARCH=$(ARCH) but this host is $(DEB_HOST_ARCH)" >&2; exit 1; }
	$(MAKE) release-binary
	rm -rf target/deb-root
	rm -f target/dist/wren_*.deb
	mkdir -p target/dist
	uv run --locked --group test python tools/package.py --binary "$(RELEASE_BINARY)" --system-layout --release-version "$(RELEASE_VERSION)" --output target/deb-root/usr
	WREN_DEB_VERSION="$(RELEASE_VERSION)" WREN_DEB_ARCH="$(ARCH)" "$(NFPM)" pkg --packager deb --config packaging/nfpm.yaml --target target/dist/
	@ls -l target/dist/wren_*.deb

app:
	@[ "$$(uname -s)" = Darwin ] || { echo "make app builds Wren.app on macOS" >&2; exit 1; }
	$(MAKE) release-binary
	rm -rf target/dist/macos
	third_party/sparkle/fetch.sh
	uv run --locked --group test python tools/package.py --binary "$(RELEASE_BINARY)" --release-version "$(RELEASE_VERSION)" --output target/dist/macos/Wren.app \
		--sparkle-feed-url "$(SPARKLE_FEED_URL)" --sparkle-public-key-file "$(SPARKLE_PUBLIC_KEY)"
	scripts/macos/embed-sparkle.sh target/dist/macos/Wren.app
	scripts/macos/sign-app.sh target/dist/macos/Wren.app

dmg:
	scripts/macos/make-dmg.sh target/dist/macos/Wren.app "$(RELEASE_VERSION)" "target/dist/Wren-$(RELEASE_VERSION).dmg"

# Icon rendering needs cairosvg (and the system libcairo) plus Pillow, supplied
# in a throwaway environment so the locked project dependencies stay untouched.
# Outputs are committed; builds and CI never run this target.
icons:
	uv run --no-project --with cairosvg==2.9.1 --with pillow==13.0.0 python tools/generate_icons.py $(ICON_COLORS)

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

# The editor and board crates also build on their own, without features the
# app unifies in.
clippy:
	cargo clippy --workspace --all-targets --locked -- -D warnings
	cargo clippy --workspace --all-targets --locked --features smoke-test -- -D warnings
	cargo clippy -p wren-editor --all-targets --locked -- -D warnings
	cargo clippy -p wren-board --all-targets --locked -- -D warnings
	cargo clippy -p wren-canvas --all-targets --locked -- -D warnings

check: fmt-check clippy test test-python check-patches

# A [patch.crates-io] entry silently stops applying when a dependency bump
# needs a newer version, so assert the vendored crates are still the ones used.
check-patches:
	@cargo tree --locked -p wren-iced | grep -q 'swash v0.2.10 (.*vendor/swash)' || \
		( echo "swash [patch.crates-io] not applied"; exit 1 )
	@cargo tree --locked -p wren-iced | grep -q 'cosmic-text v0.15.0 (.*vendor/cosmic-text)' || \
		( echo "cosmic-text [patch.crates-io] not applied"; exit 1 )

build-smoke:
	cargo build --locked -p wren-iced --features smoke-test

smoke-x11: build-smoke
	uv run --locked --group test python tools/run_display.py x11 -- uv run --locked --group test pytest $(SMOKE_TESTS) --junitxml=target/wrentest/x11-$(or $(ICED_BACKEND),default)/junit.xml

smoke-wayland: build-smoke
	uv run --locked --group test python tools/run_display.py wayland -- uv run --locked --group test pytest $(SMOKE_TESTS) -m 'not native_x11' --junitxml=target/wrentest/wayland-$(or $(ICED_BACKEND),default)/junit.xml
