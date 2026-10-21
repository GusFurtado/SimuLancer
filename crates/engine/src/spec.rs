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
    /// Weekly working times for sources, servers and resources.
    pub schedules: Vec<Schedule>,
    pub experiment: Experiment,
}

/// Units shared by servers: a server holds the units it needs while it
/// processes a part.
#[derive(Debug, Clone, PartialEq)]
pub struct Resource {
    pub name: String,
    pub capacity: u32,
    /// Index into [`Spec::schedules`]; `None` is always available.
    pub schedule: Option<usize>,
}

/// When something works, repeating every week. Time 0 is Monday 00:00.
#[derive(Debug, Clone, PartialEq)]
pub struct Schedule {
    pub name: String,
    /// Working periods within the week, as [start, end) offsets from Monday
    /// 00:00: sorted, not overlapping, not touching, within one week.
    pub periods: Vec<(SimTime, SimTime)>,
}

impl Schedule {
    pub const WEEK: SimTime = SimTime::from_micros(7 * 24 * 3600 * 1_000_000);

    /// Whether `t` falls in a working period.
    pub fn is_on(&self, t: SimTime) -> bool {
        let w = SimTime::from_micros(t.as_micros().rem_euclid(Self::WEEK.as_micros()));
        self.periods.iter().any(|&(s, e)| s <= w && w < e)
    }

    /// How long it's on between `from` and `to`.
    pub fn on_time(&self, from: SimTime, to: SimTime) -> SimTime {
        let week = Self::WEEK.as_micros();
        let (from, to) = (from.as_micros(), to.as_micros());
        let mut total = 0;
        // Each week overlapping [from, to), period by period.
        let mut base = from - from.rem_euclid(week);
        while base < to {
            for &(s, e) in &self.periods {
                let (s, e) = (base + s.as_micros(), base + e.as_micros());
                total += (e.min(to) - s.max(from)).max(0);
            }
            base += week;
        }
        SimTime::from_micros(total)
    }

    /// The first time after `t` at which `is_on` changes, if it ever does.
    pub fn next_change(&self, t: SimTime) -> Option<SimTime> {
        let week = Self::WEEK.as_micros();
        if self.periods.is_empty() || self.periods == [(SimTime::ZERO, Self::WEEK)] {
            return None;
        }
        let w = t.as_micros().rem_euclid(week);
        let base = t.as_micros() - w;
        let boundaries = self
            .periods
            .iter()
            .flat_map(|&(s, e)| [s.as_micros(), e.as_micros()]);
        // Periods don't touch, so every boundary is a real change (a period
        // ending at the week's end and one starting at 0 count as one).
        let wraps = self.periods.first().is_some_and(|p| p.0 == SimTime::ZERO)
            && self.periods.last().is_some_and(|p| p.1 == Self::WEEK);
        let real = |b: i64| !(wraps && (b == 0 || b == week));
        let next = boundaries
            .clone()
            .filter(|&b| b > w && real(b))
            .min()
            .map(|b| base + b)
            .or_else(|| {
                boundaries
                    .filter(|&b| real(b))
                    .min()
                    .map(|b| base + week + b)
            })?;
        Some(SimTime::from_micros(next))
    }
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
    pub arrivals: Arrivals,
    /// Parts per arrival; `None` is one.
    pub group: Option<Count>,
    pub when_blocked: WhenBlocked,
    /// Indices into [`Spec::conveyors`], in file order.
    pub outputs: Vec<usize>,
    pub routing: Routing,
    /// Index into [`Spec::schedules`]: parts only arrive while it's on.
    pub schedule: Option<usize>,
}

/// A whole number of parts: always the same, or drawn uniformly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Count {
    Fixed(u32),
    /// Each value from `min` to `max` (both included) equally likely.
    Uniform {
        min: u32,
        max: u32,
    },
}

impl Count {
    pub fn mean(&self) -> f64 {
        match *self {
            Count::Fixed(n) => f64::from(n),
            Count::Uniform { min, max } => (f64::from(min) + f64::from(max)) / 2.0,
        }
    }
}

/// When a source's parts arrive.
#[derive(Debug, Clone, PartialEq)]
pub enum Arrivals {
    /// A time between arrivals. The first part arrives at time 0, and at
    /// the start of each shift.
    Interarrival(Distribution),
    /// A rate that changes with the time of day, the same every day: a
    /// Poisson process with this rate. Periods are (start, end, parts per
    /// second), in seconds from midnight, sorted and not overlapping; the
    /// rate is zero outside them.
    Profile(Vec<(f64, f64, f64)>),
}

impl Arrivals {
    /// Parts per second at `t`, for a profile.
    pub fn rate_at(periods: &[(f64, f64, f64)], t: SimTime) -> f64 {
        let day = 24.0 * 3600.0;
        let clock = t.as_secs_f64().rem_euclid(day);
        periods
            .iter()
            .find(|&&(s, e, _)| s <= clock && clock < e)
            .map_or(0.0, |&(_, _, rate)| rate)
    }
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
    /// Index into [`Spec::schedules`]; `None` works all the time.
    pub schedule: Option<usize>,
    /// One part at a time, batches, or assembly.
    pub kind: ServerKind,
    /// After processing, each part becomes this many parts of this type
    /// (an index into [`Spec::entities`]); they leave one by one.
    pub splits_into: Option<(usize, Count)>,
    pub failures: Vec<Failure>,
    /// Indices into [`Spec::conveyors`]. In priority order for
    /// [`InputRule::Priority`], otherwise in file order.
    pub inputs: Vec<usize>,
    pub input_rule: InputRule,
    /// Indices into [`Spec::conveyors`], in file order.
    pub outputs: Vec<usize>,
    pub routing: Routing,
}

/// Which part types can get to each object (indexed by object, then by
/// [`Spec::entities`]): what sources create, followed through `by_type`
/// routes and assemblies (which send out what they make).
#[derive(Debug, Clone, PartialEq)]
pub struct TypesAt {
    pub sources: Vec<Vec<bool>>,
    pub servers: Vec<Vec<bool>>,
    pub sinks: Vec<Vec<bool>>,
    /// Which part types travel on each conveyor or queue.
    pub conveyors: Vec<Vec<bool>>,
}

impl Spec {
    pub fn types_at(&self) -> TypesAt {
        let n = self.entities.len();
        let mut at = TypesAt {
            sources: vec![vec![false; n]; self.sources.len()],
            servers: vec![vec![false; n]; self.servers.len()],
            sinks: vec![vec![false; n]; self.sinks.len()],
            conveyors: vec![vec![false; n]; self.conveyors.len()],
        };
        let mut stack: Vec<(Node, usize)> = Vec::new();
        for (i, s) in self.sources.iter().enumerate() {
            for &(e, _) in &s.creates {
                at.sources[i][e] = true;
                stack.push((Node::Source(i), e));
            }
        }
        while let Some((node, e)) = stack.pop() {
            let (outputs, routing, e) = match node {
                Node::Source(i) => (&self.sources[i].outputs, &self.sources[i].routing, e),
                Node::Server(i) => {
                    let s = &self.servers[i];
                    // An assembly sends out what it makes, whatever came in;
                    // a split, what it splits into.
                    let out = match (s.splits_into, &s.kind) {
                        (Some((into, _)), _) => into,
                        (None, ServerKind::Assembly { makes, .. }) => *makes,
                        _ => e,
                    };
                    (&s.outputs, &s.routing, out)
                }
                Node::Sink(_) => continue,
            };
            let next: Vec<usize> = match routing {
                Routing::ByType(routes) => routes.get(e).copied().flatten().into_iter().collect(),
                _ => outputs.clone(),
            };
            for c in next {
                at.conveyors[c][e] = true;
                let seen = match self.conveyors[c].to {
                    Node::Server(s) => &mut at.servers[s][e],
                    Node::Sink(s) => &mut at.sinks[s][e],
                    Node::Source(_) => continue,
                };
                if !*seen {
                    *seen = true;
                    stack.push((self.conveyors[c].to, e));
                }
            }
        }
        at
    }
}

/// What a server does with the parts it takes in.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerKind {
    /// Each part is processed on its own in a slot.
    Single,
    /// Collects this many parts, then processes them together: one process
    /// time, one set of resource units, one slot per part.
    Batch(u32),
    /// Collects a kit, (part type, count) per component, then processes it
    /// as one new part of type `makes`.
    Assembly {
        recipe: Vec<(usize, u32)>,
        makes: usize,
    },
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

/// A connection between two objects: an accumulating conveyor (parts queue
/// up at the end when the next object can't take them), or a queue.
///
/// A queue is a conveyor with no length and no spacing: parts are at its end
/// as soon as they get on and leave first in, first out, so the conveyor
/// rules (blocking, merging) apply unchanged.
#[derive(Debug, Clone, PartialEq)]
pub struct Conveyor {
    pub name: String,
    pub kind: ConnectionKind,
    pub from: Node,
    pub to: Node,
    /// Meters.
    pub length: f64,
    /// Meters per second.
    pub speed: f64,
    /// Minimum distance between parts, in meters.
    pub spacing: f64,
    /// Maximum number of parts on the conveyor: `floor(length / spacing)`.
    /// For a queue, its capacity (`u32::MAX` when unlimited).
    pub capacity: u32,
    /// Queues only: the wait a service level is measured against, in seconds.
    pub target_wait: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionKind {
    Conveyor,
    Queue,
}

impl Conveyor {
    /// A queue between two objects; `None` capacity is unlimited.
    pub fn queue(
        name: String,
        from: Node,
        to: Node,
        capacity: Option<u32>,
        target_wait: Option<f64>,
    ) -> Conveyor {
        Conveyor {
            name,
            kind: ConnectionKind::Queue,
            from,
            to,
            length: 0.0,
            speed: 1.0,
            spacing: 0.0,
            capacity: capacity.unwrap_or(u32::MAX),
            target_wait,
        }
    }

    pub fn is_queue(&self) -> bool {
        self.kind == ConnectionKind::Queue
    }
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
