use std::fmt;
use std::ops::{Add, AddAssign, Sub};

/// Simulation time as integer microseconds since the start of the run.
///
/// Integer time keeps simultaneous events exactly simultaneous, so tie-breaking
/// follows an explicit rule instead of floating-point rounding, and runs are
/// reproducible across platforms. `i64` microseconds covers about 292,000 years.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SimTime(i64);

impl SimTime {
    pub const ZERO: SimTime = SimTime(0);
    pub const MAX: SimTime = SimTime(i64::MAX);

    const MICROS_PER_SEC: f64 = 1_000_000.0;

    pub const fn from_micros(micros: i64) -> Self {
        SimTime(micros)
    }

    /// Converts seconds to `SimTime`, rounding to the nearest microsecond.
    ///
    /// Panics if `secs` is not finite or out of range. Values drawn from
    /// distributions must be validated before they reach the engine.
    pub fn from_secs_f64(secs: f64) -> Self {
        let micros = (secs * Self::MICROS_PER_SEC).round();
        assert!(
            micros.is_finite() && micros >= i64::MIN as f64 && micros <= i64::MAX as f64,
            "time out of range: {secs} s"
        );
        SimTime(micros as i64)
    }

    pub const fn as_micros(self) -> i64 {
        self.0
    }

    pub fn as_secs_f64(self) -> f64 {
        self.0 as f64 / Self::MICROS_PER_SEC
    }
}

impl Add for SimTime {
    type Output = SimTime;

    fn add(self, rhs: SimTime) -> SimTime {
        SimTime(self.0.checked_add(rhs.0).expect("simulation time overflow"))
    }
}

impl AddAssign for SimTime {
    fn add_assign(&mut self, rhs: SimTime) {
        *self = *self + rhs;
    }
}

impl Sub for SimTime {
    type Output = SimTime;

    fn sub(self, rhs: SimTime) -> SimTime {
        SimTime(self.0.checked_sub(rhs.0).expect("simulation time overflow"))
    }
}

impl fmt::Display for SimTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} s", self.as_secs_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_to_nearest_microsecond() {
        assert_eq!(SimTime::from_secs_f64(1.0000004).as_micros(), 1_000_000);
        assert_eq!(SimTime::from_secs_f64(1.0000006).as_micros(), 1_000_001);
    }

    #[test]
    fn repeated_addition_is_exact() {
        // The float version of this drifts: 0.1 summed a million times != 100_000.0
        let step = SimTime::from_secs_f64(0.1);
        let mut t = SimTime::ZERO;
        for _ in 0..1_000_000 {
            t += step;
        }
        assert_eq!(t, SimTime::from_secs_f64(100_000.0));
    }

    #[test]
    fn simultaneous_events_compare_equal() {
        let a = SimTime::from_secs_f64(0.1) + SimTime::from_secs_f64(0.2);
        let b = SimTime::from_secs_f64(0.3);
        assert_eq!(a, b); // 0.1 + 0.2 != 0.3 in f64
    }

    #[test]
    #[should_panic(expected = "time out of range")]
    fn rejects_nan() {
        SimTime::from_secs_f64(f64::NAN);
    }
}
