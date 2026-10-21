//! What one replication measures, after the warm-up.

use crate::SimTime;

#[derive(Debug, Clone, PartialEq)]
pub struct ReplicationResult {
    pub replication: u32,
    /// Length of the measured period, in seconds (the experiment's run length).
    pub measured: f64,
    pub sources: Vec<SourceResult>,
    pub servers: Vec<ServerResult>,
    pub conveyors: Vec<ConveyorResult>,
    pub sinks: Vec<SinkResult>,
    pub resources: Vec<ResourceResult>,
    /// Counts over the whole run, warm-up included, for conservation checks.
    pub totals: Totals,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceResult {
    pub created: u64,
    /// Parts discarded because the conveyor was full (`when_blocked: drop`).
    pub lost: u64,
    /// Fraction of time the source was stopped by a full conveyor (`when_blocked: wait`).
    pub blocked: f64,
}

/// State fractions are shares of the server's capacity over time, so they
/// add up to 1. While the server is failed, all of its capacity counts as
/// failed; while off shift, as off shift (even if it's also being repaired).
#[derive(Debug, Clone, PartialEq)]
pub struct ServerResult {
    /// Nothing to work on (starved).
    pub idle: f64,
    pub busy: f64,
    /// Finished parts waiting for room on the outgoing conveyor.
    pub blocked: f64,
    pub failed: f64,
    /// Not working because of its schedule.
    pub off_shift: f64,
    /// Parts that left the server.
    pub processed: u64,
    pub failures: u64,
    /// Fraction of time a part was ready but resource units weren't (part
    /// of `idle`).
    pub waiting_for_resources: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResourceResult {
    /// Time-weighted share of units in use.
    pub busy: f64,
    /// Share of time off shift.
    pub off_shift: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConveyorResult {
    /// Time-weighted mean number of parts on the conveyor.
    pub mean_parts: f64,
    pub max_parts: u64,
    /// Fraction of time the conveyor held its full capacity.
    pub full: f64,
    /// Parts that got on the conveyor.
    pub entered: u64,
    /// Seconds from getting on to leaving, for parts that left. For a
    /// queue, the time waiting in it.
    pub time: Tally,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SinkResult {
    pub exited: u64,
    /// Seconds from creation to reaching the sink, for parts that exited.
    pub time_in_system: Tally,
    /// The same, per part type (indexed like [`crate::Spec::entities`]).
    pub by_type: Vec<TypeResult>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeResult {
    pub exited: u64,
    pub time_in_system: Tally,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Totals {
    /// By sources.
    pub created: u64,
    /// Parts made by assembly servers.
    pub assembled: u64,
    /// Components used up by assembly servers.
    pub consumed: u64,
    pub lost: u64,
    pub exited: u64,
    /// Parts still on conveyors, in servers or held by sources when the run ended.
    pub in_system: u64,
}

/// Running statistics of individual observations.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tally {
    pub count: u64,
    sum: f64,
    sum_sq: f64,
    pub min: f64,
    pub max: f64,
}

impl Tally {
    pub fn add(&mut self, x: f64) {
        if self.count == 0 {
            self.min = x;
            self.max = x;
        } else {
            self.min = self.min.min(x);
            self.max = self.max.max(x);
        }
        self.count += 1;
        self.sum += x;
        self.sum_sq += x * x;
    }

    /// NaN when there are no observations.
    pub fn mean(&self) -> f64 {
        self.sum / self.count as f64
    }

    pub fn sd(&self) -> f64 {
        if self.count < 2 {
            return f64::NAN;
        }
        let n = self.count as f64;
        ((self.sum_sq - self.sum * self.sum / n) / (n - 1.0))
            .max(0.0)
            .sqrt()
    }
}

/// Time-weighted average of a value that changes at event times. Exact:
/// integer value × integer microseconds.
#[derive(Debug, Clone, Default)]
pub(crate) struct TimeAverage {
    area: u128,
    value: u64,
    max: u64,
    last: SimTime,
    start: SimTime,
}

impl TimeAverage {
    pub fn set(&mut self, now: SimTime, value: u64) {
        let elapsed = (now - self.last).as_micros();
        debug_assert!(elapsed >= 0, "time went backwards");
        self.area += u128::from(self.value) * elapsed as u128;
        self.last = now;
        self.value = value;
        self.max = self.max.max(value);
    }

    /// Starts measuring from `now`, keeping the current value.
    pub fn reset(&mut self, now: SimTime) {
        self.area = 0;
        self.last = now;
        self.start = now;
        self.max = self.value;
    }

    /// Mean over [start, last]. Call `set` at the end time first.
    pub fn mean(&self) -> f64 {
        let span = (self.last - self.start).as_micros();
        if span == 0 {
            return self.value as f64;
        }
        self.area as f64 / span as f64
    }

    pub fn max(&self) -> u64 {
        self.max
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_average_weights_by_duration() {
        let t = SimTime::from_micros;
        let mut avg = TimeAverage::default();
        avg.set(t(0), 2); // 2 for 10 µs
        avg.set(t(10), 4); // 4 for 30 µs
        avg.set(t(40), 0);
        assert_eq!(avg.mean(), (2.0 * 10.0 + 4.0 * 30.0) / 40.0);
        assert_eq!(avg.max(), 4);

        avg.reset(t(40));
        avg.set(t(50), 1);
        avg.set(t(60), 1);
        assert_eq!(avg.mean(), 0.5);
        assert_eq!(avg.max(), 1);
    }

    #[test]
    fn tally() {
        let mut tally = Tally::default();
        for x in [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0] {
            tally.add(x);
        }
        assert_eq!(tally.mean(), 5.0);
        assert!((tally.sd() - 2.138_089_935).abs() < 1e-9);
        assert_eq!((tally.min, tally.max), (2.0, 9.0));
    }
}
