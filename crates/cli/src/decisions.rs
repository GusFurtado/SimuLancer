//! What managers may change, and what they read: the `decisions` and
//! `results` an engineer exposes in a project's `project.yaml`.
//!
//! ```yaml
//! decisions:
//!   arrival_routing:
//!     label: How new parts are assigned to cells
//!     choices:                       # the first option is the current setup
//!       shortest_queue: { label: To the cell with the shorter queue }
//!       round_robin:
//!         label: Alternate between the cells
//!         changes: { sources.arrivals.output: { rule: round_robin } }
//!   inspectors:
//!     label: Inspectors
//!     path: servers.inspect.capacity   # a number, within bounds
//!     min: 1
//!     max: 3
//! results:
//!   - { label: Parts shipped per hour, object: shipping, measure: throughput, better: higher }
//! ```
//!
//! A manager's scenario is a pick per decision (an option's id, or a number);
//! decisions left out stay as they are. Picks become ordinary model changes.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::measures::{Measure, Unit};
use simulancer_model::Words;

/// Model sections decisions can't touch: comparisons pair replications by
/// their random numbers, which needs the same experiment in every scenario.
const FIXED_SECTIONS: &[&str] = &["format", "name", "description", "experiment", "outputs"];

#[derive(Debug, Clone, Serialize)]
pub struct Decision {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The current setup: the first option's id, or the number in the model.
    pub current: Value,
    #[serde(flatten)]
    pub kind: Kind,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Kind {
    Choice {
        choices: Vec<Choice>,
    },
    Number {
        path: String,
        min: f64,
        max: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        step: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct Choice {
    pub id: String,
    pub label: String,
    pub changes: Map<String, Value>,
}

/// Which way a result should move.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Better {
    Higher,
    Lower,
    /// Neither: the result is shown without judging it.
    #[default]
    Neither,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResultSpec {
    pub label: String,
    pub object: String,
    pub measure: String,
    pub unit: Unit,
    pub better: Better,
}

/// Everything a project exposes to managers.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Exposed {
    pub decisions: Vec<Decision>,
    pub results: Vec<ResultSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDecision {
    label: String,
    description: Option<String>,
    choices: Option<Map<String, Value>>,
    path: Option<String>,
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
    unit: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawChoice {
    label: String,
    #[serde(default)]
    changes: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawResult {
    label: String,
    object: String,
    measure: String,
    #[serde(default)]
    better: Better,
}

/// Checks that changes give a valid model; `Err` says why not.
pub type TryChanges<'a> = &'a dyn Fn(&Map<String, Value>) -> Result<(), String>;

/// The model the decisions apply to, for checking them.
pub struct Target<'a> {
    /// The model's source text.
    pub source: &'a str,
    /// The measures the model reports (values aren't needed).
    pub measures: &'a [Measure],
    pub try_changes: TryChanges<'a>,
    /// What flows through the model, for default result labels.
    pub words: &'a Words,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Reads what a project exposes. Returns what could be read, and every
/// problem found, worded for the engineer. Without a `target` (the model has
/// errors of its own) only the file's structure is checked.
pub fn read(
    decisions: &Map<String, Value>,
    results: &[Value],
    target: Option<&Target>,
) -> (Exposed, Vec<String>) {
    let mut problems = Vec::new();
    let mut exposed = Exposed::default();
    let tree = target.and_then(|t| simulancer_model::to_tree(t.source).ok());

    for (id, raw) in decisions {
        match read_decision(id, raw, tree.as_ref(), target) {
            Ok((decision, mut found)) => {
                problems.append(&mut found);
                exposed.decisions.push(decision);
            }
            Err(problem) => problems.push(problem),
        }
    }
    problems.extend(overlaps(&exposed.decisions));

    let measures = target.map(|t| t.measures);
    for (i, raw) in results.iter().enumerate() {
        let raw: RawResult = match serde_json::from_value(raw.clone()) {
            Ok(r) => r,
            Err(e) => {
                problems.push(format!("result {}: {e}", i + 1));
                continue;
            }
        };
        let unit = match measures {
            None => Unit::Count,
            Some(measures) => {
                match measures
                    .iter()
                    .find(|m| m.object == raw.object && m.name == raw.measure)
                {
                    Some(m) => m.unit,
                    None => {
                        let here: Vec<&str> = measures
                            .iter()
                            .filter(|m| m.object == raw.object)
                            .map(|m| m.name.as_str())
                            .collect();
                        problems.push(if here.is_empty() {
                            format!(
                                "result `{}`: the model reports nothing for `{}`",
                                raw.label, raw.object
                            )
                        } else {
                            format!(
                                "result `{}`: the model doesn't report `{}` for `{}` (it reports: {})",
                                raw.label,
                                raw.measure,
                                raw.object,
                                here.join(", ")
                            )
                        });
                        continue;
                    }
                }
            }
        };
        exposed.results.push(ResultSpec {
            label: raw.label,
            object: raw.object,
            measure: raw.measure,
            unit,
            better: raw.better,
        });
    }
    if results.is_empty()
        && let Some(target) = target
    {
        exposed.results = default_results(target.measures, target.words);
    }
    (exposed, problems)
}

/// Without a `results` list: what every sink delivers, and how long it takes.
fn default_results(measures: &[Measure], words: &Words) -> Vec<ResultSpec> {
    let many = {
        let mut c = words.many.chars();
        c.next().map_or(String::new(), |f| {
            f.to_uppercase().collect::<String>() + c.as_str()
        })
    };
    measures
        .iter()
        .filter(|m| {
            m.object_type == "sink" && matches!(m.name.as_str(), "throughput" | "time in system")
        })
        .map(|m| ResultSpec {
            label: match m.name.as_str() {
                "throughput" => format!(
                    "{many} per {} at {}",
                    match m.unit {
                        Unit::PartsPerDay => "day",
                        Unit::PartsPerWeek => "week",
                        Unit::PartsPerWorkingHour => "working hour",
                        _ => "hour",
                    },
                    m.object
                ),
                "time in system" => format!("Time in system to {}", m.object),
                other => format!("{} {other}", m.object),
            },
            object: m.object.clone(),
            measure: m.name.to_string(),
            unit: m.unit,
            better: match m.name.as_str() {
                "throughput" => Better::Higher,
                "time in system" => Better::Lower,
                _ => Better::Neither,
            },
        })
        .collect()
}

fn read_decision(
    id: &str,
    raw: &Value,
    tree: Option<&Value>,
    target: Option<&Target>,
) -> Result<(Decision, Vec<String>), String> {
    if !valid_id(id) {
        return Err(format!(
            "decision `{id}`: ids are lowercase letters, digits, `_` and `-`"
        ));
    }
    let raw: RawDecision =
        serde_json::from_value(raw.clone()).map_err(|e| format!("decision `{id}`: {e}"))?;
    let mut problems = Vec::new();
    if raw.label.trim().is_empty() {
        problems.push(format!("decision `{id}`: the label is empty"));
    }
    let name = format!("decision `{id}`");
    let check = |changes: &Map<String, Value>, what: &str, problems: &mut Vec<String>| {
        for path in changes.keys() {
            if let Some(section) = FIXED_SECTIONS
                .iter()
                .find(|s| path == *s || path.starts_with(&format!("{s}.")))
            {
                problems.push(format!(
                    "{what}: decisions can't change `{section}` (every scenario must run the same experiment)"
                ));
                return;
            }
        }
        if let Some(target) = target
            && let Err(e) = (target.try_changes)(changes)
        {
            problems.push(format!("{what}: {e}"));
        }
    };

    let decision = match (raw.choices, raw.path) {
        (Some(_), Some(_)) => {
            return Err(format!(
                "{name}: give either `choices` or `path` (a number), not both"
            ));
        }
        (None, None) => {
            return Err(format!(
                "{name}: give `choices` (options to pick from) or `path` (a number to set)"
            ));
        }
        (Some(choices), None) => {
            if raw.min.is_some() || raw.max.is_some() || raw.step.is_some() || raw.unit.is_some() {
                problems.push(format!(
                    "{name}: `min`, `max`, `step` and `unit` are for numbers, not choices"
                ));
            }
            let mut out = Vec::new();
            for (choice_id, value) in &choices {
                if !valid_id(choice_id) {
                    problems.push(format!(
                        "{name}: option ids are lowercase letters, digits, `_` and `-` (`{choice_id}` isn't)"
                    ));
                    continue;
                }
                let choice: RawChoice = match serde_json::from_value(value.clone()) {
                    Ok(c) => c,
                    Err(e) => {
                        problems.push(format!("{name}, option `{choice_id}`: {e}"));
                        continue;
                    }
                };
                let what = format!("{name}, option `{choice_id}`");
                if out.is_empty() {
                    if !choice.changes.is_empty() {
                        problems.push(format!(
                            "{what}: the first option is the current setup, so it can't have changes"
                        ));
                    }
                } else if choice.changes.is_empty() {
                    problems.push(format!(
                        "{what}: only the first option (the current setup) can be without changes"
                    ));
                } else if let Some(tree) = tree
                    && choice
                        .changes
                        .iter()
                        .all(|(path, value)| value_at(tree, path) == Some(value))
                {
                    problems.push(format!(
                        "{what}: the model already does this, so the first option (`{}`) no longer describes the current setup; put the option that matches the model first",
                        out.first().map_or("", |c: &Choice| c.id.as_str())
                    ));
                } else {
                    check(&choice.changes, &what, &mut problems);
                }
                out.push(Choice {
                    id: choice_id.clone(),
                    label: choice.label,
                    changes: choice.changes,
                });
            }
            if choices.len() < 2 {
                problems.push(format!(
                    "{name}: give at least two options (the first is the current setup)"
                ));
            }
            Decision {
                id: id.to_string(),
                label: raw.label,
                description: raw.description,
                current: out.first().map_or(Value::Null, |c| json!(c.id)),
                kind: Kind::Choice { choices: out },
            }
        }
        (None, Some(path)) => {
            let (Some(min), Some(max)) = (raw.min, raw.max) else {
                return Err(format!("{name}: a number needs `min` and `max`"));
            };
            if min >= max {
                problems.push(format!("{name}: `min` must be below `max`"));
            }
            if raw.step.is_some_and(|s| s <= 0.0) {
                problems.push(format!("{name}: `step` must be above zero"));
            }
            let mut current = Value::Null;
            if let Some(tree) = tree {
                match current_number(tree, &path, raw.unit.as_deref()) {
                    Ok(n) => {
                        if n < min || n > max {
                            problems.push(format!(
                                "{name}: the model has {} at `{path}`, outside {} to {}",
                                number_text(n),
                                number_text(min),
                                number_text(max)
                            ));
                        }
                        current = json!(n);
                    }
                    Err(e) => problems.push(format!("{name}: {e}")),
                }
            }
            for bound in [min, max] {
                let mut changes = Map::new();
                changes.insert(path.clone(), number_value(bound, raw.unit.as_deref()));
                check(
                    &changes,
                    &format!("{name} at {}", number_text(bound)),
                    &mut problems,
                );
            }
            Decision {
                id: id.to_string(),
                label: raw.label,
                description: raw.description,
                current,
                kind: Kind::Number {
                    path,
                    min,
                    max,
                    step: raw.step,
                    unit: raw.unit,
                },
            }
        }
    };
    Ok((decision, problems))
}

/// The value at a change path in the model, if it's written there.
fn value_at<'a>(tree: &'a Value, path: &str) -> Option<&'a Value> {
    let mut node = tree;
    for segment in path.split('.') {
        node = match node {
            Value::Object(map) => map.get(segment),
            Value::Array(items) => segment.parse::<usize>().ok().and_then(|i| items.get(i)),
            _ => None,
        }?;
    }
    Some(node)
}

/// The number at `path` in the model: plain (`2`) or with the decision's
/// unit (`8 m`).
fn current_number(tree: &Value, path: &str, unit: Option<&str>) -> Result<f64, String> {
    let node = value_at(tree, path).ok_or_else(|| {
        format!("the model has no `{path}`; write its current value in the model first")
    })?;
    match (node, unit) {
        (Value::Number(n), None) => n.as_f64().ok_or_else(|| "not a number".into()),
        (Value::Number(_), Some(unit)) => Err(format!(
            "`{path}` is written without a unit, but the decision says `unit: {unit}`"
        )),
        (Value::String(s), Some(unit)) => {
            let mut parts = s.split_whitespace();
            match (
                parts.next().and_then(|n| n.parse::<f64>().ok()),
                parts.next(),
                parts.next(),
            ) {
                (Some(n), Some(u), None) if u == unit => Ok(n),
                _ => Err(format!(
                    "`{path}` is `{s}` in the model; write it in {unit} (like `1 {unit}`) to match the decision"
                )),
            }
        }
        (Value::String(s), None) => Err(format!(
            "`{path}` is `{s}` in the model; give the decision a `unit` to match it"
        )),
        (other, _) => Err(format!("`{path}` is {other}, not a number")),
    }
}

/// Rounds away float noise from steps (0.1 + 0.2).
fn tidy(n: f64) -> f64 {
    (n * 1e9).round() / 1e9
}

fn number_text(n: f64) -> String {
    format!("{}", tidy(n))
}

/// A number as the model writes it: `2`, or `8 m` with a unit.
fn number_value(n: f64, unit: Option<&str>) -> Value {
    let n = tidy(n);
    match unit {
        Some(unit) => json!(format!("{} {unit}", number_text(n))),
        None if n.fract() == 0.0 && n.abs() < 1e15 => json!(n as i64),
        None => json!(n),
    }
}

/// Model paths each decision can change.
fn paths(decision: &Decision) -> Vec<&str> {
    match &decision.kind {
        Kind::Choice { choices } => choices
            .iter()
            .flat_map(|c| c.changes.keys().map(String::as_str))
            .collect(),
        Kind::Number { path, .. } => vec![path.as_str()],
    }
}

/// Two decisions changing the same thing would undo each other.
fn overlaps(decisions: &[Decision]) -> Vec<String> {
    let mut problems = Vec::new();
    for (i, a) in decisions.iter().enumerate() {
        for b in &decisions[i + 1..] {
            let clash = paths(a).into_iter().find_map(|p| {
                paths(b).into_iter().find_map(|q| {
                    let related = p == q
                        || p.starts_with(&format!("{q}."))
                        || q.starts_with(&format!("{p}."));
                    related.then_some(if p.len() <= q.len() { p } else { q })
                })
            });
            if let Some(path) = clash {
                problems.push(format!(
                    "decisions `{}` and `{}` both change `{path}`; one decision per thing",
                    a.id, b.id
                ));
            }
        }
    }
    problems
}

impl Exposed {
    /// The model changes for a manager's picks (decision id -> option id or
    /// number). Decisions left out, or picked as they are, change nothing.
    pub fn changes_for(&self, picks: &Map<String, Value>) -> Result<Map<String, Value>, String> {
        let mut changes = Map::new();
        for (id, pick) in picks {
            let decision = self
                .decisions
                .iter()
                .find(|d| &d.id == id)
                .ok_or_else(|| format!("there's no decision `{id}`"))?;
            match &decision.kind {
                Kind::Choice { choices } => {
                    let choice = pick
                        .as_str()
                        .and_then(|p| choices.iter().find(|c| c.id == p))
                        .ok_or_else(|| {
                            let ids: Vec<&str> = choices.iter().map(|c| c.id.as_str()).collect();
                            format!(
                                "`{}` has no option {pick} (options: {})",
                                decision.label,
                                ids.join(", ")
                            )
                        })?;
                    changes.extend(choice.changes.clone());
                }
                Kind::Number {
                    path,
                    min,
                    max,
                    step,
                    unit,
                } => {
                    let n = pick
                        .as_f64()
                        .ok_or_else(|| format!("`{}` needs a number", decision.label))?;
                    if n < *min || n > *max {
                        return Err(format!(
                            "`{}` must be between {} and {}",
                            decision.label,
                            number_text(*min),
                            number_text(*max)
                        ));
                    }
                    if let Some(step) = step {
                        let k = (n - min) / step;
                        if (k - k.round()).abs() > 1e-6 {
                            return Err(format!(
                                "`{}` goes in steps of {} from {}",
                                decision.label,
                                number_text(*step),
                                number_text(*min)
                            ));
                        }
                    }
                    if decision.current.as_f64() != Some(n) {
                        changes.insert(path.clone(), number_value(n, unit.as_deref()));
                    }
                }
            }
        }
        Ok(changes)
    }

    /// What the picks change, in words: "Inspectors: 2 · Routing: Alternate".
    /// Empty when they change nothing.
    pub fn describe(&self, picks: &Map<String, Value>) -> String {
        let mut parts = Vec::new();
        for decision in &self.decisions {
            let Some(pick) = picks.get(&decision.id) else {
                continue;
            };
            if *pick == decision.current
                || (pick.as_f64().is_some() && pick.as_f64() == decision.current.as_f64())
            {
                continue;
            }
            let value = match &decision.kind {
                Kind::Choice { choices } => choices
                    .iter()
                    .find(|c| Some(c.id.as_str()) == pick.as_str())
                    .map_or_else(|| pick.to_string(), |c| c.label.clone()),
                Kind::Number { unit, .. } => match (pick.as_f64(), unit) {
                    (Some(n), Some(u)) => format!("{} {u}", number_text(n)),
                    (Some(n), None) => number_text(n),
                    _ => pick.to_string(),
                },
            };
            parts.push(format!("{}: {value}", decision.label));
        }
        parts.join(" · ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL: &str = "servers:\n  inspect:\n    capacity: 1\nconveyors:\n  c2:\n    length: \"8 m\"\nsources:\n  arrivals:\n    output: { rule: shortest_queue }\n";

    fn measures() -> Vec<Measure> {
        vec![Measure {
            object_type: "sink",
            object: "shipping".into(),
            name: "throughput".into(),
            unit: Unit::PartsPerHour,
            values: vec![],
        }]
    }

    fn yaml(text: &str) -> Value {
        simulancer_model::to_tree(text).unwrap()
    }

    fn read_with(text: &str, fails: &[&str]) -> (Exposed, Vec<String>) {
        let file = yaml(text);
        let decisions = file["decisions"].as_object().cloned().unwrap_or_default();
        let results = file["results"].as_array().cloned().unwrap_or_default();
        let measures = measures();
        let try_changes = |changes: &Map<String, Value>| match changes
            .keys()
            .find(|k| fails.contains(&k.as_str()))
        {
            Some(k) => Err(format!("`{k}` is wrong")),
            None => Ok(()),
        };
        read(
            &decisions,
            &results,
            Some(&Target {
                source: MODEL,
                measures: &measures,
                try_changes: &try_changes,
                words: &Words::default(),
            }),
        )
    }

    const GOOD: &str = r#"
decisions:
  routing:
    label: Routing
    choices:
      shortest_queue: { label: Shorter queue }
      round_robin:
        label: Alternate
        changes: { sources.arrivals.output: { rule: round_robin } }
  inspectors: { label: Inspectors, path: servers.inspect.capacity, min: 1, max: 3 }
  buffer: { label: Buffer, path: conveyors.c2.length, min: 4, max: 24, step: 2, unit: m }
results:
  - { label: Parts per hour, object: shipping, measure: throughput, better: higher }
"#;

    #[test]
    fn reads_choices_numbers_and_results() {
        let (exposed, problems) = read_with(GOOD, &[]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(exposed.decisions.len(), 3);
        assert_eq!(exposed.decisions[0].current, json!("shortest_queue"));
        assert_eq!(exposed.decisions[1].current, json!(1.0));
        assert_eq!(exposed.decisions[2].current, json!(8.0));
        assert_eq!(exposed.results[0].unit, Unit::PartsPerHour);
        assert_eq!(exposed.results[0].better, Better::Higher);
    }

    #[test]
    fn picks_become_changes() {
        let (exposed, _) = read_with(GOOD, &[]);
        let picks = |v: Value| v.as_object().unwrap().clone();
        let changes = exposed
            .changes_for(&picks(
                json!({ "routing": "round_robin", "inspectors": 2, "buffer": 12 }),
            ))
            .unwrap();
        assert_eq!(
            Value::Object(changes),
            json!({ "sources.arrivals.output": { "rule": "round_robin" },
                    "servers.inspect.capacity": 2, "conveyors.c2.length": "12 m" })
        );
        // Current values change nothing.
        let same = picks(json!({ "routing": "shortest_queue", "inspectors": 1 }));
        assert!(exposed.changes_for(&same).unwrap().is_empty());
        assert_eq!(exposed.describe(&same), "");
        assert_eq!(
            exposed.describe(&picks(json!({ "inspectors": 2, "buffer": 12 }))),
            "Inspectors: 2 · Buffer: 12 m"
        );
        for (bad, message) in [
            (json!({ "nope": 1 }), "no decision `nope`"),
            (json!({ "routing": "random" }), "no option"),
            (json!({ "inspectors": 4 }), "between 1 and 3"),
            (json!({ "buffer": 5 }), "steps of 2 from 4"),
            (json!({ "inspectors": "two" }), "needs a number"),
        ] {
            let e = exposed.changes_for(&picks(bad)).unwrap_err();
            assert!(e.contains(message), "{e}");
        }
    }

    #[test]
    fn finds_problems() {
        for (text, fails, expected) in [
            (
                "decisions:\n  d: { label: D, choices: { a: { label: A, changes: { x: 1 } }, b: { label: B, changes: { y: 1 } } } }",
                &[][..],
                "the first option is the current setup",
            ),
            (
                "decisions:\n  d: { label: D, choices: { a: { label: A } } }",
                &[],
                "at least two options",
            ),
            (
                "decisions:\n  d: { label: D, choices: { a: { label: A }, b: { label: B, changes: { experiment.seed: 1 } } } }",
                &[],
                "can't change `experiment`",
            ),
            (
                "decisions:\n  d: { label: D, choices: { a: { label: A }, b: { label: B, changes: { servers.x: 1 } } } }",
                &["servers.x"],
                "option `b`: `servers.x` is wrong",
            ),
            (
                "decisions:\n  d: { label: D, choices: { a: { label: A }, b: { label: B, changes: { sources.arrivals.output: { rule: shortest_queue } } } } }",
                &[],
                "the model already does this",
            ),
            (
                "decisions:\n  d: { label: D, path: servers.inspect.capacity, min: 2, max: 3 }",
                &[],
                "outside 2 to 3",
            ),
            (
                "decisions:\n  d: { label: D, path: servers.mill.capacity, min: 1, max: 3 }",
                &[],
                "has no `servers.mill.capacity`",
            ),
            (
                "decisions:\n  d: { label: D, path: conveyors.c2.length, min: 1, max: 30 }",
                &[],
                "give the decision a `unit`",
            ),
            (
                "decisions:\n  d: { label: D, path: conveyors.c2.length, min: 1, max: 30, unit: ft }",
                &[],
                "write it in ft",
            ),
            (
                "decisions:\n  a: { label: A, path: servers.inspect.capacity, min: 1, max: 3 }\n  b: { label: B, choices: { x: { label: X }, y: { label: Y, changes: { servers.inspect: {} } } } }",
                &[],
                "both change `servers.inspect`",
            ),
            (
                "decisions:\n  d: { label: D, path: x, min: 1, max: 3, choices: {} }",
                &[],
                "not both",
            ),
            (
                "decisions:\n  d: { label: D, colour: red, path: x, min: 1, max: 2 }",
                &[],
                "unknown field `colour`",
            ),
            (
                "results:\n  - { label: L, object: shipping, measure: speed }",
                &[],
                "doesn't report `speed` for `shipping` (it reports: throughput)",
            ),
            (
                "results:\n  - { label: L, object: mill, measure: busy }",
                &[],
                "reports nothing for `mill`",
            ),
        ] {
            let (_, problems) = read_with(text, fails);
            assert!(
                problems.iter().any(|p| p.contains(expected)),
                "{text}\n{problems:?}"
            );
        }
    }

    #[test]
    fn default_results_are_what_sinks_deliver() {
        let (exposed, problems) = read_with("decisions: {}", &[]);
        assert!(problems.is_empty());
        assert_eq!(exposed.results[0].label, "Parts per hour at shipping");
        assert_eq!(exposed.results[0].better, Better::Higher);
    }
}
