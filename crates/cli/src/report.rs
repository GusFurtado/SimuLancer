//! The summary table printed by `run`.

use outliers_sim_engine::stats::Summary;

use crate::measures::{Measure, Unit};

pub fn table(measures: &[(Measure, Summary)]) -> String {
    let mut rows: Vec<[String; 5]> = Vec::new();
    let mut group: Option<(&str, &str)> = None;
    for (m, s) in measures {
        if group != Some((m.object_type, &m.object)) {
            group = Some((m.object_type, &m.object));
            rows.push([
                format!("{} {}", m.object_type, m.object),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ]);
        }
        let fmt = Format::for_measure(m.unit, s.mean);
        rows.push([
            format!("  {}", m.name),
            fmt.value(s.mean),
            if s.half_width.is_nan() {
                "n/a".into()
            } else {
                format!("± {}", fmt.value(s.half_width))
            },
            fmt.value(s.min),
            fmt.value(s.max),
        ]);
    }

    let header = [
        String::new(),
        "mean".into(),
        "95% CI".into(),
        "min".into(),
        "max".into(),
    ];
    let mut widths = [0; 5];
    for row in std::iter::once(&header).chain(&rows) {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let line = |row: &[String; 5]| {
        let mut text = format!("{:<w$}", row[0], w = widths[0]);
        for (cell, w) in row.iter().zip(widths).skip(1) {
            text += &format!("  {cell:>w$}");
        }
        text.trim_end().to_string() + "\n"
    };
    std::iter::once(&header).chain(&rows).map(line).collect()
}

/// How to show the numbers of one measure: same unit for mean, CI and range.
struct Format {
    scale: f64,
    decimals: usize,
    suffix: &'static str,
}

impl Format {
    fn for_measure(unit: Unit, mean: f64) -> Format {
        let (scale, decimals, suffix) = match unit {
            Unit::Fraction => (100.0, 1, "%"),
            Unit::Parts => (1.0, 2, ""),
            Unit::Count => (1.0, 1, ""),
            Unit::PartsPerHour => (1.0, 1, " /h"),
            Unit::Seconds => match mean.abs() {
                m if m >= 2.0 * 3600.0 => (1.0 / 3600.0, 2, " h"),
                m if m >= 120.0 => (1.0 / 60.0, 2, " min"),
                _ => (1.0, 2, " s"),
            },
        };
        Format {
            scale,
            decimals,
            suffix,
        }
    }

    fn value(&self, v: f64) -> String {
        if v.is_nan() {
            return "n/a".into();
        }
        format!("{:.*}{}", self.decimals, v * self.scale, self.suffix)
    }
}
