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
//! - Failure clocks run only while the server is up. `busy_time` clocks run
//!   only while at least one slot is processing. While the server is down it
//!   does nothing: no processing, no taking parts, no releasing blocked parts.
//! - Events at the same time run in the order they were scheduled. Events at
//!   exactly the end of the run are not processed.

use std::collections::VecDeque;

use crate::SimTime;
use crate::events::{Event, EventList};
use crate::results::{
    ConveyorResult, ReplicationResult, ServerResult, SinkResult, SourceResult, Tally, TimeAverage,
    Totals,
};
use crate::rng::Rng;
use crate::spec::{Conveyor, FailureClock, InputRule, Node, OnFailure, Routing, Spec, WhenBlocked};
use crate::trace::{Observer, PartId, TraceEvent};

/// Position tolerance in meters, for float error in positions.
const EPS: f64 = 1e-9;

#[derive(Debug, Clone, Copy)]
struct Part {
    id: PartId,
    created: SimTime,
}

/// The conveyor a waiting part will leave on; `None` means any output with
/// room (`first_available`).
type Target = Option<usize>;

struct SourceState {
    rng: Rng,
    routing_rng: Rng,
    next_output: usize,
    /// Part waiting to get on a conveyor (`when_blocked: wait`).
    held: Option<(Part, Target)>,
    created: u64,
    lost: u64,
    blocked: TimeAverage,
}

#[derive(Debug, Clone, Copy)]
enum Slot {
    Idle,
    Busy {
        part: Part,
        finish: SimTime,
        total: SimTime,
    },
    /// Processing interrupted by a failure.
    Suspended {
        part: Part,
        remaining: SimTime,
        total: SimTime,
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
    failures: Vec<FailureState>,
    /// The failure the server is currently down for.
    down_for: Option<usize>,
    processed: u64,
    failure_count: u64,
    busy: TimeAverage,
    blocked: TimeAverage,
    failed: TimeAverage,
}

impl ServerState {
    fn is_up(&self) -> bool {
        self.down_for.is_none()
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
    parts: TimeAverage,
    full: TimeAverage,
}

impl ConveyorState {
    /// Real positions at `now`, head first: each part is capped by the end
    /// (head) or by the part ahead minus one spacing.
    fn positions(&self, c: &Conveyor, now: SimTime) -> Vec<f64> {
        let mut limit = c.length;
        self.belt
            .iter()
            .map(|p| {
                let free = p.pos + c.speed * (now - p.since).as_secs_f64();
                let pos = free.min(limit);
                limit = pos - c.spacing;
                pos
            })
            .collect()
    }

    fn can_enter(&self, c: &Conveyor, now: SimTime) -> bool {
        if self.belt.len() >= c.capacity as usize {
            return false;
        }
        match self.positions(c, now).last() {
            None => true,
            Some(&last) => last >= c.spacing - EPS,
        }
    }

    fn head_at_end(&self, c: &Conveyor, now: SimTime) -> bool {
        self.positions(c, now)
            .first()
            .is_some_and(|&head| head >= c.length - EPS)
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
fn pick_output(routing: &Routing, outputs: &[usize], rng: &mut Rng, next: &mut usize) -> Target {
    match routing {
        Routing::Single => Some(outputs[0]),
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

/// Runs one replication. `replication` (1-based) selects the random streams.
pub fn simulate<O: Observer>(spec: &Spec, replication: u32, observer: &mut O) -> ReplicationResult {
    let mut sim = Sim::new(spec, replication, observer);
    sim.run();
    sim.result()
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
    sink_exited: Vec<u64>,
    sink_times: Vec<Tally>,
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
                next_output: 0,
                held: None,
                created: 0,
                lost: 0,
                blocked: TimeAverage::default(),
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
                    failures,
                    down_for: None,
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
            .map(|_| ConveyorState {
                belt: VecDeque::new(),
                generation: 0,
                upstream_waiting: false,
                head_arrived: None,
                entered: 0,
                parts: TimeAverage::default(),
                full: TimeAverage::default(),
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
            sink_exited: vec![0; spec.sinks.len()],
            sink_times: vec![Tally::default(); spec.sinks.len()],
            totals: Totals::default(),
        }
    }

    fn run(&mut self) {
        self.events.schedule(self.end, Event::RunEnd);
        if self.spec.experiment.warmup > SimTime::ZERO {
            self.events
                .schedule(self.spec.experiment.warmup, Event::WarmupEnd);
        }
        for source in 0..self.spec.sources.len() {
            self.events
                .schedule(SimTime::ZERO, Event::Arrival { source });
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
                Event::Arrival { source } => self.arrival(source),
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
            }
            self.sample_state();
        }
        self.sample_state();
    }

    // --- Sources ---

    fn arrival(&mut self, source: usize) {
        let part = Part {
            id: self.next_part,
            created: self.now,
        };
        self.next_part += 1;
        self.sources[source].created += 1;
        self.totals.created += 1;
        self.observer.record(
            self.now,
            TraceEvent::Created {
                part: part.id,
                source,
            },
        );

        let spec = self.spec;
        let source_spec = &spec.sources[source];
        let state = &mut self.sources[source];
        let target = pick_output(
            &source_spec.routing,
            &source_spec.outputs,
            &mut state.routing_rng,
            &mut state.next_output,
        );
        if self.try_send(part, target, &source_spec.outputs, &source_spec.routing) {
            self.schedule_arrival(source);
            return;
        }
        match source_spec.when_blocked {
            WhenBlocked::Wait => {
                self.sources[source].held = Some((part, target));
                self.wait_for(target, &source_spec.outputs);
            }
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
                self.schedule_arrival(source);
            }
        }
    }

    fn schedule_arrival(&mut self, source: usize) {
        let gap = self.sources[source]
            .rng
            .sample(&self.spec.sources[source].interarrival);
        self.events
            .schedule(self.now + duration(gap), Event::Arrival { source });
    }

    /// A conveyor the source was waiting for has room.
    fn release_held(&mut self, source: usize) {
        let Some((part, target)) = self.sources[source].held else {
            return; // already left by another conveyor (`first_available`)
        };
        let spec = &self.spec.sources[source];
        let outputs = &spec.outputs;
        if self.try_send(part, target, outputs, &spec.routing) {
            self.sources[source].held = None;
            self.schedule_arrival(source);
        } else {
            self.wait_for(target, outputs);
        }
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
            None => {
                let mut order = outputs.to_vec();
                if *routing == Routing::ShortestQueue {
                    // Stable sort: ties keep file order.
                    order.sort_by_key(|&c| self.conveyors[c].belt.len());
                }
                order.into_iter().any(|c| self.try_enter(c, part))
            }
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
        state.belt.push_back(OnBelt {
            part,
            pos: 0.0,
            since: self.now,
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
        let Some(&head) = state.positions(c, self.now).first() else {
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
        // real positions as the new free-movement baseline.
        let positions = self.conveyors[conveyor].positions(c, self.now);
        let state = &mut self.conveyors[conveyor];
        for (p, pos) in state.belt.iter_mut().zip(positions) {
            p.pos = pos;
            p.since = self.now;
        }
        let part = state.belt.pop_front().expect("head exists").part;
        self.schedule_reach_end(conveyor);
        part
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
        if state.belt.len() >= c.capacity as usize {
            return; // the next departure calls back
        }
        // Room depends on the last part moving one spacing from the entrance.
        let last = *state.belt.back().expect("not empty, or we could enter");
        let real = *state.positions(c, self.now).last().expect("not empty");
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
        loop {
            let s = &self.servers[server];
            if !s.is_up() || s.idle_slot().is_none() {
                return;
            }
            let Some(conveyor) = self.choose_input(server) else {
                return;
            };
            let part = self.take_head(conveyor);
            self.start_process(server, part);
            self.after_departure(conveyor);
        }
    }

    /// The input conveyor whose head the server takes next, if any is waiting.
    fn choose_input(&mut self, server: usize) -> Option<usize> {
        let spec = self.spec;
        let inputs = &spec.servers[server].inputs;
        let waiting: Vec<bool> = inputs
            .iter()
            .map(|&c| self.conveyors[c].head_at_end(&spec.conveyors[c], self.now))
            .collect();
        match spec.servers[server].input_rule {
            InputRule::Single | InputRule::Priority => inputs
                .iter()
                .zip(&waiting)
                .find(|(_, w)| **w)
                .map(|(&c, _)| c),
            // `min_by_key` keeps the first of equal keys: ties go to the input listed first.
            InputRule::FirstCome => inputs
                .iter()
                .zip(&waiting)
                .filter(|(_, w)| **w)
                .map(|(&c, _)| c)
                .min_by_key(|&c| self.conveyors[c].head_arrived.unwrap_or(SimTime::MAX)),
            InputRule::RoundRobin => {
                let n = inputs.len();
                let start = self.servers[server].next_input;
                let i = (0..n).map(|k| (start + k) % n).find(|&i| waiting[i])?;
                self.servers[server].next_input = (i + 1) % n;
                Some(inputs[i])
            }
        }
    }

    fn start_process(&mut self, server: usize, part: Part) {
        let s = &mut self.servers[server];
        let slot = s.idle_slot().expect("caller checked for a free slot");
        let total = duration(s.rng.sample(&self.spec.servers[server].process_time));
        s.slots[slot] = Slot::Busy {
            part,
            finish: self.now + total,
            total,
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
        let s = &mut self.servers[server];
        let Slot::Busy { part, .. } = s.slots[slot] else {
            unreachable!("a current ProcessDone event belongs to a busy slot");
        };
        let target = pick_output(
            &spec.routing,
            &spec.outputs,
            &mut s.routing_rng,
            &mut s.next_output,
        );
        s.slots[slot] = Slot::Blocked { part, target };
        s.blocked_order.push_back(slot);
        self.observer.record(
            self.now,
            TraceEvent::FinishedProcess {
                part: part.id,
                server,
            },
        );
        self.update_failure_clocks(server);
        self.release_blocked(server);
    }

    /// Moves finished parts onto their outgoing conveyors where there's
    /// room, then takes new parts into the freed slots.
    fn release_blocked(&mut self, server: usize) {
        if !self.servers[server].is_up() {
            return;
        }
        let spec = &self.spec.servers[server];
        let outputs = &spec.outputs;
        let order: Vec<usize> = self.servers[server].blocked_order.iter().copied().collect();
        let mut still_waiting: Vec<Target> = Vec::new();
        for slot in order {
            let Slot::Blocked { part, target } = self.servers[server].slots[slot] else {
                unreachable!("blocked_order only holds blocked slots");
            };
            if self.try_send(part, target, outputs, &spec.routing) {
                let s = &mut self.servers[server];
                s.blocked_order.retain(|&x| x != slot);
                s.slots[slot] = Slot::Idle;
                s.processed += 1;
            } else if !still_waiting.contains(&target) {
                still_waiting.push(target);
            }
        }
        for target in still_waiting {
            self.wait_for(target, outputs);
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
        for (slot, generation) in s.slots.iter_mut().zip(&mut s.slot_generation) {
            if let Slot::Busy {
                part,
                finish,
                total,
            } = *slot
            {
                let remaining = match on_failure {
                    OnFailure::Resume => finish - now,
                    OnFailure::Restart => total,
                };
                *slot = Slot::Suspended {
                    part,
                    remaining,
                    total,
                };
                *generation += 1; // cancel the scheduled ProcessDone
            }
        }
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
        for slot in 0..self.servers[server].slots.len() {
            if let Slot::Suspended {
                part,
                remaining,
                total,
            } = self.servers[server].slots[slot]
            {
                let finish = self.now + remaining;
                self.servers[server].slots[slot] = Slot::Busy {
                    part,
                    finish,
                    total,
                };
                self.schedule_process_done(server, slot, finish);
            }
        }
        self.update_failure_clocks(server);
        self.release_blocked(server);
    }

    // --- Sinks ---

    fn exit(&mut self, sink: usize, part: Part) {
        self.sink_exited[sink] += 1;
        self.totals.exited += 1;
        self.sink_times[sink].add((self.now - part.created).as_secs_f64());
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
            s.blocked.set(now, u64::from(s.held.is_some()));
        }
        for (s, spec) in self.servers.iter_mut().zip(&self.spec.servers) {
            let (busy, blocked, failed) = if s.is_up() {
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
        }
        for (c, spec) in self.conveyors.iter_mut().zip(&self.spec.conveyors) {
            let n = c.belt.len() as u64;
            c.parts.set(now, n);
            c.full.set(now, u64::from(n >= u64::from(spec.capacity)));
        }
    }

    fn reset_statistics(&mut self) {
        self.sample_state();
        let now = self.now;
        for s in &mut self.sources {
            s.created = 0;
            s.lost = 0;
            s.blocked.reset(now);
        }
        for s in &mut self.servers {
            s.processed = 0;
            s.failure_count = 0;
            s.busy.reset(now);
            s.blocked.reset(now);
            s.failed.reset(now);
        }
        for c in &mut self.conveyors {
            c.entered = 0;
            c.parts.reset(now);
            c.full.reset(now);
        }
        self.sink_exited.iter_mut().for_each(|n| *n = 0);
        self.sink_times
            .iter_mut()
            .for_each(|t| *t = Tally::default());
    }

    fn result(&self) -> ReplicationResult {
        let in_system = self.sources.iter().filter(|s| s.held.is_some()).count()
            + self
                .servers
                .iter()
                .flat_map(|s| &s.slots)
                .filter(|s| !matches!(s, Slot::Idle))
                .count()
            + self.conveyors.iter().map(|c| c.belt.len()).sum::<usize>();
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
                })
                .collect(),
            servers: self
                .servers
                .iter()
                .zip(&self.spec.servers)
                .map(|(s, spec)| {
                    let capacity = f64::from(spec.capacity);
                    let busy = s.busy.mean() / capacity;
                    let blocked = s.blocked.mean() / capacity;
                    let failed = s.failed.mean() / capacity;
                    ServerResult {
                        idle: (1.0 - busy - blocked - failed).max(0.0),
                        busy,
                        blocked,
                        failed,
                        processed: s.processed,
                        failures: s.failure_count,
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
                })
                .collect(),
            sinks: self
                .sink_exited
                .iter()
                .zip(&self.sink_times)
                .map(|(&exited, times)| SinkResult {
                    exited,
                    time_in_system: times.clone(),
                })
                .collect(),
            totals: Totals {
                in_system: in_system as u64,
                ..self.totals.clone()
            },
        }
    }
}
