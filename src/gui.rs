//! The window: glue between `ui/app.slint` and the scanner and runner.

use std::cell::RefCell;
use std::f32::consts::TAU;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use slint::{CloseRequestResponse, ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::preset::Preset;
use crate::progress::Estimator;
use crate::runner::{self, Event, FileResult};
use crate::scan::{self, AUDIO_EXTENSIONS, Plan, ScanOptions, VIDEO_EXTENSIONS};
use crate::text;

slint::include_modules!();

/// How often the progress view is refreshed; the runner reports far more often than that.
const REFRESH: Duration = Duration::from_millis(40);
/// The moving parts are redrawn this often while converting.
const FRAME: Duration = Duration::from_millis(33);
/// Typing in the output field rescans only after this pause.
const RESCAN_DELAY: Duration = Duration::from_millis(350);

#[derive(Default)]
struct State {
    input: Option<PathBuf>,
    /// Once the output was chosen or typed, picking another input no longer replaces it.
    output_chosen: bool,
    /// The scan matching what the window shows, with durations once they are known.
    plan: Option<Plan>,
    /// Only the newest scan may update the window.
    scan: u64,
    scan_cancel: Arc<AtomicBool>,
    run_cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    /// Advances the window's animation clock while converting.
    ticker: Option<Timer>,
    reveal: Option<PathBuf>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

pub fn run(input: Option<PathBuf>) -> Result<(), slint::PlatformError> {
    let ui = App::new()?;
    ui.set_version(env!("CARGO_PKG_VERSION").into());
    ui.set_preset_sizes(preset_sizes(None));

    match std::env::var("NITFEH_THEME").as_deref() {
        Ok("dark") => ui.invoke_use_scheme(true),
        Ok("light") => ui.invoke_use_scheme(false),
        _ => {}
    }

    ui.global::<Shapes>().on_wave(wave);
    ui.global::<Shapes>().on_scallop(scallop);

    let rescan_timer = Rc::new(Timer::default());

    ui.on_choose_file({
        let weak = ui.as_weak();
        move || {
            let weak = weak.clone();
            let _ = slint::spawn_local(async move {
                let extensions: Vec<&str> = VIDEO_EXTENSIONS.iter().chain(AUDIO_EXTENSIONS).copied().collect();
                let picked = rfd::AsyncFileDialog::new()
                    .add_filter("Video and audio", &extensions)
                    .add_filter("All files", &["*"])
                    .pick_file()
                    .await;
                if let (Some(file), Some(ui)) = (picked, weak.upgrade()) {
                    set_source(&ui, file.path().to_path_buf());
                }
            });
        }
    });
    ui.on_choose_folder({
        let weak = ui.as_weak();
        move || {
            let weak = weak.clone();
            let _ = slint::spawn_local(async move {
                let picked = rfd::AsyncFileDialog::new().pick_folder().await;
                if let (Some(folder), Some(ui)) = (picked, weak.upgrade()) {
                    set_source(&ui, folder.path().to_path_buf());
                }
            });
        }
    });
    ui.on_choose_output({
        let weak = ui.as_weak();
        move || {
            let weak = weak.clone();
            let _ = slint::spawn_local(async move {
                let Some(ui) = weak.upgrade() else { return };
                let is_folder = ui.get_source_is_folder();
                let current = PathBuf::from(ui.get_output().as_str());
                let picked = if is_folder {
                    rfd::AsyncFileDialog::new().pick_folder().await
                } else {
                    let mut dialog = rfd::AsyncFileDialog::new().add_filter("Opus audio", &["opus"]);
                    if let Some(name) = current.file_name() {
                        dialog = dialog.set_file_name(name.to_string_lossy());
                    }
                    if let Some(parent) = current.parent().filter(|p| p.is_dir()) {
                        dialog = dialog.set_directory(parent);
                    }
                    dialog.save_file().await
                };
                if let (Some(path), Some(ui)) = (picked, weak.upgrade()) {
                    ui.set_output(path.path().to_string_lossy().as_ref().into());
                    state(|s| s.output_chosen = true);
                    rescan(&ui);
                }
            });
        }
    });
    ui.on_options_changed({
        let weak = ui.as_weak();
        let timer = rescan_timer.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let default = state(|s| s.input.as_deref().map(scan::default_output));
            let typed = PathBuf::from(ui.get_output().as_str());
            state(|s| s.output_chosen = default.as_deref() != Some(typed.as_path()));
            show_estimates(&ui);
            let weak = weak.clone();
            timer.start(TimerMode::SingleShot, RESCAN_DELAY, move || {
                if let Some(ui) = weak.upgrade() {
                    rescan(&ui);
                }
            });
        }
    });
    ui.on_begin({
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                start(&ui);
            }
        }
    });
    ui.on_cancel({
        let weak = ui.as_weak();
        move || {
            state(|s| s.run_cancel.store(true, Ordering::SeqCst));
            if let Some(ui) = weak.upgrade() {
                ui.set_cancelling(true);
            }
        }
    });
    ui.on_again({
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_stage(Stage::Setup);
                rescan(&ui);
            }
        }
    });
    ui.on_open_output(|| {
        if let Some(path) = state(|s| s.reveal.clone()) {
            reveal(&path);
        }
    });
    // Closing while converting stops cleanly: the file in progress is removed before the program ends.
    ui.window().on_close_requested(|| {
        state(|s| {
            s.run_cancel.store(true, Ordering::SeqCst);
            s.scan_cancel.store(true, Ordering::SeqCst);
        });
        CloseRequestResponse::HideWindow
    });

    if let Some(input) = input {
        set_source(&ui, input);
    }
    ui.run()?;

    state(|s| s.run_cancel.store(true, Ordering::SeqCst));
    if let Some(worker) = state(|s| s.worker.take()) {
        let _ = worker.join();
    }
    Ok(())
}

fn set_source(ui: &App, input: PathBuf) {
    let is_folder = input.is_dir();
    let full = std::path::absolute(&input).unwrap_or_else(|_| input.clone());
    let name = full.file_name().unwrap_or(full.as_os_str()).to_string_lossy().into_owned();
    let place = full.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    ui.set_has_source(true);
    ui.set_source_is_folder(is_folder);
    ui.set_source_name(name.into());
    ui.set_source_place(place.into());
    if !state(|s| s.output_chosen) {
        ui.set_output(scan::default_output(&full).to_string_lossy().as_ref().into());
    }
    state(|s| s.input = Some(full));
    rescan(ui);
}

fn options(ui: &App) -> ScanOptions {
    ScanOptions { recursive: ui.get_recursive(), include_audio: ui.get_include_audio(), overwrite: false }
}

fn preset(ui: &App) -> Preset {
    Preset::ALL[(ui.get_preset().max(0) as usize).min(Preset::ALL.len() - 1)]
}

/// Scans in the background, then reads durations; the window is updated after each step.
fn rescan(ui: &App) {
    let Some(input) = state(|s| s.input.clone()) else { return };
    let output = PathBuf::from(ui.get_output().as_str());
    let options = options(ui);
    let (id, cancel) = state(|s| {
        s.scan_cancel.store(true, Ordering::SeqCst);
        s.scan_cancel = Arc::new(AtomicBool::new(false));
        s.scan += 1;
        s.plan = None;
        (s.scan, s.scan_cancel.clone())
    });
    ui.set_scanning(true);
    ui.set_scan_problem(SharedString::new());

    let weak = ui.as_weak();
    thread::spawn(move || {
        let output = (!output.as_os_str().is_empty()).then_some(output);
        let scanned = scan::plan(&input, output.as_deref(), options);
        let show = |weak: &slint::Weak<App>, result: Result<Plan, String>, complete: bool| {
            let _ = weak.upgrade_in_event_loop(move |ui| {
                if state(|s| s.scan) == id {
                    show_scan(&ui, result, complete);
                }
            });
        };
        match scanned {
            Err(e) => show(&weak, Err(e.to_string()), true),
            Ok(mut plan) => {
                show(&weak, Ok(plan.clone()), false);
                runner::probe_durations(&mut plan.jobs, &cancel, &mut |_| {});
                if !cancel.load(Ordering::SeqCst) {
                    show(&weak, Ok(plan), true);
                }
            }
        }
    });
}

fn show_scan(ui: &App, result: Result<Plan, String>, complete: bool) {
    ui.set_scanning(false);
    match result {
        Err(problem) => {
            ui.set_scan_problem(problem.into());
            ui.set_to_convert(0);
            ui.set_already_done(0);
            state(|s| s.plan = None);
        }
        Ok(plan) => {
            ui.set_to_convert(plan.jobs.len() as i32);
            ui.set_already_done(plan.existing.len() as i32);
            ui.set_source_size(text::bytes(plan.jobs.iter().map(|j| j.input_bytes).sum()).into());
            let total: Duration = plan.jobs.iter().filter_map(|j| j.duration).sum();
            let known = complete && !total.is_zero();
            ui.set_source_duration(if known { text::duration(total).into() } else { SharedString::new() });
            state(|s| s.plan = Some(plan));
        }
    }
    show_estimates(ui);
}

/// Under each preset: the expected total once durations are known, otherwise the size per hour.
fn show_estimates(ui: &App) {
    let total = state(|s| {
        let jobs = &s.plan.as_ref()?.jobs;
        // A file whose length cannot be read (it will fail anyway) does not hide the estimate for the rest.
        let known = jobs.iter().any(|j| j.duration.is_some());
        known.then(|| jobs.iter().filter_map(|j| j.duration).sum::<Duration>())
    });
    ui.set_sizes_are_totals(total.is_some());
    ui.set_preset_sizes(preset_sizes(total));
}

fn preset_sizes(total: Option<Duration>) -> ModelRc<SharedString> {
    let sizes: Vec<SharedString> = Preset::ALL
        .iter()
        .map(|preset| {
            let hours = total.map_or(1.0, |t| t.as_secs_f64() / 3600.0);
            text::bytes((preset.mb_per_hour() * 1e6 * hours) as u64).into()
        })
        .collect();
    ModelRc::new(VecModel::from(sizes))
}

fn start(ui: &App) {
    let Some(plan) = state(|s| s.plan.take()) else { return };
    if plan.jobs.is_empty() {
        return;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let reveal = if plan.output.extension().is_some() && !plan.output.is_dir() {
        plan.output.parent().map(Path::to_path_buf)
    } else {
        Some(plan.output.clone())
    };
    state(|s| {
        s.scan_cancel.store(true, Ordering::SeqCst);
        s.scan += 1;
        s.run_cancel = cancel.clone();
        s.reveal = reveal;
    });

    ui.set_stage(Stage::Working);
    ui.set_reading(true);
    ui.set_cancelling(false);
    ui.set_progress(0.0);
    ui.set_speed(SharedString::new());
    ui.set_time_left(SharedString::new());
    ui.set_total_files(plan.jobs.len() as i32);
    ui.set_current_index(1);
    ui.set_current_name(plan.jobs[0].name.as_str().into());

    let ticker = Timer::default();
    let (weak, began) = (ui.as_weak(), Instant::now());
    ticker.start(TimerMode::Repeated, FRAME, move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_clock(began.elapsed().as_secs_f32());
        }
    });
    state(|s| s.ticker = Some(ticker));

    let preset = preset(ui);
    let weak = ui.as_weak();
    let worker = thread::spawn(move || {
        let mut estimator = Estimator::new();
        let mut started = Instant::now();
        let mut refreshed = Instant::now() - REFRESH;
        let mut problems: Vec<(String, String, bool)> = Vec::new();
        let mut files = 0;

        runner::run(plan, preset, &cancel, |event| match event {
            Event::Probing { .. } => {}
            Event::Started { files: count, .. } => {
                files = count;
                started = Instant::now();
                let _ = weak.upgrade_in_event_loop(|ui| ui.set_reading(false));
            }
            Event::FileStarted { index, name } => {
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    ui.set_current_index(index as i32 + 1);
                    ui.set_current_name(name.into());
                });
            }
            Event::Progress { index, done, total, .. } => {
                estimator.update(started.elapsed(), done);
                if refreshed.elapsed() < REFRESH {
                    return;
                }
                refreshed = Instant::now();
                // Without known durations, each finished file is an equal step.
                let fraction = if total.is_zero() {
                    index as f32 / files.max(1) as f32
                } else {
                    (done.as_secs_f64() / total.as_secs_f64()) as f32
                };
                let speed = estimator.speed().map(text::speed).unwrap_or_default();
                let left = estimator.remaining(total).map(text::duration).unwrap_or_default();
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    ui.set_progress(fraction);
                    ui.set_speed(speed.into());
                    ui.set_time_left(left.into());
                });
            }
            Event::FileDone { name, result, .. } => match result {
                FileResult::Converted { warning: Some(warning), .. } => problems.push((name, warning, false)),
                FileResult::Failed(reason) => problems.push((name, reason, true)),
                _ => {}
            },
            Event::Finished(summary) => {
                let problems = std::mem::take(&mut problems);
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    let rows: Vec<Problem> = problems
                        .into_iter()
                        .map(|(name, reason, failed)| Problem { name: name.into(), reason: reason.into(), failed })
                        .collect();
                    ui.set_problems(ModelRc::new(VecModel::from(rows)));
                    ui.set_was_cancelled(summary.cancelled);
                    ui.set_converted(summary.converted as i32);
                    ui.set_failed(summary.failed.len() as i32);
                    ui.set_size_before(text::bytes(summary.input_bytes).into());
                    ui.set_size_after(text::bytes(summary.output_bytes).into());
                    ui.set_share(text::percent_of(summary.output_bytes, summary.input_bytes).into());
                    ui.set_elapsed(text::duration(summary.elapsed).into());
                    if !summary.cancelled {
                        ui.set_progress(1.0);
                    }
                    ui.set_stage(Stage::Done);
                    state(|s| s.ticker = None);
                });
            }
        });
    });
    state(|s| s.worker = Some(worker));
}

/// Opens the folder in the system's file manager.
fn reveal(path: &Path) {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = Command::new(program).arg(path).spawn();
}

/// The active part of the progress bar: a sine from the left edge to the progress point, in a box of
/// `width` by `height`.
fn wave(width: f32, height: f32, progress: f32, phase: f32, amplitude: f32) -> SharedString {
    const WAVELENGTH: f32 = 30.0;
    let middle = height / 2.0;
    let end = width.max(0.0) * progress.clamp(0.0, 1.0);
    let mut path = String::with_capacity(64);
    if amplitude < 0.05 || end < 1.0 {
        let _ = write!(path, "M 0 {middle:.1} L {end:.1} {middle:.1}");
        return path.into();
    }
    let y = |x: f32| middle + amplitude * (TAU * (x / WAVELENGTH - phase)).sin();
    let _ = write!(path, "M 0 {:.2}", y(0.0));
    let mut x = 0.0;
    while x < end {
        x = (x + 2.0).min(end);
        let _ = write!(path, " L {x:.1} {:.2}", y(x));
    }
    path.into()
}

/// A circle with soft lobes, the Material "cookie" shape, rotated by `turn` of a full turn.
fn scallop(size: f32, turn: f32) -> SharedString {
    const LOBES: f32 = 9.0;
    const POINTS: usize = 144;
    let centre = size / 2.0;
    let mut path = String::with_capacity(POINTS * 14);
    for i in 0..POINTS {
        let angle = TAU * i as f32 / POINTS as f32;
        let radius = centre * (0.93 + 0.07 * (LOBES * angle).cos());
        let rotated = angle + TAU * turn;
        let (x, y) = (centre + radius * rotated.cos(), centre + radius * rotated.sin());
        let _ = write!(path, "{} {x:.2} {y:.2} ", if i == 0 { 'M' } else { 'L' });
    }
    path.push('Z');
    path.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(path: &str) -> Vec<(f32, f32)> {
        let numbers: Vec<f32> = path.split_whitespace().filter_map(|word| word.parse().ok()).collect();
        numbers.chunks(2).map(|pair| (pair[0], pair[1])).collect()
    }

    #[test]
    fn wave_ends_at_the_progress_point_and_stays_inside() {
        let path = wave(400.0, 11.0, 0.25, 0.3, 3.0);
        let points = points(&path);
        assert_eq!(points.first().unwrap().0, 0.0);
        assert_eq!(points.last().unwrap().0, 100.0);
        assert!(points.iter().all(|&(_, y)| (2.4..=8.6).contains(&y)), "{path}");
    }

    #[test]
    fn wave_is_a_straight_line_when_flat() {
        assert_eq!(wave(400.0, 11.0, 1.0, 0.7, 0.0).as_str(), "M 0 5.5 L 400.0 5.5");
    }

    #[test]
    fn scallop_is_closed_and_fits_its_box() {
        let path = scallop(104.0, 0.37);
        assert!(path.ends_with('Z'));
        assert!(points(&path).iter().all(|&(x, y)| (0.0..=104.0).contains(&x) && (0.0..=104.0).contains(&y)));
    }
}
