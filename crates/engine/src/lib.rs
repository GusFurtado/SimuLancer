//! Discrete event simulation engine core.
//!
//! The engine works in standard units only: time is [`SimTime`] (integer
//! microseconds) and length is meters. Model files, units and output formats
//! are handled outside this crate (see `simulancer-model`).

mod events;
mod experiment;
pub mod results;
mod rng;
mod sim;
pub mod spec;
pub mod stats;
mod time;
pub mod trace;

pub use experiment::parallel_replications;
pub use results::ReplicationResult;
pub use sim::simulate;
pub use spec::Spec;
pub use time::SimTime;
pub use trace::{NoTrace, Observer, TraceEvent};
