# Agent notes

- Read `README.md` first; keep it in step with behaviour.
- Pushing a tag `v<version>` publishes a release (`.github/workflows/release.yml`); the tag must equal the version in
  `Cargo.toml`. `ci.yml` and `release.yml` both call `build.yml`, which builds and starts all three files.
- Use only `just` recipes (`just` lists them); they use the Rust pinned in `rust-toolchain.toml`. New commands
  become recipes in `justfile`. The box in `distrobox.ini` installs its tools itself (init hooks);
  `first_time_setup.sh` is the bootstrap outside the box; `tool/` holds what recipes call.
- Everything produced locally goes under `work/` (FFmpeg and libopus in `work/deps`, release files in `work/dist`,
  screenshots in `work/shots`) or `target/`; git ignores both.
- `just check` must pass before handing work back.
- For visual checks of the window: `just shot <name> [--gui <input>]`, then read the PNG. `NITFEH_THEME=dark|light`
  forces the theme.
- The encoder settings in `src/preset.rs` and the pipeline in `native/transcode.c` are what the output files depend
  on. Do not change them without saying so: the result is checked against an `ffmpeg` built from the same sources.
- A sandbox cannot build for macOS, run a real Windows, or open the system's file pickers. Say what was only
  cross-built or run under wine (`just try-windows`) rather than calling it tested.
- Every text the window shows is an `@tr()` in `ui/app.slint`.
- Comments only where they matter. Keep tests light and high value.
