//! Whether a run has enough replications: the half-width of each key result's
//! confidence interval against the model's precision (`experiment.precision`,
//! ±5% by default), and how many replications would make it narrow enough.
//!
//! The half-width shrinks with the square root of the number of replications,
//! so n replications with half-width h need about n · (h / wanted)² for the
//! wanted one (Law, *Simulation Modeling and Analysis*, the relative-precision
//! rule). It's an estimate: the standard deviation is itself estimated, and the
//! t quantile shrinks a little with more replications.

use serde::Serialize;
use simulancer_engine::spec::Spec;
use simulancer_engine::stats::Summary;

use crate::measures::{Measure, Unit};

/// The precision when the model doesn't set one: ±5% of the mean.
pub const DEFAULT: f64 = 0.05;
/// Shares of time (utilization, blocked...) are judged in points instead, at a
/// fifth of the precision: ±1 point for ±5%. A share near zero would otherwise
/// need endless replications for a relative precision.
const POINTS_PER_PRECISION: f64 = 0.2;

/// A result to judge instead of the sinks' throughput and time in system:
/// the results a project shows on the Simulation page.
pub struct KeyResult {
    pub label: String,
    pub object: String,
    pub measure: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrecisionCheck {
    /// The precision asked for, as a share of the mean (0.05 for ±5%).
    pub target: f64,
    pub replications: u32,
    pub results: Vec<Precision>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Precision {
    pub label: String,
    pub object_type: &'static str,
    pub object: String,
    pub measure: String,
    /// `relative` (a share of the mean) or `points` (shares of time, as a
    /// fraction: 0.01 is one point).
    pub scale: &'static str,
    /// The interval's half-width on that scale, and the one wanted.
    pub half_width: f64,
    pub wanted: f64,
    pub enough: bool,
    /// About how many replications would make it narrow enough, when it isn't.
    pub replications_needed: Option<u64>,
}

impl PrecisionCheck {
    /// Replications that would make every key result narrow enough.
    pub fn replications_needed(&self) -> Option<u64> {
        self.results
            .iter()
            .filter_map(|r| r.replications_needed)
            .max()
    }
}

impl Precision {
    /// "± 6.3%", "± 1.4 points".
    pub fn half_width_text(&self) -> String {
        scaled(self.scale, self.half_width)
    }
}

fn scaled(scale: &str, value: f64) -> String {
    if scale == "points" {
        let n = number(value * 100.0);
        format!("± {n} point{}", if n == "1" { "" } else { "s" })
    } else {
        format!("± {}", percent(value))
    }
}

/// Replications for half-width `wanted`, from `n` with half-width `half_width`.
pub fn needed(n: u32, half_width: f64, wanted: f64) -> u64 {
    let ratio = half_width / wanted;
    // Saturates instead of overflowing for a hopeless ratio.
    (f64::from(n) * ratio * ratio).ceil().min(u64::MAX as f64) as u64
}

/// The check of a run's key results: `chosen` when there are any (a
/// project's results), or else each sink's throughput and time in system.
/// None with a single replication (no intervals).
pub fn check(
    spec: &Spec,
    measures: &[(Measure, Summary)],
    chosen: &[KeyResult],
) -> Option<PrecisionCheck> {
    let e = &spec.experiment;
    if e.replications < 2 {
        return None;
    }
    let target = e.precision.unwrap_or(DEFAULT);
    let find = |object_type: Option<&str>, object: &str, measure: &str| {
        measures.iter().find(|(m, _)| {
            object_type.is_none_or(|t| m.object_type == t)
                && m.object == object
                && m.name == measure
        })
    };
    let mut picks: Vec<(&Measure, &Summary, String)> = Vec::new();
    for k in chosen {
        if let Some((m, s)) = find(None, &k.object, &k.measure) {
            picks.push((m, s, k.label.clone()));
        }
    }
    if chosen.is_empty() {
        for sink in &spec.sinks {
            for name in ["throughput", "time in system"] {
                if let Some((m, s)) = find(Some("sink"), &sink.name, name) {
                    picks.push((m, s, format!("{name} at {}", sink.name)));
                }
            }
        }
    }
    let results: Vec<Precision> = picks
        .into_iter()
        .filter_map(|(m, s, label)| {
            if s.n < 2 || !s.half_width.is_finite() {
                return None;
            }
            let (scale, half_width, wanted) = if m.unit == Unit::Fraction {
                ("points", s.half_width, target * POINTS_PER_PRECISION)
            } else if s.mean.is_finite() && s.mean.abs() > 1e-12 {
                ("relative", s.half_width / s.mean.abs(), target)
            } else {
                // A zero mean has no relative precision.
                return None;
            };
            let enough = half_width <= wanted;
            Some(Precision {
                label,
                object_type: m.object_type,
                object: m.object.clone(),
                measure: m.name.clone(),
                scale,
                half_width,
                wanted,
                enough,
                replications_needed: (!enough).then(|| needed(e.replications, half_width, wanted)),
            })
        })
        .collect();
    Some(PrecisionCheck {
        target,
        replications: e.replications,
        results,
    })
}

/// A share as a short percentage: 0.05 → `5%`, 0.025 → `2.5%`.
pub fn percent(share: f64) -> String {
    format!("{}%", number(share * 100.0))
}

/// No more decimals than needed: 5, 2.5, 0.25.
fn number(v: f64) -> String {
    let places = match v.abs() {
        a if a >= 10.0 => 0,
        a if a >= 1.0 => 1,
        _ => 2,
    };
    let text = format!("{v:.places$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_times_the_replications_halve_the_interval() {
        assert_eq!(needed(10, 0.10, 0.05), 40);
        assert_eq!(needed(30, 0.05, 0.05), 30);
        assert_eq!(needed(20, 0.06, 0.05), 29); // 28.8, rounded up
        assert_eq!(needed(10, f64::INFINITY, 0.05), u64::MAX);
    }

    #[test]
    fn short_percentages() {
        assert_eq!(percent(0.05), "5%");
        assert_eq!(percent(0.025), "2.5%");
        assert_eq!(percent(0.0632), "6.3%");
        assert_eq!(percent(0.13), "13%");
        assert_eq!(percent(0.0025), "0.25%");
        assert_eq!(scaled("points", 0.01), "± 1 point");
        assert_eq!(scaled("points", 0.014), "± 1.4 points");
    }
}
