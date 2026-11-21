# nitfeh: the command front door. Run `just` to list recipes.
#
# Everything a recipe produces lands in target/ (cargo) or work/ (downloads, the
# FFmpeg and libopus builds, release files, screenshots); git ignores both.

set shell := ["bash", "-euo", "pipefail", "-c"]
set positional-arguments

root := justfile_directory()
dist := root / "work" / "dist"
shots := root / "work" / "shots"
host := `rustc -vV | sed -n 's/^host: //p'`

default:
    @just --list --unsorted

# Build and start nitfeh: no arguments opens the window, `just run lecture.mp4` converts.
[no-cd]
run *args: build
    @"{{ root }}/target/release/nitfeh" "$@"

# The pinned Rust, the crates in Cargo.lock, and FFmpeg + libopus for this computer.
setup: deps
    cd "{{ root }}" && cargo fetch --locked

# Static FFmpeg + libopus for a target, into work/deps/ (once per target; downloads the sources the first time).
deps target=host:
    @"{{ root }}/tool/deps.sh" {{ target }}

# --------------------------------------------------------------------- quality --

# The full local gate: formatting, lints, every test, shell lint.
check: format-check lint test lint-shell

# Apply formatting.
format:
    cargo fmt --all

# Formatting check (CI form).
format-check:
    cargo fmt --all --check

# Lints, warnings fatal.
lint: deps
    cargo clippy --all-targets -- -D warnings

# Unit tests, and the conversions in tests/ (those need ffmpeg and ffprobe to make their sample files).
test: deps
    cargo test

# shellcheck the scripts.
lint-shell:
    @shellcheck tool/*.sh first_time_setup.sh

# Screenshot the window on a headless display into work/shots/<name>.png: `just shot folder --gui some/folder`.
shot name="window" *args: build
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{ shots }}"
    shift || true
    xvfb-run -a -s "-screen 0 600x760x24" bash -c '
      "$0" "$@" 2>/dev/null &
      pid=$!
      sleep 4
      scrot -o "{{ shots }}/{{ name }}.png"
      kill "$pid" 2>/dev/null || true
    ' "{{ root }}/target/release/nitfeh" "$@"
    echo "{{ shots }}/{{ name }}.png"

# ---------------------------------------------------------------------- builds --

# Release build for this computer.
build: deps
    cd "{{ root }}" && cargo build --release

# The release file for Linux: work/dist/nitfeh-linux-x86_64.
dist-linux: (deps "x86_64-unknown-linux-gnu")
    cargo build --release --target x86_64-unknown-linux-gnu
    mkdir -p "{{ dist }}" && cp target/x86_64-unknown-linux-gnu/release/nitfeh "{{ dist }}/nitfeh-linux-x86_64"

# The release file for Windows, cross-built with mingw-w64: work/dist/nitfeh-windows-x86_64.exe.
dist-windows: (deps "x86_64-pc-windows-gnu")
    cargo build --release --target x86_64-pc-windows-gnu
    mkdir -p "{{ dist }}" && cp target/x86_64-pc-windows-gnu/release/nitfeh.exe "{{ dist }}/nitfeh-windows-x86_64.exe"

# The release file for macOS, on a Mac: work/dist/Nitfeh-macos.zip (Nitfeh.app, for Intel and Apple silicon).
dist-macos: (deps "aarch64-apple-darwin") (deps "x86_64-apple-darwin")
    rustup target add aarch64-apple-darwin x86_64-apple-darwin
    MACOSX_DEPLOYMENT_TARGET=11.0 cargo build --release --target aarch64-apple-darwin
    MACOSX_DEPLOYMENT_TARGET=11.0 cargo build --release --target x86_64-apple-darwin
    mkdir -p "{{ dist }}"
    tool/macos-app.sh target/aarch64-apple-darwin/release/nitfeh target/x86_64-apple-darwin/release/nitfeh "{{ dist }}"

# Both release files a Linux computer can build. The macOS one is `just dist-macos` on a Mac, or CI.
dist: dist-linux dist-windows
    @ls -la "{{ dist }}"

# Convert a generated sound with a built file and check the result: `just smoke work/dist/nitfeh-linux-x86_64`.
[no-cd]
smoke binary:
    @"{{ root }}/tool/smoke.sh" "{{ binary }}"

# Run the Windows file under wine: `just try-windows lecture.mp4`. A smoke test, not a real Windows.
[no-cd]
try-windows *args: dist-windows
    @WINEDEBUG=-all wine "{{ dist }}/nitfeh-windows-x86_64.exe" "$@"

# Re-render the icon files in assets/ from tool/icon.py (needs Pillow).
icons:
    python3 tool/icon.py
