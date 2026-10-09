//! Distribution expressions, like `triangular(1 min, 3 min, 6 min)`.
//!
//! Deliberately small: one function call, or a single time (a constant).
//! Arguments are positional or named (`mean: 10 min`). Most are times with
//! units; shape parameters are plain numbers.

use outliers_sim_engine::spec::Distribution;

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
    let text = text.trim();
    let Some((name, rest)) = text.split_once('(') else {
        let value = parse_quantity(text, Dimension::Time, defaults)?;
        return build("constant", &[value]);
    };
    let name = name.trim();
    let Some(inner) = rest.trim_end().strip_suffix(')') else {
        return Err(format!("missing `)` at the end of `{text}`"));
    };
    let Some((_, params)) = SIGNATURES.iter().find(|(n, _)| *n == name) else {
        let known: Vec<&str> = SIGNATURES.iter().map(|(n, _)| *n).collect();
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
            Time => parse_quantity(value_text, Dimension::Time, defaults)?,
            Number => value_text
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| {
                    format!(
                        "`{param}` of `{name}` is a plain number without a unit, got `{}`",
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
        ];
        for (text, expected) in cases {
            let err = parse(text).unwrap_err();
            assert!(err.contains(expected), "{text}: {err}");
        }
    }
}
