#!/usr/bin/env bash
# One-time bootstrap for nitfeh. Run from Bash, ideally inside the distrobox (see
# distrobox.ini). Installs checksum-verified Just and rustup when they are missing
# or too old, checks what building nitfeh needs, then runs `just setup`, which
# installs the Rust pinned in rust-toolchain.toml and builds FFmpeg and libopus.
# Safe to re-run.
set -euo pipefail

JUST_VERSION="1.57.0"
RUSTUP_VERSION="1.29.3"
ROOT_DIR="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="${NITFEH_BIN_DIR:-${HOME:-}/.local/bin}"
CARGO_BIN_DIR="${CARGO_HOME:-${HOME:-}/.cargo}/bin"

fail() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

has_command() {
  command -v "$1" >/dev/null 2>&1
}

# version_at_least <have> <want>
version_at_least() {
  [[ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n1)" == "$2" ]]
}

sha256_of() {
  if has_command sha256sum; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# fetch <url> <file> <sha256>
fetch() {
  has_command curl || fail "curl is needed to download $(basename "$2")."
  curl -fsSL --retry 3 --connect-timeout 10 --max-time 600 -o "$2" "$1" || fail "Could not download $1."
  [[ "$(sha256_of "$2")" == "$3" ]] || fail "Checksum verification failed for $(basename "$2")."
}

select_assets() {
  local just_target
  case "$(uname -s):$(uname -m)" in
    Linux:x86_64)
      just_target="x86_64-unknown-linux-musl"
      RUSTUP_TARGET="x86_64-unknown-linux-gnu"
      JUST_SHA256="45b548094283cb9739af8f13273b8cddeee869f5b4ef2bb631b1f311cb566155"
      RUSTUP_SHA256="b1c8cb14e30889e7e2999e6b69e62d9e9b9ffc0f4df8180564d306d9680e6c55"
      ;;
    Linux:aarch64 | Linux:arm64)
      just_target="aarch64-unknown-linux-musl"
      RUSTUP_TARGET="aarch64-unknown-linux-gnu"
      JUST_SHA256="f225044a81adea6e0b3a8b9370aaf374e6af76c8735ae263ac993df55fd137ec"
      RUSTUP_SHA256="9f608c142e0e38a78957d0661ba1dfaf0b6fb37184cfaa4b9a0aa57af6184282"
      ;;
    Darwin:x86_64)
      just_target="x86_64-apple-darwin"
      RUSTUP_TARGET="x86_64-apple-darwin"
      JUST_SHA256="5e6ade3698095576274b2b32cc9e5d467185e8e40b04949004c04cc3d7e962dc"
      RUSTUP_SHA256="bc107f1fb0228de7fa32413c89c1b305cea9cd256e94b01150cd054128145429"
      ;;
    Darwin:arm64 | Darwin:aarch64)
      just_target="aarch64-apple-darwin"
      RUSTUP_TARGET="aarch64-apple-darwin"
      JUST_SHA256="0381db216c2f97ce31d838a1562c1064dfbfa73f5a8a81581338a2cd9217df47"
      RUSTUP_SHA256="63752a1919a3a4793173ec70f8f212621a7ce38ba6a246095984482f4de20f9f"
      ;;
    *) fail "Unsupported system: $(uname -s) $(uname -m). Build nitfeh on Linux or macOS." ;;
  esac
  JUST_ARCHIVE="just-$JUST_VERSION-$just_target.tar.gz"
  JUST_URL="https://github.com/casey/just/releases/download/$JUST_VERSION/$JUST_ARCHIVE"
  RUSTUP_URL="https://static.rust-lang.org/rustup/archive/$RUSTUP_VERSION/$RUSTUP_TARGET/rustup-init"
}

just_is_recent() {
  local have
  have="$(just --version 2>/dev/null)" && version_at_least "${have#just }" "$JUST_VERSION"
}

install_just() {
  echo "Installing Just $JUST_VERSION into $BIN_DIR."
  fetch "$JUST_URL" "$TEMP_DIR/$JUST_ARCHIVE" "$JUST_SHA256"
  mkdir -p "$TEMP_DIR/just" "$BIN_DIR"
  tar -xzf "$TEMP_DIR/$JUST_ARCHIVE" -C "$TEMP_DIR/just"
  [[ -f "$TEMP_DIR/just/just" ]] || fail "The Just archive did not contain the expected binary."
  install -m755 "$TEMP_DIR/just/just" "$BIN_DIR/just"
  hash -r
}

# rustup only; the toolchain itself is the one rust-toolchain.toml names, installed on first use.
install_rustup() {
  echo "Installing rustup $RUSTUP_VERSION."
  fetch "$RUSTUP_URL" "$TEMP_DIR/rustup-init" "$RUSTUP_SHA256"
  chmod +x "$TEMP_DIR/rustup-init"
  "$TEMP_DIR/rustup-init" -y --no-modify-path --profile minimal --default-toolchain none
  hash -r
}

# Only needed when a tool went into a directory the shell does not search yet.
add_to_profile() { # <directory>
  local profile="$HOME/.bashrc"
  local entry="export PATH=\"$1:\$PATH\""
  [[ "${SHELL:-}" == */zsh ]] && profile="$HOME/.zshrc"
  if [[ -f "$profile" ]] && grep -qxF "$entry" "$profile"; then
    return
  fi
  printf '\n%s\n' "$entry" >>"$profile" || fail "Cannot add $1 to the PATH in $profile."
  echo "Added $1 to the PATH in $profile (open a new shell, or run: $entry)."
}

# Warn (don't fail) about what the recipes need; the distrobox has all of it.
check_prerequisites() {
  local missing=()
  local tool

  for tool in cc make pkg-config curl tar xz git; do
    has_command "$tool" || missing+=("$tool")
  done
  if [[ "$(uname -s)" == "Linux" ]]; then
    if has_command pkg-config && ! pkg-config --exists fontconfig; then
      missing+=("fontconfig-dev")
    fi
    has_command x86_64-w64-mingw32-gcc || missing+=("mingw-w64 (just dist-windows)")
    has_command wine || missing+=("wine (just try-windows)")
    for tool in Xvfb scrot xdotool; do
      has_command "$tool" || missing+=("$tool (just shot)")
    done
  fi
  has_command nasm || missing+=("nasm (faster decoding; optional)")
  for tool in ffmpeg ffprobe; do
    has_command "$tool" || missing+=("$tool (the conversion tests skip themselves without it)")
  done
  has_command shellcheck || missing+=("shellcheck (just check)")
  if [[ ${#missing[@]} -gt 0 ]]; then
    echo "WARNING: missing for some recipes: ${missing[*]}"
    echo "         The distrobox (distrobox.ini) provides all of them."
  fi
}

main() {
  local dir

  [[ -n "${HOME:-}" ]] || fail "HOME is not set."
  select_assets
  TEMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/nitfeh-setup.XXXXXX")" || fail "Cannot create a temporary directory."
  trap 'rm -rf -- "$TEMP_DIR"' EXIT

  local missing_from_path=()
  for dir in "$BIN_DIR" "$CARGO_BIN_DIR"; do
    case ":$PATH:" in
      *":$dir:"*) ;;
      *) missing_from_path+=("$dir") ;;
    esac
  done
  export PATH="$BIN_DIR:$CARGO_BIN_DIR:$PATH"

  if just_is_recent; then
    echo "$(just --version) is already installed (>= $JUST_VERSION)."
  else
    install_just
    [[ " ${missing_from_path[*]} " != *" $BIN_DIR "* ]] || add_to_profile "$BIN_DIR"
  fi

  if has_command rustup; then
    echo "$(rustup --version 2>/dev/null | head -n1 | cut -d' ' -f1-2) is already installed."
  elif has_command cargo; then
    echo "NOTE: cargo is here without rustup, so rust-toolchain.toml is not applied; nitfeh needs Rust 1.88 or newer."
  else
    install_rustup
    [[ " ${missing_from_path[*]} " != *" $CARGO_BIN_DIR "* ]] || add_to_profile "$CARGO_BIN_DIR"
  fi

  if has_command rustup; then
    echo "Installing the Rust pinned in rust-toolchain.toml (skipped if present)."
    (cd "$ROOT_DIR" && rustup toolchain install --no-self-update)
  fi

  check_prerequisites
  if has_command cc && has_command make; then
    echo "Running project setup (the first run downloads and builds FFmpeg and libopus)."
    (cd "$ROOT_DIR" && just setup)
    echo "nitfeh is ready. Start with: just run   (or: just --list)"
  else
    echo "A C compiler and make are missing, so the project was not set up. Enter the distrobox and run this again."
  fi
}

main "$@"
