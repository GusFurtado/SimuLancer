//! Model file loading: YAML parsing, validation and unit conversion.
//!
//! This is the only layer that knows about units. Everything it hands to the
//! engine is already in standard units (seconds as `SimTime`, meters).

mod build;
mod diagnostic;
mod expr;
mod layout;
mod raw;
mod scenario;
mod units;

use outliers_sim_engine::Spec;
use serde::de::DeserializeOwned;
use serde_saphyr::{MessageFormatter, UserMessageFormatter};

pub use diagnostic::{Diagnostic, LoadError, Position};
pub use layout::{Layout, PlacedConveyor, PlacedObject, load_layout};
pub use raw::{EventLog, SummaryItem};
pub use scenario::{
    Merged, SCENARIO_FORMAT, Scenario, ScenarioError, is_scenario, load_scenario, merge,
    parse_scenario,
};

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
    let raw: raw::RawModel = parse_yaml(source)?;
    build::build(raw).map_err(LoadError::Invalid)
}

/// A model file as a generic tree (maps in file order): what scenario change
/// paths address. Not validated; use [`load_str`] for that.
pub fn to_tree(source: &str) -> Result<serde_json::Value, LoadError> {
    parse_yaml(source)
}

/// Parses YAML with the project's strict settings.
fn parse_yaml<T: DeserializeOwned>(source: &str) -> Result<T, LoadError> {
    let options = serde_saphyr::options! {
        // Only `true`/`false` are booleans; `yes`, `no`, `on` and `off` are not.
        strict_booleans: true,
    };
    serde_saphyr::from_str_with_options(source, options).map_err(|err| {
        LoadError::Syntax(Diagnostic {
            position: err.location().as_ref().and_then(Position::from_location),
            message: UserMessageFormatter.format_message(&err).into_owned(),
        })
    })
}
