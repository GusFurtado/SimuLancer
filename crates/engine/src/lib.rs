//! Discrete event simulation engine core.
//!
//! The engine works in standard units only: time is [`SimTime`] (integer
//! microseconds) and length is meters. Model files, units and output formats
//! are handled outside this crate (see `outliers-sim-model`).

mod events;
pub mod results;
mod rng;
mod sim;
pub mod spec;
mod time;
pub mod trace;

pub use results::ReplicationResult;
pub use sim::simulate;
pub use spec::Spec;
pub use time::SimTime;
pub use trace::{NoTrace, Observer, TraceEvent};
