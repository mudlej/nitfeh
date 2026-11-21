# istma

Turns lecture videos and recordings into small audio files: one voice channel, 16 kHz, Opus tuned for speech.
An hour of lecture is about 5 MB. It is one file per system, with a command line and a window, and needs
nothing installed.

```
istma lecture.mp4                  # lecture.opus beside it
istma lecture.mp4 small.opus
istma course/                      # every video in course/ into course-opus/
istma -r -p small course/ out/     # subfolders too, at 10 kbps
istma                              # the window
```

| Preset | Bitrate | An hour is about | |
|---|---|---|---|
| `standard` (default) | 12 kbps | 5.4 MB | Clear speech |
| `small` | 10 kbps | 4.5 MB | A little rougher on noisy recordings |
| `smallest` | 9 kbps | 4.1 MB | The lowest that keeps wideband speech; below it Opus drops to telephone quality |

All three are mono, 16 kHz, libopus with `application=voip`, 60 ms frames and VBR, in an Ogg `.opus` file.

## What it does with files

- A file goes to `name.opus` beside it; a folder goes to a sibling `<folder>-opus/`. Give a second argument to choose.
- Folders are read flat. `-r` takes subfolders too and keeps their layout. `-a` also converts audio files.
- What is already converted is left alone (`--overwrite` to redo), so an interrupted run can simply be started again.
- A file is written as `name.opus.part` and renamed when complete. Ctrl-C, Cancel or closing the window removes it.
- `lec.mkv` and `lec.mp4` in one folder become `lec.mkv.opus` and `lec.mp4.opus`.
- A file that cannot be converted is reported at the end; the rest still run. Exit code 1 then, 130 when cancelled.

`ISTMA_THEME=light|dark` overrides the system's theme for the window.

## Building

```
distrobox assemble create --file distrobox.ini && distrobox enter istma
./first_time_setup.sh
just run
```

The box (`distrobox.ini`, Debian 12) carries the system packages; `first_time_setup.sh` adds Just and rustup when
they are missing, installs the Rust pinned in `rust-toolchain.toml`, and builds static FFmpeg and libopus into
`work/deps/`. It also runs outside the box, on Linux or macOS, and says what is missing.

| Command | What it does |
|---|---|
| `just run [args]` | Build and start: no arguments opens the window, `just run lecture.mp4` converts |
| `just check` | Format check, lints, tests, shellcheck |
| `just shot <name> [--gui <input>]` | Screenshot the window on a headless display into `work/shots/` |
| `just dist` | `work/dist/istma-linux-x86_64` and `istma-windows-x86_64.exe` |
| `just try-windows [args]` | Run the Windows file under wine |
| `just deps [target]` | FFmpeg and libopus for a target (checksum-pinned sources) |
| `just icons` | Re-render the icon files from `tool/icon.py` |

The tests use the system's `ffmpeg` and `ffprobe` to make sample files, and skip themselves when those are
missing. Without `nasm`, FFmpeg is built without its assembly: slower decoding, same results.

| System | How it is built | State |
|---|---|---|
| Linux x86_64 | `just dist-linux` | Built and tested here. Uses the system's glibc and fontconfig. |
| Windows x86_64 | `just dist-windows` (cross-built with mingw-w64) | Built here, tested under Wine only. Imports only DLLs that ship with Windows. |
| macOS (Intel and Apple silicon) | `.github/workflows/build.yml` | Not built yet: it needs a Mac or the workflow running on GitHub. |

`cargo build --no-default-features` leaves the window out and gives the command line alone.

## Layout

| Path | What |
|---|---|
| `native/transcode.c` | The audio pipeline on FFmpeg's libraries: demux, decode, downmix and resample, libopus, Ogg |
| `src/engine.rs` | Safe wrapper over it: `probe`, `transcode` |
| `src/scan.rs` | Input path and options into a plan: which files, where each goes |
| `src/runner.rs` | Runs a plan and reports events; the CLI and the window only see those |
| `src/progress.rs` | Speed and time left, from media time done |
| `src/cli.rs`, `src/gui.rs` | The two front ends |
| `ui/` | The window (Slint): `app.slint`, the Expressive widgets in `widgets.slint`, colours in `theme.slint`, and Slint's Material components in `material/` |
| `distrobox.ini`, `first_time_setup.sh`, `justfile` | The development box, the bootstrap, and every command |
| `tool/deps.sh` | Fetches (checksum-pinned) and builds FFmpeg and libopus for a target |
| `archive/` | The shell script this replaces |
| `work/` | Downloads, dependency builds, release files (not in git) |

## Licences of what is inside

The released file contains FFmpeg (LGPL 2.1 or later, built without GPL or non-free parts), libopus (BSD),
IBM Plex Sans Arabic (SIL Open Font License, `assets/fonts/OFL.txt`), and Slint with its Material components
(Slint under its royalty-free licence, which asks for the attribution shown in About; the components are MIT).
Because FFmpeg is linked in statically, the LGPL asks that anyone who receives the file can rebuild it with a
changed FFmpeg: publishing this source with it, including `tool/deps.sh`, does that.
