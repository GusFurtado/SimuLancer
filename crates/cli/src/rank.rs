//! Which of several alternatives (the current setup and scenarios, compared
//! on common random numbers) does best on a result, and how sure that is.
//!
//! The best is the one with the best mean. It's compared with each of the
//! others on paired differences, replication by replication, with
//! Bonferroni-adjusted intervals: with k alternatives there are k − 1
//! comparisons, each at 1 − 0.05 / (k − 1), so all of them hold together with
//! at least 95% confidence. An alternative the best isn't clearly better than
//! could be as good; more replications can tell them apart.

use serde_json::{Value, json};
use simulancer_engine::stats::t_quantile;

use crate::decisions::{Better, ResultSpec};
use crate::measures::Unit;

/// Differences smaller than these don't matter (as in `plain`).
const SMALL_RELATIVE: f64 = 0.01;
const SMALL_POINTS: f64 = 0.01;

/// The ranking of one result. `values[i]` holds alternative i's value in each
/// replication (NaN where undefined). None for a result without a direction,
/// or with fewer than three alternatives (the comparison with now says it).
pub fn rank(spec: &ResultSpec, names: &[String], values: &[Vec<f64>]) -> Option<Value> {
    let sign = match spec.better {
        Better::Higher => 1.0,
        Better::Lower => -1.0,
        Better::Neither => return None,
    };
    let k = names.len();
    if k < 3 || values.len() != k {
        return None;
    }
    let mean = |v: &[f64]| {
        let finite: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
        (!finite.is_empty()).then(|| finite.iter().sum::<f64>() / finite.len() as f64)
    };
    let means: Vec<Option<f64>> = values.iter().map(|v| mean(v)).collect();
    let best = (0..k)
        .filter(|&i| means[i].is_some())
        .max_by(|&a, &b| (sign * means[a].unwrap()).total_cmp(&(sign * means[b].unwrap())))?;
    let level = 1.0 - 0.05 / (k - 1) as f64;
    let mut others = Vec::new();
    for j in (0..k).filter(|&j| j != best) {
        // How much better the best is than j, replication by replication.
        let diffs: Vec<f64> = values[best]
            .iter()
            .zip(&values[j])
            .filter(|(a, b)| a.is_finite() && b.is_finite())
            .map(|(a, b)| sign * (a - b))
            .collect();
        let n = diffs.len();
        let (d, hw) = if n >= 2 {
            let d = diffs.iter().sum::<f64>() / n as f64;
            let sd = (diffs.iter().map(|x| (x - d).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt();
            let t = t_quantile(1.0 - (1.0 - level) / 2.0, (n - 1) as f64);
            (d, t * sd / (n as f64).sqrt())
        } else {
            (f64::NAN, f64::NAN)
        };
        let clear = n >= 2 && d - hw > 0.0;
        let scale = match (spec.unit, means[j]) {
            (Unit::Fraction, _) => Some(SMALL_POINTS),
            (_, Some(m)) if m.abs() > 1e-12 => Some(SMALL_RELATIVE * m.abs()),
            _ => None,
        };
        let small = clear && scale.is_some_and(|s| d < s);
        others.push(json!({
            "name": names[j],
            "clear": clear,
            "small": small,
            "difference": finite(d),
            "low": finite(d - hw),
            "high": finite(d + hw),
        }));
    }
    // Every difference to the best is under 1% (or 1 point), surely: nothing
    // to choose between them.
    let negligible = others
        .iter()
        .zip((0..k).filter(|&j| j != best))
        .all(|(o, j)| {
            let limit = match (spec.unit, means[j]) {
                (Unit::Fraction, _) => SMALL_POINTS,
                (_, Some(m)) if m.abs() > 1e-12 => SMALL_RELATIVE * m.abs(),
                _ => 0.0,
            };
            o["high"].as_f64().is_some_and(|h| h < limit)
        });
    let unclear: Vec<&str> = others
        .iter()
        .filter(|o| o["clear"] != json!(true))
        .map(|o| o["name"].as_str().unwrap_or(""))
        .collect();
    let small: Vec<&str> = others
        .iter()
        .filter(|o| o["small"] == json!(true))
        .map(|o| o["name"].as_str().unwrap_or(""))
        .collect();
    let clear_winner = unclear.is_empty();
    let mut sentence = format!("{}: {} does best", spec.label, names[best]);
    if negligible {
        sentence = format!(
            "{}: all {k} are within 1% of each other, too small to matter",
            spec.label
        );
    } else if unclear.len() == k - 1 {
        sentence = format!(
            "{}: none clearly does best; {} has the best mean, but any of them could be as good, and more replications could tell",
            spec.label, names[best]
        );
    } else if clear_winner {
        sentence += &format!(
            ", clearly better than each of the other {} (95% sure of all {} comparisons at once)",
            k - 1,
            k - 1
        );
    } else {
        sentence += &format!(
            ", but isn't clearly better than {}: {} could be as good, and more replications could tell",
            list(&unclear),
            if unclear.len() == 1 { "it" } else { "they" }
        );
    }
    if !negligible && !small.is_empty() {
        sentence += &format!(
            "; its lead over {} is under 1%, too small to matter",
            list(&small)
        );
    }
    sentence.push('.');
    Some(json!({
        "label": spec.label,
        "object": spec.object,
        "measure": spec.measure,
        "best": names[best],
        // The best is clearly better than every other one.
        "clear": clear_winner && !negligible,
        "confidence": level,
        "others": others,
        "sentence": sentence,
    }))
}

/// "a", "a and b", "a, b and c".
fn list(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn finite(v: f64) -> Value {
    if v.is_finite() { json!(v) } else { Value::Null }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(better: Better) -> ResultSpec {
        ResultSpec {
            label: "Lead time".into(),
            object: "shipping".into(),
            measure: "time in system".into(),
            unit: Unit::Seconds,
            better,
        }
    }

    fn names(n: usize) -> Vec<String> {
        (1..=n).map(|i| format!("Packers: {i}")).collect()
    }

    #[test]
    fn the_best_is_clearly_better_than_the_rest() {
        // Lower is better; 3 packers is clearly fastest.
        let values = vec![
            vec![600.0, 620.0, 610.0, 605.0, 615.0],
            vec![500.0, 515.0, 505.0, 498.0, 512.0],
            vec![400.0, 410.0, 402.0, 399.0, 408.0],
        ];
        let r = rank(&spec(Better::Lower), &names(3), &values).unwrap();
        assert_eq!(r["best"], "Packers: 3");
        assert_eq!(
            r["sentence"],
            "Lead time: Packers: 3 does best, clearly better than each of the other 2 (95% sure of all 2 comparisons at once)."
        );
        assert!((r["confidence"].as_f64().unwrap() - 0.975).abs() < 1e-12);
    }

    #[test]
    fn a_close_second_could_be_as_good() {
        let values = vec![
            vec![600.0, 620.0, 610.0, 605.0, 615.0],
            vec![401.0, 409.0, 405.0, 397.0, 412.0],
            vec![400.0, 410.0, 402.0, 399.0, 408.0],
        ];
        let r = rank(&spec(Better::Lower), &names(3), &values).unwrap();
        assert_eq!(r["best"], "Packers: 3");
        assert!(
            r["sentence"]
                .as_str()
                .unwrap()
                .contains("isn't clearly better than Packers: 2: it could be as good")
        );
        assert_eq!(r["others"][0]["clear"], true);
        assert_eq!(r["others"][1]["clear"], false);
        assert_eq!(r["clear"], false);
    }

    #[test]
    fn ties_and_tiny_differences() {
        // Noise, no real difference: no winner.
        let values = vec![
            vec![500.0, 520.0, 480.0, 510.0, 490.0],
            vec![505.0, 512.0, 488.0, 503.0, 496.0],
            vec![498.0, 523.0, 482.0, 507.0, 493.0],
        ];
        let r = rank(&spec(Better::Lower), &names(3), &values).unwrap();
        assert!(
            r["sentence"]
                .as_str()
                .unwrap()
                .contains("none clearly does best"),
            "{r}"
        );
        // Clear, but every difference far under 1%.
        let values = vec![
            vec![500.0, 520.0, 480.0, 510.0, 490.0],
            vec![500.1, 520.1, 480.1, 510.1, 490.1],
            vec![500.2, 520.2, 480.2, 510.2, 490.2],
        ];
        let r = rank(&spec(Better::Lower), &names(3), &values).unwrap();
        assert_eq!(
            r["sentence"],
            "Lead time: all 3 are within 1% of each other, too small to matter."
        );
        assert_eq!(r["clear"], false);
    }

    #[test]
    fn no_ranking_without_a_direction_or_with_two() {
        let values = vec![vec![1.0, 2.0], vec![2.0, 3.0], vec![3.0, 4.0]];
        assert!(rank(&spec(Better::Neither), &names(3), &values).is_none());
        assert!(rank(&spec(Better::Lower), &names(2), &values[..2]).is_none());
    }
}
