//! Human-readable sizes, durations and counts, shared by the CLI and the GUI.

use std::time::Duration;

pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    match unit {
        0 => format!("{n} B"),
        _ if value >= 100.0 => format!("{value:.0} {}", UNITS[unit]),
        _ => format!("{value:.1} {}", UNITS[unit]),
    }
}

/// `45s`, `2m10s`, `1h05m`.
pub fn duration(d: Duration) -> String {
    let s = d.as_secs_f64().round() as u64;
    match (s / 3600, s % 3600 / 60, s % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, s) => format!("{m}m{s:02}s"),
        (h, m, _) => format!("{h}h{m:02}m"),
    }
}

/// How many times faster than real time: `380×`.
pub fn speed(x: f64) -> String {
    if x >= 10.0 { format!("{x:.0}×") } else { format!("{x:.1}×") }
}

pub fn files(n: usize) -> String {
    if n == 1 { "1 file".into() } else { format!("{n} files") }
}

/// The share of the original size that is left, as `2.5%`.
pub fn percent_of(part: u64, whole: u64) -> String {
    if whole == 0 {
        return "0%".into();
    }
    let p = part as f64 * 100.0 / whole as f64;
    if p < 10.0 { format!("{p:.1}%") } else { format!("{p:.0}%") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(5_140_000), "5.1 MB");
        assert_eq!(bytes(12_400_000_000), "12.4 GB");
        assert_eq!(bytes(310_000_000), "310 MB");
        assert_eq!(duration(Duration::from_secs(45)), "45s");
        assert_eq!(duration(Duration::from_secs(130)), "2m10s");
        assert_eq!(duration(Duration::from_secs(3900)), "1h05m");
        assert_eq!(speed(380.4), "380×");
        assert_eq!(percent_of(25, 1000), "2.5%");
    }
}
