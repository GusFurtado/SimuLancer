//! Model file loading: YAML parsing, validation and unit conversion.
//!
//! This is the only layer that knows about units. Everything it hands to the
//! engine is already in standard units (seconds as `SimTime`, meters).

mod build;
mod diagnostic;
mod expr;
mod raw;
mod units;

use outliers_sim_engine::Spec;
use serde_saphyr::{MessageFormatter, UserMessageFormatter};

pub use diagnostic::{Diagnostic, LoadError, Position};
pub use raw::{EventLog, SummaryItem};

/// The `format:` value this version reads.
pub const FORMAT: &str = "outliers-sim/0.1";

/// A loaded and validated model.
#[derive(Debug, Clone)]
pub struct Model {
    pub name: String,
    pub description: Option<String>,
    pub spec: Spec,
    pub outputs: Outputs,
}

#[derive(Debug, Clone)]
pub struct Outputs {
    pub event_log: EventLog,
    pub summary: Vec<SummaryItem>,
}

/// Parses and validates a model from YAML source text.
pub fn load_str(source: &str) -> Result<Model, LoadError> {
    let options = serde_saphyr::options! {
        // Only `true`/`false` are booleans; `yes`, `no`, `on` and `off` are not.
        strict_booleans: true,
    };
    let raw: raw::RawModel =
        serde_saphyr::from_str_with_options(source, options).map_err(|err| {
            LoadError::Syntax(Diagnostic {
                position: err.location().as_ref().and_then(Position::from_location),
                message: UserMessageFormatter.format_message(&err).into_owned(),
            })
        })?;
    build::build(raw).map_err(LoadError::Invalid)
}
