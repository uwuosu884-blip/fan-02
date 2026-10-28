use crate::config::CurvePoint;

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
}
