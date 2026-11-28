//! Statistics across replications: means with Student-t confidence intervals.
//!
//! Implemented here (log-gamma, incomplete beta, t quantiles) to keep the
//! engine free of dependencies, like the random number generator.

/// Summary of one measure across replications.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    /// Replications with a value (a measure can be undefined in some, like
    /// time in system when no part exited).
    pub n: usize,
    pub mean: f64,
    pub sd: f64,
    /// Half-width of the confidence interval; NaN with fewer than 2 values.
    pub half_width: f64,
    pub min: f64,
    pub max: f64,
}

impl Summary {
    /// Summarizes the finite values, with a confidence interval at `level` (e.g. 0.95).
    pub fn of(values: &[f64], level: f64) -> Summary {
        let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
        let n = finite.len();
        if n == 0 {
            return Summary {
                n,
                mean: f64::NAN,
                sd: f64::NAN,
                half_width: f64::NAN,
                min: f64::NAN,
                max: f64::NAN,
            };
        }
        let mean = finite.iter().sum::<f64>() / n as f64;
        let (sd, half_width) = if n < 2 {
            (f64::NAN, f64::NAN)
        } else {
            let var = finite.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1) as f64;
            let sd = var.sqrt();
            let t = t_quantile(0.5 + level / 2.0, (n - 1) as f64);
            (sd, t * sd / (n as f64).sqrt())
        };
        Summary {
            n,
            mean,
            sd,
            half_width,
            min: finite.iter().copied().fold(f64::INFINITY, f64::min),
            max: finite.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        }
    }
}

/// Natural log of the gamma function (Lanczos, g = 7), for x > 0.
pub fn ln_gamma(x: f64) -> f64 {
    const COEFFICIENTS: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        // Reflection formula.
        return (std::f64::consts::PI / (std::f64::consts::PI * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut sum = COEFFICIENTS[0];
    for (i, c) in COEFFICIENTS.iter().enumerate().skip(1) {
        sum += c / (x + i as f64);
    }
    let t = x + 7.5;
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + sum.ln()
}

/// Continued fraction for the incomplete beta function (modified Lentz).
fn beta_continued_fraction(a: f64, b: f64, x: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let clamp = |v: f64| if v.abs() < TINY { TINY } else { v };
    let (qab, qap, qam) = (a + b, a + 1.0, a - 1.0);
    let mut c = 1.0;
    let mut d = 1.0 / clamp(1.0 - qab * x / qap);
    let mut h = d;
    for m in 1..=500 {
        let m = f64::from(m);
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 / clamp(1.0 + aa * d);
        c = clamp(1.0 + aa / c);
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 / clamp(1.0 + aa * d);
        c = clamp(1.0 + aa / c);
        let delta = d * c;
        h *= delta;
        if (delta - 1.0).abs() < 1e-15 {
            break;
        }
    }
    h
}

/// Regularized incomplete beta function I_x(a, b).
fn incomplete_beta(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let front =
        (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        front * beta_continued_fraction(a, b, x) / a
    } else {
        1.0 - front * beta_continued_fraction(b, a, 1.0 - x) / b
    }
}

/// Student-t cumulative distribution function.
pub fn t_cdf(t: f64, df: f64) -> f64 {
    let tail = 0.5 * incomplete_beta(df / 2.0, 0.5, df / (df + t * t));
    if t >= 0.0 { 1.0 - tail } else { tail }
}

/// Student-t quantile: the t with `t_cdf(t, df) == p`, for 0.5 <= p < 1.
pub fn t_quantile(p: f64, df: f64) -> f64 {
    assert!((0.5..1.0).contains(&p), "p = {p} out of range");
    let mut high = 1.0;
    while t_cdf(high, df) < p {
        high *= 2.0;
    }
    let mut low = 0.0;
    for _ in 0..200 {
        let mid = (low + high) / 2.0;
        if t_cdf(mid, df) < p {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ln_gamma_known_values() {
        assert!((ln_gamma(1.0)).abs() < 1e-13);
        assert!((ln_gamma(5.0) - 24f64.ln()).abs() < 1e-12);
        assert!((ln_gamma(0.5) - std::f64::consts::PI.sqrt().ln()).abs() < 1e-12);
    }

    #[test]
    fn t_quantiles_match_tables() {
        // 97.5th percentiles (two-sided 95%), from standard tables.
        let table = [
            (1.0, 12.706_205),
            (2.0, 4.302_653),
            (5.0, 2.570_582),
            (9.0, 2.262_157),
            (19.0, 2.093_024),
            (29.0, 2.045_230),
            (100.0, 1.983_972),
            (1e7, 1.959_964),
        ];
        for (df, expected) in table {
            let t = t_quantile(0.975, df);
            assert!((t - expected).abs() < 1e-5, "df {df}: {t} vs {expected}");
        }
        // 99.5th percentile, df 10.
        assert!((t_quantile(0.995, 10.0) - 3.169_273).abs() < 1e-5);
    }

    #[test]
    fn summary() {
        let s = Summary::of(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0], 0.95);
        assert_eq!(s.n, 8);
        assert_eq!(s.mean, 5.0);
        assert!((s.sd - 2.138_089_935).abs() < 1e-9);
        assert!((s.half_width - 2.364_624 * s.sd / 8f64.sqrt()).abs() < 1e-5);
        assert_eq!((s.min, s.max), (2.0, 9.0));
    }

    #[test]
    fn summary_skips_undefined_values() {
        let s = Summary::of(&[1.0, f64::NAN, 3.0], 0.95);
        assert_eq!((s.n, s.mean), (2, 2.0));
        let one = Summary::of(&[1.0], 0.95);
        assert!(one.half_width.is_nan());
    }
}
