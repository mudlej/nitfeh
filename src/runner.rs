//! Runs a `Plan`: probes durations, converts each file, and reports everything as `Event`s.
//! The CLI and the GUI only ever see these events.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::engine::{self, Outcome};
use crate::preset::Preset;
use crate::scan::{Job, Plan};

#[derive(Debug, Clone)]
pub enum Event {
    /// Reading durations before the first conversion.
    Probing {
        done: usize,
        total: usize,
    },
    Started {
        files: usize,
        media: Duration,
    },
    FileStarted {
        index: usize,
        name: String,
    },
    /// Media time converted so far, in this file and overall.
    Progress {
        index: usize,
        file_done: Duration,
        done: Duration,
        total: Duration,
    },
    FileDone {
        index: usize,
        name: String,
        result: FileResult,
    },
    Finished(Summary),
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileResult {
    Converted { input_bytes: u64, output_bytes: u64, warning: Option<Warning> },
    Failed(Failure),
    Cancelled,
}

/// Something worth telling about a file that was converted all the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Warning {
    /// The input stopped on a read error; what came before it was converted.
    Truncated,
    /// This many damaged packets were skipped.
    DamagedPackets(u32),
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Warning::Truncated => f.write_str("the file is damaged or incomplete: converted up to where it stops"),
            Warning::DamagedPackets(n) => write!(f, "{n} damaged audio packets were skipped"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    NoAudio,
    /// The reason, as the engine or the system gave it.
    Other(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::NoAudio => f.write_str("no audio track"),
            Failure::Other(reason) => f.write_str(reason),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub converted: usize,
    /// Outputs that were already there.
    pub existing: usize,
    /// (name, reason)
    pub failed: Vec<(String, String)>,
    pub cancelled: bool,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub elapsed: Duration,
}

/// Blocks until the plan is done or `cancel` is set. Call it from a worker thread in a GUI.
pub fn run(mut plan: Plan, preset: Preset, cancel: &AtomicBool, mut emit: impl FnMut(Event)) -> Summary {
    let started = Instant::now();
    let mut summary = Summary { existing: plan.existing.len(), ..Default::default() };

    probe_durations(&mut plan.jobs, cancel, &mut emit);
    let total: Duration = plan.jobs.iter().filter_map(|job| job.duration).sum();
    emit(Event::Started { files: plan.jobs.len(), media: total });

    let mut done_before = Duration::ZERO;
    for (index, job) in plan.jobs.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            break;
        }
        emit(Event::FileStarted { index, name: job.name.clone() });
        let planned = job.duration.unwrap_or_default();
        let result = convert(job, preset, cancel, |file_done| {
            let done = done_before + file_done.min(planned);
            emit(Event::Progress { index, file_done, done, total });
        });
        done_before += planned;
        match &result {
            FileResult::Converted { input_bytes, output_bytes, .. } => {
                summary.converted += 1;
                summary.input_bytes += input_bytes;
                summary.output_bytes += output_bytes;
            }
            FileResult::Failed(reason) => summary.failed.push((job.name.clone(), reason.to_string())),
            FileResult::Cancelled => summary.cancelled = true,
        }
        if result != FileResult::Cancelled {
            emit(Event::Progress { index, file_done: planned, done: done_before, total });
        }
        emit(Event::FileDone { index, name: job.name.clone(), result });
    }

    summary.elapsed = started.elapsed();
    emit(Event::Finished(summary.clone()));
    summary
}

/// Appends to the whole file name, so `a.opus` is written as `a.opus.part`.
fn part_path(output: &Path) -> PathBuf {
    let mut name: OsString = output.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    output.with_file_name(name)
}

fn convert(job: &Job, preset: Preset, cancel: &AtomicBool, mut progress: impl FnMut(Duration)) -> FileResult {
    if let Some(parent) = job.output.parent().filter(|p| !p.as_os_str().is_empty())
        && let Err(e) = fs::create_dir_all(parent)
    {
        return FileResult::Failed(Failure::Other(format!("cannot create {}: {e}", parent.display())));
    }
    // The output only gets its real name once it is complete, so an interrupted run never looks finished.
    let part = part_path(&job.output);
    let outcome = engine::transcode(&job.input, &part, preset, &mut |done, _| {
        progress(done);
        !cancel.load(Ordering::Relaxed)
    });
    let encoded = match outcome {
        Ok(Outcome::Done(encoded)) => encoded,
        Ok(Outcome::Cancelled) => {
            let _ = fs::remove_file(&part);
            return FileResult::Cancelled;
        }
        Err(e) => {
            let _ = fs::remove_file(&part);
            return FileResult::Failed(match e {
                engine::Error::NoAudio => Failure::NoAudio,
                engine::Error::Failed(reason) => Failure::Other(reason),
            });
        }
    };
    let output_bytes = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    // Windows does not replace an existing file on rename.
    if cfg!(windows) && job.output.exists() {
        let _ = fs::remove_file(&job.output);
    }
    if let Err(e) = fs::rename(&part, &job.output) {
        let _ = fs::remove_file(&part);
        return FileResult::Failed(Failure::Other(format!("cannot write {}: {e}", job.output.display())));
    }
    let warning = if encoded.truncated {
        Some(Warning::Truncated)
    } else if encoded.decode_errors > 0 {
        Some(Warning::DamagedPackets(encoded.decode_errors))
    } else {
        None
    };
    FileResult::Converted { input_bytes: job.input_bytes, output_bytes, warning }
}

/// Fills in `duration` for every job, a few files at a time: only headers are read.
pub fn probe_durations(jobs: &mut [Job], cancel: &AtomicBool, emit: &mut impl FnMut(Event)) {
    let total = jobs.len();
    if total == 0 {
        return;
    }
    let workers = thread::available_parallelism().map_or(2, |n| n.get()).min(8).min(total);
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel::<(usize, Option<Duration>)>();
    let paths: Vec<&Path> = jobs.iter().map(|job| job.input.as_path()).collect();

    let durations: Vec<(usize, Option<Duration>)> = thread::scope(|scope| {
        for _ in 0..workers {
            let sender = sender.clone();
            let (next, paths) = (&next, &paths);
            scope.spawn(move || {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= paths.len() || cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let duration = engine::probe(paths[i]).ok().and_then(|info| info.duration);
                    if sender.send((i, duration)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        let mut found = Vec::with_capacity(total);
        for item in receiver {
            found.push(item);
            emit(Event::Probing { done: found.len(), total });
        }
        found
    });
    for (i, duration) in durations {
        jobs[i].duration = duration;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_file_keeps_the_full_name() {
        assert_eq!(part_path(Path::new("out/a.b.opus")), Path::new("out/a.b.opus.part"));
    }
}
