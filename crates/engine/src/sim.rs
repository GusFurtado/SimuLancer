//! One replication of a model.
//!
//! Rules (also documented in DECISIONS.md):
//! - Each source's first part arrives at time 0.
//! - A source whose conveyor is full either waits, holding the part and not
//!   starting the next interarrival time until the part gets on, or drops it.
//! - Conveyors accumulate. Parts are points moving at the conveyor's speed.
//!   The head stops at the end if the next object can't take it, and the
//!   parts behind stop one spacing apart. A part can get on when the conveyor
//!   holds fewer than its capacity and the last part is at least one spacing
//!   from the entrance. All parts start moving again together when the head leaves.
//! - A server takes a part when it's up and has a free slot. A finished part
//!   stays in its slot (blocked) until an outgoing conveyor has room. Each
//!   blocked part leaves as soon as its conveyor has room; parts waiting for
//!   the same conveyor leave in the order they finished.
//! - Splitting (several outgoing conveyors): the conveyor is chosen when the
//!   part is ready to leave (created, or finished processing). `probability`
//!   draws from the object's own `routing` stream; `round_robin` takes the
//!   next output in turn; for both, the part waits for that conveyor.
//!   `first_available` takes the first output in order with room;
//!   `shortest_queue` the one holding the fewest parts (ties to the first
//!   listed) among those with room. For both, if none has room, the part
//!   waits for whichever gets room first.
//! - Merging (several incoming conveyors): when the server can take a part,
//!   it picks among the conveyors whose head is waiting at the end, by
//!   `first_come` (earliest arrival at the end, ties to the input listed
//!   first), `priority` (input order) or `round_robin` (in turn, skipping
//!   empty inputs). The choice is made in a `Pull` event at the current time,
//!   after every event already due, so simultaneous arrivals compete fairly.
//! - Shared resources: a server starts a part only when it can take every
//!   resource unit it needs at once. Units are held while processing (also
//!   while suspended by a failure) and given back when processing finishes.
//!   Servers waiting for units wait in one queue, first come first served:
//!   when units come back, waiting servers try again in the order they
//!   started waiting.
//! - Schedules repeat weekly; time 0 is Monday 00:00. Off shift, a source
//!   creates nothing (the first arrival of each shift comes at its start); a
//!   server stops like a failed one (processing pauses and resumes at the
//!   next shift, nothing is taken or released, failure clocks pause); a
//!   resource hands out no units, but units in use finish their part.
//! - Batch servers collect parts (any of their inputs, by the input rule)
//!   until the batch is full, then process them together: one process time,
//!   one set of resource units, one slot per part. Parts leave one by one.
//! - Assembly servers collect one kit at a time, taking a waiting part only
//!   if the kit still needs its type (others wait on their conveyor). A
//!   complete kit becomes one new part of the server's `makes` type, created
//!   when its earliest component was, and is processed like any part.
//! - Batches and kits are collected while the server is up, even if all its
//!   slots are busy; they start when there are enough free slots and units.
//! - Failure clocks run only while the server is up. `busy_time` clocks run
//!   only while at least one slot is processing. While the server is down it
//!   does nothing: no processing, no taking parts, no releasing blocked parts.
//! - Events at the same time run in the order they were scheduled. Events at
//!   exactly the end of the run are not processed.

use std::collections::VecDeque;

use crate::SimTime;
use crate::events::{Event, EventList};
use crate::results::{
    ConveyorResult, FleetResult, Histogram, ReplicationResult, ResourceResult, ServerResult,
    SinkResult, SourceResult, Tally, TimeAverage, Totals, TypeResult,
};
use crate::rng::Rng;
use crate::spec::{
    Arrivals, Conveyor, Distribution, FailureClock, InputRule, Node, OnFailure, OrderQuantity,
    Restock, Routing, ServerKind, Spec, WhenBlocked,
};
use crate::trace::{Observer, PartId, TraceEvent};

/// Position tolerance in meters, for float error in positions.
const EPS: f64 = 1e-9;

#[derive(Debug, Clone, Copy)]
struct Part {
    id: PartId,
    created: SimTime,
    /// Index into [`Spec::entities`].
    entity: usize,
}

/// The conveyor a waiting part will leave on; `None` means any output with
/// room (`first_available`).
type Target = Option<usize>;

struct SourceState {
    rng: Rng,
    routing_rng: Rng,
    /// Draws the part type, for sources that create a mix.
    type_rng: Rng,
    /// Accepts or rejects candidate arrivals, for rate profiles (thinning).
    thinning_rng: Rng,
    /// Draws how many parts arrive together (`group`).
    group_rng: Rng,
    next_output: usize,
    /// Parts waiting to get on a conveyor (`when_blocked: wait`), in order:
    /// one, or the rest of a group.
    held: VecDeque<(Part, Target)>,
    off_shift: bool,
    /// Bumped at each shift end, to cancel the pending arrival.
    arrival_generation: u64,
    created: u64,
    /// Created over the whole run (not reset after the warm-up): restocking
    /// counts it in the stock position.
    created_total: u64,
    lost: u64,
    blocked: TimeAverage,
    /// Restocking: parts ordered and not yet delivered.
    on_order: u64,
    on_order_avg: TimeAverage,
    orders: u64,
    lead_rng: Rng,
    /// Deliveries that came while off shift, delivered at the next shift.
    pending: VecDeque<u32>,
}

#[derive(Debug, Clone, Copy)]
enum Slot {
    Idle,
    Busy {
        part: Part,
        finish: SimTime,
        total: SimTime,
        /// Gives the server's resource units back when done (one slot per
        /// batch holds them).
        holds: bool,
    },
    /// Processing interrupted by a failure.
    Suspended {
        part: Part,
        remaining: SimTime,
        total: SimTime,
        holds: bool,
    },
    Blocked {
        part: Part,
        target: Target,
    },
}

struct FailureState {
    uptime_rng: Rng,
    downtime_rng: Rng,
    /// Uptime left until this failure happens, measured on its clock.
    remaining: SimTime,
    /// When the clock last started running; `None` while paused.
    running_since: Option<SimTime>,
    generation: u64,
}

struct ServerState {
    rng: Rng,
    routing_rng: Rng,
    next_output: usize,
    /// Round-robin position among the inputs.
    next_input: usize,
    /// A `Pull` event is scheduled.
    pull_pending: bool,
    slots: Vec<Slot>,
    slot_generation: Vec<u64>,
    /// Slots holding finished parts, in the order they finished.
    blocked_order: VecDeque<usize>,
    /// Per slot: the rest of a split part, leaving after the blocked one.
    split_rest: Vec<VecDeque<Part>>,
    /// Per slot: the part type it's set up for (the last it started).
    set_up_for: Vec<Option<usize>>,
    changeover_rng: Rng,
    changeovers: u64,
    /// Seconds of setup started since the warm-up.
    setup_time: f64,
    /// Draws how many parts a split makes.
    split_rng: Rng,
    /// Waiting in the resource queue: a part is ready, units aren't.
    waiting_for_resources: bool,
    /// The batch or kit being collected.
    staging: Vec<Part>,
    waiting: TimeAverage,
    failures: Vec<FailureState>,
    /// The failure the server is currently down for.
    down_for: Option<usize>,
    off_shift: bool,
    /// The shift is over, but it's finishing the work in hand.
    closing: bool,
    overtime: TimeAverage,
    off: TimeAverage,
    processed: u64,
    failure_count: u64,
    busy: TimeAverage,
    blocked: TimeAverage,
    failed: TimeAverage,
}

impl ServerState {
    /// Working: not failed and on shift.
    fn is_up(&self) -> bool {
        self.down_for.is_none() && !self.off_shift
    }

    fn idle_slot(&self) -> Option<usize> {
        self.slots.iter().position(|s| matches!(s, Slot::Idle))
    }

    fn any_busy(&self) -> bool {
        self.slots.iter().any(|s| matches!(s, Slot::Busy { .. }))
    }
}

#[derive(Debug, Clone, Copy)]
struct OnBelt {
    part: Part,
    /// Position (m from the entrance) at `since`, assuming it has moved
    /// freely since then. The real position is capped by the end and by
    /// the part ahead; see `ConveyorState::positions`.
    pos: f64,
    since: SimTime,
    /// When it got on.
    entered: SimTime,
}

struct ConveyorState {
    /// Front is the head (closest to the end).
    belt: VecDeque<OnBelt>,
    /// Bumped whenever the head changes, to invalidate `ReachEnd` events.
    generation: u64,
    /// The object feeding this conveyor is waiting for room.
    upstream_waiting: bool,
    /// When the current head reached the end (for `first_come` merging).
    head_arrived: Option<SimTime>,
    entered: u64,
    /// Time from getting on to leaving.
    time: Tally,
    time_by_type: Vec<Tally>,
    /// Queues only.
    times: Option<Histogram>,
    within_target: u64,
    /// Parts that left over the whole run (not reset after the warm-up).
    departed_total: u64,
    /// Transports: parts waiting for a vehicle, with when they got on and
    /// whether a vehicle is on its way for them; and parts being carried.
    pickup: VecDeque<(Part, SimTime, bool)>,
    in_transit: usize,
    pickup_wait: Tally,
    parts: TimeAverage,
    full: TimeAverage,
    empty: TimeAverage,
}

impl ConveyorState {
    /// Real positions at `now`, head first: each part is capped by the end
    /// (head) or by the part ahead minus one spacing. Lazy and allocation-free:
    /// this runs several times per event, so `.next()` for the head stops early.
    fn positions(&self, c: &Conveyor, now: SimTime) -> impl Iterator<Item = f64> + '_ {
        let (speed, spacing) = (c.speed, c.spacing);
        let mut limit = c.length;
        self.belt.iter().map(move |p| {
            let free = p.pos + speed * (now - p.since).as_secs_f64();
            let pos = free.min(limit);
            limit = pos - spacing;
            pos
        })
    }

    /// Parts on it: on the belt, and for a transport, waiting or on their way.
    fn occupancy(&self) -> usize {
        self.belt.len() + self.pickup.len() + self.in_transit
    }

    fn can_enter(&self, c: &Conveyor, now: SimTime) -> bool {
        if self.occupancy() >= c.capacity as usize {
            return false;
        }
        match self.positions(c, now).last() {
            None => true,
            Some(last) => last >= c.spacing - EPS,
        }
    }

    fn head_at_end(&self, c: &Conveyor, now: SimTime) -> bool {
        self.positions(c, now)
            .next()
            .is_some_and(|head| head >= c.length - EPS)
    }
}

/// The earliest time at which something moving at `speed` has covered `meters`.
fn after(now: SimTime, meters: f64, speed: f64) -> SimTime {
    let micros = (meters.max(0.0) / speed * 1e6).ceil();
    now + SimTime::from_micros(micros as i64)
}

fn duration(seconds: f64) -> SimTime {
    SimTime::from_secs_f64(seconds.max(0.0))
}

/// Picks the conveyor a part ready to leave will take (see the module docs).
fn pick_output(
    routing: &Routing,
    outputs: &[usize],
    entity: usize,
    rng: &mut Rng,
    next: &mut usize,
) -> Target {
    match routing {
        Routing::Single => Some(outputs[0]),
        Routing::ByType(routes) => {
            Some(routes[entity].expect("validated: every type that gets here has a route"))
        }
        Routing::Probability(weights) => {
            let u = 1.0 - rng.open01(); // [0, 1)
            let mut cumulative = 0.0;
            for (&c, w) in outputs.iter().zip(weights) {
                cumulative += w;
                if u < cumulative {
                    return Some(c);
                }
            }
            outputs.last().copied() // rounding: weights sum to 1 within float error
        }
        Routing::RoundRobin => {
            let c = outputs[*next % outputs.len()];
            *next = (*next + 1) % outputs.len();
            Some(c)
        }
        Routing::FirstAvailable | Routing::ShortestQueue => None,
    }
}

/// The share of time something with this much time off shift could work,
/// to turn shares of all time into shares of working time. Never zero, so
/// something always off shift reports 0 rather than NaN.
fn working_share(off_shift: f64) -> f64 {
    (1.0 - off_shift).max(1e-12)
}

/// Runs one replication. `replication` (1-based) selects the random streams.
pub fn simulate<O: Observer>(spec: &Spec, replication: u32, observer: &mut O) -> ReplicationResult {
    let mut sim = Sim::new(spec, replication, observer);
    sim.run();
    sim.result()
}

/// What a vehicle is doing.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Idle,
    /// Travelling empty to pick a part up.
    ToPickup,
    Loading,
    Carrying,
    Unloading,
}

struct VehicleState {
    /// Where it is, or where it's going.
    at: Node,
    phase: Phase,
    /// The transport and the part it's fetching or carrying, with when the
    /// part got on the transport.
    job: Option<(usize, Part, SimTime)>,
}

struct FleetState {
    vehicles: Vec<VehicleState>,
    handling_rng: Rng,
    trips: u64,
    busy: TimeAverage,
    empty: TimeAverage,
}

struct ResourceState {
    in_use: u32,
    off_shift: bool,
    /// The shift is over, but units still go to work in hand.
    closing: bool,
    overtime: TimeAverage,
    busy: TimeAverage,
    off: TimeAverage,
}

struct Sim<'a, O: Observer> {
    spec: &'a Spec,
    replication: u32,
    observer: &'a mut O,
    now: SimTime,
    end: SimTime,
    events: EventList,
    next_part: PartId,
    sources: Vec<SourceState>,
    servers: Vec<ServerState>,
    conveyors: Vec<ConveyorState>,
    resources: Vec<ResourceState>,
    /// Servers waiting for resource units, in the order they started waiting.
    resource_queue: VecDeque<usize>,
    /// Per conveyor: restocking sources that check after each part leaves it.
    restockers: Vec<Vec<usize>>,
    /// Some server or resource is finishing work after its shift.
    closing: bool,
    fleets: Vec<FleetState>,
    sink_exited: Vec<u64>,
    sink_times: Vec<Tally>,
    /// Per sink, per part type: parts that exited and their times in system.
    sink_by_type: Vec<Vec<(u64, Tally)>>,
    totals: Totals,
}

impl<'a, O: Observer> Sim<'a, O> {
    fn new(spec: &'a Spec, replication: u32, observer: &'a mut O) -> Self {
        let seed = spec.experiment.seed;
        let stream = |name: String| Rng::stream(seed, replication, &name);
        let sources = spec
            .sources
            .iter()
            .map(|s| SourceState {
                rng: stream(format!("source/{}/interarrival", s.name)),
                routing_rng: stream(format!("source/{}/routing", s.name)),
                type_rng: stream(format!("source/{}/type", s.name)),
                thinning_rng: stream(format!("source/{}/thinning", s.name)),
                group_rng: stream(format!("source/{}/group", s.name)),
                next_output: 0,
                held: VecDeque::new(),
                off_shift: false,
                arrival_generation: 0,
                created: 0,
                created_total: 0,
                lost: 0,
                blocked: TimeAverage::default(),
                on_order: 0,
                on_order_avg: TimeAverage::default(),
                orders: 0,
                lead_rng: stream(format!("source/{}/lead_time", s.name)),
                pending: VecDeque::new(),
            })
            .collect();
        let servers = spec
            .servers
            .iter()
            .map(|s| {
                let failures = s
                    .failures
                    .iter()
                    .map(|f| {
                        let mut uptime_rng =
                            stream(format!("server/{}/failure/{}/uptime", s.name, f.name));
                        let remaining = duration(uptime_rng.sample(&f.uptime));
                        FailureState {
                            uptime_rng,
                            downtime_rng: stream(format!(
                                "server/{}/failure/{}/downtime",
                                s.name, f.name
                            )),
                            remaining,
                            running_since: None,
                            generation: 0,
                        }
                    })
                    .collect();
                ServerState {
                    rng: stream(format!("server/{}/process", s.name)),
                    routing_rng: stream(format!("server/{}/routing", s.name)),
                    next_output: 0,
                    next_input: 0,
                    pull_pending: false,
                    slots: vec![Slot::Idle; s.capacity as usize],
                    slot_generation: vec![0; s.capacity as usize],
                    blocked_order: VecDeque::new(),
                    split_rest: vec![VecDeque::new(); s.capacity as usize],
                    set_up_for: vec![None; s.capacity as usize],
                    changeover_rng: stream(format!("server/{}/changeover", s.name)),
                    changeovers: 0,
                    setup_time: 0.0,
                    split_rng: stream(format!("server/{}/split", s.name)),
                    waiting_for_resources: false,
                    staging: Vec::new(),
                    waiting: TimeAverage::default(),
                    failures,
                    down_for: None,
                    off_shift: false,
                    closing: false,
                    overtime: TimeAverage::default(),
                    off: TimeAverage::default(),
                    processed: 0,
                    failure_count: 0,
                    busy: TimeAverage::default(),
                    blocked: TimeAverage::default(),
                    failed: TimeAverage::default(),
                }
            })
            .collect();
        let conveyors = spec
            .conveyors
            .iter()
            .map(|c| ConveyorState {
                belt: VecDeque::new(),
                generation: 0,
                upstream_waiting: false,
                head_arrived: None,
                entered: 0,
                time: Tally::default(),
                time_by_type: vec![Tally::default(); spec.entities.len()],
                times: c.is_queue().then(Histogram::default),
                within_target: 0,
                departed_total: 0,
                pickup: VecDeque::new(),
                in_transit: 0,
                pickup_wait: Tally::default(),
                parts: TimeAverage::default(),
                full: TimeAverage::default(),
                empty: TimeAverage::default(),
            })
            .collect();
        let experiment = &spec.experiment;
        Sim {
            spec,
            replication,
            observer,
            now: SimTime::ZERO,
            end: experiment.warmup + experiment.run_length,
            events: EventList::default(),
            next_part: 0,
            sources,
            servers,
            conveyors,
            resources: spec
                .resources
                .iter()
                .map(|_| ResourceState {
                    in_use: 0,
                    off_shift: false,
                    closing: false,
                    overtime: TimeAverage::default(),
                    busy: TimeAverage::default(),
                    off: TimeAverage::default(),
                })
                .collect(),
            resource_queue: VecDeque::new(),
            closing: false,
            fleets: spec
                .vehicles
                .iter()
                .map(|f| FleetState {
                    vehicles: (0..f.count)
                        .map(|_| VehicleState {
                            at: f.home,
                            phase: Phase::Idle,
                            job: None,
                        })
                        .collect(),
                    handling_rng: stream(format!("vehicles/{}/handling", f.name)),
                    trips: 0,
                    busy: TimeAverage::default(),
                    empty: TimeAverage::default(),
                })
                .collect(),
            restockers: (0..spec.conveyors.len())
                .map(|c| {
                    (0..spec.sources.len())
                        .filter(|&s| {
                            matches!(&spec.sources[s].arrivals,
                                Arrivals::Restock(r) if r.queue == c && r.review_every.is_none())
                        })
                        .collect()
                })
                .collect(),
            sink_exited: vec![0; spec.sinks.len()],
            sink_times: vec![Tally::default(); spec.sinks.len()],
            sink_by_type: vec![vec![(0, Tally::default()); spec.entities.len()]; spec.sinks.len()],
            totals: Totals::default(),
        }
    }

    fn run(&mut self) {
        self.events.schedule(self.end, Event::RunEnd);
        if self.spec.experiment.warmup > SimTime::ZERO {
            self.events
                .schedule(self.spec.experiment.warmup, Event::WarmupEnd);
        }
        // Who is off shift at time 0, and when each schedule next changes.
        for (i, schedule) in self.spec.schedules.iter().enumerate() {
            let on = schedule.is_on(SimTime::ZERO);
            for (s, spec) in self.spec.sources.iter().enumerate() {
                if spec.schedule == Some(i) {
                    self.sources[s].off_shift = !on;
                }
            }
            for (s, spec) in self.spec.servers.iter().enumerate() {
                if spec.schedule == Some(i) {
                    self.servers[s].off_shift = !on;
                }
            }
            for (r, spec) in self.spec.resources.iter().enumerate() {
                if spec.schedule == Some(i) {
                    self.resources[r].off_shift = !on;
                }
            }
            if let Some(at) = schedule.next_change(SimTime::ZERO) {
                self.events
                    .schedule(at, Event::ScheduleChange { schedule: i });
            }
        }
        self.stock_queues();
        for source in 0..self.spec.sources.len() {
            if let Arrivals::Restock(r) = &self.spec.sources[source].arrivals {
                // Restocking sources check their stock at once, and order
                // whatever the time; deliveries wait for a shift.
                match r.review_every {
                    Some(_) => self
                        .events
                        .schedule(SimTime::ZERO, Event::Review { source }),
                    None => self.review(source),
                }
            } else if !self.sources[source].off_shift {
                self.first_arrival(source);
            }
        }
        for server in 0..self.spec.servers.len() {
            self.update_failure_clocks(server);
        }

        while let Some((time, event)) = self.events.pop() {
            debug_assert!(time >= self.now);
            self.now = time;
            match event {
                Event::RunEnd => break,
                Event::WarmupEnd => self.reset_statistics(),
                Event::Arrival { source, generation } => {
                    if self.sources[source].arrival_generation == generation {
                        self.arrival(source);
                    }
                }
                Event::ScheduleChange { schedule } => self.schedule_change(schedule),
                Event::ReachEnd {
                    conveyor,
                    generation,
                } => {
                    if self.conveyors[conveyor].generation == generation {
                        let head = self.conveyors[conveyor].belt[0].part.id;
                        self.observer.record(
                            self.now,
                            TraceEvent::ReachedEnd {
                                part: head,
                                conveyor,
                            },
                        );
                        self.conveyors[conveyor].head_arrived = Some(self.now);
                        self.head_at_end(conveyor);
                    }
                }
                Event::EntryCheck { conveyor } => self.entry_check(conveyor),
                Event::ProcessDone {
                    server,
                    slot,
                    generation,
                } => {
                    if self.servers[server].slot_generation[slot] == generation {
                        self.process_done(server, slot);
                    }
                }
                Event::Failure {
                    server,
                    mode,
                    generation,
                } => {
                    if self.servers[server].failures[mode].generation == generation {
                        self.fail(server, mode);
                    }
                }
                Event::Repair { server, mode } => self.repair(server, mode),
                Event::Pull { server } => self.pull(server),
                Event::Delivery { source, parts } => self.delivery(source, parts),
                Event::Vehicle { fleet, vehicle } => self.vehicle_step(fleet, vehicle),
                Event::Review { source } => {
                    self.review(source);
                    if let Arrivals::Restock(Restock {
                        review_every: Some(every),
                        ..
                    }) = &self.spec.sources[source].arrivals
                    {
                        self.events
                            .schedule(self.now + *every, Event::Review { source });
                    }
                }
            }
            if self.closing {
                self.finish_overtime();
                self.closing = self.servers.iter().any(|s| s.closing)
                    || self.resources.iter().any(|r| r.closing);
            }
            self.sample_state();
        }
        self.sample_state();
    }

    // --- Sources ---

    fn arrival(&mut self, source: usize) {
        let n = match &self.spec.sources[source].group {
            None => 1,
            Some(count) => self.sources[source].group_rng.count(count),
        };
        self.deliver(source, n);
    }

    /// `n` new parts at a source get on in order; once one has to wait, the
    /// rest wait behind it (or, with `drop`, each that can't is lost). The
    /// next arrival time starts when the last has got on.
    fn deliver(&mut self, source: usize, n: u32) {
        let spec = self.spec;
        let source_spec = &spec.sources[source];
        for _ in 0..n {
            let part = self.create_part(source);
            let state = &mut self.sources[source];
            let target = pick_output(
                &source_spec.routing,
                &source_spec.outputs,
                part.entity,
                &mut state.routing_rng,
                &mut state.next_output,
            );
            if self.sources[source].held.is_empty()
                && self.try_send(part, target, &source_spec.outputs, &source_spec.routing)
            {
                continue;
            }
            match source_spec.when_blocked {
                WhenBlocked::Wait => self.sources[source].held.push_back((part, target)),
                WhenBlocked::Drop => {
                    self.sources[source].lost += 1;
                    self.totals.lost += 1;
                    self.observer.record(
                        self.now,
                        TraceEvent::Lost {
                            part: part.id,
                            source,
                        },
                    );
                }
            }
        }
        match self.sources[source].held.front() {
            None => self.schedule_arrival(source),
            Some(&(_, target)) => self.wait_for(target, &source_spec.outputs),
        }
    }

    // --- Restocking ---

    /// Parts a queue starts with, at time 0.
    fn stock_queues(&mut self) {
        for conveyor in 0..self.spec.conveyors.len() {
            let Some((entity, n)) = self.spec.conveyors[conveyor].starts_with else {
                continue;
            };
            for _ in 0..n {
                let part = Part {
                    id: self.next_part,
                    created: self.now,
                    entity,
                };
                self.next_part += 1;
                self.totals.created += 1;
                self.observer.record(
                    self.now,
                    TraceEvent::Stocked {
                        part: part.id,
                        conveyor,
                        entity,
                    },
                );
                self.conveyors[conveyor].belt.push_back(OnBelt {
                    part,
                    pos: 0.0,
                    since: self.now,
                    entered: self.now,
                });
                self.conveyors[conveyor].entered += 1;
            }
            if n > 0 {
                self.schedule_reach_end(conveyor);
            }
        }
    }

    /// A restocking source checks its stock position and orders if it's at
    /// or below the reorder point.
    fn review(&mut self, source: usize) {
        let Arrivals::Restock(r) = &self.spec.sources[source].arrivals else {
            return;
        };
        let q = r.queue;
        let initial = self.spec.conveyors[q]
            .starts_with
            .map_or(0, |(_, n)| u64::from(n));
        let s = &self.sources[source];
        let mut position = (initial + s.created_total + s.on_order) as i64
            - self.conveyors[q].departed_total as i64;
        let reorder_point = i64::from(r.reorder_point);
        while position <= reorder_point {
            let quantity = match r.quantity {
                OrderQuantity::Fixed(q) => i64::from(q),
                OrderQuantity::UpTo(level) => i64::from(level) - position,
            };
            if quantity <= 0 {
                return; // validated: up-to levels are above the reorder point
            }
            let s = &mut self.sources[source];
            s.on_order += quantity as u64;
            s.orders += 1;
            let lead = duration(s.lead_rng.sample(&r.lead_time));
            self.events.schedule(
                self.now + lead,
                Event::Delivery {
                    source,
                    parts: quantity as u32,
                },
            );
            position += quantity;
        }
    }

    /// An order arrives: delivered now, or at the next shift if off shift.
    fn delivery(&mut self, source: usize, parts: u32) {
        let s = &mut self.sources[source];
        s.on_order -= u64::from(parts);
        if s.off_shift {
            s.pending.push_back(parts);
        } else {
            self.deliver(source, parts);
        }
    }

    /// A new part at a source, its type drawn from the source's mix.
    fn create_part(&mut self, source: usize) -> Part {
        let creates = &self.spec.sources[source].creates;
        let entity = if creates.len() == 1 {
            creates[0].0
        } else {
            // Drawn from the source's own stream, so adding a type elsewhere
            // doesn't change these draws.
            let u = 1.0 - self.sources[source].type_rng.open01(); // [0, 1)
            let mut cumulative = 0.0;
            creates
                .iter()
                .find(|(_, share)| {
                    cumulative += share;
                    u < cumulative
                })
                .unwrap_or(creates.last().expect("validated: at least one type"))
                .0
        };
        let part = Part {
            id: self.next_part,
            created: self.now,
            entity,
        };
        self.next_part += 1;
        self.sources[source].created += 1;
        self.sources[source].created_total += 1;
        self.totals.created += 1;
        self.observer.record(
            self.now,
            TraceEvent::Created {
                part: part.id,
                source,
                entity,
            },
        );
        part
    }

    /// The first arrival of the run or of a shift: at once with a time
    /// between arrivals, drawn from the rate with a profile.
    fn first_arrival(&mut self, source: usize) {
        match self.spec.sources[source].arrivals {
            Arrivals::Interarrival(_) => {
                let generation = self.sources[source].arrival_generation;
                self.events
                    .schedule(self.now, Event::Arrival { source, generation });
            }
            Arrivals::Profile(_) => self.schedule_arrival(source),
            // Deliveries that waited for the shift come now.
            Arrivals::Restock(_) => {
                while let Some(parts) = self.sources[source].pending.pop_front() {
                    self.deliver(source, parts);
                }
            }
        }
    }

    fn schedule_arrival(&mut self, source: usize) {
        if self.sources[source].off_shift {
            return; // the next shift's start brings the next arrival
        }
        let at = match &self.spec.sources[source].arrivals {
            Arrivals::Restock(_) => return, // orders bring parts, not a clock
            Arrivals::Interarrival(d) => self.now + duration(self.sources[source].rng.sample(d)),
            Arrivals::Profile(periods) => {
                // Thinning: candidates at the highest rate, each kept with
                // probability rate(t) / highest rate.
                let max = periods.iter().map(|p| p.2).fold(0.0, f64::max);
                if max <= 0.0 {
                    return; // never any arrivals
                }
                let candidate = Distribution::Exponential { mean: 1.0 / max };
                let s = &mut self.sources[source];
                let mut t = self.now;
                loop {
                    t += duration(s.rng.sample(&candidate));
                    if t >= self.end {
                        return;
                    }
                    let u = 1.0 - s.thinning_rng.open01(); // [0, 1)
                    if u * max < Arrivals::rate_at(periods, t) {
                        break t;
                    }
                }
            }
        };
        let generation = self.sources[source].arrival_generation;
        self.events
            .schedule(at, Event::Arrival { source, generation });
    }

    // --- Schedules ---

    fn schedule_change(&mut self, schedule: usize) {
        let spec = self.spec;
        let on = spec.schedules[schedule].is_on(self.now);
        if let Some(at) = spec.schedules[schedule].next_change(self.now) {
            self.events.schedule(at, Event::ScheduleChange { schedule });
        }
        for source in 0..spec.sources.len() {
            if spec.sources[source].schedule != Some(schedule) {
                continue;
            }
            let s = &mut self.sources[source];
            s.off_shift = !on;
            if on {
                // The first arrival of the shift comes at its start (or, with
                // a rate profile, when the rate brings it); waiting
                // deliveries come now.
                if s.held.is_empty()
                    || matches!(spec.sources[source].arrivals, Arrivals::Restock(_))
                {
                    self.first_arrival(source);
                }
            } else {
                s.arrival_generation += 1;
            }
        }
        for server in 0..spec.servers.len() {
            if spec.servers[server].schedule != Some(schedule) {
                continue;
            }
            if on {
                if self.servers[server].closing {
                    // Still finishing the last shift's work: it never stopped.
                    self.servers[server].closing = false;
                    continue;
                }
                self.observer
                    .record(self.now, TraceEvent::ShiftStarted { server });
                self.servers[server].off_shift = false;
                if self.servers[server].down_for.is_none() {
                    self.resume_slots(server);
                }
                self.update_failure_clocks(server);
                self.release_blocked(server);
            } else if spec.schedules[schedule].finish_work {
                self.servers[server].closing = true; // stops when the work in hand is done
                self.closing = true;
            } else {
                self.stop_server(server);
            }
        }
        let mut freed = false;
        for r in 0..spec.resources.len() {
            if spec.resources[r].schedule == Some(schedule) {
                let res = &mut self.resources[r];
                if on {
                    freed |= res.off_shift;
                    res.off_shift = false;
                    res.closing = false;
                } else if spec.schedules[schedule].finish_work {
                    res.closing = true;
                    self.closing = true;
                } else {
                    res.off_shift = true;
                }
            }
        }
        if freed {
            self.serve_resource_queue();
        }
    }

    /// A server's shift ends: what it's processing pauses until the next one.
    fn stop_server(&mut self, server: usize) {
        self.observer
            .record(self.now, TraceEvent::ShiftEnded { server });
        self.suspend_slots(server, OnFailure::Resume);
        let s = &mut self.servers[server];
        s.off_shift = true;
        s.closing = false;
        self.update_failure_clocks(server);
    }

    /// Servers and resources finishing their work after a shift stop once
    /// there's none left for them. Called after every event.
    fn finish_overtime(&mut self) {
        for server in 0..self.servers.len() {
            if self.servers[server].closing && !self.has_work(server) {
                self.stop_server(server);
            }
        }
        for r in 0..self.resources.len() {
            if self.resources[r].closing && self.resources[r].in_use == 0 {
                let needed = (0..self.spec.servers.len()).any(|s| {
                    self.spec.servers[s].needs.iter().any(|&(x, _)| x == r) && self.has_work(s)
                });
                if !needed {
                    let res = &mut self.resources[r];
                    res.closing = false;
                    res.off_shift = true;
                }
            }
        }
    }

    /// Whether a server has work in hand: parts in it, or parts that can
    /// still come to it (on its way in, in an upstream object that's still
    /// working, or from a source still on shift).
    fn has_work(&self, server: usize) -> bool {
        let busy = |s: &ServerState| {
            !s.staging.is_empty() || s.slots.iter().any(|x| !matches!(x, Slot::Idle))
        };
        if busy(&self.servers[server]) {
            return true;
        }
        let mut seen = vec![server];
        let mut stack = vec![server];
        while let Some(s) = stack.pop() {
            for &c in &self.spec.servers[s].inputs {
                if !self.conveyors[c].belt.is_empty() {
                    return true;
                }
                match self.spec.conveyors[c].from {
                    Node::Source(src) => {
                        let source = &self.sources[src];
                        if !source.off_shift || !source.held.is_empty() {
                            return true;
                        }
                    }
                    Node::Server(up) if !seen.contains(&up) => {
                        let state = &self.servers[up];
                        if !state.off_shift && busy(state) {
                            return true;
                        }
                        seen.push(up);
                        stack.push(up);
                    }
                    _ => {}
                }
            }
        }
        false
    }

    /// Pauses every part being processed (a failure, or the end of a shift).
    fn suspend_slots(&mut self, server: usize, how: OnFailure) {
        let now = self.now;
        let s = &mut self.servers[server];
        for (slot, generation) in s.slots.iter_mut().zip(&mut s.slot_generation) {
            if let Slot::Busy {
                part,
                finish,
                total,
                holds,
            } = *slot
            {
                let remaining = match how {
                    OnFailure::Resume => finish - now,
                    OnFailure::Restart => total,
                };
                *slot = Slot::Suspended {
                    part,
                    remaining,
                    total,
                    holds,
                };
                *generation += 1; // cancel the scheduled ProcessDone
            }
        }
    }

    /// Continues every paused part (after a repair, or at a shift's start).
    fn resume_slots(&mut self, server: usize) {
        for slot in 0..self.servers[server].slots.len() {
            if let Slot::Suspended {
                part,
                remaining,
                total,
                holds,
            } = self.servers[server].slots[slot]
            {
                let finish = self.now + remaining;
                self.servers[server].slots[slot] = Slot::Busy {
                    part,
                    finish,
                    total,
                    holds,
                };
                self.schedule_process_done(server, slot, finish);
            }
        }
    }

    /// A conveyor the source was waiting for has room: held parts get on in
    /// order, and the next interarrival time starts when the last is on.
    fn release_held(&mut self, source: usize) {
        if self.sources[source].held.is_empty() {
            return; // already left by another conveyor (`first_available`)
        }
        let spec = &self.spec.sources[source];
        let outputs = &spec.outputs;
        while let Some(&(part, target)) = self.sources[source].held.front() {
            if !self.try_send(part, target, outputs, &spec.routing) {
                self.wait_for(target, outputs);
                return;
            }
            self.sources[source].held.pop_front();
        }
        self.schedule_arrival(source);
    }

    /// Puts a part on its target conveyor or, with no target, on the
    /// preferred output with room (see the module docs).
    fn try_send(
        &mut self,
        part: Part,
        target: Target,
        outputs: &[usize],
        routing: &Routing,
    ) -> bool {
        match target {
            Some(conveyor) => self.try_enter(conveyor, part),
            None if *routing == Routing::ShortestQueue => {
                // Among the outputs with room, the one holding the fewest
                // parts; `min_by_key` keeps the first of equal keys, so ties
                // go to the output listed first.
                let now = self.now;
                let chosen = outputs
                    .iter()
                    .copied()
                    .filter(|&c| self.conveyors[c].can_enter(&self.spec.conveyors[c], now))
                    .min_by_key(|&c| self.conveyors[c].belt.len());
                chosen.is_some_and(|c| self.try_enter(c, part))
            }
            None => outputs.iter().any(|&c| self.try_enter(c, part)),
        }
    }

    fn wait_for(&mut self, target: Target, outputs: &[usize]) {
        match target {
            Some(conveyor) => self.wait_for_room(conveyor),
            None => {
                for &c in outputs {
                    self.wait_for_room(c);
                }
            }
        }
    }

    // --- Conveyors ---

    fn try_enter(&mut self, conveyor: usize, part: Part) -> bool {
        let c = &self.spec.conveyors[conveyor];
        let state = &mut self.conveyors[conveyor];
        if !state.can_enter(c, self.now) {
            return false;
        }
        if let Some((fleet, _)) = c.transport_by() {
            // Waits where it is for a vehicle.
            state.pickup.push_back((part, self.now, false));
            state.entered += 1;
            self.observer.record(
                self.now,
                TraceEvent::EnteredConveyor {
                    part: part.id,
                    conveyor,
                },
            );
            self.dispatch(fleet);
            return true;
        }
        state.belt.push_back(OnBelt {
            part,
            pos: 0.0,
            since: self.now,
            entered: self.now,
        });
        state.entered += 1;
        self.observer.record(
            self.now,
            TraceEvent::EnteredConveyor {
                part: part.id,
                conveyor,
            },
        );
        if self.conveyors[conveyor].belt.len() == 1 {
            self.schedule_reach_end(conveyor);
        }
        true
    }

    fn schedule_reach_end(&mut self, conveyor: usize) {
        let c = &self.spec.conveyors[conveyor];
        let state = &mut self.conveyors[conveyor];
        state.generation += 1;
        state.head_arrived = None;
        let Some(head) = state.positions(c, self.now).next() else {
            return;
        };
        let at = after(self.now, c.length - head, c.speed);
        self.events.schedule(
            at,
            Event::ReachEnd {
                conveyor,
                generation: state.generation,
            },
        );
    }

    /// The head part reached the end: sinks take it at once; servers decide.
    fn head_at_end(&mut self, conveyor: usize) {
        match self.spec.conveyors[conveyor].to {
            Node::Sink(sink) => {
                let part = self.take_head(conveyor);
                self.exit(sink, part);
                self.after_departure(conveyor);
            }
            Node::Server(server) => self.request_pull(server),
            Node::Source(_) => unreachable!("validated: conveyors don't end at sources"),
        }
    }

    /// Removes the head part, which must be at the end.
    fn take_head(&mut self, conveyor: usize) -> Part {
        let c = &self.spec.conveyors[conveyor];
        debug_assert!(self.conveyors[conveyor].head_at_end(c, self.now));
        // Everyone stopped behind the head starts moving again: record
        // real positions as the new free-movement baseline. Same arithmetic
        // as `positions`, done in place.
        let now = self.now;
        let state = &mut self.conveyors[conveyor];
        let mut limit = c.length;
        for p in state.belt.iter_mut() {
            let free = p.pos + c.speed * (now - p.since).as_secs_f64();
            let pos = free.min(limit);
            limit = pos - c.spacing;
            p.pos = pos;
            p.since = now;
        }
        let head = state.belt.pop_front().expect("head exists");
        state.departed_total += 1;
        let time = (now - head.entered).as_secs_f64();
        state.time.add(time);
        state.time_by_type[head.part.entity].add(time);
        if let Some(times) = &mut state.times {
            times.add(time);
        }
        if c.target_wait.is_some_and(|target| time <= target + 1e-9) {
            state.within_target += 1;
        }
        self.schedule_reach_end(conveyor);
        // Restocking sources watching this queue check their stock.
        for i in 0..self.restockers[conveyor].len() {
            self.review(self.restockers[conveyor][i]);
        }
        head.part
    }

    /// A part left the conveyor: the object feeding it may now have room.
    fn after_departure(&mut self, conveyor: usize) {
        if self.conveyors[conveyor].upstream_waiting {
            self.wait_for_room(conveyor);
        }
    }

    /// The object feeding `conveyor` has a part that can't get on. Notify it
    /// now if there's room, or arrange to check again when there may be.
    fn wait_for_room(&mut self, conveyor: usize) {
        let c = &self.spec.conveyors[conveyor];
        let state = &mut self.conveyors[conveyor];
        state.upstream_waiting = true;
        if state.can_enter(c, self.now) {
            self.notify_upstream(conveyor);
            return;
        }
        if state.occupancy() >= c.capacity as usize {
            return; // the next departure calls back
        }
        // Room depends on the last part moving one spacing from the entrance.
        let last = *state.belt.back().expect("not empty, or we could enter");
        let real = state.positions(c, self.now).last().expect("not empty");
        let free = last.pos + c.speed * (self.now - last.since).as_secs_f64();
        if real >= free - EPS {
            // Moving: check when it should be one spacing in. It may get
            // stopped before that; the check then waits for a departure.
            let at = after(self.now, c.spacing - real, c.speed);
            self.events.schedule(at, Event::EntryCheck { conveyor });
        }
        // Otherwise it's stopped behind a queue; the next departure calls back.
    }

    fn entry_check(&mut self, conveyor: usize) {
        if self.conveyors[conveyor].upstream_waiting {
            self.wait_for_room(conveyor);
        }
    }

    fn notify_upstream(&mut self, conveyor: usize) {
        self.conveyors[conveyor].upstream_waiting = false;
        match self.spec.conveyors[conveyor].from {
            Node::Source(source) => self.release_held(source),
            Node::Server(server) => self.release_blocked(server),
            Node::Sink(_) => unreachable!("validated: conveyors don't start at sinks"),
        }
    }

    // --- Vehicles ---

    /// How far apart two objects are for a vehicle: along a transport
    /// between them (either way), or `fallback` if there's none.
    fn distance(&self, a: Node, b: Node, fallback: f64) -> f64 {
        if a == b {
            return 0.0;
        }
        self.spec
            .conveyors
            .iter()
            .find_map(|c| {
                let (_, d) = c.transport_by()?;
                ((c.from == a && c.to == b) || (c.from == b && c.to == a)).then_some(d)
            })
            .unwrap_or(fallback)
    }

    /// Sends idle vehicles to the parts that have waited longest, each to
    /// the nearest idle vehicle.
    fn dispatch(&mut self, fleet: usize) {
        let spec = self.spec;
        // Parts no vehicle is coming for yet, oldest first.
        let mut pending: Vec<(SimTime, usize, usize)> = Vec::new();
        for (i, c) in spec.conveyors.iter().enumerate() {
            if c.transport_by().is_some_and(|(f, _)| f == fleet) {
                for (k, &(_, since, assigned)) in self.conveyors[i].pickup.iter().enumerate() {
                    if !assigned {
                        pending.push((since, i, k));
                    }
                }
            }
        }
        pending.sort_by_key(|&(since, i, k)| (since, i, k));
        for (_, conveyor, k) in pending {
            let c = &spec.conveyors[conveyor];
            let (_, distance) = c.transport_by().expect("a transport");
            let nearest = self.fleets[fleet]
                .vehicles
                .iter()
                .enumerate()
                .filter(|(_, v)| v.phase == Phase::Idle)
                .map(|(i, v)| (i, self.distance(v.at, c.from, distance)))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((vehicle, empty)) = nearest else {
                return; // all busy: the next one free comes back here
            };
            let (part, since, _) = self.conveyors[conveyor].pickup[k];
            self.conveyors[conveyor].pickup[k].2 = true;
            let speed = spec.vehicles[fleet].speed;
            let until = self.now + duration(empty / speed);
            let v = &mut self.fleets[fleet].vehicles[vehicle];
            let from = v.at;
            v.phase = Phase::ToPickup;
            v.at = c.from;
            v.job = Some((conveyor, part, since));
            self.observer.record(
                self.now,
                TraceEvent::VehicleTrip {
                    fleet,
                    vehicle,
                    from,
                    to: c.from,
                    until,
                    part: None,
                },
            );
            self.events
                .schedule(until, Event::Vehicle { fleet, vehicle });
        }
    }

    /// A vehicle finished a step: arrived to pick up, loaded, arrived to
    /// deliver, or unloaded.
    fn vehicle_step(&mut self, fleet: usize, vehicle: usize) {
        let spec = self.spec;
        let f = &spec.vehicles[fleet];
        let v = &self.fleets[fleet].vehicles[vehicle];
        let (conveyor, part, since) = v.job.expect("a busy vehicle has a job");
        let c = &spec.conveyors[conveyor];
        let (_, distance) = c.transport_by().expect("a transport");
        match v.phase {
            Phase::ToPickup => {
                // The part leaves the pickup for the vehicle.
                let state = &mut self.conveyors[conveyor];
                let k = state
                    .pickup
                    .iter()
                    .position(|(p, _, _)| p.id == part.id)
                    .expect("the part waits for this vehicle");
                state.pickup.remove(k);
                state.in_transit += 1;
                state.pickup_wait.add((self.now - since).as_secs_f64());
                let load = duration(self.fleets[fleet].handling_rng.sample(&f.load_time));
                self.fleets[fleet].vehicles[vehicle].phase = Phase::Loading;
                self.events
                    .schedule(self.now + load, Event::Vehicle { fleet, vehicle });
            }
            Phase::Loading => {
                let until = self.now + duration(distance / f.speed);
                let v = &mut self.fleets[fleet].vehicles[vehicle];
                v.phase = Phase::Carrying;
                v.at = c.to;
                self.observer.record(
                    self.now,
                    TraceEvent::VehicleTrip {
                        fleet,
                        vehicle,
                        from: c.from,
                        to: c.to,
                        until,
                        part: Some(part.id),
                    },
                );
                self.events
                    .schedule(until, Event::Vehicle { fleet, vehicle });
            }
            Phase::Carrying => {
                let unload = duration(self.fleets[fleet].handling_rng.sample(&f.unload_time));
                self.fleets[fleet].vehicles[vehicle].phase = Phase::Unloading;
                self.events
                    .schedule(self.now + unload, Event::Vehicle { fleet, vehicle });
            }
            Phase::Unloading => {
                // Delivered: at the transport's end, for the next object.
                let state = &mut self.conveyors[conveyor];
                state.in_transit -= 1;
                state.belt.push_back(OnBelt {
                    part,
                    pos: 0.0,
                    since: self.now,
                    entered: since,
                });
                self.observer.record(
                    self.now,
                    TraceEvent::Delivered {
                        part: part.id,
                        conveyor,
                    },
                );
                if self.conveyors[conveyor].belt.len() == 1 {
                    self.schedule_reach_end(conveyor);
                }
                let fs = &mut self.fleets[fleet];
                fs.trips += 1;
                let v = &mut fs.vehicles[vehicle];
                v.phase = Phase::Idle;
                v.job = None;
                self.dispatch(fleet);
            }
            Phase::Idle => unreachable!("idle vehicles have no pending step"),
        }
    }

    // --- Servers ---

    /// The server may be able to take a part. With one input it takes it
    /// now; with several, a `Pull` event decides after everything due now.
    fn request_pull(&mut self, server: usize) {
        if self.spec.servers[server].inputs.len() == 1 {
            self.pull(server);
        } else if !self.servers[server].pull_pending {
            self.servers[server].pull_pending = true;
            self.events.schedule(self.now, Event::Pull { server });
        }
    }

    /// Takes waiting parts into free slots, choosing inputs by the server's rule.
    fn pull(&mut self, server: usize) {
        self.servers[server].pull_pending = false;
        if self.spec.servers[server].kind != ServerKind::Single {
            return self.pull_collecting(server);
        }
        loop {
            let s = &self.servers[server];
            if !s.is_up() || s.idle_slot().is_none() {
                return;
            }
            if !self.resources_available(server) {
                if self.any_input_waiting(server) && !self.servers[server].waiting_for_resources {
                    self.servers[server].waiting_for_resources = true;
                    self.resource_queue.push_back(server);
                }
                return;
            }
            let Some(conveyor) = self.choose_input(server, |_| true) else {
                return;
            };
            let part = self.take_head(conveyor);
            self.start_process(server, part, true);
            self.after_departure(conveyor);
        }
    }

    /// Batch and assembly servers: collect parts into the batch or kit, and
    /// start it when it's complete.
    fn pull_collecting(&mut self, server: usize) {
        let spec = &self.spec.servers[server];
        loop {
            if !self.servers[server].is_up() {
                return;
            }
            // Collect what the batch or kit still needs.
            loop {
                // How many more of each part type it takes (batches take any).
                let staging = &self.servers[server].staging;
                let still: Vec<usize> = match &spec.kind {
                    ServerKind::Batch(n) => vec![*n as usize - staging.len()],
                    ServerKind::Assembly { recipe, .. } => {
                        let mut still = vec![0; self.spec.entities.len()];
                        for &(e, count) in recipe {
                            still[e] =
                                count as usize - staging.iter().filter(|p| p.entity == e).count();
                        }
                        still
                    }
                    ServerKind::Single => unreachable!("single servers don't collect"),
                };
                let batch = matches!(spec.kind, ServerKind::Batch(_));
                let accepts = |part: &Part| {
                    if batch {
                        still[0] > 0
                    } else {
                        still[part.entity] > 0
                    }
                };
                let Some(conveyor) = self.choose_input(server, accepts) else {
                    break;
                };
                let part = self.take_head(conveyor);
                self.servers[server].staging.push(part);
                self.observer.record(
                    self.now,
                    TraceEvent::Collected {
                        part: part.id,
                        server,
                    },
                );
                self.after_departure(conveyor);
            }
            let s = &self.servers[server];
            let (complete, slots) = match &spec.kind {
                ServerKind::Batch(n) => (s.staging.len() == *n as usize, *n as usize),
                ServerKind::Assembly { recipe, .. } => {
                    let total: u32 = recipe.iter().map(|&(_, count)| count).sum();
                    (s.staging.len() == total as usize, 1)
                }
                ServerKind::Single => unreachable!(),
            };
            let free = s.slots.iter().filter(|x| matches!(x, Slot::Idle)).count();
            if !complete || free < slots {
                return;
            }
            if !self.resources_available(server) {
                if !self.servers[server].waiting_for_resources {
                    self.servers[server].waiting_for_resources = true;
                    self.resource_queue.push_back(server);
                }
                return;
            }
            let staged = std::mem::take(&mut self.servers[server].staging);
            match &spec.kind {
                ServerKind::Batch(_) => self.start_batch(server, staged),
                ServerKind::Assembly {
                    makes, clock_of, ..
                } => {
                    for p in &staged {
                        self.observer
                            .record(self.now, TraceEvent::Consumed { part: p.id, server });
                    }
                    self.totals.consumed += staged.len() as u64;
                    let part = Part {
                        id: self.next_part,
                        // When its earliest component arrived, or its
                        // earliest of the type that keeps the clock (an order).
                        created: staged
                            .iter()
                            .filter(|p| clock_of.is_none_or(|e| p.entity == e))
                            .map(|p| p.created)
                            .min()
                            .expect("validated: the clock's type is in the kit"),
                        entity: *makes,
                    };
                    self.next_part += 1;
                    self.totals.assembled += 1;
                    self.observer.record(
                        self.now,
                        TraceEvent::Assembled {
                            part: part.id,
                            server,
                            entity: part.entity,
                        },
                    );
                    self.start_process(server, part, true);
                }
                ServerKind::Single => unreachable!(),
            }
        }
    }

    /// Starts a full batch: one process time and one set of units for all.
    fn start_batch(&mut self, server: usize, parts: Vec<Part>) {
        for &(r, units) in &self.spec.servers[server].needs {
            self.resources[r].in_use += units;
        }
        let s = &mut self.servers[server];
        let entity = parts[0].entity;
        let first = s.idle_slot().expect("caller checked for free slots");
        let setup = self.setup(server, first, entity);
        let s = &mut self.servers[server];
        let total = setup
            + duration(
                s.rng
                    .sample(self.spec.servers[server].process_time.get(entity)),
            );
        for (i, part) in parts.into_iter().enumerate() {
            let s = &mut self.servers[server];
            let slot = s.idle_slot().expect("caller checked for free slots");
            s.set_up_for[slot] = Some(entity);
            s.slots[slot] = Slot::Busy {
                part,
                finish: self.now + total,
                total,
                holds: i == 0,
            };
            self.schedule_process_done(server, slot, self.now + total);
            self.observer.record(
                self.now,
                TraceEvent::StartedProcess {
                    part: part.id,
                    server,
                },
            );
        }
        self.update_failure_clocks(server);
    }

    /// The setup a slot needs before a part of `entity` (zero if it's set
    /// up for it, or the server has no changeovers), counted; the slot is
    /// then set up for it.
    fn setup(&mut self, server: usize, slot: usize, entity: usize) -> SimTime {
        let s = &mut self.servers[server];
        let last = s.set_up_for[slot].replace(entity);
        let Some(changeover) = &self.spec.servers[server].changeover else {
            return SimTime::ZERO;
        };
        let Some(d) = last.and_then(|from| changeover.between(from, entity)) else {
            return SimTime::ZERO;
        };
        let setup = duration(s.changeover_rng.sample(d));
        s.changeovers += 1;
        s.setup_time += setup.as_secs_f64();
        setup
    }

    fn resources_available(&self, server: usize) -> bool {
        self.spec.servers[server].needs.iter().all(|&(r, units)| {
            !self.resources[r].off_shift
                && self.resources[r].in_use + units <= self.spec.resources[r].capacity
        })
    }

    fn any_input_waiting(&self, server: usize) -> bool {
        let spec = self.spec;
        spec.servers[server]
            .inputs
            .iter()
            .any(|&c| self.conveyors[c].head_at_end(&spec.conveyors[c], self.now))
    }

    /// Units came back: waiting servers try again, in the order they
    /// started waiting. Those that still can't start queue up again, in the
    /// same order.
    fn serve_resource_queue(&mut self) {
        let waiting = std::mem::take(&mut self.resource_queue);
        for server in waiting {
            self.servers[server].waiting_for_resources = false;
            self.pull(server);
        }
    }

    /// The input conveyor whose head the server takes next, if any is
    /// waiting and `accepts` it.
    fn choose_input(&mut self, server: usize, accepts: impl Fn(&Part) -> bool) -> Option<usize> {
        let spec = self.spec;
        let now = self.now;
        let inputs = &spec.servers[server].inputs;
        let waiting = |c: usize| {
            self.conveyors[c].head_at_end(&spec.conveyors[c], now)
                && accepts(&self.conveyors[c].belt[0].part)
        };
        match spec.servers[server].input_rule {
            InputRule::Single | InputRule::Priority => inputs.iter().copied().find(|&c| waiting(c)),
            // `min_by_key` keeps the first of equal keys: ties go to the input listed first.
            InputRule::FirstCome => inputs
                .iter()
                .copied()
                .filter(|&c| waiting(c))
                .min_by_key(|&c| self.conveyors[c].head_arrived.unwrap_or(SimTime::MAX)),
            InputRule::RoundRobin => {
                let n = inputs.len();
                let start = self.servers[server].next_input;
                let i = (0..n)
                    .map(|k| (start + k) % n)
                    .find(|&i| waiting(inputs[i]))?;
                self.servers[server].next_input = (i + 1) % n;
                Some(inputs[i])
            }
        }
    }

    fn start_process(&mut self, server: usize, part: Part, holds: bool) {
        if holds {
            for &(r, units) in &self.spec.servers[server].needs {
                self.resources[r].in_use += units;
            }
        }
        let slot = self.servers[server]
            .idle_slot()
            .expect("caller checked for a free slot");
        let setup = self.setup(server, slot, part.entity);
        let s = &mut self.servers[server];
        let total = setup
            + duration(
                s.rng
                    .sample(self.spec.servers[server].process_time.get(part.entity)),
            );
        s.slots[slot] = Slot::Busy {
            part,
            finish: self.now + total,
            total,
            holds,
        };
        self.schedule_process_done(server, slot, self.now + total);
        self.observer.record(
            self.now,
            TraceEvent::StartedProcess {
                part: part.id,
                server,
            },
        );
        self.update_failure_clocks(server);
    }

    fn schedule_process_done(&mut self, server: usize, slot: usize, at: SimTime) {
        let s = &mut self.servers[server];
        s.slot_generation[slot] += 1;
        let generation = s.slot_generation[slot];
        self.events.schedule(
            at,
            Event::ProcessDone {
                server,
                slot,
                generation,
            },
        );
    }

    fn process_done(&mut self, server: usize, slot: usize) {
        let spec = &self.spec.servers[server];
        let Slot::Busy { part, holds, .. } = self.servers[server].slots[slot] else {
            unreachable!("a current ProcessDone event belongs to a busy slot");
        };
        if holds {
            for &(r, units) in &spec.needs {
                self.resources[r].in_use -= units;
            }
        }
        self.observer.record(
            self.now,
            TraceEvent::FinishedProcess {
                part: part.id,
                server,
            },
        );
        // A split part becomes several, which leave one by one, keeping the
        // time the original arrived.
        let part = match &spec.splits_into {
            None => part,
            Some((entity, count)) => {
                let n = self.servers[server].split_rng.count(count);
                self.observer.record(
                    self.now,
                    TraceEvent::Consumed {
                        part: part.id,
                        server,
                    },
                );
                self.totals.consumed += 1;
                self.totals.split += u64::from(n);
                let mut children: VecDeque<Part> = (0..n)
                    .map(|_| {
                        let child = Part {
                            id: self.next_part,
                            created: part.created,
                            entity: *entity,
                        };
                        self.next_part += 1;
                        self.observer.record(
                            self.now,
                            TraceEvent::Split {
                                part: child.id,
                                server,
                                entity: *entity,
                            },
                        );
                        child
                    })
                    .collect();
                let first = children
                    .pop_front()
                    .expect("validated: splits make at least one part");
                self.servers[server].split_rest[slot] = children;
                first
            }
        };
        let s = &mut self.servers[server];
        let target = pick_output(
            &spec.routing,
            &spec.outputs,
            part.entity,
            &mut s.routing_rng,
            &mut s.next_output,
        );
        s.slots[slot] = Slot::Blocked { part, target };
        s.blocked_order.push_back(slot);
        self.update_failure_clocks(server);
        self.release_blocked(server);
        if !self.spec.servers[server].needs.is_empty() {
            self.serve_resource_queue();
        }
    }

    /// Moves finished parts onto their outgoing conveyors where there's
    /// room, then takes new parts into the freed slots.
    fn release_blocked(&mut self, server: usize) {
        if !self.servers[server].is_up() {
            return;
        }
        let spec = &self.spec.servers[server];
        let outputs = &spec.outputs;
        // In finishing order; sending a part never touches this server's
        // blocked list, so walking it by index is safe (and allocation-free).
        let mut i = 0;
        while i < self.servers[server].blocked_order.len() {
            let slot = self.servers[server].blocked_order[i];
            let Slot::Blocked { part, target } = self.servers[server].slots[slot] else {
                unreachable!("blocked_order only holds blocked slots");
            };
            if self.try_send(part, target, outputs, &spec.routing) {
                let s = &mut self.servers[server];
                s.processed += 1;
                if let Some(next) = s.split_rest[slot].pop_front() {
                    // The next part of a split takes its place and tries at once.
                    let target = pick_output(
                        &spec.routing,
                        outputs,
                        next.entity,
                        &mut s.routing_rng,
                        &mut s.next_output,
                    );
                    s.slots[slot] = Slot::Blocked { part: next, target };
                } else {
                    s.blocked_order.remove(i);
                    s.slots[slot] = Slot::Idle;
                }
            } else {
                i += 1;
            }
        }
        // Wait for each distinct target the remaining parts need, once.
        let mut i = 0;
        while i < self.servers[server].blocked_order.len() {
            let target_of = |sim: &Self, k: usize| match sim.servers[server].slots
                [sim.servers[server].blocked_order[k]]
            {
                Slot::Blocked { target, .. } => target,
                _ => unreachable!("blocked_order only holds blocked slots"),
            };
            let target = target_of(self, i);
            if (0..i).all(|k| target_of(self, k) != target) {
                self.wait_for(target, outputs);
            }
            i += 1;
        }
        self.request_pull(server);
    }

    fn update_failure_clocks(&mut self, server: usize) {
        let s = &mut self.servers[server];
        let up = s.is_up();
        let busy = s.any_busy();
        for (mode, f) in self.spec.servers[server].failures.iter().enumerate() {
            let should_run = up
                && match f.clock {
                    FailureClock::ClockTime => true,
                    FailureClock::BusyTime => busy,
                };
            let state = &mut s.failures[mode];
            match (state.running_since, should_run) {
                (Some(since), false) => {
                    state.remaining = state.remaining - (self.now - since);
                    state.running_since = None;
                    state.generation += 1; // cancel the scheduled failure
                }
                (None, true) => {
                    state.running_since = Some(self.now);
                    state.generation += 1;
                    self.events.schedule(
                        self.now + state.remaining,
                        Event::Failure {
                            server,
                            mode,
                            generation: state.generation,
                        },
                    );
                }
                _ => {}
            }
        }
    }

    fn fail(&mut self, server: usize, mode: usize) {
        let now = self.now;
        let on_failure = self.spec.servers[server].failures[mode].on_failure;
        let s = &mut self.servers[server];
        let state = &mut s.failures[mode];
        state.remaining = SimTime::ZERO;
        state.running_since = None;
        state.generation += 1;
        s.down_for = Some(mode);
        s.failure_count += 1;
        self.suspend_slots(server, on_failure);
        let s = &mut self.servers[server];
        let downtime = s.failures[mode]
            .downtime_rng
            .sample(&self.spec.servers[server].failures[mode].downtime);
        self.events
            .schedule(now + duration(downtime), Event::Repair { server, mode });
        self.observer.record(
            now,
            TraceEvent::Failed {
                server,
                failure: mode,
            },
        );
        self.update_failure_clocks(server);
    }

    fn repair(&mut self, server: usize, mode: usize) {
        let s = &mut self.servers[server];
        s.down_for = None;
        let uptime = s.failures[mode]
            .uptime_rng
            .sample(&self.spec.servers[server].failures[mode].uptime);
        s.failures[mode].remaining = duration(uptime);
        self.observer.record(
            self.now,
            TraceEvent::Repaired {
                server,
                failure: mode,
            },
        );
        // Off shift, paused parts wait for the next shift.
        if !self.servers[server].off_shift {
            self.resume_slots(server);
        }
        self.update_failure_clocks(server);
        self.release_blocked(server);
    }

    // --- Sinks ---

    fn exit(&mut self, sink: usize, part: Part) {
        self.sink_exited[sink] += 1;
        self.totals.exited += 1;
        let time = (self.now - part.created).as_secs_f64();
        self.sink_times[sink].add(time);
        let by_type = &mut self.sink_by_type[sink][part.entity];
        by_type.0 += 1;
        by_type.1.add(time);
        self.observer.record(
            self.now,
            TraceEvent::Exited {
                part: part.id,
                sink,
            },
        );
    }

    // --- Statistics ---

    /// Records the current state of everything. Called after every event,
    /// so time averages are exact.
    fn sample_state(&mut self) {
        let now = self.now;
        for s in &mut self.sources {
            s.blocked.set(now, u64::from(!s.held.is_empty()));
            s.on_order_avg.set(now, s.on_order);
        }
        for (s, spec) in self.servers.iter_mut().zip(&self.spec.servers) {
            s.off.set(
                now,
                if s.off_shift {
                    u64::from(spec.capacity)
                } else {
                    0
                },
            );
            let (busy, blocked, failed) = if s.off_shift {
                (0, 0, 0)
            } else if s.is_up() {
                let count = |f: fn(&Slot) -> bool| s.slots.iter().filter(|x| f(x)).count() as u64;
                (
                    count(|x| matches!(x, Slot::Busy { .. })),
                    count(|x| matches!(x, Slot::Blocked { .. })),
                    0,
                )
            } else {
                (0, 0, u64::from(spec.capacity))
            };
            s.busy.set(now, busy);
            s.blocked.set(now, blocked);
            s.failed.set(now, failed);
            s.waiting.set(now, u64::from(s.waiting_for_resources));
            s.overtime.set(now, u64::from(s.closing));
        }
        for f in &mut self.fleets {
            let count =
                |phase: Phase| f.vehicles.iter().filter(|v| v.phase == phase).count() as u64;
            f.busy
                .set(now, f.vehicles.len() as u64 - count(Phase::Idle));
            f.empty.set(now, count(Phase::ToPickup));
        }
        for r in &mut self.resources {
            r.busy.set(now, u64::from(r.in_use));
            r.off.set(now, u64::from(r.off_shift));
            r.overtime.set(now, u64::from(r.closing));
        }
        for (c, spec) in self.conveyors.iter_mut().zip(&self.spec.conveyors) {
            let n = c.occupancy() as u64;
            c.parts.set(now, n);
            c.empty.set(now, u64::from(n == 0));
            c.full.set(now, u64::from(n >= u64::from(spec.capacity)));
        }
    }

    fn reset_statistics(&mut self) {
        self.sample_state();
        let now = self.now;
        for s in &mut self.sources {
            s.created = 0;
            s.lost = 0;
            s.orders = 0;
            s.blocked.reset(now);
            s.on_order_avg.reset(now);
        }
        for s in &mut self.servers {
            s.processed = 0;
            s.failure_count = 0;
            s.changeovers = 0;
            s.setup_time = 0.0;
            s.busy.reset(now);
            s.blocked.reset(now);
            s.failed.reset(now);
            s.waiting.reset(now);
            s.off.reset(now);
            s.overtime.reset(now);
        }
        for f in &mut self.fleets {
            f.trips = 0;
            f.busy.reset(now);
            f.empty.reset(now);
        }
        for r in &mut self.resources {
            r.busy.reset(now);
            r.off.reset(now);
            r.overtime.reset(now);
        }
        for c in &mut self.conveyors {
            c.entered = 0;
            c.time = Tally::default();
            c.time_by_type
                .iter_mut()
                .for_each(|t| *t = Tally::default());
            if let Some(times) = &mut c.times {
                *times = Histogram::default();
            }
            c.within_target = 0;
            c.pickup_wait = Tally::default();
            c.parts.reset(now);
            c.full.reset(now);
            c.empty.reset(now);
        }
        self.sink_exited.iter_mut().for_each(|n| *n = 0);
        self.sink_times
            .iter_mut()
            .for_each(|t| *t = Tally::default());
        for types in &mut self.sink_by_type {
            types.iter_mut().for_each(|t| *t = (0, Tally::default()));
        }
    }

    fn result(&self) -> ReplicationResult {
        let in_system = self.sources.iter().map(|s| s.held.len()).sum::<usize>()
            + self
                .servers
                .iter()
                .flat_map(|s| &s.split_rest)
                .map(VecDeque::len)
                .sum::<usize>()
            + self.servers.iter().map(|s| s.staging.len()).sum::<usize>()
            + self
                .servers
                .iter()
                .flat_map(|s| &s.slots)
                .filter(|s| !matches!(s, Slot::Idle))
                .count()
            + self.conveyors.iter().map(|c| c.occupancy()).sum::<usize>();
        ReplicationResult {
            replication: self.replication,
            measured: self.spec.experiment.run_length.as_secs_f64(),
            sources: self
                .sources
                .iter()
                .map(|s| SourceResult {
                    created: s.created,
                    lost: s.lost,
                    blocked: s.blocked.mean(),
                    orders: s.orders,
                    on_order: s.on_order_avg.mean(),
                })
                .collect(),
            servers: self
                .servers
                .iter()
                .zip(&self.spec.servers)
                .map(|(s, spec)| {
                    let capacity = f64::from(spec.capacity);
                    let off_shift = s.off.mean() / capacity;
                    // States are shares of the time the server could work:
                    // all of it, or its shifts if it has a schedule.
                    let on = working_share(off_shift);
                    let busy = s.busy.mean() / capacity / on;
                    let blocked = s.blocked.mean() / capacity / on;
                    let failed = s.failed.mean() / capacity / on;
                    ServerResult {
                        idle: (1.0 - busy - blocked - failed).max(0.0),
                        off_shift,
                        busy,
                        blocked,
                        failed,
                        processed: s.processed,
                        failures: s.failure_count,
                        waiting_for_resources: s.waiting.mean() / on,
                        overtime: s.overtime.mean(),
                        changeovers: s.changeovers,
                        changeover: s.setup_time
                            / (capacity * self.spec.experiment.run_length.as_secs_f64() * on),
                    }
                })
                .collect(),
            conveyors: self
                .conveyors
                .iter()
                .map(|c| ConveyorResult {
                    mean_parts: c.parts.mean(),
                    max_parts: c.parts.max(),
                    full: c.full.mean(),
                    entered: c.entered,
                    time: c.time.clone(),
                    time_by_type: c.time_by_type.clone(),
                    times: c.times.clone(),
                    within_target: c.within_target,
                    empty: c.empty.mean(),
                    pickup_wait: c.pickup_wait.clone(),
                })
                .collect(),
            resources: self
                .resources
                .iter()
                .zip(&self.spec.resources)
                .map(|(r, spec)| ResourceResult {
                    // Of the time it's on shift (all the time without a schedule).
                    busy: r.busy.mean() / f64::from(spec.capacity) / working_share(r.off.mean()),
                    off_shift: r.off.mean(),
                    overtime: r.overtime.mean(),
                })
                .collect(),
            sinks: self
                .sink_exited
                .iter()
                .zip(&self.sink_times)
                .zip(&self.sink_by_type)
                .map(|((&exited, times), by_type)| SinkResult {
                    exited,
                    time_in_system: times.clone(),
                    by_type: by_type
                        .iter()
                        .map(|(exited, times)| TypeResult {
                            exited: *exited,
                            time_in_system: times.clone(),
                        })
                        .collect(),
                })
                .collect(),
            vehicles: self
                .fleets
                .iter()
                .map(|f| {
                    let n = f.vehicles.len().max(1) as f64;
                    FleetResult {
                        busy: f.busy.mean() / n,
                        empty: f.empty.mean() / n,
                        trips: f.trips,
                    }
                })
                .collect(),
            totals: Totals {
                in_system: in_system as u64,
                ..self.totals.clone()
            },
        }
    }
}
