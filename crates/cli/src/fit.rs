//! `simulancer fit`: suggests a distribution for a column of observed
//! durations. Each candidate is fitted by maximum likelihood (uniform and
//! triangular by their range), ranked by the Kolmogorov-Smirnov distance
//! between its CDF and the data's, and printed as the expression a model
//! takes, ready to paste.
//!
//! The p-values use Stephens' approximation of the Kolmogorov distribution.
//! They're optimistic, since the parameters come from the same data (Law,
//! *Simulation Modeling and Analysis*, §6.6): read them as a ranking aid, not
//! a test.

use std::fs;
use std::path::Path;

use simulancer_engine::stats::ln_gamma;

/// A CDF of a fitted distribution.
type Cdf = Box<dyn Fn(f64) -> f64>;

/// A fitted candidate.
pub struct Candidate {
    /// The model expression, in the data's unit: `gamma(2.1, 3.4 min)`.
    pub expression: String,
    /// Kolmogorov-Smirnov distance to the data.
    pub ks: f64,
    /// Approximate p-value (optimistic: parameters were estimated).
    pub p: f64,
    pub mean: f64,
    pub sd: f64,
}

/// The values of one column of a CSV file (or a plain list, one per line).
/// `column` is a header name or a 1-based position; without it, the file
/// must have a single column. A first row that isn't numeric is a header.
pub fn read_column(path: &Path, column: Option<&str>) -> Result<Vec<f64>, String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    parse_column(&text, &path.display().to_string(), column)
}

/// The separator of a CSV text: the one the first line uses most, if any.
fn separator(text: &str) -> Option<char> {
    let first = text.lines().next().unwrap_or("");
    [',', ';', '\t']
        .into_iter()
        .max_by_key(|s| first.matches(*s).count())
        .filter(|s| first.contains(*s))
}

/// The fields of each non-blank line.
fn rows(text: &str) -> impl Iterator<Item = Vec<String>> + '_ {
    let separator = separator(text);
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(move |line| match separator {
            Some(s) => line
                .split(s)
                .map(|f| f.trim().trim_matches('"').to_string())
                .collect(),
            None => vec![line.trim().trim_matches('"').to_string()],
        })
}

fn is_number(field: &str) -> bool {
    field.replace(',', ".").parse::<f64>().is_ok()
}

/// The columns of a CSV text: the header's names, or `1`, `2`... when the
/// first row is already data.
pub fn columns(text: &str) -> Vec<String> {
    let Some(first) = rows(text).next() else {
        return Vec::new();
    };
    if first.iter().all(|f| is_number(f)) {
        (1..=first.len()).map(|n| n.to_string()).collect()
    } else {
        first
    }
}

/// [`read_column`] on text already read; `name` is the file's name, for messages.
pub fn parse_column(text: &str, name: &str, column: Option<&str>) -> Result<Vec<f64>, String> {
    let separator = separator(text);
    let mut rows = rows(text);
    let Some(first) = rows.next() else {
        return Err(format!("{name} is empty"));
    };
    let header = !first.iter().all(|f| is_number(f));
    let index = match column {
        None if first.len() == 1 => 0,
        None => {
            return Err(format!(
                "{} has {} columns: pick one with --column ({})",
                name,
                first.len(),
                if header {
                    first.join(", ")
                } else {
                    format!("1 to {}", first.len())
                }
            ));
        }
        Some(c) => match c.parse::<usize>() {
            Ok(n) if (1..=first.len()).contains(&n) => n - 1,
            _ => first.iter().position(|h| header && h == c).ok_or_else(|| {
                format!(
                    "{} has no column `{c}` ({})",
                    name,
                    if header {
                        first.join(", ")
                    } else {
                        format!("columns 1 to {}", first.len())
                    }
                )
            })?,
        },
    };
    let data_rows: Vec<Vec<String>> = if header {
        rows.collect()
    } else {
        std::iter::once(first).chain(rows).collect()
    };
    let mut values = Vec::with_capacity(data_rows.len());
    for (i, row) in data_rows.iter().enumerate() {
        let Some(field) = row.get(index).filter(|f| !f.is_empty()) else {
            continue; // a blank cell
        };
        // A decimal comma is fine when the separator isn't a comma.
        let field = if separator == Some(',') {
            field.clone()
        } else {
            field.replace(',', ".")
        };
        let v = field
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| {
                format!(
                    "{}: row {} has `{field}`, not a number",
                    name,
                    i + 1 + usize::from(header)
                )
            })?;
        if v < 0.0 {
            return Err(format!(
                "{}: row {} has a negative duration ({v})",
                name,
                i + 1 + usize::from(header)
            ));
        }
        values.push(v);
    }
    if values.len() < 2 {
        return Err(format!(
            "{} needs at least two values to fit, has {}",
            name,
            values.len()
        ));
    }
    Ok(values)
}

/// Every candidate that fits the data, best (smallest KS distance) first.
/// `unit` is appended to times in the expressions.
pub fn fit(values: &[f64], unit: &str) -> Vec<Candidate> {
    let mut data = values.to_vec();
    data.sort_by(f64::total_cmp);
    let n = data.len() as f64;
    let mean = data.iter().sum::<f64>() / n;
    let (min, max) = (data[0], data[data.len() - 1]);
    let t = |v: f64| format!("{} {unit}", short(v));
    // (expression, CDF, mean, sd)
    let mut out: Vec<(String, Cdf, f64, f64)> = Vec::new();

    if mean > 0.0 {
        out.push((
            format!("exponential({})", t(mean)),
            Box::new(move |x| 1.0 - (-x / mean).exp()),
            mean,
            mean,
        ));
    }
    if max > min {
        let (a, b) = (min, max);
        out.push((
            format!("uniform({}, {})", t(a), t(b)),
            Box::new(move |x| ((x - a) / (b - a)).clamp(0.0, 1.0)),
            (a + b) / 2.0,
            (b - a) / 12f64.sqrt(),
        ));
        // The mode that matches the mean, kept inside the range.
        let c = (3.0 * mean - a - b).clamp(a, b);
        let tri_mean = (a + b + c) / 3.0;
        let tri_var = (a * a + b * b + c * c - a * b - a * c - b * c) / 18.0;
        out.push((
            format!("triangular({}, {}, {})", t(a), t(c), t(b)),
            Box::new(move |x| triangular_cdf(x, a, c, b)),
            tri_mean,
            tri_var.sqrt(),
        ));
    }
    // The rest need every value above zero.
    if min > 0.0 && max > min {
        let logs: Vec<f64> = data.iter().map(|x| x.ln()).collect();
        let mean_log = logs.iter().sum::<f64>() / n;
        let sd_log = (logs.iter().map(|l| (l - mean_log).powi(2)).sum::<f64>() / n).sqrt();
        if sd_log > 0.0 {
            let (mu, sigma) = (mean_log, sd_log);
            let ln_mean = (mu + sigma * sigma / 2.0).exp();
            let ln_sd = ln_mean * ((sigma * sigma).exp() - 1.0).sqrt();
            out.push((
                format!("lognormal(mean: {}, sd: {})", t(ln_mean), t(ln_sd)),
                Box::new(move |x| normal_cdf((x.ln() - mu) / sigma)),
                ln_mean,
                ln_sd,
            ));
        }
        if let Some(shape) = gamma_shape(mean, mean_log) {
            let scale = mean / shape;
            out.push((
                format!("gamma({}, {})", short(shape), t(scale)),
                Box::new(move |x| gamma_p(shape, x / scale)),
                shape * scale,
                shape.sqrt() * scale,
            ));
        }
        if let Some((shape, scale)) = weibull(&data, mean_log) {
            let m = scale * ln_gamma(1.0 + 1.0 / shape).exp();
            let m2 = scale * scale * ln_gamma(1.0 + 2.0 / shape).exp();
            out.push((
                format!("weibull({}, {})", short(shape), t(scale)),
                Box::new(move |x| 1.0 - (-(x / scale).powf(shape)).exp()),
                m,
                (m2 - m * m).max(0.0).sqrt(),
            ));
        }
    }
    let mut candidates: Vec<Candidate> = out
        .into_iter()
        .map(|(expression, cdf, mean, sd)| {
            let ks = ks_distance(&data, &*cdf);
            Candidate {
                expression,
                ks,
                p: ks_p_value(ks, data.len()),
                mean,
                sd,
            }
        })
        .collect();
    candidates.sort_by(|a, b| a.ks.total_cmp(&b.ks));
    candidates
}

/// The data's own summary: mean, sd, min, max.
pub fn describe(values: &[f64]) -> (f64, f64, f64, f64) {
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let sd = (values.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (mean, sd, min, max)
}

/// `empirical(...)` of the values as they are, in `unit`.
pub fn empirical(values: &[f64], unit: &str) -> String {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let items: Vec<String> = sorted
        .iter()
        .map(|v| format!("{} {unit}", short(*v)))
        .collect();
    format!("empirical({})", items.join(", "))
}

/// Four significant digits, no trailing zeros.
pub fn short(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let digits = (3 - v.abs().log10().floor() as i32).clamp(0, 10) as usize;
    let text = format!("{v:.digits$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

fn triangular_cdf(x: f64, a: f64, c: f64, b: f64) -> f64 {
    if x <= a {
        0.0
    } else if x >= b {
        1.0
    } else if x <= c {
        (x - a).powi(2) / ((b - a) * (c - a))
    } else {
        1.0 - (b - x).powi(2) / ((b - a) * (b - c))
    }
}

/// Largest gap between the data's step CDF and `cdf`; `data` is sorted.
fn ks_distance(data: &[f64], cdf: &dyn Fn(f64) -> f64) -> f64 {
    let n = data.len() as f64;
    data.iter()
        .enumerate()
        .map(|(i, x)| {
            let f = cdf(*x);
            (f - i as f64 / n).max((i + 1) as f64 / n - f)
        })
        .fold(0.0, f64::max)
}

/// P(D > d) for n values, by Stephens' approximation of Kolmogorov's distribution.
fn ks_p_value(d: f64, n: usize) -> f64 {
    let rn = (n as f64).sqrt();
    let lambda = (rn + 0.12 + 0.11 / rn) * d;
    if lambda < 0.2 {
        return 1.0;
    }
    let mut sum = 0.0;
    for j in 1..=100 {
        let j = j as f64;
        let term = 2.0
            * (if j % 2.0 == 1.0 { 1.0 } else { -1.0 })
            * (-2.0 * j * j * lambda * lambda).exp();
        sum += term;
        if term.abs() < 1e-12 {
            break;
        }
    }
    sum.clamp(0.0, 1.0)
}

/// Gamma shape by maximum likelihood: solves ln k − ψ(k) = ln(mean) − mean(ln x)
/// by Newton's method from Minka's starting point.
fn gamma_shape(mean: f64, mean_log: f64) -> Option<f64> {
    let s = mean.ln() - mean_log;
    if s <= 0.0 || !s.is_finite() {
        return None;
    }
    let mut k = (3.0 - s + ((s - 3.0).powi(2) + 24.0 * s).sqrt()) / (12.0 * s);
    for _ in 0..50 {
        let f = k.ln() - digamma(k) - s;
        let step = f / (1.0 / k - trigamma(k));
        let next = (k - step).max(k / 10.0);
        if (next - k).abs() < 1e-12 * k {
            return Some(next);
        }
        k = next;
    }
    k.is_finite().then_some(k)
}

/// Weibull shape and scale by maximum likelihood: the shape solves
/// Σ xᵏ ln x / Σ xᵏ − 1/k − mean(ln x) = 0 (increasing in k), by bisection.
fn weibull(data: &[f64], mean_log: f64) -> Option<(f64, f64)> {
    // Scaled by the largest value, so xᵏ can't overflow.
    let top = data[data.len() - 1];
    let g = |k: f64| {
        let (mut sum, mut weighted) = (0.0, 0.0);
        for x in data {
            let y = (x / top).powf(k);
            sum += y;
            weighted += y * x.ln();
        }
        weighted / sum - 1.0 / k - mean_log
    };
    let (mut lo, mut hi) = (0.02, 200.0);
    if g(lo) > 0.0 || g(hi) < 0.0 {
        return None;
    }
    for _ in 0..200 {
        let mid = (lo * hi).sqrt();
        if g(mid) < 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
        if hi / lo < 1.0 + 1e-12 {
            break;
        }
    }
    let k = (lo * hi).sqrt();
    let mean_pow = data.iter().map(|x| (x / top).powf(k)).sum::<f64>() / data.len() as f64;
    Some((k, top * mean_pow.powf(1.0 / k)))
}

fn digamma(mut x: f64) -> f64 {
    let mut result = 0.0;
    while x < 6.0 {
        result -= 1.0 / x;
        x += 1.0;
    }
    let f = 1.0 / (x * x);
    result + x.ln()
        - 0.5 / x
        - f * (1.0 / 12.0 - f * (1.0 / 120.0 - f * (1.0 / 252.0 - f * (1.0 / 240.0 - f / 132.0))))
}

fn trigamma(mut x: f64) -> f64 {
    let mut result = 0.0;
    while x < 6.0 {
        result += 1.0 / (x * x);
        x += 1.0;
    }
    let f = 1.0 / (x * x);
    result
        + 1.0 / x
        + f / 2.0
        + f / x * (1.0 / 6.0 - f * (1.0 / 30.0 - f * (1.0 / 42.0 - f / 30.0)))
}

/// Regularized lower incomplete gamma P(a, x): a series below a + 1, a
/// continued fraction above (Numerical Recipes, §6.2).
fn gamma_p(a: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let front = (-x + a * x.ln() - ln_gamma(a)).exp();
    if x < a + 1.0 {
        let (mut term, mut sum, mut ap) = (1.0 / a, 1.0 / a, a);
        for _ in 0..1000 {
            ap += 1.0;
            term *= x / ap;
            sum += term;
            if term.abs() < sum.abs() * 1e-15 {
                break;
            }
        }
        (sum * front).clamp(0.0, 1.0)
    } else {
        let tiny = 1e-300;
        let mut b = x + 1.0 - a;
        let mut c = 1.0 / tiny;
        let mut d = 1.0 / b;
        let mut h = d;
        for i in 1..1000 {
            let an = -(i as f64) * (i as f64 - a);
            b += 2.0;
            d = an * d + b;
            if d.abs() < tiny {
                d = tiny;
            }
            c = b + an / c;
            if c.abs() < tiny {
                c = tiny;
            }
            d = 1.0 / d;
            let delta = d * c;
            h *= delta;
            if (delta - 1.0).abs() < 1e-15 {
                break;
            }
        }
        (1.0 - front * h).clamp(0.0, 1.0)
    }
}

/// Standard normal CDF, from erfc (Numerical Recipes' Chebyshev fit, error
/// below 1.2e-7).
fn normal_cdf(z: f64) -> f64 {
    let x = -z / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.5 * x.abs());
    let y = t
        * (-x * x - 1.265_512_23
            + t * (1.000_023_68
                + t * (0.374_091_96
                    + t * (0.096_784_18
                        + t * (-0.186_288_06
                            + t * (0.278_868_07
                                + t * (-1.135_203_98
                                    + t * (1.488_515_87
                                        + t * (-0.822_152_23 + t * 0.170_872_77)))))))))
            .exp();
    let erfc = if x >= 0.0 { y } else { 2.0 - y };
    0.5 * erfc
}

#[cfg(test)]
mod tests {
    use super::*;
    use simulancer_engine::rng::Rng;
    use simulancer_engine::spec::Distribution;

    fn draws(d: &Distribution, n: usize) -> Vec<f64> {
        let mut rng = Rng::from_seed(20261127);
        (0..n).map(|_| rng.sample(d)).collect()
    }

    fn number_in(expression: &str, prefix: &str) -> f64 {
        let rest = &expression[expression.find(prefix).unwrap() + prefix.len()..];
        let end = rest.find([',', ' ', ')']).unwrap();
        rest[..end].parse().unwrap()
    }

    #[test]
    fn recovers_the_distribution_that_made_the_data() {
        let gamma = draws(
            &Distribution::Gamma {
                shape: 3.0,
                scale: 2.0,
            },
            5000,
        );
        let best = &fit(&gamma, "min")[0];
        assert!(best.expression.starts_with("gamma("), "{}", best.expression);
        assert!((number_in(&best.expression, "gamma(") - 3.0).abs() < 0.15);

        let weibull = draws(
            &Distribution::Weibull {
                shape: 2.5,
                scale: 10.0,
            },
            5000,
        );
        let best = &fit(&weibull, "min")[0];
        assert!(
            best.expression.starts_with("weibull("),
            "{}",
            best.expression
        );
        assert!((number_in(&best.expression, "weibull(") - 2.5).abs() < 0.1);

        let lognormal = draws(&Distribution::Lognormal { mean: 5.0, sd: 4.0 }, 5000);
        assert!(
            fit(&lognormal, "min")[0]
                .expression
                .starts_with("lognormal(")
        );

        let exponential = draws(&Distribution::Exponential { mean: 4.0 }, 5000);
        let fits = fit(&exponential, "min");
        let e = fits
            .iter()
            .find(|c| c.expression.starts_with("exponential("))
            .unwrap();
        assert!(e.p > 0.05, "{}", e.p);
        // Gamma and Weibull nest the exponential, so one of the three wins.
        assert!(
            ["exponential(", "gamma(", "weibull("]
                .iter()
                .any(|p| fits[0].expression.starts_with(p))
        );

        let uniform = draws(&Distribution::Uniform { min: 2.0, max: 6.0 }, 5000);
        assert!(fit(&uniform, "min")[0].expression.starts_with("uniform("));
    }

    #[test]
    fn special_functions() {
        // P(1, x) is the exponential CDF; P(a, x) for a = 2.5 at x = 2.5 is 0.584.
        assert!((gamma_p(1.0, 2.0) - (1.0 - (-2.0f64).exp())).abs() < 1e-12);
        assert!((gamma_p(2.5, 2.5) - 0.584_119_4).abs() < 1e-6);
        assert!((gamma_p(3.0, 10.0) - 0.997_230_3).abs() < 1e-6);
        assert!((normal_cdf(1.96) - 0.975).abs() < 1e-4);
        assert!((normal_cdf(-1.0) - 0.158_655_25).abs() < 1e-6);
        assert!((digamma(1.0) + std::f64::consts::EULER_GAMMA).abs() < 1e-9);
        assert!((trigamma(1.0) - std::f64::consts::PI.powi(2) / 6.0).abs() < 1e-9);
        assert!((ks_p_value(0.0, 100) - 1.0).abs() < 1e-12);
        // λ ≈ 1.36 gives p ≈ 0.05.
        assert!((ks_p_value(1.358 / (10.0 + 0.12 + 0.011), 100) - 0.05).abs() < 0.002);
    }

    #[test]
    fn short_numbers() {
        assert_eq!(short(2.71234), "2.712");
        assert_eq!(short(1234.6), "1235");
        assert_eq!(short(0.012345), "0.01235");
        assert_eq!(short(2.0), "2");
    }
}
