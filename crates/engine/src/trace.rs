//! What happened to each part, as it happens. The event log and the animation
//! are built from this; the engine itself doesn't write files.

use crate::SimTime;
use crate::spec::Node;

pub type PartId = u64;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceEvent {
    /// A source or server set one of the part's attributes (an index into
    /// [`crate::Spec`]'s `attributes`); times are in seconds.
    Set {
        part: PartId,
        attribute: usize,
        value: f64,
    },
    /// A vehicle sets off from one object to another, arriving at `until`;
    /// carrying `part`, or empty.
    VehicleTrip {
        fleet: usize,
        vehicle: usize,
        from: Node,
        to: Node,
        until: SimTime,
        part: Option<PartId>,
    },
    /// Unloaded at the end of a transport (`conveyor`).
    Delivered {
        part: PartId,
        conveyor: usize,
    },
    /// In a queue when the run starts (`starts_with`).
    Stocked {
        part: PartId,
        conveyor: usize,
        entity: usize,
    },
    /// Made by splitting a part (which is `Consumed`) at a server.
    Split {
        part: PartId,
        server: usize,
        entity: usize,
    },
    Created {
        part: PartId,
        source: usize,
        /// The part type, an index into [`crate::Spec::entities`].
        entity: usize,
    },
    /// Discarded by a source because its conveyor was full.
    Lost {
        part: PartId,
        source: usize,
    },
    /// Left a queue after waiting longer than its patience.
    GaveUp {
        part: PartId,
        conveyor: usize,
    },
    EnteredConveyor {
        part: PartId,
        conveyor: usize,
    },
    /// The part reached the end of the conveyor. It may wait there if the
    /// next object can't take it.
    ReachedEnd {
        part: PartId,
        conveyor: usize,
    },
    /// Processing starts in one of the server's slots (0 to capacity - 1).
    StartedProcess {
        part: PartId,
        server: usize,
        slot: usize,
    },
    /// A server took units of a resource it `needs` to process `part` (the
    /// first part of a batch), and holds them until `ResourceReturned`.
    ResourceTaken {
        resource: usize,
        units: u32,
        server: usize,
        part: PartId,
    },
    /// The units taken for `part` came back when its processing finished.
    ResourceReturned {
        resource: usize,
        units: u32,
        server: usize,
        part: PartId,
    },
    /// Taken off its conveyor into a batch or kit the server is collecting.
    Collected {
        part: PartId,
        server: usize,
    },
    /// A component used up by an assembly.
    Consumed {
        part: PartId,
        server: usize,
    },
    /// A new part made from a complete kit; it starts processing next.
    Assembled {
        part: PartId,
        server: usize,
        /// Its part type, an index into [`crate::Spec::entities`].
        entity: usize,
    },
    FinishedProcess {
        part: PartId,
        server: usize,
    },
    Failed {
        server: usize,
        failure: usize,
    },
    Repaired {
        server: usize,
        failure: usize,
    },
    /// The server's schedule says stop: work pauses until the next shift.
    ShiftEnded {
        server: usize,
    },
    ShiftStarted {
        server: usize,
    },
    Exited {
        part: PartId,
        sink: usize,
    },
}

pub trait Observer {
    fn record(&mut self, time: SimTime, event: TraceEvent);
}

/// Discards everything; for runs without an event log.
pub struct NoTrace;

impl Observer for NoTrace {
    #[inline]
    fn record(&mut self, _: SimTime, _: TraceEvent) {}
}

/// Keeps everything in memory; for tests and small runs.
impl Observer for Vec<(SimTime, TraceEvent)> {
    fn record(&mut self, time: SimTime, event: TraceEvent) {
        self.push((time, event));
    }
}
