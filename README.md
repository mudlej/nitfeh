# Nitfeh

**Small audio from long lectures.**

[![CI](https://github.com/mudlej/nitfeh/actions/workflows/ci.yml/badge.svg)](https://github.com/mudlej/nitfeh/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/mudlej/nitfeh)](https://github.com/mudlej/nitfeh/releases/latest)
[![Licence: GPL v3](https://img.shields.io/badge/licence-GPLv3-blue)](LICENSE)

Nitfeh turns lecture videos and recordings into small audio files: one voice channel, 16 kHz, Opus tuned for
speech. An hour of lecture is about 5 MB. It is one file per system, with a command line and a window, and needs
nothing installed.

## Getting started

Download the file for your system from the [latest release](https://github.com/mudlej/nitfeh/releases/latest):

| System | File | First start |
|---|---|---|
| Linux (x86-64, glibc 2.35 or newer) | `nitfeh-linux-x86_64` | `chmod +x nitfeh-linux-x86_64` |
| Windows 10 and 11 (x86-64) | `nitfeh-windows-x86_64.exe` | SmartScreen: "More info", then "Run anyway" |
| macOS 11 or newer (Intel and Apple silicon) | `Nitfeh-macos.dmg` | Open it and drag Nitfeh to Applications; the first time, right-click the app and choose Open |

Start it without arguments for the window, or from a terminal:

    nitfeh lecture.mp4                  # lecture.opus beside it
    nitfeh lecture.mp4 small.opus
    nitfeh course/                      # every video in course/ into course-opus/
    nitfeh -r -p small course/ out/     # subfolders too, at 10 kbps

On macOS the command line is inside the app: `/Applications/Nitfeh.app/Contents/MacOS/nitfeh`.

| Preset | Bitrate | An hour is about | |
|---|---|---|---|
| `standard` (default) | 12 kbps | 5.4 MB | Clear speech |
| `small` | 10 kbps | 4.5 MB | A little rougher on noisy recordings |
| `smallest` | 9 kbps | 4.1 MB | The lowest that keeps wideband speech; below it Opus drops to telephone quality |

All three are mono, 16 kHz, libopus with `application=voip`, 60 ms frames and VBR, in an Ogg `.opus` file.

## What it does with files

- A file goes to `name.opus` beside it; a folder goes to a sibling `<folder>-opus/`. Give a second argument to choose.
- Folders are read flat. `-r` takes subfolders too and keeps their layout. `-a` also converts audio files.
- What is already converted is left alone, so an interrupted run can simply be started again. `--overwrite`
  converts it again in place; `--copy` keeps both, saving the new one as `name (2).opus`. The window asks which
  when everything you chose is already converted.
- A file is written as `name.opus.part` and renamed when complete. Ctrl-C, Cancel or closing the window removes it.
- `lec.mkv` and `lec.mp4` in one folder become `lec.mkv.opus` and `lec.mp4.opus`.
- A file that cannot be converted is reported at the end; the rest still run. Exit code 1 then, 130 when cancelled.

The window is in English or Arabic: the button in its top corner switches, and the choice is remembered.
`NITFEH_LANG=ar|en` and `NITFEH_THEME=light|dark` override the language and the system's theme for one run.

## Building from source

    distrobox assemble create --file distrobox.ini && distrobox enter nitfeh
    just run        # build and open the window; `just run lecture.mp4` converts
    just check      # formatting, lints, tests and shellcheck
    just dist       # work/dist/nitfeh-linux-x86_64 and nitfeh-windows-x86_64.exe
    just dist-macos # on a Mac: work/dist/Nitfeh-macos.dmg

Creating the box installs every tool, Just and rustup included. The first recipe then installs the Rust pinned in
`rust-toolchain.toml` and builds static FFmpeg and libopus from checksum-pinned sources into `work/deps/`.
Without the box, on Linux or macOS, run `./first_time_setup.sh` once instead: it adds Just and rustup when they
are missing and says which system packages it could not find.

`just` lists every recipe. The tests use the system's `ffmpeg` and `ffprobe` to make sample files and skip
themselves without them.

Pushing a tag `v<version>` (the version in `Cargo.toml`) builds the three files in CI and publishes a release.

| Path | What |
|---|---|
| `native/transcode.c` | The audio pipeline on FFmpeg's libraries: demux, decode, downmix and resample, libopus, Ogg |
| `src/engine.rs` | Safe wrapper over it: `probe`, `transcode` |
| `src/scan.rs` | Input path and options into a plan: which files, where each goes |
| `src/runner.rs` | Runs a plan and reports events; the command line and the window only see those |
| `src/progress.rs` | Speed and time left, from media time done |
| `src/cli.rs`, `src/gui.rs` | The two front ends |
| `ui/` | The window (Slint): `app.slint`, the Expressive widgets in `widgets.slint`, colours in `theme.slint`, and Slint's Material components in `material/` |
| `lang/` | The window's Arabic (`just strings` brings it up to date with `ui/`) |
| `tool/` | What recipes call: `deps.sh` (FFmpeg and libopus for a target), `macos-app.sh`, `smoke.sh`, `icon.py` |
| `archive/` | The shell script this replaces |
| `work/` | Downloads, dependency builds, release files (not in git) |

## Licence

Made by [Mudlej](https://mudlej.com). GPL-3.0-or-later; see [LICENSE](LICENSE).

The released files contain FFmpeg (LGPL 2.1 or later, built without GPL or non-free parts; `tool/deps.sh` is the
exact build), libopus (BSD), IBM Plex Sans Arabic (SIL Open Font License, `assets/fonts/OFL.txt`), Slint
(used under GPLv3) with its Material components (MIT), and Google's Material Symbols icons (Apache 2.0,
`ui/icons/LICENSE`).
