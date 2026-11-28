//! Distribution expressions, like `triangular(1 min, 3 min, 6 min)`.
//!
//! Deliberately small: one function call, or a single time (a constant).
//! Arguments are positional or named (`mean: 10 min`). Most are times with
//! units; shape parameters are plain numbers.
//!
//! Two take a list instead: `empirical(...)`, observed values, and
//! `discrete(...)`, values with probabilities (`2 min: 30%, 5 min: 70%`) or
//! observed values to draw from as they are.

use std::sync::Arc;

use simulancer_engine::spec::Distribution;

use crate::units::{DefaultUnits, Dimension, parse_quantity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A time with a unit (or a plain number in the default time unit).
    Time,
    /// A plain number, without a unit.
    Number,
}

use Kind::{Number, Time};

/// Parameters of each distribution, in positional order.
const SIGNATURES: &[(&str, &[(&str, Kind)])] = &[
    ("constant", &[("value", Time)]),
    ("exponential", &[("mean", Time)]),
    ("uniform", &[("min", Time), ("max", Time)]),
    (
        "triangular",
        &[("min", Time), ("mode", Time), ("max", Time)],
    ),
    ("lognormal", &[("mean", Time), ("sd", Time)]),
    ("weibull", &[("shape", Number), ("scale", Time)]),
    ("gamma", &[("shape", Number), ("scale", Time)]),
];

pub fn parse_time_distribution(
    text: &str,
    defaults: &DefaultUnits,
) -> Result<Distribution, String> {
    parse_distribution(text, defaults, false)
}

/// A distribution of plain numbers, like `uniform(1, 10)` in a formula: the
/// parameters that are times elsewhere are plain numbers here.
pub fn parse_number_distribution(text: &str) -> Result<Distribution, String> {
    parse_distribution(text, &DefaultUnits::default(), true)
}

/// The names of the distributions, for formulas.
pub fn is_distribution(name: &str) -> bool {
    SIGNATURES.iter().any(|(n, _)| *n == name) || LISTS.contains(&name)
}

/// Distributions that take a list of values.
const LISTS: &[&str] = &["empirical", "discrete"];

/// Most values a list may have, so a pasted column stays reasonable.
const MOST_VALUES: usize = 10_000;

/// `discrete(...)`'s items: values, and their probabilities as cumulative
/// shares rising to 1. Each item is `value: probability` (a percentage, or a
/// plain weight), or a bare value, equally likely as the other bare ones.
pub fn discrete_items<T>(
    name: &str,
    inner: &str,
    value: impl Fn(&str) -> Result<T, String>,
) -> Result<(Vec<T>, Vec<f64>), String> {
    let items = list(name, inner)?;
    let mut values = Vec::with_capacity(items.len());
    let mut weights = Vec::with_capacity(items.len());
    let (mut percents, mut plain) = (0, 0);
    for item in &items {
        let (v, w) = match item.rsplit_once(':') {
            Some((v, p)) => {
                let p = p.trim();
                let weight = match p.strip_suffix('%') {
                    Some(n) => {
                        percents += 1;
                        n.trim().parse::<f64>().ok()
                    }
                    None => {
                        plain += 1;
                        p.parse::<f64>().ok()
                    }
                }
                .filter(|w| w.is_finite() && *w > 0.0)
                .ok_or_else(|| {
                    format!("the probability `{p}` in `{name}` must be a positive percentage, like `30%`, or a positive weight")
                })?;
                (v, weight)
            }
            None => {
                plain += 1;
                (item.as_str(), 1.0)
            }
        };
        values.push(value(v.trim())?);
        weights.push(w);
    }
    if percents > 0 && plain > 0 {
        return Err(format!(
            "give every value of `{name}` a percentage, or none (weights and bare values are relative)"
        ));
    }
    let total: f64 = weights.iter().sum();
    if percents > 0 && (total - 100.0).abs() > 0.01 {
        return Err(format!(
            "the percentages in `{name}` add up to {}%, not 100%",
            (total * 100.0).round() / 100.0
        ));
    }
    let mut running = 0.0;
    let mut cumulative: Vec<f64> = weights
        .iter()
        .map(|w| {
            running += w;
            running / total
        })
        .collect();
    *cumulative.last_mut().expect("a list has items") = 1.0;
    Ok((values, cumulative))
}

/// The comma-separated items of a list distribution, at least one.
fn list(name: &str, inner: &str) -> Result<Vec<String>, String> {
    let items: Vec<String> = inner
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if items.is_empty() {
        return Err(format!("`{name}` needs a list of values"));
    }
    if items.len() > MOST_VALUES {
        return Err(format!(
            "`{name}` takes at most {MOST_VALUES} values, got {}",
            items.len()
        ));
    }
    Ok(items)
}

/// `empirical(...)` or `discrete(...)` of times (or plain numbers).
fn parse_list(
    name: &str,
    inner: &str,
    defaults: &DefaultUnits,
    plain: bool,
) -> Result<Distribution, String> {
    let value = |text: &str| -> Result<f64, String> {
        let v = if plain {
            text.parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| format!("`{name}` takes plain numbers here, got `{text}`"))?
        } else {
            parse_quantity(text, Dimension::Time, defaults)?
        };
        if !plain && v < 0.0 {
            return Err(format!(
                "a time in `{name}` can't be negative, got `{text}`"
            ));
        }
        Ok(v)
    };
    if name == "discrete" {
        let (values, cumulative) = discrete_items(name, inner, value)?;
        return Ok(Distribution::Discrete {
            values: values.into(),
            cumulative: cumulative.into(),
        });
    }
    let mut points = list(name, inner)?
        .iter()
        .map(|item| {
            if item.contains(':') {
                Err(format!(
                    "`empirical` takes observed values only, got `{item}`; for values with probabilities, use `discrete`"
                ))
            } else {
                value(item)
            }
        })
        .collect::<Result<Vec<f64>, String>>()?;
    points.sort_by(f64::total_cmp);
    if points.len() < 2 || points[0] == points[points.len() - 1] {
        return Err("`empirical` needs at least two different observed values".to_string());
    }
    Ok(Distribution::Empirical {
        points: Arc::from(points),
    })
}

fn parse_distribution(
    text: &str,
    defaults: &DefaultUnits,
    plain: bool,
) -> Result<Distribution, String> {
    let text = text.trim();
    let Some((name, rest)) = text.split_once('(') else {
        let value = parse_quantity(text, Dimension::Time, defaults)?;
        return build("constant", &[value]);
    };
    let name = name.trim();
    let Some(inner) = rest.trim_end().strip_suffix(')') else {
        return Err(format!("missing `)` at the end of `{text}`"));
    };
    if LISTS.contains(&name) {
        return parse_list(name, inner, defaults, plain);
    }
    let Some((_, params)) = SIGNATURES.iter().find(|(n, _)| *n == name) else {
        let known: Vec<&str> = SIGNATURES
            .iter()
            .map(|(n, _)| *n)
            .chain(LISTS.iter().copied())
            .collect();
        return Err(format!(
            "unknown distribution `{name}` (expected one of: {})",
            known.join(", ")
        ));
    };

    let names: Vec<&str> = params.iter().map(|(n, _)| *n).collect();
    let mut values: Vec<Option<f64>> = vec![None; params.len()];
    let mut seen_named = false;
    let args: Vec<&str> = if inner.trim().is_empty() {
        Vec::new()
    } else {
        inner.split(',').map(str::trim).collect()
    };
    for (position, arg) in args.iter().enumerate() {
        let (slot, value_text) = match arg.split_once(':') {
            Some((key, value)) => {
                seen_named = true;
                let key = key.trim();
                let Some(slot) = names.iter().position(|p| *p == key) else {
                    return Err(format!(
                        "`{name}` has no parameter `{key}` (expected: {})",
                        names.join(", ")
                    ));
                };
                (slot, value)
            }
            None if seen_named => {
                return Err(format!(
                    "positional argument `{arg}` after a named one in `{text}`"
                ));
            }
            None if position >= params.len() => {
                return Err(format!(
                    "`{name}` takes {} argument(s) ({}), got {}",
                    params.len(),
                    names.join(", "),
                    args.len()
                ));
            }
            None => (position, *arg),
        };
        if values[slot].is_some() {
            return Err(format!("`{}` is given twice in `{text}`", names[slot]));
        }
        let (param, kind) = params[slot];
        values[slot] = Some(match kind {
            Time if !plain => parse_quantity(value_text, Dimension::Time, defaults)?,
            Time | Number => value_text
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| {
                    format!(
                        "`{param}` of `{name}` is a plain number without a unit here, got `{}`",
                        value_text.trim()
                    )
                })?,
        });
    }

    let mut resolved = Vec::with_capacity(params.len());
    for (param, value) in names.iter().zip(&values) {
        match value {
            Some(v) => resolved.push(*v),
            None => return Err(format!("`{name}` is missing `{param}` in `{text}`")),
        }
    }
    build(name, &resolved)
}

/// A constant duration, in seconds.
pub fn constant(seconds: f64) -> Result<Distribution, String> {
    build("constant", &[seconds])
}

fn build(name: &str, v: &[f64]) -> Result<Distribution, String> {
    let dist = match name {
        "constant" => {
            check(v[0] >= 0.0, "a constant time can't be negative")?;
            Distribution::Constant(v[0])
        }
        "exponential" => {
            check(v[0] > 0.0, "exponential `mean` must be positive")?;
            Distribution::Exponential { mean: v[0] }
        }
        "uniform" => {
            check(v[0] >= 0.0, "uniform `min` can't be negative")?;
            check(v[0] < v[1], "uniform needs `min` < `max`")?;
            Distribution::Uniform {
                min: v[0],
                max: v[1],
            }
        }
        "triangular" => {
            check(v[0] >= 0.0, "triangular `min` can't be negative")?;
            check(
                v[0] <= v[1] && v[1] <= v[2] && v[0] < v[2],
                "triangular needs `min` <= `mode` <= `max`, with `min` < `max`",
            )?;
            Distribution::Triangular {
                min: v[0],
                mode: v[1],
                max: v[2],
            }
        }
        "lognormal" => {
            check(v[0] > 0.0, "lognormal `mean` must be positive")?;
            check(v[1] > 0.0, "lognormal `sd` must be positive")?;
            Distribution::Lognormal {
                mean: v[0],
                sd: v[1],
            }
        }
        "weibull" => {
            check(v[0] > 0.0, "weibull `shape` must be positive")?;
            check(v[1] > 0.0, "weibull `scale` must be positive")?;
            Distribution::Weibull {
                shape: v[0],
                scale: v[1],
            }
        }
        "gamma" => {
            check(v[0] > 0.0, "gamma `shape` must be positive")?;
            check(v[1] > 0.0, "gamma `scale` must be positive")?;
            Distribution::Gamma {
                shape: v[0],
                scale: v[1],
            }
        }
        _ => unreachable!("every name in SIGNATURES is handled"),
    };
    Ok(dist)
}

fn check(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Distribution, String> {
        parse_time_distribution(text, &DefaultUnits::default())
    }

    #[test]
    fn positional_arguments() {
        assert_eq!(
            parse("triangular(1 min, 3 min, 6 min)"),
            Ok(Distribution::Triangular {
                min: 60.0,
                mode: 180.0,
                max: 360.0
            })
        );
        assert_eq!(
            parse("exponential(2 min)"),
            Ok(Distribution::Exponential { mean: 120.0 })
        );
    }

    #[test]
    fn named_arguments_in_any_order() {
        assert_eq!(
            parse("lognormal(sd: 4 min, mean: 10 min)"),
            Ok(Distribution::Lognormal {
                mean: 600.0,
                sd: 240.0
            })
        );
    }

    #[test]
    fn shape_parameters_are_plain_numbers() {
        assert_eq!(
            parse("weibull(1.5, 2 h)"),
            Ok(Distribution::Weibull {
                shape: 1.5,
                scale: 7200.0
            })
        );
        assert_eq!(
            parse("gamma(scale: 30 s, shape: 2)"),
            Ok(Distribution::Gamma {
                shape: 2.0,
                scale: 30.0
            })
        );
    }

    #[test]
    fn a_bare_time_is_a_constant() {
        assert_eq!(parse("30 s"), Ok(Distribution::Constant(30.0)));
    }

    #[test]
    fn mixed_units_are_converted() {
        assert_eq!(
            parse("uniform(30 s, 2 min)"),
            Ok(Distribution::Uniform {
                min: 30.0,
                max: 120.0
            })
        );
    }

    #[test]
    fn lists() {
        assert_eq!(
            parse("empirical(4 min, 1 min, 2 min)"),
            Ok(Distribution::Empirical {
                points: vec![60.0, 120.0, 240.0].into()
            })
        );
        assert_eq!(
            parse("discrete(1 min: 25%, 3 min: 75%)"),
            Ok(Distribution::Discrete {
                values: vec![60.0, 180.0].into(),
                cumulative: vec![0.25, 1.0].into()
            })
        );
        // Weights are relative; bare values are equally likely.
        assert_eq!(
            parse("discrete(1 min: 1, 3 min: 3)"),
            parse("discrete(1 min: 25%, 3 min: 75%)")
        );
        assert_eq!(
            parse("discrete(1 min, 3 min, 3 min, 3 min)").map(|d| d.mean()),
            Ok(150.0)
        );
        assert_eq!(
            parse_number_distribution("discrete(1: 50%, 3: 50%)").map(|d| d.mean()),
            Ok(2.0)
        );
    }

    #[test]
    fn errors() {
        let cases = [
            ("normal(1 min, 2 min)", "unknown distribution `normal`"),
            ("uniform(1 min)", "missing `max`"),
            ("uniform(1 min, 2 min, 3 min)", "takes 2 argument(s)"),
            ("uniform(2 min, 1 min)", "`min` < `max`"),
            (
                "triangular(1 min, 9 min, 6 min)",
                "`min` <= `mode` <= `max`",
            ),
            ("lognormal(mean: 1 min, 2 min)", "positional argument"),
            ("lognormal(mean: 1 min, mean: 2 min)", "given twice"),
            ("exponential(rate: 1 min)", "no parameter `rate`"),
            ("exponential(2 min", "missing `)`"),
            ("exponential(2 m)", "is a length, but a time is expected"),
            ("weibull(2 min, 1 h)", "plain number without a unit"),
            ("gamma(0, 1 h)", "`shape` must be positive"),
            ("empirical(2 min)", "at least two different"),
            ("empirical(2 min, 2 min)", "at least two different"),
            ("empirical(1 min: 50%, 2 min: 50%)", "use `discrete`"),
            ("empirical()", "needs a list of values"),
            (
                "discrete(1 min: 30%, 2 min: 60%)",
                "add up to 90%, not 100%",
            ),
            ("discrete(1 min: 30%, 2 min)", "a percentage, or none"),
            (
                "discrete(1 min: 0%, 2 min: 100%)",
                "must be a positive percentage",
            ),
            ("discrete(-1 min: 100%)", "can't be negative"),
        ];
        for (text, expected) in cases {
            let err = parse(text).unwrap_err();
            assert!(err.contains(expected), "{text}: {err}");
        }
    }
}
