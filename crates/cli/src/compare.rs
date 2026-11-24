//! Comparing two scenarios with common random numbers.
//!
//! Replication r of both scenarios uses the same random streams (they're
//! derived from seed, replication and object names), so the two runs see the
//! same arrivals, process times and failures wherever the models agree. The
//! difference is then measured per replication (paired), which removes most of
//! the run-to-run noise from the comparison.

use simulancer_engine::stats::{Summary, t_quantile};

use crate::measures::Measure;
use crate::output::CONFIDENCE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The scenario is higher, and the confidence interval excludes zero.
    Higher,
    Lower,
    /// The confidence interval includes zero.
    NoClearDifference,
    /// Every replication gave exactly the same value.
    Identical,
    /// Too few replications to say.
    Unknown,
}

pub struct Comparison {
    pub base: Measure,
    pub base_summary: Summary,
    pub scenario_summary: Summary,
    /// Of scenario − base, per replication.
    pub difference: Summary,
    /// The CI half-width an unpaired comparison (independent random numbers)
    /// would have had, by Welch's method. For showing what pairing buys.
    pub unpaired_half_width: f64,
    pub verdict: Verdict,
}

/// Pairs up measures present in both scenarios (same object type, object
/// and measure), in the base's order.
pub fn compare(base: Vec<Measure>, scenario: Vec<Measure>) -> Vec<Comparison> {
    base.into_iter()
        .filter_map(|b| {
            let s = scenario.iter().find(|s| {
                s.object_type == b.object_type && s.object == b.object && s.name == b.name
            })?;
            Some(compare_pair(b, s))
        })
        .collect()
}

/// One measure in both scenarios, replication by replication.
pub fn compare_pair(b: Measure, s: &Measure) -> Comparison {
    let differences: Vec<f64> = b
        .values
        .iter()
        .zip(&s.values)
        .map(|(x, y)| {
            if x.is_finite() && y.is_finite() {
                y - x
            } else {
                f64::NAN
            }
        })
        .collect();
    let difference = Summary::of(&differences, CONFIDENCE);
    let base_summary = Summary::of(&b.values, CONFIDENCE);
    let scenario_summary = Summary::of(&s.values, CONFIDENCE);
    let verdict = if difference.n < 2 {
        Verdict::Unknown
    } else if differences
        .iter()
        .filter(|d| d.is_finite())
        .all(|d| *d == 0.0)
    {
        Verdict::Identical
    } else if difference.mean.abs() <= difference.half_width {
        Verdict::NoClearDifference
    } else if difference.mean > 0.0 {
        Verdict::Higher
    } else {
        Verdict::Lower
    };
    Comparison {
        unpaired_half_width: welch_half_width(&base_summary, &scenario_summary),
        base: b,
        base_summary,
        scenario_summary,
        difference,
        verdict,
    }
}

fn welch_half_width(a: &Summary, b: &Summary) -> f64 {
    if a.n < 2 || b.n < 2 {
        return f64::NAN;
    }
    let (va, vb) = (a.sd * a.sd / a.n as f64, b.sd * b.sd / b.n as f64);
    let se = (va + vb).sqrt();
    if se == 0.0 {
        return 0.0;
    }
    let df = (va + vb).powi(2) / (va * va / (a.n - 1) as f64 + vb * vb / (b.n - 1) as f64);
    t_quantile(0.5 + CONFIDENCE / 2.0, df) * se
}

/// Median of (unpaired / paired) CI width over measures where both are
/// meaningful: how much narrower common random numbers made the intervals.
pub fn median_precision_gain(comparisons: &[Comparison]) -> Option<f64> {
    let mut ratios: Vec<f64> = comparisons
        .iter()
        .filter(|c| c.difference.half_width > 0.0 && c.unpaired_half_width > 0.0)
        .map(|c| c.unpaired_half_width / c.difference.half_width)
        .collect();
    if ratios.is_empty() {
        return None;
    }
    ratios.sort_by(f64::total_cmp);
    Some(ratios[ratios.len() / 2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measures::Unit;

    fn measure(values: &[f64]) -> Measure {
        Measure {
            object_type: "server",
            object: "m".into(),
            name: "busy".into(),
            unit: Unit::Fraction,
            values: values.to_vec(),
        }
    }

    #[test]
    fn paired_differences_cancel_shared_noise() {
        // Large run-to-run noise, small consistent improvement of 0.01.
        let base = [0.50, 0.70, 0.40, 0.90, 0.60, 0.80];
        let scenario: Vec<f64> = base.iter().map(|v| v - 0.01).collect();
        let c = &compare(vec![measure(&base)], vec![measure(&scenario)])[0];
        assert!((c.difference.mean + 0.01).abs() < 1e-12);
        assert!(c.difference.half_width < 1e-9); // the same shift every time
        assert_eq!(c.verdict, Verdict::Lower);
        // Unpaired, the noise would swamp it.
        assert!(c.unpaired_half_width > 0.1);
    }

    #[test]
    fn verdicts() {
        let same = compare(vec![measure(&[1.0, 2.0])], vec![measure(&[1.0, 2.0])]);
        assert_eq!(same[0].verdict, Verdict::Identical);
        let noisy = compare(
            vec![measure(&[1.0, 2.0, 3.0])],
            vec![measure(&[1.5, 1.5, 3.1])],
        );
        assert_eq!(noisy[0].verdict, Verdict::NoClearDifference);
        let one = compare(vec![measure(&[1.0])], vec![measure(&[2.0])]);
        assert_eq!(one[0].verdict, Verdict::Unknown);
    }

    #[test]
    fn only_shared_measures_are_compared() {
        let mut other = measure(&[1.0, 2.0]);
        other.object = "elsewhere".into();
        assert!(compare(vec![measure(&[1.0, 2.0])], vec![other]).is_empty());
    }
}
