use crate::config::CurvePoint;
use std::time::{Duration, Instant};

/// Which rising-temperature delay applies at `t`: the "below threshold" one
/// or the "above threshold" one.
pub fn rise_delay_ms_for(t: f32, threshold_c: f32, below_ms: u64, above_ms: u64) -> u64 {
    if t < threshold_c {
        below_ms
    } else {
        above_ms
    }
}

/// Debounces *upward* fan-curve reactions.
///
/// A rising temperature does not move the curve immediately; it first has to
/// hold at (or keep climbing past) the new level for the configured delay.
/// Only once that delay elapses does `tracked_temp` jump to the *current*
/// temperature, so the fan never lags behind reality after the wait.
///
/// Downward moves don't pass through here at all — those are governed by
/// `hysteresis_c` in the caller.
#[derive(Debug, Default)]
pub struct RiseFilter {
    /// The rise we're currently waiting out: (temperature when noticed, when).
    pending: Option<(f32, Instant)>,
}

impl RiseFilter {
    pub fn new() -> Self {
        Self { pending: None }
    }

    /// Forget any in-flight rise. Call when the temperature is *not* rising.
    pub fn clear(&mut self) {
        self.pending = None;
    }

    /// The rise currently being waited out, as (anchor temp, since when).
    #[cfg(test)]
    pub fn pending(&self) -> Option<(f32, Instant)> {
        self.pending
    }

    /// Feed a temperature sample. Returns `Some(t)` when the caller should
    /// raise `tracked_temp` to `t` right now; `None` when the rise is being
    /// held back (or isn't a rise, or the delay deliberately doesn't fire).
    ///
    /// `tracked` is the temperature currently in effect, so this is a no-op
    /// for samples that don't rise above it.
    pub fn update(
        &mut self,
        tracked: f32,
        t: f32,
        threshold_c: f32,
        below_ms: u64,
        above_ms: u64,
        now: Instant,
    ) -> Option<f32> {
        if t <= tracked {
            self.pending = None;
            return None;
        }
        let delay_ms = rise_delay_ms_for(t, threshold_c, below_ms, above_ms);
        if delay_ms == 0 {
            self.pending = None;
            return Some(t);
        }
        let delay = Duration::from_millis(delay_ms);
        match self.pending {
            // Still at or above the level we started waiting at — keep the
            // original timestamp so the wait isn't restarted by every
            // further upward tick.
            Some((anchor, since)) if t >= anchor => {
                if now.saturating_duration_since(since) >= delay {
                    self.pending = None;
                    Some(t)
                } else {
                    None
                }
            }
            // First rise, or the temperature dipped below the anchor: (re)start.
            _ => {
                self.pending = Some((t, now));
                None
            }
        }
    }
}

/// Piecewise-linear interpolation of the fan curve: returns a speed
/// percentage (0-100) for a given temperature. Below the first point it
/// returns 0 (caller applies `idle_rpm` in that case); above the last point
/// it clamps to the last point's percentage.
pub fn speed_percent_for_temp(curve: &[CurvePoint], temp_c: f32) -> u8 {
    if curve.is_empty() {
        return 0;
    }
    if temp_c <= curve[0].temp_c {
        return curve[0].speed_percent;
    }
    if temp_c >= curve[curve.len() - 1].temp_c {
        return curve[curve.len() - 1].speed_percent;
    }
    for w in curve.windows(2) {
        let (a, b) = (w[0], w[1]);
        if temp_c >= a.temp_c && temp_c <= b.temp_c {
            let span = b.temp_c - a.temp_c;
            if span <= 0.0 {
                return b.speed_percent;
            }
            let t = (temp_c - a.temp_c) / span;
            let pa = a.speed_percent as f32;
            let pb = b.speed_percent as f32;
            return (pa + (pb - pa) * t).round() as u8;
        }
    }
    curve[curve.len() - 1].speed_percent
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_curve() -> Vec<CurvePoint> {
        vec![
            CurvePoint { temp_c: 40.0, speed_percent: 0 },
            CurvePoint { temp_c: 60.0, speed_percent: 50 },
            CurvePoint { temp_c: 80.0, speed_percent: 100 },
        ]
    }

    #[test]
    fn below_first_point_clamps_low() {
        assert_eq!(speed_percent_for_temp(&sample_curve(), 10.0), 0);
    }

    #[test]
    fn above_last_point_clamps_high() {
        assert_eq!(speed_percent_for_temp(&sample_curve(), 120.0), 100);
    }

    #[test]
    fn midpoint_interpolates() {
        assert_eq!(speed_percent_for_temp(&sample_curve(), 50.0), 25);
        assert_eq!(speed_percent_for_temp(&sample_curve(), 70.0), 75);
    }

    #[test]
    fn exact_point_matches() {
        assert_eq!(speed_percent_for_temp(&sample_curve(), 60.0), 50);
    }

    #[test]
    fn empty_curve_returns_zero() {
        assert_eq!(speed_percent_for_temp(&[], 60.0), 0);
    }

    // ---- RiseFilter -------------------------------------------------

    const THRESH: f32 = 70.0;

    fn feed(
        f: &mut RiseFilter,
        tracked: &mut f32,
        t: f32,
        below_ms: u64,
        above_ms: u64,
        at: Duration,
    ) -> bool {
        let now = Instant::now() + at;
        match f.update(*tracked, t, THRESH, below_ms, above_ms, now) {
            Some(new_t) => {
                *tracked = new_t;
                true
            }
            None => false,
        }
    }

    #[test]
    fn zero_delay_applies_immediately() {
        let mut f = RiseFilter::new();
        let mut tracked = 50.0;
        assert!(feed(&mut f, &mut tracked, 62.0, 0, 0, Duration::ZERO));
        assert_eq!(tracked, 62.0);
    }

    #[test]
    fn rise_below_threshold_waits_full_delay() {
        let mut f = RiseFilter::new();
        let mut tracked = 50.0;
        // noticed at t=0
        assert!(!feed(&mut f, &mut tracked, 60.0, 5000, 0, Duration::ZERO));
        assert_eq!(tracked, 50.0);
        // 4s later, still climbing: still held, and the wait did NOT restart
        assert!(!feed(&mut f, &mut tracked, 61.0, 5000, 0, Duration::from_secs(4)));
        assert_eq!(tracked, 50.0);
        // 5s after the original notice: fires, jumping to current temp
        assert!(feed(&mut f, &mut tracked, 61.5, 5000, 0, Duration::from_secs(5)));
        assert_eq!(tracked, 61.5);
    }

    #[test]
    fn rise_above_threshold_uses_its_own_delay() {
        let mut f = RiseFilter::new();
        let mut tracked = 70.0;
        // 75C is >= threshold -> above_ms (=1000) governs, not below_ms (=60000)
        assert!(!feed(&mut f, &mut tracked, 75.0, 60_000, 1000, Duration::ZERO));
        assert!(feed(&mut f, &mut tracked, 75.0, 60_000, 1000, Duration::from_secs(1)));
        assert_eq!(tracked, 75.0);
    }

    #[test]
    fn falling_dip_below_anchor_restarts_the_wait() {
        let mut f = RiseFilter::new();
        let mut tracked = 50.0;
        assert!(!feed(&mut f, &mut tracked, 60.0, 5000, 0, Duration::ZERO));
        // drops back to just above tracked but below the 60.0 anchor
        assert!(!feed(&mut f, &mut tracked, 55.0, 5000, 0, Duration::from_secs(4)));
        // clock restarted at 4s, so 5s (1s later) must NOT fire yet
        assert!(!feed(&mut f, &mut tracked, 58.0, 5000, 0, Duration::from_secs(5)));
        assert!(feed(&mut f, &mut tracked, 58.0, 5000, 0, Duration::from_secs(9)));
    }

    #[test]
    fn non_rising_sample_clears_pending() {
        let mut f = RiseFilter::new();
        let mut tracked = 50.0;
        feed(&mut f, &mut tracked, 60.0, 5000, 0, Duration::ZERO);
        assert!(f.pending().is_some());
        assert!(!feed(&mut f, &mut tracked, 50.0, 5000, 0, Duration::from_secs(1)));
        assert!(f.pending().is_none());
        // a fresh rise starts its clock over
        assert!(!feed(&mut f, &mut tracked, 60.0, 5000, 0, Duration::from_secs(2)));
        assert!(!feed(&mut f, &mut tracked, 60.0, 5000, 0, Duration::from_secs(3)));
    }

    #[test]
    fn delay_selection_around_threshold() {
        assert_eq!(rise_delay_ms_for(69.9, THRESH, 5000, 500), 5000);
        assert_eq!(rise_delay_ms_for(70.0, THRESH, 5000, 500), 500);
        assert_eq!(rise_delay_ms_for(80.0, THRESH, 5000, 500), 500);
    }
}
