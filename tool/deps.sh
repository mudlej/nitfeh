#!/usr/bin/env bash
# Builds static libopus and a trimmed, LGPL-only FFmpeg into work/deps/<target>/ for build.rs to link.
# Usage: tool/deps.sh [rust-target-triple]   (default: the host)
set -euo pipefail

ROOT="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

OPUS_VERSION=1.6.1
OPUS_URL="https://downloads.xiph.org/releases/opus/opus-$OPUS_VERSION.tar.gz"
OPUS_SHA256=6ffcb593207be92584df15b32466ed64bbec99109f007c82205f0194572411a1
FFMPEG_VERSION=9.0.2
FFMPEG_URL="https://ffmpeg.org/releases/ffmpeg-$FFMPEG_VERSION.tar.xz"
FFMPEG_SHA256=8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e

die() { printf 'deps: %s\n' "$*" >&2; exit 1; }
sha256_of() { # macOS has shasum, not sha256sum
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}
say() { printf '\033[36m▸\033[0m %s\n' "$*"; }

TARGET="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
SRC="$ROOT/work/src"
BUILD="$ROOT/work/build/$TARGET"
PREFIX="$ROOT/work/deps/$TARGET"
JOBS="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"

opus_flags=()
ffmpeg_flags=()
case "$TARGET" in
  x86_64-unknown-linux-gnu | aarch64-unknown-linux-gnu) ;;
  x86_64-unknown-linux-musl)
    command -v musl-gcc >/dev/null || die "musl-gcc is not installed"
    export CC=musl-gcc
    ffmpeg_flags+=(--cc=musl-gcc)
    ;;
  x86_64-pc-windows-gnu)
    command -v x86_64-w64-mingw32-gcc >/dev/null || die "x86_64-w64-mingw32-gcc is not installed"
    # Stack protector and _FORTIFY_SOURCE need libssp on mingw, which would be one more DLL: opus does without.
    opus_flags+=(--host=x86_64-w64-mingw32 --disable-stack-protector "CFLAGS=-O2 -D_FORTIFY_SOURCE=0")
    ffmpeg_flags+=(--arch=x86_64 --target-os=mingw32 --cross-prefix=x86_64-w64-mingw32- --pkg-config=pkg-config)
    ;;
  aarch64-apple-darwin | x86_64-apple-darwin)
    arch="${TARGET%%-*}"
    [[ "$arch" == aarch64 ]] && arch=arm64
    min="-mmacosx-version-min=11.0"
    export CC="clang -arch $arch $min"
    opus_flags+=(--host="${TARGET%%-*}-apple-darwin")
    ffmpeg_flags+=(--arch="$arch" --target-os=darwin --cc="clang -arch $arch" --extra-cflags="$min"
      --extra-ldflags="$min" --enable-cross-compile)
    ;;
  *) die "unsupported target: $TARGET" ;;
esac

# Hand-written x86 assembly needs nasm; without it FFmpeg still works, only slower.
if [[ "$TARGET" == x86_64-* ]] && ! command -v nasm >/dev/null; then
  say "nasm not found: building FFmpeg without x86 assembly (slower decoding)"
  ffmpeg_flags+=(--disable-x86asm)
fi

STAMP="$PREFIX/.stamp"
WANT="opus=$OPUS_VERSION ffmpeg=$FFMPEG_VERSION flags=${opus_flags[*]} ${ffmpeg_flags[*]} script=$(sha256_of "${BASH_SOURCE[0]}" | cut -c1-16)"
if [[ -f "$STAMP" && "$(cat "$STAMP")" == "$WANT" ]]; then
  say "work/deps/$TARGET is up to date"
  exit 0
fi

fetch() { # <url> <sha256>
  local file="$SRC/${1##*/}"
  mkdir -p "$SRC"
  if [[ ! -f "$file" ]]; then
    curl -fsSL --retry 3 -o "$file.tmp" "$1" || die "download failed: $1"
    mv "$file.tmp" "$file"
  fi
  [[ "$(sha256_of "$file")" == "$2" ]] || die "checksum mismatch: $file"
}

say "fetching sources"
fetch "$OPUS_URL" "$OPUS_SHA256"
fetch "$FFMPEG_URL" "$FFMPEG_SHA256"

rm -rf "$BUILD" "$PREFIX"
mkdir -p "$BUILD" "$PREFIX"
tar -xf "$SRC/opus-$OPUS_VERSION.tar.gz" -C "$BUILD"
tar -xf "$SRC/ffmpeg-$FFMPEG_VERSION.tar.xz" -C "$BUILD"

say "building libopus $OPUS_VERSION for $TARGET"
(
  cd "$BUILD/opus-$OPUS_VERSION"
  ./configure --prefix="$PREFIX" --enable-static --disable-shared --with-pic \
    --disable-doc --disable-extra-programs "${opus_flags[@]}" >"$BUILD/opus-configure.log" 2>&1 ||
    die "opus configure failed: $BUILD/opus-configure.log"
  make -j"$JOBS" >"$BUILD/opus-make.log" 2>&1 || die "opus build failed: $BUILD/opus-make.log"
  make install >>"$BUILD/opus-make.log" 2>&1
)

# Every native audio decoder, read from FFmpeg's own registry (video and subtitle decoders are left out).
audio_decoders="$(awk '
  /\/\* audio codecs \*\//      { on = 1 }
  /\/\* subtitles \*\//         { on = 0 }
  /\/\* external libraries \*\// { on = 0 }
  on && match($0, /ff_[a-z0-9_]+_decoder/) { print substr($0, RSTART + 3, RLENGTH - 11) }
' "$BUILD/ffmpeg-$FFMPEG_VERSION/libavcodec/allcodecs.c" | sort -u | paste -sd, -)"
[[ "$audio_decoders" == *aac* && "$audio_decoders" == *wmav2* ]] || die "could not read the audio decoder list"

say "building FFmpeg $FFMPEG_VERSION for $TARGET"
(
  cd "$BUILD/ffmpeg-$FFMPEG_VERSION"
  PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig" PKG_CONFIG_PATH="" ./configure \
    --prefix="$PREFIX" --enable-static --disable-shared --enable-pic \
    --disable-programs --disable-doc --disable-debug --disable-autodetect --disable-network \
    --disable-avdevice --disable-avfilter --disable-swscale \
    --disable-everything --enable-demuxers --enable-parsers \
    --enable-decoder="$audio_decoders" \
    --enable-libopus --enable-encoder=libopus --enable-muxer=ogg,opus --enable-protocol=file \
    --pkg-config-flags=--static "${ffmpeg_flags[@]}" >"$BUILD/ffmpeg-configure.log" 2>&1 ||
    die "ffmpeg configure failed: $BUILD/ffmpeg-configure.log (and ffbuild/config.log)"
  grep -q '^License: LGPL' "$BUILD/ffmpeg-configure.log" || die "FFmpeg is not configured as LGPL"
  make -j"$JOBS" >"$BUILD/ffmpeg-make.log" 2>&1 || die "ffmpeg build failed: $BUILD/ffmpeg-make.log"
  make install >>"$BUILD/ffmpeg-make.log" 2>&1
  cp ffbuild/config.log "$PREFIX/ffmpeg-config.log"
)

# Kept beside the libraries: the exact configuration is part of what the LGPL asks to be published.
sed -n 's/^# \(.*configure.*\)$/\1/p' "$PREFIX/ffmpeg-config.log" | head -1 >"$PREFIX/ffmpeg-configure.txt"
echo "$WANT" >"$STAMP"
say "done: $(du -sh "$PREFIX/lib" | cut -f1) in work/deps/$TARGET/lib"
