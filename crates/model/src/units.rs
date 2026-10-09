//! Quantities with units, converted to standard units (seconds, meters, m/s).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Time,
    Length,
    Speed,
}

impl Dimension {
    fn describe(self) -> &'static str {
        match self {
            Dimension::Time => "a time",
            Dimension::Length => "a length",
            Dimension::Speed => "a speed",
        }
    }

    fn example(self) -> &'static str {
        match self {
            Dimension::Time => "`5 min`",
            Dimension::Length => "`12 m`",
            Dimension::Speed => "`0.5 m/s`",
        }
    }
}

/// Unit symbol and how many seconds it is.
const TIME_UNITS: &[(&str, f64)] = &[
    ("ms", 1e-3),
    ("s", 1.0),
    ("min", 60.0),
    ("h", 3600.0),
    ("d", 86400.0),
];

/// Unit symbol and how many meters it is.
const LENGTH_UNITS: &[(&str, f64)] = &[("mm", 1e-3), ("cm", 1e-2), ("m", 1.0), ("km", 1e3)];

fn lookup(table: &[(&str, f64)], unit: &str) -> Option<f64> {
    table
        .iter()
        .find(|(name, _)| *name == unit)
        .map(|(_, f)| *f)
}

fn dimension_of(unit: &str) -> Option<Dimension> {
    if lookup(TIME_UNITS, unit).is_some() {
        Some(Dimension::Time)
    } else if lookup(LENGTH_UNITS, unit).is_some() {
        Some(Dimension::Length)
    } else if speed_factor(unit).is_some() {
        Some(Dimension::Speed)
    } else {
        None
    }
}

fn speed_factor(unit: &str) -> Option<f64> {
    let (length, time) = unit.split_once('/')?;
    Some(lookup(LENGTH_UNITS, length)? / lookup(TIME_UNITS, time)?)
}

fn factor(unit: &str, dimension: Dimension) -> Option<f64> {
    match dimension {
        Dimension::Time => lookup(TIME_UNITS, unit),
        Dimension::Length => lookup(LENGTH_UNITS, unit),
        Dimension::Speed => speed_factor(unit),
    }
}

/// Units used for plain numbers, from the model's optional `units:` block.
#[derive(Debug, Clone, Default)]
pub struct DefaultUnits {
    pub time: Option<String>,
    pub length: Option<String>,
}

impl DefaultUnits {
    /// Checks that `unit` is a valid unit of `dimension`, for the `units:` block.
    pub fn check(unit: &str, dimension: Dimension) -> Result<(), String> {
        match factor(unit, dimension) {
            Some(_) => Ok(()),
            None => Err(format!(
                "`{unit}` is not {} unit (expected one of: {})",
                dimension.describe(),
                unit_names(dimension)
            )),
        }
    }

    fn unit_for(&self, dimension: Dimension) -> Option<String> {
        match dimension {
            Dimension::Time => self.time.clone(),
            Dimension::Length => self.length.clone(),
            Dimension::Speed => Some(format!("{}/{}", self.length.as_ref()?, self.time.as_ref()?)),
        }
    }
}

fn unit_names(dimension: Dimension) -> String {
    let names: Vec<&str> = match dimension {
        Dimension::Time => TIME_UNITS.iter().map(|(n, _)| *n).collect(),
        Dimension::Length => LENGTH_UNITS.iter().map(|(n, _)| *n).collect(),
        Dimension::Speed => return "a length unit / a time unit, like m/s or m/min".into(),
    };
    names.join(", ")
}

/// Converts a plain number, using the model's default unit for `dimension`.
pub fn convert_number(
    value: f64,
    dimension: Dimension,
    defaults: &DefaultUnits,
) -> Result<f64, String> {
    if !value.is_finite() {
        return Err(format!("`{value}` is not a finite number"));
    }
    let Some(unit) = defaults.unit_for(dimension) else {
        let hint = match dimension {
            Dimension::Time => "`units.time`",
            Dimension::Length => "`units.length`",
            Dimension::Speed => "`units.length` and `units.time`",
        };
        return Err(format!(
            "`{value}` has no unit: write {} instead, or set {hint} at the top of the model",
            dimension.example()
        ));
    };
    let factor = factor(&unit, dimension).expect("default units are checked when loaded");
    Ok(value * factor)
}

/// Parses `"<number> <unit>"` (or a plain number) and converts it to standard units.
pub fn parse_quantity(
    text: &str,
    dimension: Dimension,
    defaults: &DefaultUnits,
) -> Result<f64, String> {
    let text = text.trim();
    let (number, unit) = match text.split_once(char::is_whitespace) {
        Some((number, unit)) => (number, Some(unit.trim())),
        None => (text, None),
    };
    let Ok(value) = number.parse::<f64>() else {
        return Err(format!(
            "`{text}` is not {}: expected a number and a unit, like {}",
            dimension.describe(),
            dimension.example()
        ));
    };
    let Some(unit) = unit else {
        return convert_number(value, dimension, defaults);
    };
    if !value.is_finite() {
        return Err(format!("`{text}` is not a finite number"));
    }
    match factor(unit, dimension) {
        Some(factor) => Ok(value * factor),
        None => Err(match dimension_of(unit) {
            Some(actual) => format!(
                "`{text}` is {}, but {} is expected here",
                actual.describe(),
                dimension.describe()
            ),
            None => format!(
                "unknown unit `{unit}` in `{text}` (expected {})",
                unit_names(dimension)
            ),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> DefaultUnits {
        DefaultUnits::default()
    }

    #[test]
    fn converts_to_standard_units() {
        assert_eq!(parse_quantity("8 h", Dimension::Time, &none()), Ok(28800.0));
        assert_eq!(
            parse_quantity("1.5 min", Dimension::Time, &none()),
            Ok(90.0)
        );
        assert_eq!(parse_quantity("12 m", Dimension::Length, &none()), Ok(12.0));
        assert_eq!(parse_quantity("50 cm", Dimension::Length, &none()), Ok(0.5));
        assert_eq!(
            parse_quantity("0.5 m/s", Dimension::Speed, &none()),
            Ok(0.5)
        );
        assert_eq!(
            parse_quantity("30 m/min", Dimension::Speed, &none()),
            Ok(0.5)
        );
    }

    #[test]
    fn rejects_wrong_dimension() {
        let err = parse_quantity("5 min", Dimension::Speed, &none()).unwrap_err();
        assert!(err.contains("is a time, but a speed is expected"), "{err}");
    }

    #[test]
    fn rejects_unknown_unit() {
        let err = parse_quantity("5 parsecs", Dimension::Length, &none()).unwrap_err();
        assert!(err.contains("unknown unit `parsecs`"), "{err}");
    }

    #[test]
    fn plain_numbers_need_a_default_unit() {
        let err = parse_quantity("12", Dimension::Length, &none()).unwrap_err();
        assert!(err.contains("has no unit"), "{err}");

        let defaults = DefaultUnits {
            time: Some("min".into()),
            length: Some("m".into()),
        };
        assert_eq!(parse_quantity("12", Dimension::Length, &defaults), Ok(12.0));
        assert_eq!(parse_quantity("2", Dimension::Time, &defaults), Ok(120.0));
        assert_eq!(parse_quantity("30", Dimension::Speed, &defaults), Ok(0.5));
    }

    #[test]
    fn explicit_unit_overrides_default() {
        let defaults = DefaultUnits {
            time: Some("min".into()),
            length: None,
        };
        assert_eq!(parse_quantity("30 s", Dimension::Time, &defaults), Ok(30.0));
    }
}
