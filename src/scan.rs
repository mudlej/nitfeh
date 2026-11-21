//! Turns an input path and options into a `Plan`: which files to convert and where each one goes.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf, absolute};
use std::time::Duration;

use walkdir::WalkDir;

pub const VIDEO_EXTENSIONS: &[&str] = &[
    "3gp", "asf", "avi", "divx", "f4v", "flv", "m2t", "m2ts", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "mts", "ogm",
    "ogv", "rm", "rmvb", "ts", "vob", "webm", "wmv",
];
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "aac", "ac3", "aif", "aiff", "amr", "ape", "flac", "m4a", "m4b", "mka", "mp2", "mp3", "oga", "ogg", "opus", "ra",
    "wav", "wma", "wv",
];
const OUTPUT_EXTENSION: &str = "opus";

#[derive(Clone, Copy, Debug, Default)]
pub struct ScanOptions {
    /// Folders only: also look in subfolders, mirroring them in the output.
    pub recursive: bool,
    /// Folders only: also convert audio files. A single input file is always accepted.
    pub include_audio: bool,
    /// Convert again even when the output exists.
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub input: PathBuf,
    pub output: PathBuf,
    /// For display: the file name, or the path below the input folder.
    pub name: String,
    pub input_bytes: u64,
    /// Filled in by the runner before converting.
    pub duration: Option<Duration>,
}

#[derive(Debug, Default, Clone)]
pub struct Plan {
    pub jobs: Vec<Job>,
    /// Left alone because their output is already there.
    pub existing: Vec<Job>,
    /// The output file for a single input, otherwise the output folder.
    pub output: PathBuf,
}

#[derive(Debug)]
pub enum ScanError {
    NotFound(PathBuf),
    OutputIsInput(PathBuf),
    OutputIsFile(PathBuf),
    Io(PathBuf, io::Error),
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScanError::NotFound(p) => write!(f, "{} does not exist", p.display()),
            ScanError::OutputIsInput(p) => {
                write!(f, "the output would overwrite the input {}: choose another output", p.display())
            }
            ScanError::OutputIsFile(p) => {
                write!(f, "{} is a file, but the output for a folder must be a folder", p.display())
            }
            ScanError::Io(p, e) => write!(f, "cannot read {}: {e}", p.display()),
        }
    }
}

impl std::error::Error for ScanError {}

/// Where the output goes when none is given: beside a file, or in a sibling `<folder>-opus` folder.
pub fn default_output(input: &Path) -> PathBuf {
    if input.is_dir() {
        let full = absolute(input).unwrap_or_else(|_| input.to_path_buf());
        match (full.parent(), full.file_name()) {
            (Some(parent), Some(name)) => {
                let mut name = name.to_os_string();
                name.push("-opus");
                parent.join(name)
            }
            _ => full.join("opus"),
        }
    } else {
        input.with_extension(OUTPUT_EXTENSION)
    }
}

pub fn plan(input: &Path, output: Option<&Path>, options: ScanOptions) -> Result<Plan, ScanError> {
    let meta = fs::metadata(input).map_err(|_| ScanError::NotFound(input.to_path_buf()))?;
    let mut plan =
        if meta.is_dir() { plan_folder(input, output, options)? } else { plan_file(input, output, meta.len())? };

    let (existing, jobs) =
        std::mem::take(&mut plan.jobs).into_iter().partition(|job| !options.overwrite && job.output.exists());
    plan.jobs = jobs;
    plan.existing = existing;
    Ok(plan)
}

fn plan_file(input: &Path, output: Option<&Path>, input_bytes: u64) -> Result<Plan, ScanError> {
    let name = input.file_name().unwrap_or(input.as_os_str());
    let output = match output {
        None => default_output(input),
        // No extension means a folder, as does anything that already is one.
        Some(out) if out.is_dir() || out.extension().is_none() || ends_with_separator(out) => {
            out.join(Path::new(name).with_extension(OUTPUT_EXTENSION))
        }
        Some(out) => out.to_path_buf(),
    };
    if same_path(input, &output) {
        return Err(ScanError::OutputIsInput(input.to_path_buf()));
    }
    let job = Job {
        input: input.to_path_buf(),
        output: output.clone(),
        name: name.to_string_lossy().into_owned(),
        input_bytes,
        duration: None,
    };
    Ok(Plan { jobs: vec![job], existing: Vec::new(), output })
}

fn plan_folder(input: &Path, output: Option<&Path>, options: ScanOptions) -> Result<Plan, ScanError> {
    let root = output.map(Path::to_path_buf).unwrap_or_else(|| default_output(input));
    if root.is_file() {
        return Err(ScanError::OutputIsFile(root));
    }
    let full_input = absolute(input).unwrap_or_else(|_| input.to_path_buf());
    let full_root = absolute(&root).unwrap_or_else(|_| root.clone());
    // An output folder inside the input is never read as input (unless it is the input folder itself).
    let inside_output =
        |path: &Path| full_root != full_input && absolute(path).is_ok_and(|p| p.starts_with(&full_root));

    let mut files: Vec<(PathBuf, u64)> = Vec::new();
    let depth = if options.recursive { usize::MAX } else { 1 };
    let mut walk = WalkDir::new(input).min_depth(1).max_depth(depth).follow_links(true).into_iter();
    while let Some(entry) = walk.next() {
        let entry = match entry {
            Ok(entry) => entry,
            // The folder itself must be readable; anything unreadable below it is passed over.
            Err(e) if e.depth() <= 1 && e.path() == Some(input) => {
                return Err(ScanError::Io(input.to_path_buf(), e.into()));
            }
            Err(_) => continue,
        };
        if entry.file_type().is_dir() {
            if inside_output(entry.path()) {
                walk.skip_current_dir();
            }
            continue;
        }
        if wanted(entry.path(), options.include_audio) && !inside_output(entry.path()) {
            let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
            files.push((entry.into_path(), bytes));
        }
    }
    files.sort_by(|a, b| natural_path_order(&a.0, &b.0));

    let mut jobs: Vec<Job> = files
        .into_iter()
        .map(|(path, input_bytes)| {
            let relative = path.strip_prefix(input).unwrap_or(&path).to_path_buf();
            Job {
                output: root.join(&relative).with_extension(OUTPUT_EXTENSION),
                name: relative.to_string_lossy().into_owned(),
                input: path,
                input_bytes,
                duration: None,
            }
        })
        .collect();

    // A file that is its own output (an .opus beside its source, output folder = input folder) stays as it is.
    jobs.retain(|job| !same_path(&job.input, &job.output));

    // lec2.mkv and lec2.mp4 in one folder: both keep their extension, as lec2.mkv.opus and lec2.mp4.opus.
    let mut by_output: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, job) in jobs.iter().enumerate() {
        by_output.entry(job.output.to_string_lossy().to_lowercase()).or_default().push(i);
    }
    for clash in by_output.into_values().filter(|group| group.len() > 1) {
        for i in clash {
            let mut name = jobs[i].input.file_name().unwrap_or_default().to_os_string();
            name.push(".");
            name.push(OUTPUT_EXTENSION);
            jobs[i].output.set_file_name(name);
        }
    }
    Ok(Plan { jobs, existing: Vec::new(), output: root })
}

fn wanted(path: &Path, include_audio: bool) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    VIDEO_EXTENSIONS.contains(&ext.as_str()) || (include_audio && AUDIO_EXTENSIONS.contains(&ext.as_str()))
}

fn ends_with_separator(path: &Path) -> bool {
    path.as_os_str().to_string_lossy().ends_with(['/', std::path::MAIN_SEPARATOR])
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => absolute(a).ok() == absolute(b).ok(),
    }
}

fn natural_path_order(a: &Path, b: &Path) -> Ordering {
    let mut a = a.components();
    let mut b = b.components();
    loop {
        match (a.next(), b.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let order = natural_order(&x.as_os_str().to_string_lossy(), &y.as_os_str().to_string_lossy());
                if order != Ordering::Equal {
                    return order;
                }
            }
        }
    }
}

/// Case-insensitive, with runs of digits compared as numbers: "lecture 2" sorts before "lecture 10".
fn natural_order(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let number = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(d) = it.next_if(char::is_ascii_digit) {
                        digits.push(d);
                    }
                    digits.trim_start_matches('0').to_owned()
                };
                let (x, y) = (number(&mut a), number(&mut b));
                let order = x.len().cmp(&y.len()).then_with(|| x.cmp(&y));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tree(files: &[&str]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for file in files {
            let path = dir.path().join("in").join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"x").unwrap();
        }
        dir
    }

    fn outputs(plan: &Plan, root: &Path) -> Vec<String> {
        plan.jobs.iter().map(|j| j.output.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/")).collect()
    }

    #[test]
    fn folder_is_flat_unless_recursive() {
        let dir = tree(&["b.mp4", "a.mkv", "notes.txt", "song.mp3", "week1/c.mp4"]);
        let (input, out) = (dir.path().join("in"), dir.path().join("out"));

        let flat = plan(&input, Some(&out), ScanOptions::default()).unwrap();
        assert_eq!(outputs(&flat, &out), ["a.opus", "b.opus"]);

        let deep = plan(&input, Some(&out), ScanOptions { recursive: true, ..Default::default() }).unwrap();
        assert_eq!(outputs(&deep, &out), ["a.opus", "b.opus", "week1/c.opus"]);

        let audio = plan(&input, Some(&out), ScanOptions { include_audio: true, ..Default::default() }).unwrap();
        assert_eq!(outputs(&audio, &out), ["a.opus", "b.opus", "song.opus"]);
    }

    #[test]
    fn default_output_is_a_sibling_folder() {
        let dir = tree(&["a.mp4"]);
        let made = plan(&dir.path().join("in"), None, ScanOptions::default()).unwrap();
        assert_eq!(made.output, dir.path().join("in-opus"));
        assert_eq!(made.jobs[0].output, dir.path().join("in-opus/a.opus"));
    }

    #[test]
    fn same_name_different_extension_keeps_both() {
        let dir = tree(&["lec2.mkv", "lec2.mp4", "lec3.mp4"]);
        let out = dir.path().join("out");
        let made = plan(&dir.path().join("in"), Some(&out), ScanOptions::default()).unwrap();
        assert_eq!(outputs(&made, &out), ["lec2.mkv.opus", "lec2.mp4.opus", "lec3.opus"]);
    }

    #[test]
    fn existing_outputs_are_skipped_unless_overwriting() {
        let dir = tree(&["a.mp4", "b.mp4"]);
        let (input, out) = (dir.path().join("in"), dir.path().join("out"));
        fs::create_dir_all(&out).unwrap();
        fs::write(out.join("a.opus"), b"done").unwrap();

        let made = plan(&input, Some(&out), ScanOptions::default()).unwrap();
        assert_eq!(outputs(&made, &out), ["b.opus"]);
        assert_eq!(made.existing.len(), 1);

        let again = plan(&input, Some(&out), ScanOptions { overwrite: true, ..Default::default() }).unwrap();
        assert_eq!(again.jobs.len(), 2);
    }

    #[test]
    fn output_folder_inside_the_input_is_not_scanned() {
        let dir = tree(&["a.mp4", "out/a.opus", "out/old.mp4"]);
        let input = dir.path().join("in");
        let out = input.join("out");
        let options = ScanOptions { recursive: true, include_audio: true, ..Default::default() };
        let made = plan(&input, Some(&out), options).unwrap();
        assert!(made.jobs.is_empty());
        assert_eq!(made.existing.len(), 1);
        assert_eq!(made.existing[0].name, "a.mp4");
    }

    #[test]
    fn output_beside_the_inputs_never_converts_its_own_results() {
        let dir = tree(&["a.mp4", "a.opus", "b.opus"]);
        let input = dir.path().join("in");
        let options = ScanOptions { include_audio: true, ..Default::default() };
        let made = plan(&input, Some(&input), options).unwrap();
        assert!(made.jobs.is_empty(), "{:?}", made.jobs);
    }

    #[test]
    fn single_file_outputs() {
        let dir = tree(&["talk.v2.mp4", "voice.opus"]);
        let input = dir.path().join("in/talk.v2.mp4");

        let beside = plan(&input, None, ScanOptions::default()).unwrap();
        assert_eq!(beside.jobs[0].output, dir.path().join("in/talk.v2.opus"));

        let named = plan(&input, Some(&dir.path().join("x.opus")), ScanOptions::default()).unwrap();
        assert_eq!(named.jobs[0].output, dir.path().join("x.opus"));

        let folder = plan(&input, Some(&dir.path().join("new")), ScanOptions::default()).unwrap();
        assert_eq!(folder.jobs[0].output, dir.path().join("new/talk.v2.opus"));

        let onto_itself = plan(&dir.path().join("in/voice.opus"), None, ScanOptions::default());
        assert!(matches!(onto_itself, Err(ScanError::OutputIsInput(_))));
    }

    #[test]
    fn natural_order_of_names() {
        let dir = tree(&["Lecture 10.mp4", "lecture 2.mp4", "Lecture 1.mp4"]);
        let out = dir.path().join("out");
        let made = plan(&dir.path().join("in"), Some(&out), ScanOptions::default()).unwrap();
        assert_eq!(outputs(&made, &out), ["Lecture 1.opus", "lecture 2.opus", "Lecture 10.opus"]);
    }
}
