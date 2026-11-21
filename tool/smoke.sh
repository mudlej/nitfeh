#!/usr/bin/env bash
# Converts two seconds of silence with the given nitfeh and checks that an Ogg Opus file comes out.
# Needs nothing but bash, so it runs the same on Linux, macOS and Windows (Git Bash).
# Usage: tool/smoke.sh <nitfeh executable>
set -euo pipefail

BINARY="${1:?usage: smoke.sh <nitfeh executable>}"
DIR="$(mktemp -d)"
trap 'rm -rf -- "$DIR"' EXIT

chmod +x "$BINARY" 2>/dev/null || true
"$BINARY" --version

# A 44-byte WAV header for mono, 16 kHz, 16 bit, then 64000 bytes of samples.
printf 'RIFF\x24\xfa\x00\x00WAVEfmt \x10\x00\x00\x00\x01\x00\x01\x00\x80\x3e\x00\x00\x00\x7d\x00\x00\x02\x00\x10\x00data\x00\xfa\x00\x00' >"$DIR/silence.wav"
head -c 64000 /dev/zero >>"$DIR/silence.wav"

"$BINARY" "$DIR/silence.wav" "$DIR/out.opus"
[[ "$(head -c 4 "$DIR/out.opus")" == "OggS" ]] || { echo "smoke: the output is not an Ogg file" >&2; exit 1; }
grep -q OpusHead "$DIR/out.opus" || { echo "smoke: the output holds no Opus stream" >&2; exit 1; }
echo "smoke: ok ($(wc -c <"$DIR/out.opus" | tr -d ' ') bytes)"
