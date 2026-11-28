//! Expected results (`expect:` in a model): each names a measure and the
//! range its mean, or its whole 95% interval, should stay in. Checked after
//! every run; `simulancer test` fails when one isn't met, so a model can be
//! tested in CI like code.

use serde::Serialize;
use simulancer_engine::stats::Summary;
use simulancer_model::{Bound, Expectation, Model};

use crate::measures::{self, Measure, Unit};
use crate::report::Format;

/// An expectation with its limits in the measure's standard units.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub label: String,
    pub object_type: &'static str,
    pub object: String,
    pub measure: String,
    pub unit: Unit,
    pub at_least: Option<f64>,
    pub at_most: Option<f64>,
    pub whole_interval: bool,
}

/// How an expectation came out in a run.
#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub label: String,
    pub object_type: &'static str,
    pub object: String,
    pub measure: String,
    pub unit: &'static str,
    pub at_least: Option<f64>,
    pub at_most: Option<f64>,
    /// `mean` or `interval`.
    pub check: &'static str,
    pub mean: Option<f64>,
    pub half_width: Option<f64>,
    pub met: bool,
    /// The result now, for people: "55.9 /h ± 0.3 /h".
    pub now: String,
    /// What was expected: "the mean at least 54.0 /h".
    pub expected: String,
    /// Both in one line, with the verdict.
    pub text: String,
}

/// The model's expectations with their limits read for each measure's unit,
/// or what's wrong with them (an unknown measure, a limit in the wrong kind
/// of unit). Needs no results: the measures a model reports are known before
/// it runs.
pub fn resolve(model: &Model) -> Result<Vec<Resolved>, Vec<String>> {
    let known = measures::collect(&model.spec, &model.outputs, &[]);
    let mut out = Vec::new();
    let mut problems = Vec::new();
    for e in &model.expectations {
        let at = e.line.map(|l| format!("line {l}: ")).unwrap_or_default();
        let Some(m) = known
            .iter()
            .find(|m| m.object == e.object && m.name == e.measure)
        else {
            let of_object: Vec<&str> = known
                .iter()
                .filter(|m| m.object == e.object)
                .map(|m| m.name.as_str())
                .collect();
            problems.push(if of_object.is_empty() {
                format!(
                    "{at}expected results: no object `{}` reports results",
                    e.object
                )
            } else {
                format!(
                    "{at}expected results: `{}` has no measure `{}` (it has: {})",
                    e.object,
                    e.measure,
                    of_object.join(", ")
                )
            });
            continue;
        };
        let read = |b: &Option<Bound>, what: &str| -> Result<Option<f64>, String> {
            let Some(b) = b else { return Ok(None) };
            read_value(m.unit, b).map(Some).map_err(|wanted| {
                format!(
                    "{at}expected results: `{what}` for `{}` {} is {wanted}, got `{}`",
                    e.object, e.measure, b.text
                )
            })
        };
        match (read(&e.at_least, "at_least"), read(&e.at_most, "at_most")) {
            (Ok(lo), Ok(hi)) => {
                if let (Some(lo), Some(hi)) = (lo, hi)
                    && lo > hi
                {
                    problems.push(format!(
                        "{at}expected results: `at_least` is above `at_most` for `{}` {}",
                        e.object, e.measure
                    ));
                    continue;
                }
                out.push(Resolved {
                    label: label(e),
                    object_type: m.object_type,
                    object: m.object.clone(),
                    measure: m.name.clone(),
                    unit: m.unit,
                    at_least: lo,
                    at_most: hi,
                    whole_interval: e.whole_interval,
                });
            }
            (a, b) => problems.extend([a.err(), b.err()].into_iter().flatten()),
        }
    }
    if problems.is_empty() {
        Ok(out)
    } else {
        Err(problems)
    }
}

/// A value as written, read for a measure's unit: a time for seconds, a
/// percentage for shares of time, a plain number otherwise. The error says
/// what was wanted.
pub fn read_value(unit: Unit, b: &Bound) -> Result<f64, &'static str> {
    let (value, wanted) = match unit {
        Unit::Seconds => (b.seconds, "a time, like `30 min`"),
        Unit::Fraction => (b.share, "a percentage, like `85%`"),
        _ => (b.number, "a plain number"),
    };
    value.ok_or(wanted)
}

fn label(e: &Expectation) -> String {
    e.label
        .clone()
        .unwrap_or_else(|| format!("{} at {}", e.measure, e.object))
}

/// Checks the expectations against a run's measures.
pub fn check(expected: &[Resolved], measures: &[(Measure, Summary)]) -> Vec<Outcome> {
    expected
        .iter()
        .map(|r| {
            let summary = measures
                .iter()
                .find(|(m, _)| {
                    m.object_type == r.object_type && m.object == r.object && m.name == r.measure
                })
                .map(|(_, s)| s);
            let mean = summary.map(|s| s.mean).filter(|v| v.is_finite());
            let half_width = summary.map(|s| s.half_width).filter(|v| v.is_finite());
            // Without an interval (one replication), the mean is all there is.
            let (low, high) = match (mean, half_width) {
                (Some(m), Some(h)) if r.whole_interval => (m - h, m + h),
                (Some(m), _) => (m, m),
                (None, _) => (f64::NAN, f64::NAN),
            };
            let met = mean.is_some()
                && r.at_least.is_none_or(|lo| low >= lo)
                && r.at_most.is_none_or(|hi| high <= hi);
            let format = Format::for_measure(r.unit, mean.unwrap_or(0.0));
            let now = match (mean, half_width) {
                (Some(m), Some(h)) => format!("{} ± {}", format.value(m), format.value(h)),
                (Some(m), None) => format.value(m),
                (None, _) => "no value".to_string(),
            };
            let range = match (r.at_least, r.at_most) {
                (Some(lo), Some(hi)) => {
                    format!("between {} and {}", format.value(lo), format.value(hi))
                }
                (Some(lo), None) => format!("at least {}", format.value(lo)),
                (None, Some(hi)) => format!("at most {}", format.value(hi)),
                (None, None) => unreachable!("an expectation has a limit"),
            };
            let what = if r.whole_interval {
                "the whole interval"
            } else {
                "the mean"
            };
            Outcome {
                label: r.label.clone(),
                object_type: r.object_type,
                object: r.object.clone(),
                measure: r.measure.clone(),
                unit: r.unit.file_name(),
                at_least: r.at_least,
                at_most: r.at_most,
                check: if r.whole_interval { "interval" } else { "mean" },
                mean,
                half_width,
                met,
                text: format!(
                    "{}: {now}, expected {what} {range} · {}",
                    r.label,
                    if met { "met" } else { "NOT MET" }
                ),
                expected: format!("{what} {range}"),
                now,
            }
        })
        .collect()
}
