//! Speed and time-left estimates from how much media time has been processed.

use std::collections::VecDeque;
use std::time::Duration;

/// Speed is measured over this much recent wall time, so it follows changes without jumping around.
const WINDOW: Duration = Duration::from_secs(20);
/// No estimate before this much wall time has passed: the first moments are not representative.
const WARM_UP: Duration = Duration::from_millis(1500);

#[derive(Debug, Default)]
pub struct Estimator {
    /// (wall time since the start, media time done)
    points: VecDeque<(Duration, Duration)>,
}

impl Estimator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, elapsed: Duration, done: Duration) {
        if self.points.is_empty() {
            self.points.push_back((Duration::ZERO, Duration::ZERO));
        }
        self.points.push_back((elapsed, done));
        while self.points.len() > 2 && elapsed.saturating_sub(self.points[1].0) >= WINDOW {
            self.points.pop_front();
        }
    }

    /// Media seconds processed per wall second.
    pub fn speed(&self) -> Option<f64> {
        let (&(t0, d0), &(t1, d1)) = (self.points.front()?, self.points.back()?);
        let wall = t1.checked_sub(t0)?.as_secs_f64();
        let media = d1.checked_sub(d0)?.as_secs_f64();
        (t1 >= WARM_UP && wall > 0.0 && media > 0.0).then(|| media / wall)
    }

    pub fn remaining(&self, total: Duration) -> Option<Duration> {
        let speed = self.speed()?;
        let left = total.saturating_sub(self.points.back()?.1).as_secs_f64();
        Some(Duration::from_secs_f64(left / speed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn steady_speed_gives_exact_estimate() {
        let mut est = Estimator::new();
        for t in 1..=10 {
            est.update(secs(t), secs(t * 300));
        }
        assert_eq!(est.speed().unwrap().round(), 300.0);
        // 3000 s of 9000 s done at 300x: 20 s left.
        assert_eq!(est.remaining(secs(9000)).unwrap().as_secs(), 20);
    }

    #[test]
    fn no_estimate_during_warm_up() {
        let mut est = Estimator::new();
        est.update(Duration::from_millis(200), secs(60));
        assert!(est.speed().is_none());
        assert!(est.remaining(secs(600)).is_none());
    }

    #[test]
    fn follows_a_slowdown() {
        let mut est = Estimator::new();
        for t in 1..=30 {
            est.update(secs(t), secs(t * 400));
        }
        // From 30 s on, only 100x.
        for t in 31..=60 {
            est.update(secs(t), secs(12_000 + (t - 30) * 100));
        }
        assert_eq!(est.speed().unwrap().round(), 100.0);
    }

    #[test]
    fn finished_means_nothing_left() {
        let mut est = Estimator::new();
        est.update(secs(5), secs(1000));
        assert_eq!(est.remaining(secs(1000)).unwrap(), Duration::ZERO);
    }
}
