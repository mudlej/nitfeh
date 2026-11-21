//! The command line: `istma [OPTIONS] <INPUT> [OUTPUT]`.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};

use crate::preset::Preset;
use crate::progress::Estimator;
use crate::runner::{self, Event, FileResult, Summary};
use crate::scan::{self, ScanOptions};
use crate::text;

const EXIT_FAILED: u8 = 1;
const EXIT_CANCELLED: u8 = 130;

#[derive(Parser, Debug)]
#[command(
    name = "istma",
    version,
    about = "Compress lecture audio: any video or audio file to a small mono Opus file.",
    after_help = "Examples:\n  istma lecture.mp4                 lecture.opus beside it\n  istma lecture.mp4 small.opus\n  istma course/                     every video in course/ into course-opus/\n  istma -r -p small course/ out/    subfolders too, at 10 kbps"
)]
pub struct Args {
    /// A video or audio file, or a folder of them
    #[arg(required_unless_present = "gui")]
    pub input: Option<PathBuf>,

    /// Output file or folder [default: beside the file, or a sibling "<folder>-opus" folder]
    pub output: Option<PathBuf>,

    /// How hard to compress
    #[arg(short, long, value_enum, default_value_t = PresetArg::Standard)]
    pub preset: PresetArg,

    /// Folders: also convert the files in subfolders
    #[arg(short, long)]
    pub recursive: bool,

    /// Folders: also convert audio files, not only videos
    #[arg(short = 'a', long)]
    pub include_audio: bool,

    /// Convert again even when the output already exists
    #[arg(long)]
    pub overwrite: bool,

    /// No progress bar: only problems and the final summary
    #[arg(short, long)]
    pub quiet: bool,

    /// Open the window, with INPUT already chosen (starting without any arguments also opens it)
    #[arg(long)]
    pub gui: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum PresetArg {
    /// 12 kbps, about 5.4 MB per hour
    Standard,
    /// 10 kbps, about 4.5 MB per hour
    Small,
    /// 9 kbps, about 4.1 MB per hour: the smallest that keeps wideband speech
    Smallest,
}

impl From<PresetArg> for Preset {
    fn from(arg: PresetArg) -> Self {
        match arg {
            PresetArg::Standard => Preset::Standard,
            PresetArg::Small => Preset::Small,
            PresetArg::Smallest => Preset::Smallest,
        }
    }
}

pub fn main() -> ExitCode {
    if std::env::args_os().len() <= 1 {
        return window(None);
    }
    let args = Args::parse();
    if args.gui { window(args.input) } else { run(args) }
}

#[cfg(feature = "gui")]
fn window(input: Option<PathBuf>) -> ExitCode {
    #[cfg(windows)]
    release_own_console();
    match crate::gui::run(input) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("istma: cannot open a window: {e}");
            eprintln!("The command line works without one: istma --help");
            ExitCode::from(EXIT_FAILED)
        }
    }
}

/// Started by double-click on Windows before 11 24H2, the program gets a console window of its own; let go
/// of it. A console shared with a terminal (more than one process attached) is kept.
#[cfg(all(windows, feature = "gui"))]
fn release_own_console() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
        fn FreeConsole() -> i32;
    }
    let mut ids = [0u32; 2];
    unsafe {
        if GetConsoleProcessList(ids.as_mut_ptr(), ids.len() as u32) == 1 {
            FreeConsole();
        }
    }
}

#[cfg(not(feature = "gui"))]
fn window(_input: Option<PathBuf>) -> ExitCode {
    use clap::CommandFactory;
    let _ = Args::command().print_help();
    ExitCode::from(EXIT_FAILED)
}

pub fn run(args: Args) -> ExitCode {
    let options =
        ScanOptions { recursive: args.recursive, include_audio: args.include_audio, overwrite: args.overwrite };
    let Some(input) = args.input.as_deref() else {
        eprintln!("istma: no input given");
        return ExitCode::from(EXIT_FAILED);
    };
    let plan = match scan::plan(input, args.output.as_deref(), options) {
        Ok(plan) => plan,
        Err(e) => {
            eprintln!("istma: {e}");
            return ExitCode::from(EXIT_FAILED);
        }
    };
    if plan.jobs.is_empty() {
        if plan.existing.is_empty() {
            let hint = match (args.recursive, args.include_audio) {
                (false, false) => " (subfolders: -r, audio files: -a)",
                (false, true) => " (subfolders: -r)",
                (true, false) => " (audio files: -a)",
                (true, true) => "",
            };
            eprintln!("istma: nothing to convert in {}{hint}", input.display());
            return ExitCode::from(EXIT_FAILED);
        }
        println!("Nothing to do: all {} already converted (--overwrite to redo).", text::files(plan.existing.len()));
        return ExitCode::SUCCESS;
    }

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let cancel = cancel.clone();
        // A second Ctrl-C does not wait for the current file to be cleaned up.
        let _ = ctrlc::set_handler(move || {
            if cancel.swap(true, Ordering::SeqCst) {
                std::process::exit(EXIT_CANCELLED as i32);
            }
        });
    }

    let mut display = Display::new(args.quiet, plan.jobs.len());
    let summary = runner::run(plan, args.preset.into(), &cancel, |event| display.handle(event));
    display.finish();
    print_summary(&summary);

    if summary.cancelled {
        ExitCode::from(EXIT_CANCELLED)
    } else if summary.failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(EXIT_FAILED)
    }
}

/// Two lines on a terminal (current file, then the bar); one line per finished file when piped.
struct Display {
    bars: Option<(MultiProgress, ProgressBar, ProgressBar)>,
    quiet: bool,
    files: usize,
    started: Instant,
    estimator: Estimator,
}

impl Display {
    fn new(quiet: bool, files: usize) -> Self {
        let bars = (!quiet && std::io::stderr().is_terminal()).then(|| {
            let multi = MultiProgress::with_draw_target(ProgressDrawTarget::stderr_with_hz(8));
            let file = multi.add(ProgressBar::new_spinner());
            file.set_style(ProgressStyle::with_template("{prefix:.bold} {wide_msg}").unwrap());
            let bar = multi.add(ProgressBar::new(0));
            bar.set_style(
                ProgressStyle::with_template("{bar:32.cyan/238} {percent:>3}%  {msg}").unwrap().progress_chars("━━─"),
            );
            (multi, file, bar)
        });
        Display { bars, quiet, files, started: Instant::now(), estimator: Estimator::new() }
    }

    fn line(&self, text: String) {
        match &self.bars {
            Some((multi, ..)) => drop(multi.println(text)),
            None => eprintln!("{text}"),
        }
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Probing { done, total } => {
                if let Some((_, file, _)) = &self.bars {
                    file.set_message(format!("Reading {done}/{total}"));
                }
            }
            Event::Started { media, .. } => {
                self.started = Instant::now();
                self.estimator = Estimator::new();
                if let Some((.., bar)) = &self.bars {
                    bar.set_length(media.as_millis() as u64);
                }
            }
            Event::FileStarted { index, name } => {
                if let Some((_, file, _)) = &self.bars {
                    if self.files > 1 {
                        file.set_prefix(format!("[{}/{}]", index + 1, self.files));
                    }
                    file.set_message(name);
                }
            }
            Event::Progress { done, total, .. } => {
                self.estimator.update(self.started.elapsed(), done);
                if let Some((.., bar)) = &self.bars {
                    bar.set_position(done.as_millis() as u64);
                    let mut parts = Vec::new();
                    if let Some(speed) = self.estimator.speed() {
                        parts.push(text::speed(speed));
                    }
                    if let Some(left) = self.estimator.remaining(total) {
                        parts.push(format!("{} left", text::duration(left)));
                    }
                    bar.set_message(parts.join(" · "));
                }
            }
            Event::FileDone { index, name, result } => match result {
                FileResult::Converted { input_bytes, output_bytes, warning } => {
                    if self.bars.is_none() && !self.quiet {
                        eprintln!(
                            "[{}/{}] {name}  {} → {}",
                            index + 1,
                            self.files,
                            text::bytes(input_bytes),
                            text::bytes(output_bytes)
                        );
                    }
                    if let Some(warning) = warning {
                        self.line(format!("warning: {name}: {warning}"));
                    }
                }
                FileResult::Failed(reason) => self.line(format!("failed: {name}: {reason}")),
                FileResult::Cancelled => {}
            },
            Event::Finished(_) => {}
        }
    }

    fn finish(&mut self) {
        if let Some((_, file, bar)) = self.bars.take() {
            file.finish_and_clear();
            bar.finish_and_clear();
        }
    }
}

fn print_summary(summary: &Summary) {
    let mut parts = Vec::new();
    if summary.cancelled {
        parts.push("Cancelled".to_owned());
    }
    parts.push(format!("{} converted", text::files(summary.converted)));
    if summary.existing > 0 {
        parts.push(format!("{} already done", summary.existing));
    }
    if !summary.failed.is_empty() {
        parts.push(format!("{} failed", summary.failed.len()));
    }
    let mut line = parts.join(", ");
    if summary.converted > 0 {
        line.push_str(&format!(
            " · {} → {} ({} of the original) · {}",
            text::bytes(summary.input_bytes),
            text::bytes(summary.output_bytes),
            text::percent_of(summary.output_bytes, summary.input_bytes),
            text::duration(summary.elapsed.max(Duration::from_secs(1))),
        ));
    }
    println!("{line}");
}
