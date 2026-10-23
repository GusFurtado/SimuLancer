//! The validated model the engine runs, in standard units.
//!
//! Built by `outliers-sim-model` from a YAML file. Objects refer to each other
//! by index, in file order, so the same file always produces the same `Spec`.
//! All durations are seconds and all lengths are meters.

use crate::SimTime;
use crate::rng::Rng;
use crate::stats::ln_gamma;

/// Most attributes a model can declare: each part carries all of them.
pub const MAX_ATTRIBUTES: usize = 8;

/// Most resources parts can carry from one server to another (`takes`).
pub const MAX_CARRIED: usize = 4;

#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub entities: Vec<String>,
    /// Values each part carries, at most [`MAX_ATTRIBUTES`].
    pub attributes: Vec<Attribute>,
    pub sources: Vec<Source>,
    pub servers: Vec<Server>,
    pub conveyors: Vec<Conveyor>,
    pub sinks: Vec<Sink>,
    /// Shared resources (operators, tools) that servers need while processing.
    pub resources: Vec<Resource>,
    /// Weekly working times for sources, servers and resources.
    pub schedules: Vec<Schedule>,
    /// Vehicles that carry parts along transports.
    pub vehicles: Vec<Fleet>,
    /// Paced lines: stations that move their parts on together.
    pub lines: Vec<PacedLine>,
    pub experiment: Experiment,
}

/// A value each part carries, like a due date or a rework count. Times are
/// in seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub name: String,
    /// Holds a time (in seconds) rather than a plain number.
    pub time: bool,
    pub default: f64,
}

/// A formula over a part's attributes, the clock and random draws. Checked
/// when the model is loaded (units, names), so evaluating it can't fail.
/// Comparisons and `and`/`or`/`not` give 1 for true and 0 for false.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A plain number.
    Value(f64),
    /// A time, in seconds.
    Seconds(f64),
    /// Index into [`Spec::attributes`].
    Attribute(usize),
    /// The simulation clock, in seconds.
    Now,
    /// A random draw of times (in seconds) or of plain numbers.
    Draw {
        distribution: Distribution,
        time: bool,
    },
    /// Whether the part is of this type (an index into [`Spec::entities`]).
    IsType(usize),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

impl Expr {
    /// The value for a part with these attributes and type, at `now`.
    pub(crate) fn eval(
        &self,
        attributes: &[f64],
        entity: usize,
        now: SimTime,
        rng: &mut Rng,
    ) -> f64 {
        let truth = |b: bool| if b { 1.0 } else { 0.0 };
        match self {
            Expr::Value(v) | Expr::Seconds(v) => *v,
            Expr::Attribute(i) => attributes[*i],
            Expr::Now => now.as_secs_f64(),
            Expr::Draw { distribution, .. } => rng.sample(distribution),
            Expr::IsType(e) => truth(entity == *e),
            Expr::Neg(x) => -x.eval(attributes, entity, now, rng),
            Expr::Not(x) => truth(x.eval(attributes, entity, now, rng) == 0.0),
            Expr::Binary(op, a, b) => {
                let a = a.eval(attributes, entity, now, rng);
                // `and` and `or` don't evaluate (or draw) their right side when the left decides.
                match op {
                    BinaryOp::And if a == 0.0 => return 0.0,
                    BinaryOp::Or if a != 0.0 => return 1.0,
                    _ => {}
                }
                let b = b.eval(attributes, entity, now, rng);
                // Times are whole microseconds in the engine; compare them at that grain.
                let close = (a - b).abs() < 1e-9;
                match op {
                    BinaryOp::Add => a + b,
                    BinaryOp::Sub => a - b,
                    BinaryOp::Mul => a * b,
                    BinaryOp::Div => a / b,
                    BinaryOp::Lt => truth(a < b && !close),
                    BinaryOp::Le => truth(a < b || close),
                    BinaryOp::Gt => truth(a > b && !close),
                    BinaryOp::Ge => truth(a > b || close),
                    BinaryOp::Eq => truth(close),
                    BinaryOp::Ne => truth(!close),
                    BinaryOp::And | BinaryOp::Or => truth(b != 0.0),
                }
            }
        }
    }
}

/// Units shared by servers: a server holds the units it needs while it
/// processes a part.
#[derive(Debug, Clone, PartialEq)]
pub struct Resource {
    pub name: String,
    pub capacity: u32,
    /// Index into [`Spec::schedules`]; `None` is always available.
    pub schedule: Option<usize>,
    /// Servers served first when units come back, highest priority first
    /// (indices into [`Spec::servers`]); others follow in the order they
    /// started waiting.
    pub order: Vec<usize>,
}

/// When something works, repeating every week. Time 0 is Monday 00:00.
#[derive(Debug, Clone, PartialEq)]
pub struct Schedule {
    pub name: String,
    /// Working periods within the week, as [start, end) offsets from Monday
    /// 00:00: sorted, not overlapping, not touching, within one week.
    pub periods: Vec<(SimTime, SimTime)>,
    /// At the end of a shift, servers and resources on this schedule finish
    /// the work in hand (overtime) before they stop.
    pub finish_work: bool,
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
    /// Attributes set on each new part, in order: (index into
    /// [`Spec::attributes`], value).
    pub sets: Vec<(usize, Expr)>,
}

/// Time to set a station up for another part type.
#[derive(Debug, Clone, PartialEq)]
pub enum Changeover {
    /// The same for any change of type.
    Any(Distribution),
    /// Indexed `[from][to]` by [`Spec::entities`]; `None` takes no time.
    Pairs(Vec<Vec<Option<Distribution>>>),
}

impl Changeover {
    /// The setup from one type to another, if it takes any.
    pub fn between(&self, from: usize, to: usize) -> Option<&Distribution> {
        if from == to {
            return None;
        }
        match self {
            Changeover::Any(d) => Some(d),
            Changeover::Pairs(pairs) => pairs[from][to].as_ref(),
        }
    }
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
    /// Ordered to keep a queue stocked (see [`Restock`]).
    Restock(Restock),
    /// A rate that changes with the time of day, the same every day: a
    /// Poisson process with this rate. Periods are (start, end, parts per
    /// second), in seconds from midnight, sorted and not overlapping; the
    /// rate is zero outside them.
    Profile(Vec<(f64, f64, f64)>),
}

/// A source that orders parts to keep a queue stocked. Its *stock
/// position* is what the queue holds, plus what the source has created that
/// hasn't got to the queue yet, plus what's on order. When the position is
/// at or below `reorder_point`, it orders: deliveries come after the lead
/// time, all at once.
#[derive(Debug, Clone, PartialEq)]
pub struct Restock {
    /// Index into [`Spec::conveyors`]: the queue it keeps stocked.
    pub queue: usize,
    pub reorder_point: u32,
    pub quantity: OrderQuantity,
    pub lead_time: Distribution,
    /// Check every this long (from time 0) instead of after every part
    /// that leaves the queue.
    pub review_every: Option<SimTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderQuantity {
    /// Order this many, as often as needed to get above the reorder point.
    Fixed(u32),
    /// Order enough to bring the position up to this.
    UpTo(u32),
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
    /// Resource units each part takes as it starts here and keeps until a
    /// server in `gives_back` is done with it: (index into
    /// [`Spec::resources`], units). Taken together with `needs`.
    pub takes: Vec<(usize, u32)>,
    /// Resources (indices into [`Spec::resources`]) whose units a part
    /// carries that it gives back when its processing here finishes.
    pub gives_back: Vec<usize>,
    /// Index into [`Spec::schedules`]; `None` works all the time.
    pub schedule: Option<usize>,
    /// One part at a time, batches, or assembly.
    pub kind: ServerKind,
    /// After processing, each part becomes this many parts of this type
    /// (an index into [`Spec::entities`]); they leave one by one.
    pub splits_into: Option<(usize, Count)>,
    /// Setup time when a slot starts a part of another type than its last.
    pub changeover: Option<Changeover>,
    pub failures: Vec<Failure>,
    /// Indices into [`Spec::conveyors`]. In priority order for
    /// [`InputRule::Priority`], otherwise in file order.
    pub inputs: Vec<usize>,
    pub input_rule: InputRule,
    /// Indices into [`Spec::conveyors`], in file order.
    pub outputs: Vec<usize>,
    pub routing: Routing,
    /// Attributes set on each part when its processing finishes, before it
    /// picks its way out (and before a split, whose parts inherit them).
    pub sets: Vec<(usize, Expr)>,
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
        /// The component type whose arrival time the new part keeps (the
        /// earliest of that type); `None` keeps the earliest of all.
        clock_of: Option<usize>,
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
    /// The first route whose condition holds, in order; the last has no
    /// condition and takes every other part. Each is (condition, an index
    /// into [`Spec::conveyors`]). The part waits for its conveyor.
    ByCondition(Vec<(Option<Expr>, usize)>),
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
    /// The waiting part with the lowest value of this attribute (an index
    /// into [`Spec::attributes`]); ties go to the one that reached the end first.
    Lowest(usize),
    /// The waiting part with the highest value of this attribute.
    Highest(usize),
}

/// Which part a queue lets go next.
#[derive(Debug, Clone, PartialEq)]
pub enum QueueOrder {
    /// First in, first out.
    FirstIn,
    /// Last in, first out.
    LastIn,
    /// Lowest value of an attribute (an index into [`Spec::attributes`])
    /// first; ties first in, first out.
    Lowest(usize),
    Highest(usize),
    /// By part type: indexed by [`Spec::entities`], each type's rank (lower
    /// goes first); ties first in, first out.
    ByType(Vec<usize>),
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
    /// Queues only: parts in it when the run starts, as (type, count).
    pub starts_with: Option<(usize, u32)>,
    /// Queues only: which part it lets go next.
    pub order: QueueOrder,
    /// Queues only: how long a part waits before it gives up and leaves.
    pub patience: Option<PerType<Distribution>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionKind {
    Conveyor,
    Queue,
    /// Carried by a vehicle of `fleet` (an index into [`Spec::vehicles`]),
    /// `distance` meters.
    Transport {
        fleet: usize,
        distance: f64,
    },
}

/// Stations (servers, in flow order, one part each) that move their parts
/// on together every `takt`; a station that isn't done stops the line until
/// it is.
#[derive(Debug, Clone, PartialEq)]
pub struct PacedLine {
    pub name: String,
    /// Indices into [`Spec::servers`].
    pub stations: Vec<usize>,
    pub takt: SimTime,
}

/// Vehicles that carry parts along transports, one at a time.
#[derive(Debug, Clone, PartialEq)]
pub struct Fleet {
    pub name: String,
    pub count: u32,
    /// Meters per second, loaded or empty.
    pub speed: f64,
    pub load_time: Distribution,
    pub unload_time: Distribution,
    /// Where the vehicles are when the run starts.
    pub home: Node,
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
            starts_with: None,
            order: QueueOrder::FirstIn,
            patience: None,
        }
    }

    /// A transport between two objects, carried by a vehicle of `fleet`.
    pub fn transport(
        name: String,
        from: Node,
        to: Node,
        capacity: Option<u32>,
        fleet: usize,
        distance: f64,
    ) -> Conveyor {
        Conveyor {
            kind: ConnectionKind::Transport { fleet, distance },
            ..Conveyor::queue(name, from, to, capacity, None)
        }
    }

    pub fn is_queue(&self) -> bool {
        self.kind == ConnectionKind::Queue
    }

    /// The fleet and distance of a transport.
    pub fn transport_by(&self) -> Option<(usize, f64)> {
        match self.kind {
            ConnectionKind::Transport { fleet, distance } => Some((fleet, distance)),
            _ => None,
        }
    }

    /// A conveyor: it has a length, and is drawn along a path in the layout.
    /// Queues and transports are drawn straight between their objects.
    pub fn is_belt(&self) -> bool {
        self.kind == ConnectionKind::Conveyor
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
