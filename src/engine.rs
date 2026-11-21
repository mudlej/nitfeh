//! Safe wrapper over the C pipeline in `native/transcode.c`.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::Once;
use std::time::Duration;

use crate::preset::{FRAME_MS, Preset, SAMPLE_RATE};

const OK: c_int = 0;
const CANCELLED: c_int = 1;
const NO_AUDIO: c_int = -2;

#[repr(C)]
#[derive(Default)]
struct RawInfo {
    duration_us: i64,
    has_audio: c_int,
    sample_rate: c_int,
    channels: c_int,
    codec: [c_char; 32],
}

#[repr(C)]
struct RawOptions {
    bitrate: c_int,
    sample_rate: c_int,
    frame_ms: c_int,
}

#[repr(C)]
#[derive(Default)]
struct RawResult {
    duration_us: i64,
    decode_errors: c_int,
    truncated: c_int,
}

type ProgressFn = extern "C" fn(*mut c_void, i64, i64) -> c_int;

unsafe extern "C" {
    fn nitfeh_init(verbose: c_int);
    fn nitfeh_probe(path: *const c_char, info: *mut RawInfo, err: *mut c_char, err_len: c_int) -> c_int;
    fn nitfeh_transcode(
        in_path: *const c_char,
        out_path: *const c_char,
        options: *const RawOptions,
        progress: Option<ProgressFn>,
        opaque: *mut c_void,
        result: *mut RawResult,
        err: *mut c_char,
        err_len: c_int,
    ) -> c_int;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    NoAudio,
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoAudio => f.write_str("no audio track"),
            Error::Failed(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone)]
pub struct Info {
    pub duration: Option<Duration>,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct Encoded {
    pub duration: Duration,
    /// Corrupt packets that were skipped.
    pub decode_errors: u32,
    /// The input stopped on a read error instead of a clean end.
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum Outcome {
    Done(Encoded),
    Cancelled,
}

fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe { nitfeh_init(std::env::var_os("NITFEH_FFMPEG_LOG").is_some() as c_int) });
}

fn c_path(path: &Path) -> Result<CString, Error> {
    #[cfg(unix)]
    let bytes = std::os::unix::ffi::OsStrExt::as_bytes(path.as_os_str()).to_vec();
    #[cfg(not(unix))]
    let bytes =
        path.to_str().ok_or_else(|| Error::Failed("the file name is not valid Unicode".into()))?.as_bytes().to_vec();
    CString::new(bytes).map_err(|_| Error::Failed("the file name contains a NUL byte".into()))
}

fn micros(us: i64) -> Option<Duration> {
    (us > 0).then(|| Duration::from_micros(us as u64))
}

fn failure(code: c_int, err: &[c_char]) -> Error {
    if code == NO_AUDIO {
        return Error::NoAudio;
    }
    let text = unsafe { CStr::from_ptr(err.as_ptr()) }.to_string_lossy();
    Error::Failed(if text.is_empty() { "conversion failed".into() } else { text.into_owned() })
}

/// Reads the container headers: fast, no decoding.
pub fn probe(path: &Path) -> Result<Info, Error> {
    init();
    let path = c_path(path)?;
    let mut raw = RawInfo::default();
    let mut err = [0 as c_char; 256];
    let code = unsafe { nitfeh_probe(path.as_ptr(), &mut raw, err.as_mut_ptr(), err.len() as c_int) };
    if code != OK {
        return Err(failure(code, &err));
    }
    Ok(Info {
        duration: micros(raw.duration_us),
        codec: unsafe { CStr::from_ptr(raw.codec.as_ptr()) }.to_string_lossy().into_owned(),
        sample_rate: raw.sample_rate.max(0) as u32,
        channels: raw.channels.max(0) as u32,
    })
}

type Callback<'a> = &'a mut dyn FnMut(Duration, Option<Duration>) -> bool;

extern "C" fn trampoline(opaque: *mut c_void, done_us: i64, total_us: i64) -> c_int {
    let callback = unsafe { &mut *(opaque as *mut Callback) };
    let done = Duration::from_micros(done_us.max(0) as u64);
    // A panic must not unwind into C: treat it as a cancel.
    match catch_unwind(AssertUnwindSafe(|| callback(done, micros(total_us)))) {
        Ok(true) => 0,
        _ => 1,
    }
}

/// Converts `input` to an Ogg Opus file at `output`. `progress(done, total)` is called about once per
/// second of audio; returning `false` cancels. A cancelled or failed job may leave a partial `output`.
pub fn transcode(
    input: &Path,
    output: &Path,
    preset: Preset,
    progress: &mut dyn FnMut(Duration, Option<Duration>) -> bool,
) -> Result<Outcome, Error> {
    init();
    let input = c_path(input)?;
    let output = c_path(output)?;
    let options = RawOptions {
        bitrate: preset.bitrate() as c_int,
        sample_rate: SAMPLE_RATE as c_int,
        frame_ms: FRAME_MS as c_int,
    };
    let mut raw = RawResult::default();
    let mut err = [0 as c_char; 256];
    let mut callback: Callback = progress;
    let code = unsafe {
        nitfeh_transcode(
            input.as_ptr(),
            output.as_ptr(),
            &options,
            Some(trampoline),
            &mut callback as *mut Callback as *mut c_void,
            &mut raw,
            err.as_mut_ptr(),
            err.len() as c_int,
        )
    };
    match code {
        OK => Ok(Outcome::Done(Encoded {
            duration: Duration::from_micros(raw.duration_us.max(0) as u64),
            decode_errors: raw.decode_errors.max(0) as u32,
            truncated: raw.truncated != 0,
        })),
        CANCELLED => Ok(Outcome::Cancelled),
        code => Err(failure(code, &err)),
    }
}
