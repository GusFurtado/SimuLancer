//! The future event list.
//!
//! Tie-breaking rule: events scheduled for the same time run in the order
//! they were scheduled (FIFO). Because time is integer, "same time" is exact.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::SimTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Event {
    /// Stale if `generation` no longer matches the source's (its shift ended).
    Arrival {
        source: usize,
        generation: u64,
    },
    /// A schedule switches between working and not working.
    ScheduleChange {
        schedule: usize,
    },
    /// The head part of a conveyor reaches the end. Stale if `generation`
    /// no longer matches the conveyor's (its head changed).
    ReachEnd {
        conveyor: usize,
        generation: u64,
    },
    /// Re-check whether a waiting upstream object can put a part on the conveyor.
    EntryCheck {
        conveyor: usize,
    },
    ProcessDone {
        server: usize,
        slot: usize,
        generation: u64,
    },
    Failure {
        server: usize,
        mode: usize,
        generation: u64,
    },
    Repair {
        server: usize,
        mode: usize,
    },
    /// A server with several inputs picks its next part. Scheduled at the
    /// current time, so it runs after every event already due now: parts
    /// that reach their conveyor ends at the same moment all get considered.
    Pull {
        server: usize,
    },
    WarmupEnd,
    RunEnd,
}

struct Scheduled {
    time: SimTime,
    seq: u64,
    event: Event,
}

impl PartialEq for Scheduled {
    fn eq(&self, other: &Self) -> bool {
        (self.time, self.seq) == (other.time, other.seq)
    }
}

impl Eq for Scheduled {}

impl PartialOrd for Scheduled {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Scheduled {
    /// Reversed, so the max-heap pops the earliest (time, seq) first.
    fn cmp(&self, other: &Self) -> Ordering {
        (other.time, other.seq).cmp(&(self.time, self.seq))
    }
}

#[derive(Default)]
pub(crate) struct EventList {
    heap: BinaryHeap<Scheduled>,
    next_seq: u64,
}

impl EventList {
    pub fn schedule(&mut self, time: SimTime, event: Event) {
        self.heap.push(Scheduled {
            time,
            seq: self.next_seq,
            event,
        });
        self.next_seq += 1;
    }

    pub fn pop(&mut self) -> Option<(SimTime, Event)> {
        self.heap.pop().map(|s| (s.time, s.event))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earliest_first_then_fifo() {
        let mut list = EventList::default();
        let t = SimTime::from_micros;
        list.schedule(
            t(20),
            Event::Arrival {
                source: 0,
                generation: 0,
            },
        );
        list.schedule(
            t(10),
            Event::Arrival {
                source: 1,
                generation: 0,
            },
        );
        list.schedule(
            t(20),
            Event::Arrival {
                source: 2,
                generation: 0,
            },
        );
        list.schedule(
            t(10),
            Event::Arrival {
                source: 3,
                generation: 0,
            },
        );
        let order: Vec<_> = std::iter::from_fn(|| list.pop())
            .map(|(_, e)| match e {
                Event::Arrival { source, .. } => source,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(order, [1, 3, 0, 2]);
    }
}
