//! Comparisons in plain language, for people who don't read confidence
//! intervals: "Parts shipped per hour: 60.1 /h → 62.3 /h, up 3.7% (between
//! 3.2% and 4.2%) · better".
//!
//! The range is the 95% confidence interval of the paired difference. As a
//! percentage, it is divided by the current setup's mean, which ignores the
//! (small, with common random numbers) uncertainty of that mean. Shares of
//! time (utilization) change in percentage points instead.

use serde_json::{Value, json};

use crate::compare::{Comparison, Verdict};
use crate::decisions::{Better, ResultSpec};
use crate::measures::Unit;
use crate::report::Format;

/// Clear changes smaller than these are reported as too small to matter.
const SMALL_RELATIVE: f64 = 0.01;
const SMALL_POINTS: f64 = 0.01;

/// Decimals for percentages (or points) that show the smallest of them: one
/// sentence uses the same decimals throughout.
fn decimals(values: &[f64]) -> usize {
    let smallest = values
        .iter()
        .map(|v| v.abs())
        .filter(|v| *v > 0.0 && v.is_finite())
        .fold(f64::INFINITY, f64::min);
    match smallest {
        s if !s.is_finite() => 1,
        s if s >= 10.0 => 0,
        s if s >= 1.0 => 1,
        s if s >= 0.1 => 2,
        _ => 3,
    }
}

/// A result of one scenario against the current setup, as JSON with the
/// numbers, the words, and the whole sentence.
pub fn describe(spec: &ResultSpec, c: &Comparison) -> Value {
    let base = c.base_summary.mean;
    let format = Format::for_measure(spec.unit, base);
    let current_text = format.value(base);
    let scenario_text = format.value(c.scenario_summary.mean);
    let (d, hw) = (c.difference.mean, c.difference.half_width);
    let (low, high) = (d - hw, d + hw);

    // How changes are written: relative (%), or absolute for shares of time
    // and when the current value is zero.
    let relative = spec.unit != Unit::Fraction && base.abs() > 1e-12 && base.is_finite();
    let scale = if relative { 100.0 / base.abs() } else { 100.0 };
    // Enough decimals for the smallest number, and for the range's ends to differ.
    let mut places = decimals(&[d * scale, low * scale, high * scale]);
    while places < 3
        && hw > 0.0
        && format!("{:.*}", places, low * scale) == format!("{:.*}", places, high * scale)
    {
        places += 1;
    }
    let points = spec.unit == Unit::Fraction;
    // A magnitude without its unit, and with it.
    let number = |v: f64| -> String {
        if relative || points {
            format!("{:.*}", places, v * scale)
        } else {
            let text = format.difference().signed(v);
            text.trim_start_matches(['+', '-']).to_string()
        }
    };
    let with_unit = |text: String| -> String {
        if relative {
            text + "%"
        } else if points {
            let one = text.trim_end_matches('0').trim_end_matches('.') == "1";
            text + if one { " point" } else { " points" }
        } else {
            text
        }
    };
    let amount = |v: f64| with_unit(number(v));
    // A range: "between 3.2% and 4.2%", "between 0.2 and 0.3 points".
    let between = |a: String, b: String| -> String {
        if points {
            format!("between {a} and {}", with_unit(b))
        } else {
            format!("between {} and {}", with_unit(a), with_unit(b))
        }
    };
    let signed = |v: f64| -> String {
        let sign = if v < 0.0 { "−" } else { "+" };
        format!("{sign}{}", number(v.abs()))
    };
    // Too small to show with these decimals.
    let invisible = number(low.abs().max(high.abs()))
        .trim_end_matches(|c: char| !c.is_ascii_digit())
        .parse::<f64>()
        .is_ok_and(|v| v == 0.0);

    // A clear change can still be too small to matter: under 1% (or 1
    // point). It's reported, but not judged better or worse.
    let small = match c.verdict {
        Verdict::Higher | Verdict::Lower if relative => (d / base).abs() < SMALL_RELATIVE,
        Verdict::Higher | Verdict::Lower if points => d.abs() < SMALL_POINTS,
        _ => false,
    };
    let judgment = match (c.verdict, spec.better) {
        _ if small => None,
        (Verdict::Higher, Better::Higher) | (Verdict::Lower, Better::Lower) => Some("better"),
        (Verdict::Higher, Better::Lower) | (Verdict::Lower, Better::Higher) => Some("worse"),
        _ => None,
    };
    let (change, range) = match c.verdict {
        Verdict::Unknown => ("can't tell".to_string(), None),
        Verdict::Identical => ("the same in every replication".to_string(), None),
        Verdict::NoClearDifference if invisible && (relative || points) => (
            "no clear change".to_string(),
            Some(format!(
                "less than {} either way",
                with_unit(format!("{:.*}", places, 10f64.powi(-(places as i32))))
            )),
        ),
        Verdict::NoClearDifference => (
            "no clear change".to_string(),
            Some(between(signed(low), signed(high))),
        ),
        Verdict::Higher => (
            format!("up {}", amount(d)),
            Some(between(number(low), number(high))),
        ),
        Verdict::Lower => (
            format!("down {}", amount(-d)),
            Some(between(number(-high), number(-low))),
        ),
    };
    let mut sentence = match c.verdict {
        Verdict::Identical => format!("{}: {current_text}, {change}", spec.label),
        Verdict::Unknown => format!(
            "{}: {current_text} → {scenario_text}; one replication isn't enough to tell",
            spec.label
        ),
        _ => format!("{}: {current_text} → {scenario_text}, {change}", spec.label),
    };
    if let Some(range) = &range {
        sentence += &format!(" ({range})");
    }
    if small {
        sentence += " · too small to matter";
    }
    if let Some(judgment) = judgment {
        sentence += &format!(" · {judgment}");
    }

    let stats = |s: &simulancer_engine::stats::Summary| json!({ "mean": finite(s.mean), "half_width": finite(s.half_width) });
    json!({
        "label": spec.label,
        "object": spec.object,
        "measure": spec.measure,
        "unit": spec.unit,
        "better": spec.better,
        "current": stats(&c.base_summary),
        "scenario": stats(&c.scenario_summary),
        "change": { "mean": finite(d), "low": finite(low), "high": finite(high) },
        "relative": relative.then(|| json!({
            "mean": finite(d / base.abs()), "low": finite(low / base.abs()), "high": finite(high / base.abs()),
        })),
        "verdict": match c.verdict {
            Verdict::Higher => "higher",
            Verdict::Lower => "lower",
            Verdict::NoClearDifference => "no_clear_change",
            Verdict::Identical => "identical",
            Verdict::Unknown => "unknown",
        },
        "judgment": judgment,
        "small": small,
        "text": { "current": current_text, "scenario": scenario_text, "change": change, "range": range },
        "sentence": sentence,
    })
}

/// JSON has no NaN.
fn finite(v: f64) -> Value {
    if v.is_finite() { json!(v) } else { Value::Null }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::compare_pair;
    use crate::measures::Measure;

    fn spec(unit: Unit, better: Better) -> ResultSpec {
        ResultSpec {
            label: "Parts shipped per hour".into(),
            object: "shipping".into(),
            measure: "throughput".into(),
            unit,
            better,
        }
    }

    fn sentence(unit: Unit, better: Better, base: &[f64], scenario: &[f64]) -> String {
        let m = |values: &[f64]| Measure {
            object_type: "sink",
            object: "shipping".into(),
            name: "throughput".into(),
            unit,
            values: values.to_vec(),
        };
        let c = compare_pair(m(base), &m(scenario));
        describe(&spec(unit, better), &c)["sentence"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn sentences() {
        let base = [60.0, 58.0, 62.0, 61.0];
        let up: Vec<f64> = base
            .iter()
            .zip([2.1, 2.3, 2.2, 2.4])
            .map(|(a, b)| a + b)
            .collect();
        assert_eq!(
            sentence(Unit::PartsPerHour, Better::Higher, &base, &up),
            "Parts shipped per hour: 60.2 /h → 62.5 /h, up 3.7% (between 3.4% and 4.1%) · better"
        );
        let down: Vec<f64> = base.iter().map(|v| v * 0.9).collect();
        let s = sentence(Unit::PartsPerHour, Better::Higher, &base, &down);
        assert!(s.contains("down 10.0% (between 9.5% and 10.5%)"), "{s}");
        assert!(s.ends_with("· worse"), "{s}");
        let s = sentence(
            Unit::PartsPerHour,
            Better::Higher,
            &base,
            &[61.0, 57.0, 63.0, 60.0],
        );
        assert!(s.contains("no clear change (between −"), "{s}");
        assert!(s.contains("and +"), "{s}");
        assert!(!s.contains("better") && !s.contains("worse"), "{s}");
        assert_eq!(
            sentence(Unit::PartsPerHour, Better::Higher, &base, &base),
            "Parts shipped per hour: 60.2 /h, the same in every replication"
        );
        let tiny: Vec<f64> = base.iter().map(|v| v * 1.002).collect();
        let s = sentence(Unit::PartsPerHour, Better::Higher, &base, &tiny);
        assert!(s.ends_with("· too small to matter"), "{s}");
        assert!(!s.contains("better"), "{s}");
        assert!(
            sentence(Unit::PartsPerHour, Better::Higher, &[60.0], &[61.0])
                .contains("one replication isn't enough")
        );
    }

    #[test]
    fn shares_of_time_change_in_points() {
        let s = sentence(
            Unit::Fraction,
            Better::Neither,
            &[0.70, 0.72, 0.74],
            &[0.78, 0.80, 0.83],
        );
        assert!(
            s.starts_with("Parts shipped per hour: 72.0% → 80.3%, up 8.3 points (between "),
            "{s}"
        );
        assert!(!s.contains('·'), "no judgment: {s}");
    }
}
