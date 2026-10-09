//! The validated model the engine runs, in standard units.
//!
//! Built by `outliers-sim-model` from a YAML file. Objects refer to each other
//! by index, in file order, so the same file always produces the same `Spec`.
//! All durations are seconds and all lengths are meters.

use crate::SimTime;
use crate::stats::ln_gamma;

#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub entities: Vec<String>,
    pub sources: Vec<Source>,
    pub servers: Vec<Server>,
    pub conveyors: Vec<Conveyor>,
    pub sinks: Vec<Sink>,
    pub experiment: Experiment,
}

/// A source, server or sink: anything a conveyor can start or end at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Node {
    Source(usize),
    Server(usize),
    Sink(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub name: String,
    /// Index into [`Spec::entities`].
    pub entity: usize,
    pub interarrival: Distribution,
    pub when_blocked: WhenBlocked,
    /// Index into [`Spec::conveyors`].
    pub output: usize,
}

/// What a source does with a new part when its output conveyor is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhenBlocked {
    /// The source stops until the conveyor has room.
    Wait,
    /// The part is discarded and counted as a lost arrival.
    Drop,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Server {
    pub name: String,
    pub capacity: u32,
    pub process_time: Distribution,
    pub failures: Vec<Failure>,
    /// Index into [`Spec::conveyors`].
    pub input: usize,
    /// Index into [`Spec::conveyors`].
    pub output: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub name: String,
    /// Time between failures, measured on [`Failure::clock`].
    pub uptime: Distribution,
    pub downtime: Distribution,
    pub clock: FailureClock,
    pub on_failure: OnFailure,
}

/// Which time counts toward the next failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClock {
    /// Only time spent processing.
    BusyTime,
    /// All simulated time, whatever the server is doing.
    ClockTime,
}

/// What happens to a part being processed when the server fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnFailure {
    /// Processing continues with the remaining time after repair.
    Resume,
    /// Processing starts over after repair.
    Restart,
}

/// An accumulating conveyor: parts queue up at the end when the next object
/// can't take them.
#[derive(Debug, Clone, PartialEq)]
pub struct Conveyor {
    pub name: String,
    pub from: Node,
    pub to: Node,
    /// Meters.
    pub length: f64,
    /// Meters per second.
    pub speed: f64,
    /// Minimum distance between parts, in meters.
    pub spacing: f64,
    /// Maximum number of parts on the conveyor: `floor(length / spacing)`.
    pub capacity: u32,
}

impl Conveyor {
    /// Seconds for a part to travel the whole conveyor when nothing blocks it.
    pub fn transit_time(&self) -> f64 {
        self.length / self.speed
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sink {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Experiment {
    pub replications: u32,
    pub warmup: SimTime,
    /// Measured time after the warm-up.
    pub run_length: SimTime,
    pub seed: u64,
}

/// A distribution of durations. All parameters are in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Distribution {
    Constant(f64),
    Exponential {
        mean: f64,
    },
    Uniform {
        min: f64,
        max: f64,
    },
    Triangular {
        min: f64,
        mode: f64,
        max: f64,
    },
    /// Parameterized by the mean and standard deviation of the
    /// distribution itself, not of the underlying normal.
    Lognormal {
        mean: f64,
        sd: f64,
    },
    /// `shape` is a plain number: below 1 failures get rarer with age, 1 is
    /// exponential, above 1 is wear-out. `scale` is the characteristic life
    /// (63.2% fail before it).
    Weibull {
        shape: f64,
        scale: f64,
    },
    /// `shape` is a plain number; the mean is `shape × scale`.
    Gamma {
        shape: f64,
        scale: f64,
    },
}

impl Distribution {
    pub fn mean(&self) -> f64 {
        match *self {
            Distribution::Constant(value) => value,
            Distribution::Exponential { mean } => mean,
            Distribution::Uniform { min, max } => (min + max) / 2.0,
            Distribution::Triangular { min, mode, max } => (min + mode + max) / 3.0,
            Distribution::Lognormal { mean, .. } => mean,
            Distribution::Weibull { shape, scale } => scale * ln_gamma(1.0 + 1.0 / shape).exp(),
            Distribution::Gamma { shape, scale } => shape * scale,
        }
    }
}
