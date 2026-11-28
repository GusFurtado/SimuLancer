//! Observed values (`observed:` in a model): what the real system did, one
//! value per period, compared with the model after every run to help
//! validate it.
//!
//! The verdict is *consistent*, *close* or *off*:
//! - with several observed values, consistent when the 95% interval of the
//!   difference between their mean and the model's (Welch) includes zero;
//! - with one value, consistent when it falls inside the range one
//!   replication of the model would give 95% of the time (a prediction
//!   interval: mean ± t · sd · √(1 + 1/n));
//! - otherwise close when within `close_within` of the model's mean (10% by
//!   default; shares of time in points, at a fifth of it), and off beyond.

use serde::Serialize;
use simulancer_engine::stats::{Summary, t_quantile};
use simulancer_model::{Model, Observation};

use crate::expect::read_value;
use crate::measures::{self, Measure, Unit};
use crate::report::Format;

/// How far from the model's mean still counts as close, when the model
/// doesn't say: 10%.
pub const CLOSE_WITHIN: f64 = 0.10;

/// An observation with its values in the measure's standard units.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub label: String,
    pub object_type: &'static str,
    pub object: String,
    pub measure: String,
    pub unit: Unit,
    pub values: Vec<f64>,
    pub source: Option<String>,
    pub close_within: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub label: String,
    pub object_type: &'static str,
    pub object: String,
    pub measure: String,
    pub unit: &'static str,
    pub source: Option<String>,
    /// The observed values, their mean and how many.
    pub values: Vec<f64>,
    pub observed_mean: f64,
    pub model_mean: Option<f64>,
    pub model_half_width: Option<f64>,
    /// observed − model, and its 95% interval (Welch, or the prediction
    /// interval for one value).
    pub difference: Option<f64>,
    pub low: Option<f64>,
    pub high: Option<f64>,
    /// `consistent`, `close` or `off`; `unknown` without a model value.
    pub verdict: &'static str,
    /// For people: "60.2 /h (mean of 4)", "60.1 /h ± 0.3 /h".
    pub observed_text: String,
    pub model_text: Option<String>,
    pub text: String,
}

/// The model's observations, read for each measure's unit, or what's wrong.
pub fn resolve(model: &Model) -> Result<Vec<Resolved>, Vec<String>> {
    let known = measures::collect(&model.spec, &model.outputs, &[]);
    let mut out = Vec::new();
    let mut problems = Vec::new();
    for o in &model.observations {
        let at = o.line.map(|l| format!("line {l}: ")).unwrap_or_default();
        let Some(m) = known
            .iter()
            .find(|m| m.object == o.object && m.name == o.measure)
        else {
            let of_object: Vec<&str> = known
                .iter()
                .filter(|m| m.object == o.object)
                .map(|m| m.name.as_str())
                .collect();
            problems.push(if of_object.is_empty() {
                format!(
                    "{at}observed values: no object `{}` reports results",
                    o.object
                )
            } else {
                format!(
                    "{at}observed values: `{}` has no measure `{}` (it has: {})",
                    o.object,
                    o.measure,
                    of_object.join(", ")
                )
            });
            continue;
        };
        let mut values = Vec::with_capacity(o.values.len());
        for b in &o.values {
            match read_value(m.unit, b) {
                Ok(v) => values.push(v),
                Err(wanted) => problems.push(format!(
                    "{at}observed values of `{}` {} are each {wanted}, got `{}`",
                    o.object, o.measure, b.text
                )),
            }
        }
        if values.len() == o.values.len() {
            out.push(Resolved {
                label: label(o),
                object_type: m.object_type,
                object: m.object.clone(),
                measure: m.name.clone(),
                unit: m.unit,
                values,
                source: o.source.clone(),
                close_within: o.close_within.unwrap_or(CLOSE_WITHIN),
            });
        }
    }
    if problems.is_empty() {
        Ok(out)
    } else {
        Err(problems)
    }
}

fn label(o: &Observation) -> String {
    o.label
        .clone()
        .unwrap_or_else(|| format!("{} at {}", o.measure, o.object))
}

/// Compares the observations with a run's measures.
pub fn compare(observed: &[Resolved], measures: &[(Measure, Summary)]) -> Vec<Outcome> {
    observed
        .iter()
        .map(|r| {
            let k = r.values.len() as f64;
            let observed_mean = r.values.iter().sum::<f64>() / k;
            let summary = measures
                .iter()
                .find(|(m, _)| {
                    m.object_type == r.object_type && m.object == r.object && m.name == r.measure
                })
                .map(|(_, s)| s)
                .filter(|s| s.mean.is_finite());
            let format = Format::for_measure(r.unit, summary.map_or(observed_mean, |s| s.mean));
            let observed_text = format!(
                "{}{}",
                format.value(observed_mean),
                if r.values.len() > 1 {
                    format!(" (mean of {})", r.values.len())
                } else {
                    String::new()
                }
            );
            let Some(s) = summary else {
                return Outcome {
                    label: r.label.clone(),
                    object_type: r.object_type,
                    object: r.object.clone(),
                    measure: r.measure.clone(),
                    unit: r.unit.file_name(),
                    source: r.source.clone(),
                    values: r.values.clone(),
                    observed_mean,
                    model_mean: None,
                    model_half_width: None,
                    difference: None,
                    low: None,
                    high: None,
                    verdict: "unknown",
                    text: format!(
                        "{}: observed {observed_text}; the model has no value",
                        r.label
                    ),
                    observed_text,
                    model_text: None,
                };
            };
            let n = s.n as f64;
            let difference = observed_mean - s.mean;
            // The interval the difference should fall in if model and data agree.
            let half = if r.values.len() >= 2 && s.n >= 2 {
                let var_o = r
                    .values
                    .iter()
                    .map(|v| (v - observed_mean).powi(2))
                    .sum::<f64>()
                    / (k - 1.0);
                let (a, b) = (var_o / k, s.sd * s.sd / n);
                let df = (a + b).powi(2) / (a * a / (k - 1.0) + b * b / (n - 1.0));
                if a + b > 0.0 && df.is_finite() {
                    t_quantile(0.975, df.max(1.0)) * (a + b).sqrt()
                } else {
                    0.0
                }
            } else if s.n >= 2 {
                t_quantile(0.975, n - 1.0) * s.sd * (1.0 + 1.0 / n).sqrt()
            } else {
                0.0
            };
            let (low, high) = (difference - half, difference + half);
            let close = if r.unit == Unit::Fraction {
                r.close_within * 0.2
            } else {
                r.close_within * s.mean.abs()
            };
            let verdict = if low <= 0.0 && 0.0 <= high {
                "consistent"
            } else if difference.abs() <= close {
                "close"
            } else {
                "off"
            };
            let model_text = if s.half_width.is_finite() {
                format!("{} ± {}", format.value(s.mean), format.value(s.half_width))
            } else {
                format.value(s.mean)
            };
            let how = match verdict {
                "consistent" if r.values.len() >= 2 => "consistent: no clear difference",
                "consistent" => "consistent: within the model's spread",
                "close" => "close",
                _ => "off",
            };
            Outcome {
                label: r.label.clone(),
                object_type: r.object_type,
                object: r.object.clone(),
                measure: r.measure.clone(),
                unit: r.unit.file_name(),
                source: r.source.clone(),
                values: r.values.clone(),
                observed_mean,
                model_mean: Some(s.mean),
                model_half_width: s.half_width.is_finite().then_some(s.half_width),
                difference: Some(difference),
                low: Some(low),
                high: Some(high),
                verdict,
                text: format!(
                    "{}: observed {observed_text}, model {model_text} · {how}",
                    r.label
                ),
                observed_text,
                model_text: Some(model_text),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(values: &[f64], unit: Unit) -> Resolved {
        Resolved {
            label: "x".into(),
            object_type: "sink",
            object: "shipping".into(),
            measure: "throughput".into(),
            unit,
            values: values.to_vec(),
            source: None,
            close_within: CLOSE_WITHIN,
        }
    }

    fn run_of(values: &[f64], unit: Unit) -> Vec<(Measure, Summary)> {
        let m = Measure {
            object_type: "sink",
            object: "shipping".into(),
            name: "throughput".into(),
            unit,
            values: values.to_vec(),
        };
        let s = Summary::of(&m.values, 0.95);
        vec![(m, s)]
    }

    #[test]
    fn verdicts() {
        let model = run_of(&[54.0, 56.0, 55.0, 57.0, 53.0, 55.0], Unit::PartsPerHour);
        let verdict =
            |values: &[f64]| compare(&[resolved(values, Unit::PartsPerHour)], &model)[0].verdict;
        // Several values whose mean agrees with the model's.
        assert_eq!(verdict(&[54.5, 55.8, 55.2]), "consistent");
        // One value inside the spread of one replication.
        assert_eq!(verdict(&[57.5]), "consistent");
        // Clearly different, but within 10% of 55.
        assert_eq!(verdict(&[59.0, 59.5, 60.0, 59.2]), "close");
        // Far off.
        assert_eq!(verdict(&[70.0, 71.0, 69.0]), "off");
        assert_eq!(verdict(&[40.0]), "off");
    }

    #[test]
    fn shares_of_time_are_close_within_points() {
        let model = run_of(&[0.80, 0.81, 0.79, 0.80, 0.80], Unit::Fraction);
        let verdict =
            |values: &[f64]| compare(&[resolved(values, Unit::Fraction)], &model)[0].verdict;
        // 1.5 points away: close (within 2 points, a fifth of 10%).
        assert_eq!(verdict(&[0.815, 0.815, 0.816, 0.814]), "close");
        assert_eq!(verdict(&[0.90, 0.91, 0.89]), "off");
    }
}
