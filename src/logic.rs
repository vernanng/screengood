//! Pure decision logic ported 1:1 from the Python helpers embedded in
//! `Reels-Scroll.command` (`decide`, `accumulate_pixels`, `wheel_decision`,
//! swipe-point scaling).
//! No FFI and no I/O here; everything is unit-testable.

use std::time::{Duration, Instant};

/// Light scroll: ~6px of accumulated wheel movement fires one step.
pub const PIXEL_THRESHOLD: f64 = 6.0;
/// Minimum time between two fired steps (prevents double-fire).
pub const COOLDOWN: Duration = Duration::from_millis(900);
/// Accumulator resets when wheel ticks pause longer than this.
pub const ACCUM_RESET: Duration = Duration::from_millis(350);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Next,
    Prev,
}

/// Phone screen geometry: swipe/tap points scaled from the known-good
/// S25 FE (1080x2340) points to the real screen size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub pw: i64,
    pub ph: i64,
    pub cx: i64,
    pub swipe_bot: i64,
    pub swipe_top: i64,
    pub tap_y: i64,
}

fn py_round(x: f64) -> i64 {
    x.round_ties_even() as i64
}

impl Geometry {
    pub fn new(pw: i64, ph: i64) -> Self {
        Self {
            pw,
            ph,
            cx: pw / 2,
            swipe_bot: py_round(ph as f64 * 1700.0 / 2340.0),
            swipe_top: py_round(ph as f64 * 700.0 / 2340.0),
            tap_y: py_round(ph as f64 * 1150.0 / 2340.0),
        }
    }
}

impl Default for Geometry {
    fn default() -> Self {
        Geometry::new(1080, 2340)
    }
}

/// Dominant-axis wheel direction. Wheel DOWN (negative delta in the Quartz
/// convention used by the Python code) = NEXT.
pub fn decide(dx: f64, dy: f64) -> Option<Action> {
    if dx.abs() < 1e-9 && dy.abs() < 1e-9 {
        return None;
    }
    let is_next = if dx.abs() > dy.abs() {
        dx < 0.0
    } else {
        dy < 0.0
    };
    Some(if is_next { Action::Next } else { Action::Prev })
}

/// Accumulates sub-threshold pixel deltas until they add up to one step.
#[derive(Debug, Default)]
pub struct PixelAccumulator {
    pub acc_dx: f64,
    pub acc_dy: f64,
    pub last: Option<Instant>,
}

impl PixelAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn accumulate(&mut self, dx: f64, dy: f64, now: Instant) -> Option<Action> {
        if let Some(last) = self.last {
            if now.saturating_duration_since(last) > ACCUM_RESET {
                self.reset();
            }
        }
        self.last = Some(now);
        self.acc_dx += dx;
        self.acc_dy += dy;
        if self.acc_dx.abs().max(self.acc_dy.abs()) < PIXEL_THRESHOLD {
            return None;
        }
        let action = decide(self.acc_dx, self.acc_dy);
        self.reset();
        action
    }

    pub fn reset(&mut self) {
        self.acc_dx = 0.0;
        self.acc_dy = 0.0;
    }
}

#[derive(Debug, Default)]
pub struct Cooldown {
    pub last_fire: Option<Instant>,
}

impl Cooldown {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true (and records the fire time) when COOLDOWN has elapsed.
    pub fn try_fire(&mut self, now: Instant) -> bool {
        if let Some(last_fire) = self.last_fire {
            if now.saturating_duration_since(last_fire) < COOLDOWN {
                return false;
            }
        }
        self.last_fire = Some(now);
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelRoute {
    /// Plain wheel outside the mirror: leave the event for the Mac app.
    Pass,
    /// SHIFT+wheel anywhere, or plain wheel over the mirror: ours.
    Consume,
}

pub fn wheel_route(shift: bool, on_mirror: bool) -> WheelRoute {
    if shift || on_mirror {
        WheelRoute::Consume
    } else {
        WheelRoute::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decide_cases() {
        assert_eq!(decide(0.0, -1.0), Some(Action::Next));
        assert_eq!(decide(0.0, 1.0), Some(Action::Prev));
        assert_eq!(decide(-1.0, 0.0), Some(Action::Next));
        assert_eq!(decide(1.0, 0.0), Some(Action::Prev));
        assert_eq!(decide(0.0, 0.0), None);
    }

    #[test]
    fn geometry_default() {
        let geo = Geometry::default();
        assert_eq!(geo.pw, 1080);
        assert_eq!(geo.ph, 2340);
        assert_eq!(geo.cx, 540);
        assert_eq!(geo.swipe_bot, 1700);
        assert_eq!(geo.swipe_top, 700);
        assert_eq!(geo.tap_y, 1150);
        assert!(geo.swipe_bot > geo.swipe_top && geo.swipe_top > 0);
        assert!(geo.tap_y > 0 && geo.tap_y < geo.ph);
        assert!(geo.cx > 0 && geo.cx < geo.pw);
    }

    #[test]
    fn accumulate_light_scroll() {
        let mut acc = PixelAccumulator::new();
        let t0 = Instant::now();
        let acts: Vec<Option<Action>> = (0..3)
            .map(|i| acc.accumulate(0.0, -2.0, t0 + Duration::from_millis(50 * i)))
            .collect();
        assert_eq!(acts, vec![None, None, Some(Action::Next)]);
    }

    #[test]
    fn accumulate_resets_after_gap() {
        let mut acc = PixelAccumulator::new();
        let t0 = Instant::now();
        assert_eq!(acc.accumulate(0.0, -2.0, t0), None);
        assert_eq!(
            acc.accumulate(0.0, -2.0, t0 + Duration::from_millis(50)),
            None
        );
        assert_eq!(
            acc.accumulate(0.0, -2.0, t0 + Duration::from_millis(450)),
            None
        );
    }

    #[test]
    fn cooldown_blocks_and_allows() {
        let mut cd = Cooldown::new();
        let t0 = Instant::now();
        assert!(cd.try_fire(t0));
        assert!(!cd.try_fire(t0 + Duration::from_millis(100)));
        assert!(cd.try_fire(t0 + Duration::from_millis(901)));
    }

    #[test]
    fn wheel_route_truth_table() {
        assert_eq!(wheel_route(true, true), WheelRoute::Consume);
        assert_eq!(wheel_route(true, false), WheelRoute::Consume);
        assert_eq!(wheel_route(false, true), WheelRoute::Consume);
        assert_eq!(wheel_route(false, false), WheelRoute::Pass);
    }
}
