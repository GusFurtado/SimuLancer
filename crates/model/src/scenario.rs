//! Scenario files: a base model plus a list of changes.
//!
//! ```yaml
//! format: outliers-sim-scenario/0.1
//! name: two-cells-first-available
//! base: two-cells.yaml
//! changes:
//!   sources.arrivals.output: { rule: first_available }
//! ```
//!
//! Each change replaces the value at a dotted path in the base model (list
//! items by index: `servers.mill_b.failures.0.uptime`). The last segment may
//! be a new key of an existing map, e.g. adding `failures` to a server. The
//! merged model is then loaded like any other, and its errors are reported
//! at the change that caused them, in the scenario file.

use serde::Deserialize;
use serde_json::Value;
use serde_saphyr::Spanned;

use crate::diagnostic::{Diagnostic, LoadError, Position};
use crate::raw::OrderedMap;
use crate::{Model, load_str, parse_yaml};

/// The `format:` value of scenario files.
pub const SCENARIO_FORMAT: &str = "outliers-sim-scenario/0.1";

/// A parsed scenario file, before it's applied to its base.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub format: Spanned<String>,
    pub name: Spanned<String>,
    pub description: Option<String>,
    /// Path of the base model, relative to the scenario file.
    pub base: Spanned<String>,
    #[serde(default)]
    changes: OrderedMap<Value>,
}

impl Scenario {
    /// The changes as (dotted path, new value), in file order.
    pub fn changes(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.changes.0.iter().map(|(k, v)| (k.value.as_str(), v))
    }
}

/// Which file a scenario's errors refer to.
#[derive(Debug)]
pub enum ScenarioError {
    /// The scenario file itself is malformed, or a change is invalid.
    Scenario(LoadError),
    /// The base model has errors of its own.
    Base(LoadError),
}

/// True if `source` looks like a scenario file rather than a model.
pub fn is_scenario(source: &str) -> bool {
    #[derive(Deserialize)]
    struct Peek {
        format: Option<String>,
    }
    serde_saphyr::from_str::<Peek>(source)
        .ok()
        .and_then(|p| p.format)
        .is_some_and(|f| f.starts_with("outliers-sim-scenario/"))
}

pub fn parse_scenario(source: &str) -> Result<Scenario, LoadError> {
    let scenario: Scenario = parse_yaml(source)?;
    if scenario.format.value != SCENARIO_FORMAT {
        return Err(LoadError::Invalid(vec![Diagnostic {
            position: Position::from_location(&scenario.format.referenced),
            message: format!(
                "unsupported scenario format `{}` (this version reads `{SCENARIO_FORMAT}`)",
                scenario.format.value
            ),
        }]));
    }
    Ok(scenario)
}

/// Applies a scenario to its base model's source and loads the result.
pub fn load_scenario(scenario: &Scenario, base_source: &str) -> Result<Model, ScenarioError> {
    // The base must be valid on its own; its errors are the base's to fix.
    load_str(base_source).map_err(ScenarioError::Base)?;
    let merged = merge(scenario, base_source).map_err(ScenarioError::Scenario)?;
    load_str(&merged.text).map_err(|err| {
        let diagnostics = err
            .diagnostics()
            .iter()
            .map(|d| locate(scenario, &merged, d))
            .collect();
        ScenarioError::Scenario(LoadError::Invalid(diagnostics))
    })
}

/// The merged model as YAML, and the dotted path each of its lines belongs to.
pub struct Merged {
    pub text: String,
    line_paths: Vec<String>,
}

pub fn merge(scenario: &Scenario, base_source: &str) -> Result<Merged, LoadError> {
    let mut tree: Value = parse_yaml(base_source)?;
    let mut errors = Vec::new();
    for (key, value) in &scenario.changes.0 {
        if let Err(message) = set(&mut tree, &key.value, value.clone()) {
            errors.push(Diagnostic {
                position: Position::from_location(&key.referenced),
                message,
            });
        }
    }
    if !errors.is_empty() {
        return Err(LoadError::Invalid(errors));
    }
    let object = tree.as_object_mut().expect("a valid base model is a map");
    object.insert("name".into(), Value::String(scenario.name.value.clone()));
    if let Some(description) = &scenario.description {
        object.insert("description".into(), Value::String(description.clone()));
    }
    let lines = crate::yaml_out::lines_with_paths(&tree);
    let text = lines
        .iter()
        .map(|(l, _)| l.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    Ok(Merged {
        text,
        line_paths: lines.into_iter().map(|(_, p)| p).collect(),
    })
}

/// Top-level model sections that changes may create when the model doesn't
/// have them yet.
const SECTIONS: &[&str] = &[
    "units",
    "entities",
    "sources",
    "servers",
    "conveyors",
    "queues",
    "transports",
    "vehicles",
    "lines",
    "sinks",
    "resources",
    "schedules",
    "outputs",
    "parts",
];

/// Replaces (or adds, for the last segment of a map) the value at `path`.
/// `null` removes a map entry: how an object or conveyor is deleted.
fn set(tree: &mut Value, path: &str, value: Value) -> Result<(), String> {
    let segments: Vec<&str> = path.split('.').collect();
    if segments.iter().any(|s| s.is_empty()) {
        return Err(format!(
            "`{path}` is not a valid path (expected names separated by dots)"
        ));
    }
    if matches!(segments[0], "format" | "name" | "description") {
        return Err(format!(
            "`{}` can't be changed here; set it at the top of the scenario file",
            segments[0]
        ));
    }
    let mut node = tree;
    for (i, segment) in segments.iter().enumerate() {
        let last = i == segments.len() - 1;
        let so_far = segments[..i].join(".");
        let here = if so_far.is_empty() {
            "the model".to_string()
        } else {
            format!("`{so_far}`")
        };
        node = match node {
            Value::Object(map) => {
                if last {
                    if value.is_null() {
                        map.shift_remove(*segment);
                    } else {
                        map.insert(segment.to_string(), value);
                    }
                    return Ok(());
                }
                // A model section the file doesn't have yet (no resources so
                // far, say) can be started; any other unknown name is a typo.
                if i == 0 && !map.contains_key(*segment) && SECTIONS.contains(segment) {
                    map.insert(segment.to_string(), Value::Object(Default::default()));
                }
                if !map.contains_key(*segment) {
                    let keys: Vec<&str> = map.keys().map(String::as_str).collect();
                    return Err(format!(
                        "{here} has no `{segment}` (it has: {})",
                        keys.join(", ")
                    ));
                }
                map.get_mut(*segment).expect("checked above")
            }
            Value::Array(items) => {
                let len = items.len();
                let Some(item) = segment.parse::<usize>().ok().and_then(|k| items.get_mut(k))
                else {
                    return Err(format!(
                        "{here} is a list of {len}; use an index from 0 to {}",
                        len.saturating_sub(1)
                    ));
                };
                if last {
                    *item = value;
                    return Ok(());
                }
                item
            }
            _ => return Err(format!("{here} is a single value, not a map or list")),
        };
    }
    unreachable!("the loop returns at the last segment")
}

/// Maps an error in the merged model back to the change that caused it.
fn locate(scenario: &Scenario, merged: &Merged, d: &Diagnostic) -> Diagnostic {
    let line_path = d.position.and_then(|p| {
        merged
            .line_paths
            .get(usize::try_from(p.line).ok()?.checked_sub(1)?)
    });
    let change = line_path.and_then(|lp| {
        scenario.changes.0.iter().find(|(k, _)| {
            let c = k.value.as_str();
            lp == c || lp.starts_with(&format!("{c}.")) || c.starts_with(&format!("{lp}."))
        })
    });
    match change {
        Some((key, _)) => Diagnostic {
            position: Position::from_location(&key.referenced),
            message: format!("with this change: {}", d.message),
        },
        // The error isn't inside a changed value, but the base was valid,
        // so a change caused it indirectly (e.g. a removed conveyor).
        None => Diagnostic {
            position: None,
            message: format!(
                "{} (after applying the changes{})",
                d.message,
                line_path.map(|p| format!(", at `{p}`")).unwrap_or_default()
            ),
        },
    }
}
