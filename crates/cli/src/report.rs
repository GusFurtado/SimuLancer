//! The tables printed by `run` and `compare`.

use outliers_sim_engine::stats::Summary;

use crate::compare::{Comparison, Verdict};
use crate::measures::{Measure, Unit};

pub fn table(measures: &[(Measure, Summary)]) -> String {
    let mut rows = Vec::new();
    let mut group: Option<(&str, &str)> = None;
    for (m, s) in measures {
        if group != Some((m.object_type, &m.object)) {
            group = Some((m.object_type, &m.object));
            rows.push(group_row(m, 5));
        }
        let fmt = Format::for_measure(m.unit, s.mean);
        rows.push(vec![
            format!("  {}", m.name),
            fmt.value(s.mean),
            fmt.plus_minus(s.half_width),
            fmt.value(s.min),
            fmt.value(s.max),
        ]);
    }
    render(&["", "mean", "95% CI", "min", "max"], &rows)
}

/// Base vs scenario, with the paired difference and its confidence interval.
pub fn comparison_table(comparisons: &[Comparison]) -> String {
    let mut rows = Vec::new();
    let mut group: Option<(&str, &str)> = None;
    for c in comparisons {
        let m = &c.base;
        if group != Some((m.object_type, &m.object)) {
            group = Some((m.object_type, &m.object));
            rows.push(group_row(m, 6));
        }
        // One display unit per row, chosen from the base mean.
        let fmt = Format::for_measure(m.unit, c.base_summary.mean);
        let diff = fmt.difference();
        rows.push(vec![
            format!("  {}", m.name),
            fmt.value(c.base_summary.mean),
            fmt.value(c.scenario_summary.mean),
            diff.signed(c.difference.mean),
            diff.plus_minus(c.difference.half_width),
            c.verdict.describe().to_string(),
        ]);
    }
    render(&["", "base", "scenario", "difference", "95% CI", ""], &rows)
}

fn group_row(m: &Measure, columns: usize) -> Vec<String> {
    let mut row = vec![format!("{} {}", m.object_type, m.object)];
    row.resize(columns, String::new());
    row
}

/// First column left-aligned, the rest right-aligned (the last one left
/// aligned if it's text, like a verdict).
fn render(header: &[&str], rows: &[Vec<String>]) -> String {
    let header: Vec<String> = header.iter().map(|h| h.to_string()).collect();
    let mut widths = vec![0; header.len()];
    for row in std::iter::once(&header).chain(rows) {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let text_last = header.last().is_some_and(|h| h.is_empty()) && header.len() > 1;
    let line = |row: &Vec<String>| {
        let mut text = format!("{:<w$}", row[0], w = widths[0]);
        for (i, (cell, w)) in row.iter().zip(&widths).enumerate().skip(1) {
            if text_last && i == row.len() - 1 {
                text += &format!("  {cell:<w$}");
            } else {
                text += &format!("  {cell:>w$}");
            }
        }
        text.trim_end().to_string() + "\n"
    };
    std::iter::once(&header).chain(rows).map(line).collect()
}

impl Verdict {
    pub fn describe(self) -> &'static str {
        match self {
            Verdict::Higher => "higher",
            Verdict::Lower => "lower",
            Verdict::NoClearDifference => "no clear difference",
            Verdict::Identical => "identical",
            Verdict::Unknown => "n/a",
        }
    }
}

/// How to show the numbers of one measure: same unit for mean, CI and range.
pub struct Format {
    scale: f64,
    decimals: usize,
    suffix: &'static str,
}

impl Format {
    pub fn for_measure(unit: Unit, mean: f64) -> Format {
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

    /// Differences of percentages are percentage points.
    pub fn difference(&self) -> Format {
        Format {
            suffix: if self.suffix == "%" {
                " pp"
            } else {
                self.suffix
            },
            ..*self
        }
    }

    pub fn value(&self, v: f64) -> String {
        if v.is_nan() {
            return "n/a".into();
        }
        format!("{:.*}{}", self.decimals, v * self.scale, self.suffix)
    }

    pub fn signed(&self, v: f64) -> String {
        if v.is_nan() {
            return "n/a".into();
        }
        let decimals = self.decimals_for(v);
        format!("{:+.*}{}", decimals, v * self.scale, self.suffix)
    }

    fn plus_minus(&self, half_width: f64) -> String {
        if half_width.is_nan() {
            return "n/a".into();
        }
        let decimals = self.decimals_for(half_width);
        format!("± {:.*}{}", decimals, half_width * self.scale, self.suffix)
    }

    /// Enough decimals that a small nonzero value doesn't show as zero
    /// (two significant digits, at most 6 decimals).
    fn decimals_for(&self, v: f64) -> usize {
        let shown = (v * self.scale).abs();
        if shown == 0.0 || shown >= 0.5 * 10f64.powi(-(self.decimals as i32)) {
            return self.decimals;
        }
        let needed = (-shown.log10()).floor() as usize + 2;
        needed.clamp(self.decimals, 6)
    }
}
