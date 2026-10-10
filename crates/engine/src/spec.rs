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
    /// Shared resources (operators, tools) that servers need while processing.
    pub resources: Vec<Resource>,
    pub experiment: Experiment,
}

/// Units shared by servers: a server holds the units it needs while it
/// processes a part.
#[derive(Debug, Clone, PartialEq)]
pub struct Resource {
    pub name: String,
    pub capacity: u32,
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
    /// The part types it creates, as (index into [`Spec::entities`], share),
    /// shares summing to 1. One type has a share of 1.
    pub creates: Vec<(usize, f64)>,
    pub interarrival: Distribution,
    pub when_blocked: WhenBlocked,
    /// Indices into [`Spec::conveyors`], in file order.
    pub outputs: Vec<usize>,
    pub routing: Routing,
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
    pub process_time: PerType<Distribution>,
    /// Resource units needed to process a part: (index into
    /// [`Spec::resources`], units), all taken at once.
    pub needs: Vec<(usize, u32)>,
    pub failures: Vec<Failure>,
    /// Indices into [`Spec::conveyors`]. In priority order for
    /// [`InputRule::Priority`], otherwise in file order.
    pub inputs: Vec<usize>,
    pub input_rule: InputRule,
    /// Indices into [`Spec::conveyors`], in file order.
    pub outputs: Vec<usize>,
    pub routing: Routing,
}

/// A value that may differ by part type.
#[derive(Debug, Clone, PartialEq)]
pub enum PerType<T> {
    /// The same for every part type.
    Same(T),
    /// Indexed by part type ([`Spec::entities`]). `None` for types that
    /// never get here (checked when the model is loaded).
    ByType(Vec<Option<T>>),
}

impl<T> PerType<T> {
    /// The value for a part type. Panics for a type the model doesn't
    /// define here, which validation rules out.
    pub fn get(&self, entity: usize) -> &T {
        match self {
            PerType::Same(v) => v,
            PerType::ByType(values) => values[entity]
                .as_ref()
                .expect("validated: every part type that gets here has a value"),
        }
    }
}

/// How a source or server picks among its outgoing conveyors.
#[derive(Debug, Clone, PartialEq)]
pub enum Routing {
    /// One outgoing conveyor.
    Single,
    /// Chosen at random when the part is ready to leave, with one weight per
    /// output (summing to 1). The part then waits for that conveyor.
    Probability(Vec<f64>),
    /// Outputs in turn. The part waits for its turn's conveyor.
    RoundRobin,
    /// The first output, in order, with room. If none has room, the part
    /// takes whichever gets room first.
    FirstAvailable,
    /// The output holding the fewest parts, among those with room; ties go to
    /// the output listed first. If none has room, the part takes whichever
    /// gets room first.
    ShortestQueue,
    /// By part type: indexed by [`Spec::entities`], the conveyor (an index
    /// into [`Spec::conveyors`]) each type takes. `None` for types that never
    /// get here. The part waits for its conveyor.
    ByType(Vec<Option<usize>>),
}

/// How a server picks among parts waiting at the ends of its incoming
/// conveyors when it can take one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputRule {
    /// One incoming conveyor.
    Single,
    /// The part that reached the end of its conveyor first. Ties go to the
    /// input listed first.
    FirstCome,
    /// The first input, in the server's `inputs` order, with a part waiting.
    Priority,
    /// Inputs in turn, skipping inputs with nothing waiting.
    RoundRobin,
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
