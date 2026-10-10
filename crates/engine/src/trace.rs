//! What happened to each part, as it happens. The event log and the viewer
//! are built from this; the engine itself doesn't write files.

use crate::SimTime;

pub type PartId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceEvent {
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
    StartedProcess {
        part: PartId,
        server: usize,
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
