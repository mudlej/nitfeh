//! End-to-end checks of the `istma` binary. The system `ffmpeg` and `ffprobe` only make the fixtures and
//! inspect the results; without them these tests pass without checking anything.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use tempfile::TempDir;

const SECONDS: f64 = 12.0;

fn have_tools() -> bool {
    let ok = |tool: &str| Command::new(tool).arg("-version").output().is_ok_and(|o| o.status.success());
    let found = ok("ffmpeg") && ok("ffprobe");
    if !found {
        eprintln!("skipped: ffmpeg and ffprobe are needed to make fixtures");
    }
    found
}

/// A stereo tone with noise, with a small video track unless `audio_only`.
fn fixture(path: &Path, codec: &[&str], audio_only: bool) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("sine=f=440:r=44100:d={SECONDS},aformat=channel_layouts=stereo"));
    if !audio_only {
        cmd.args(["-f", "lavfi", "-i"]).arg(format!("color=c=black:s=64x64:r=5:d={SECONDS}"));
    }
    let out = cmd.args(codec).arg(path).output().unwrap();
    assert!(out.status.success(), "ffmpeg could not make {}: {}", path.display(), String::from_utf8_lossy(&out.stderr));
}

fn istma(args: &[&str], dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_istma")).args(args).current_dir(dir).output().unwrap()
}

fn probe(path: &Path, entries: &str) -> String {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", entries, "-of", "default=nw=1:nk=1"])
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().replace('\n', " ")
}

fn assert_good_opus(path: &Path) {
    assert!(path.is_file(), "{} is missing", path.display());
    assert_eq!(probe(path, "stream=codec_name,channels"), "opus 1", "{}", path.display());
    let duration: f64 = probe(path, "format=duration").parse().unwrap();
    assert!((duration - SECONDS).abs() < 0.1, "{}: {duration} s", path.display());
    let kbps = fs::metadata(path).unwrap().len() as f64 * 8.0 / duration / 1000.0;
    assert!((6.0..14.0).contains(&kbps), "{}: {kbps:.1} kbps", path.display());
}

fn leftovers(dir: &Path) -> Vec<PathBuf> {
    walk(dir).into_iter().filter(|p| p.extension().is_some_and(|e| e == "part")).collect()
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() { found.extend(walk(&path)) } else { found.push(path) }
    }
    found
}

#[test]
fn every_common_container_converts() {
    if !have_tools() {
        return;
    }
    let dir = TempDir::new().unwrap();
    let cases: [(&str, &[&str]); 7] = [
        ("a.mp4", &["-c:v", "mpeg4", "-c:a", "aac"]),
        ("b.mkv", &["-c:v", "mpeg4", "-c:a", "ac3"]),
        ("c.avi", &["-c:v", "mpeg4", "-c:a", "mp2"]),
        ("d.wmv", &["-c:v", "wmv2", "-c:a", "wmav2"]),
        ("e.ts", &["-c:v", "mpeg2video", "-c:a", "mp2"]),
        ("f.flv", &["-c:v", "flv", "-c:a", "aac", "-ar", "44100"]),
        ("g.mov", &["-c:v", "mpeg4", "-c:a", "pcm_s16le"]),
    ];
    for (name, codec) in cases {
        fixture(&dir.path().join("in").join(name), codec, false);
    }
    let out = istma(&["in", "out"], dir.path());
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    for (name, _) in cases {
        assert_good_opus(&dir.path().join("out").join(name).with_extension("opus"));
    }
}

#[test]
fn folders_names_and_resume() {
    if !have_tools() {
        return;
    }
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("دروس");
    fixture(&input.join("المحاضرة الأولى.mp4"), &["-c:v", "mpeg4", "-c:a", "aac"], false);
    fixture(&input.join("lec2.mkv"), &["-c:v", "mpeg4", "-c:a", "aac"], false);
    fixture(&input.join("lec2.mp4"), &["-c:v", "mpeg4", "-c:a", "aac"], false);
    fixture(&input.join("week 2").join("part 1: intro.mkv"), &["-c:v", "mpeg4", "-c:a", "aac"], false);
    fixture(&input.join("voice.wav"), &["-c:a", "pcm_s16le"], true);

    // Flat, videos only, into the default sibling folder.
    let out = istma(&["دروس"], dir.path());
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let output = dir.path().join("دروس-opus");
    assert_good_opus(&output.join("المحاضرة الأولى.opus"));
    assert_good_opus(&output.join("lec2.mkv.opus"));
    assert_good_opus(&output.join("lec2.mp4.opus"));
    assert!(!output.join("week 2").exists());
    assert!(!output.join("voice.opus").exists());

    // Subfolders and audio are opt-in; what is done is not done again.
    let out = istma(&["-r", "-a", "دروس"], dir.path());
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("2 files converted, 3 already done"));
    assert_good_opus(&output.join("week 2").join("part 1: intro.opus"));
    assert_good_opus(&output.join("voice.opus"));

    let out = istma(&["-r", "-a", "دروس"], dir.path());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Nothing to do"));
    assert!(leftovers(dir.path()).is_empty());
}

#[test]
fn presets_shrink_the_file() {
    if !have_tools() {
        return;
    }
    let dir = TempDir::new().unwrap();
    fixture(&dir.path().join("a.mp4"), &["-c:v", "mpeg4", "-c:a", "aac"], false);
    let size = |preset: &str| {
        let name = format!("{preset}.opus");
        let out = istma(&["-p", preset, "a.mp4", &name], dir.path());
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert_good_opus(&dir.path().join(&name));
        fs::metadata(dir.path().join(name)).unwrap().len()
    };
    let (standard, small, smallest) = (size("standard"), size("small"), size("smallest"));
    assert!(standard > small && small > smallest, "{standard} {small} {smallest}");
}

#[test]
fn bad_files_fail_without_stopping_the_rest() {
    if !have_tools() {
        return;
    }
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("in");
    fixture(&input.join("good.mp4"), &["-c:v", "mpeg4", "-c:a", "aac"], false);
    fixture(&input.join("silent.mp4"), &["-c:v", "mpeg4", "-an"], false);
    fs::write(input.join("junk.mp4"), b"this is not a video").unwrap();

    let out = istma(&["in", "out"], dir.path());
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("failed: silent.mp4: no audio track"), "{stderr}");
    assert!(stderr.contains("failed: junk.mp4"), "{stderr}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("1 file converted, 2 failed"));
    assert_good_opus(&dir.path().join("out/good.opus"));
    assert_eq!(walk(&dir.path().join("out")).len(), 1);
}

#[cfg(unix)]
#[test]
fn interrupting_leaves_nothing_half_done() {
    if !have_tools() {
        return;
    }
    let dir = TempDir::new().unwrap();
    // Six hours of silence: tiny as FLAC, but long enough to still be converting when interrupted.
    let made = Command::new("ffmpeg")
        .args(["-nostdin", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "anullsrc=r=8000:cl=mono", "-t", "21600"])
        .arg(dir.path().join("long.flac"))
        .status()
        .unwrap();
    assert!(made.success());

    let mut child = Command::new(env!("CARGO_BIN_EXE_istma"))
        .args(["long.flac", "long.opus"])
        .current_dir(dir.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let part = dir.path().join("long.opus.part");
    for _ in 0..200 {
        if part.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(part.exists(), "the conversion never started");
    Command::new("kill").args(["-INT", &child.id().to_string()]).status().unwrap();
    let status = child.wait().unwrap();

    assert_eq!(status.code(), Some(130));
    assert!(walk(dir.path()).iter().all(|p| p.ends_with("long.flac")), "{:?}", walk(dir.path()));
}
