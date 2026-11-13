//! Statistics on paired per-task values.
//!
//! Trials of the same task are correlated, so every test works on one value
//! per task: the treatment-minus-control difference of the task means (or
//! the log of their ratio). Each task gets every arm, so under the null
//! hypothesis each per-task difference is as likely positive as negative,
//! and flipping signs gives an exact test with no distributional assumption.
//!
//! Ported from old kai (commit c483307, `src/engine/stats.rs`) with the same
//! numerics, so that its test vectors hold bit for bit. Left behind: the
//! bootstrap interval, which the study spec does not use. Which of the
//! t-interval and the sign-flip test decides is not decided here but in
//! [`super::verdict`]: the interval decides, the sign-flip p is a check.

use super::plan::Rng;

/// How a sign-flip p-value was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// All 2^n sign assignments enumerated.
    SignFlipExact,
    /// Random sign assignments (more than `EXACT_LIMIT` tasks).
    SignFlipMonteCarlo,
}

/// Up to this many tasks the sign-flip test enumerates every assignment.
pub const EXACT_LIMIT: usize = 20;

/// Result on the analysis scale (difference, or log ratio).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PairedTest {
    /// The mean of the per-task values.
    pub mean: f64,
    /// Two-sided t-interval at the requested level.
    pub ci_low: f64,
    /// The upper end of that interval.
    pub ci_high: f64,
    /// One-sided lower bound at 1 - alpha, for non-inferiority.
    pub lower_bound: f64,
    /// Two-sided sign-flip p-value.
    pub p_value: f64,
    /// How `p_value` was made.
    pub method: Method,
}

/// The mean of `values`; `NaN` when there are none, so callers check first.
pub fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn std_error(values: &[f64]) -> f64 {
    let n = values.len() as f64;
    let m = mean(values);
    let var = values.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (n - 1.0);
    (var / n).sqrt()
}

/// Paired test on per-task values; `None` with fewer than two tasks.
pub fn paired_test(values: &[f64], alpha: f64, resamples: u32, seed: u64) -> Option<PairedTest> {
    if values.len() < 2 {
        return None;
    }
    let df = (values.len() - 1) as f64;
    let m = mean(values);
    let se = std_error(values);
    let two_sided = t_quantile(1.0 - alpha / 2.0, df) * se;
    let one_sided = t_quantile(1.0 - alpha, df) * se;
    let (p_value, method) = sign_flip(values, resamples, seed);
    Some(PairedTest {
        mean: m,
        ci_low: m - two_sided,
        ci_high: m + two_sided,
        lower_bound: m - one_sided,
        p_value,
        method,
    })
}

/// Two-sided sign-flip p-value for the mean of `values` against 0.
pub fn sign_flip(values: &[f64], resamples: u32, seed: u64) -> (f64, Method) {
    let n = values.len();
    let observed = values.iter().sum::<f64>().abs();
    // Guards against floating-point noise when sums are equal. Relative to
    // the size of the values, so that the p-value does not depend on their
    // unit; not to the observed sum, which is near 0 where values cancel.
    let tolerance = 1e-9 * values.iter().map(|v| v.abs()).sum::<f64>();
    if n <= EXACT_LIMIT {
        let total = 1u64 << n;
        let mut extreme = 0u64;
        for mask in 0..total {
            let sum: f64 = values
                .iter()
                .enumerate()
                .map(|(i, v)| if mask >> i & 1 == 1 { -v } else { *v })
                .sum();
            if sum.abs() >= observed - tolerance {
                extreme += 1;
            }
        }
        (extreme as f64 / total as f64, Method::SignFlipExact)
    } else {
        let mut rng = Rng::new(seed);
        let mut extreme = 0u64;
        for _ in 0..resamples {
            let sum: f64 = values
                .iter()
                .map(|v| if rng.next_u64() & 1 == 1 { -v } else { *v })
                .sum();
            if sum.abs() >= observed - tolerance {
                extreme += 1;
            }
        }
        (
            (extreme as f64 + 1.0) / (resamples as f64 + 1.0),
            Method::SignFlipMonteCarlo,
        )
    }
}

/// Smallest two-sided p-value an exact sign-flip test on `n` tasks can reach.
pub fn min_sign_flip_p(n: usize) -> f64 {
    if n > EXACT_LIMIT {
        0.0
    } else {
        2.0 / (1u64 << n) as f64
    }
}

/// Smallest true effect a paired test over `tasks` per-task values with
/// standard deviation `sd` detects with probability `power`, at `alpha`
/// two-sided after a Bonferroni split over `tests` (a conservative stand-in
/// for Holm). On the analysis scale (difference, or log ratio).
pub fn detectable_effect(
    sd: f64,
    tasks: usize,
    alpha: f64,
    tests: usize,
    power: f64,
) -> Option<f64> {
    if tasks < 2 || tests == 0 {
        return None;
    }
    let df = (tasks - 1) as f64;
    let z = t_quantile(1.0 - alpha / (2.0 * tests as f64), df) + t_quantile(power, df);
    Some(z * sd / (tasks as f64).sqrt())
}

/// Sample standard deviation; `None` below two values.
pub fn std_dev(values: &[f64]) -> Option<f64> {
    (values.len() >= 2).then(|| std_error(values) * (values.len() as f64).sqrt())
}

/// Wilson score interval for a proportion, at the level of `z`.
pub fn wilson(successes: usize, n: usize, z: f64) -> Option<(f64, f64)> {
    if n == 0 {
        return None;
    }
    let n = n as f64;
    let p = successes as f64 / n;
    let z2 = z * z;
    let center = (p + z2 / (2.0 * n)) / (1.0 + z2 / n);
    let half = z / (1.0 + z2 / n) * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt();
    Some(((center - half).max(0.0), (center + half).min(1.0)))
}

/// Holm step-down adjustment; returns adjusted p-values in input order.
pub fn holm(p_values: &[f64]) -> Vec<f64> {
    let m = p_values.len();
    let mut order: Vec<usize> = (0..m).collect();
    order.sort_by(|&a, &b| p_values[a].total_cmp(&p_values[b]));
    let mut adjusted = vec![0.0; m];
    let mut running: f64 = 0.0;
    for (rank, &i) in order.iter().enumerate() {
        running = running.max(((m - rank) as f64 * p_values[i]).min(1.0));
        adjusted[i] = running;
    }
    adjusted
}

/// Natural log of the gamma function (Lanczos approximation, g = 7).
fn ln_gamma(x: f64) -> f64 {
    const C: [f64; 9] = [
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
        std::f64::consts::PI.ln() - (std::f64::consts::PI * x).sin().ln() - ln_gamma(1.0 - x)
    } else {
        let x = x - 1.0;
        let mut a = C[0];
        let t = x + 7.5;
        for (i, c) in C.iter().enumerate().skip(1) {
            a += c / (x + i as f64);
        }
        0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
    }
}

/// Continued fraction for the incomplete beta function (modified Lentz).
fn beta_cf(a: f64, b: f64, x: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let (qab, qap, qam) = (a + b, a + 1.0, a - 1.0);
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < TINY {
        d = TINY;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=300 {
        let m = m as f64;
        let m2 = 2.0 * m;
        for aa in [
            m * (b - m) * x / ((qam + m2) * (a + m2)),
            -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2)),
        ] {
            d = 1.0 + aa * d;
            if d.abs() < TINY {
                d = TINY;
            }
            c = 1.0 + aa / c;
            if c.abs() < TINY {
                c = TINY;
            }
            d = 1.0 / d;
            h *= d * c;
        }
        if (d * c - 1.0).abs() < 1e-15 {
            break;
        }
    }
    h
}

/// Regularized incomplete beta function I_x(a, b).
fn beta_inc(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let front =
        (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        front * beta_cf(a, b, x) / a
    } else {
        1.0 - front * beta_cf(b, a, 1.0 - x) / b
    }
}

fn t_cdf(t: f64, df: f64) -> f64 {
    let tail = 0.5 * beta_inc(df / 2.0, 0.5, df / (df + t * t));
    if t > 0.0 { 1.0 - tail } else { tail }
}

/// Quantile of Student's t distribution, by bisection on the CDF.
pub fn t_quantile(p: f64, df: f64) -> f64 {
    if p == 0.5 {
        return 0.0;
    }
    if p < 0.5 {
        return -t_quantile(1.0 - p, df);
    }
    let mut hi = 1.0;
    while t_cdf(hi, df) < p {
        hi *= 2.0;
    }
    let mut lo = 0.0;
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if t_cdf(mid, df) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// The standard normal distribution function, by the Abramowitz and Stegun
/// formula 7.1.26 for the error function (absolute error below 1.5e-7).
/// For the normal approximation of the power table, where that is plenty.
pub fn normal_cdf(x: f64) -> f64 {
    let z = x.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * z);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erf = 1.0 - poly * (-z * z).exp();
    if x >= 0.0 {
        0.5 * (1.0 + erf)
    } else {
        0.5 * (1.0 - erf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every vector below was produced by running old kai's code (commit
    /// c483307) and is compared by bit pattern, not within a tolerance.
    fn same(got: f64, want: f64) {
        assert_eq!(got.to_bits(), want.to_bits(), "got {got:?}, want {want:?}");
    }

    #[test]
    fn t_quantiles_reproduce_old_kai() {
        for (p, df, want) in [
            (0.975, 1.0, 12.70620473617467),
            (0.975, 5.0, 2.570581835636313),
            (0.95, 10.0, 1.8124611228116771),
            (0.975, 23.0, 2.0686576104190477),
            (0.8, 23.0, 0.8575295536880341),
            (0.995, 3.0, 5.840909309733334),
            (0.025, 5.0, -2.570581835636313),
            (0.9875, 9.0, 2.6850108468164544),
            (0.8, 9.0, 0.8834038596855358),
            (0.95, 23.0, 1.7138715277470462),
        ] {
            same(t_quantile(p, df), want);
        }
    }

    #[test]
    fn t_quantiles_match_tables() {
        for (p, df, expected) in [
            (0.975, 1.0, 12.706_205),
            (0.975, 5.0, 2.570_582),
            (0.95, 10.0, 1.812_461),
            (0.975, 30.0, 2.042_272),
            (0.995, 3.0, 5.840_909),
        ] {
            let q = t_quantile(p, df);
            assert!((q - expected).abs() < 1e-4, "t({p}, {df}) = {q}");
        }
        assert!((t_quantile(0.025, 5.0) + 2.570_582).abs() < 1e-4);
    }

    #[test]
    fn paired_test_reproduces_old_kai() {
        let t = paired_test(&[0.1, 0.3, 0.2, 0.4], 0.05, 1000, 1).unwrap();
        same(t.mean, 0.25);
        same(t.ci_low, 0.04457397432394808);
        same(t.ci_high, 0.45542602567605195);
        same(t.lower_bound, 0.09809104349065101);
        same(t.p_value, 0.125);
        assert_eq!(t.method, Method::SignFlipExact);

        let values: Vec<f64> = (0..24)
            .map(|i| ((i * 37 % 11) as f64 - 3.0) / 10.0)
            .collect();
        let t = paired_test(&values, 0.05, 5000, 9).unwrap();
        same(t.mean, 0.17500000000000002);
        same(t.ci_low, 0.03732391488429293);
        same(t.ci_high, 0.3126760851157071);
        same(t.lower_bound, 0.06093611569983734);
        same(t.p_value, 0.01879624075184963);
        assert_eq!(t.method, Method::SignFlipMonteCarlo);
    }

    #[test]
    fn paired_test_interval_and_bounds() {
        let t = paired_test(&[0.1, 0.3, 0.2, 0.4], 0.05, 1000, 1).unwrap();
        let se = (0.05f64 / 3.0 / 4.0).sqrt();
        assert!((t.ci_low - (0.25 - 3.182_446 * se)).abs() < 1e-4);
        assert!(t.lower_bound > t.ci_low);
        // Identical values: zero-width interval.
        let flat = paired_test(&[0.0; 4], 0.05, 1000, 1).unwrap();
        assert_eq!((flat.ci_low, flat.ci_high, flat.p_value), (0.0, 0.0, 1.0));
        assert!(paired_test(&[1.0], 0.05, 1000, 1).is_none());
    }

    #[test]
    fn sign_flip_reproduces_old_kai() {
        let (p, method) = sign_flip(&[1.0; 21], 2000, 3);
        same(p, 0.0004997501249375312);
        assert_eq!(method, Method::SignFlipMonteCarlo);
        let ramp: Vec<f64> = (0..21).map(|i| (i as f64) * 0.1 - 0.7).collect();
        same(sign_flip(&ramp, 4000, 5).0, 0.042989252686828294);
        let eight = [0.5, -0.2, 0.9, 0.4, -0.1, 0.3, 0.8, 0.6];
        let (p, method) = sign_flip(&eight, 10, 1);
        same(p, 0.0390625);
        assert_eq!(method, Method::SignFlipExact);
    }

    #[test]
    fn sign_flip_is_exact_for_small_studies() {
        let (p, method) = sign_flip(&[-1.0; 6], 1000, 1);
        assert_eq!(method, Method::SignFlipExact);
        assert!((p - 2.0 / 64.0).abs() < 1e-12);
        assert_eq!(min_sign_flip_p(6), 2.0 / 64.0);
        assert_eq!(min_sign_flip_p(21), 0.0);
        // No effect at all, or effects that cancel, give p = 1.
        assert_eq!(sign_flip(&[0.0; 5], 1000, 1).0, 1.0);
        assert_eq!(sign_flip(&[1.0, -1.0], 1000, 1).0, 1.0);
        // Zero differences carry no information: three of ten tasks differ.
        let mut values = vec![0.0; 10];
        values[..3].fill(-1.0);
        assert!((sign_flip(&values, 1000, 1).0 - 0.25).abs() < 1e-12);
    }

    #[test]
    fn sign_flip_preserves_scale() {
        // The tolerance is relative to the size of the values, so the
        // p-value does not depend on their unit (old kai's D7 fix).
        let unit = sign_flip(&[1.0; 6], 1000, 1);
        let tiny = sign_flip(&[2.0f64.powi(-40); 6], 1000, 1);
        assert_eq!(unit, (2.0 / 64.0, Method::SignFlipExact));
        assert_eq!(tiny, unit);
        let unit = sign_flip(&[1.0; 21], 2000, 3);
        let tiny = sign_flip(&[2.0f64.powi(-40); 21], 2000, 3);
        assert_eq!(unit.1, Method::SignFlipMonteCarlo);
        assert_eq!(tiny, unit);
        let huge = sign_flip(&[2.0f64.powi(40); 6], 1000, 1);
        assert_eq!(huge, (2.0 / 64.0, Method::SignFlipExact));
    }

    #[test]
    fn sign_flip_guard_covers_rounding_where_values_cancel() {
        // -0.3 + 0.1 + 0.2 is 2.8e-17 in f64, not 0: a guard relative to
        // the observed sum alone would be too small.
        assert_eq!(sign_flip(&[-0.3, 0.1, 0.2, 1e-9], 1000, 1).0, 1.0);
    }

    #[test]
    fn sign_flip_monte_carlo_is_seeded() {
        let values = vec![1.0; 30];
        let (p, method) = sign_flip(&values, 2000, 3);
        assert_eq!(method, Method::SignFlipMonteCarlo);
        assert!(p < 0.002);
        assert_eq!(p, sign_flip(&values, 2000, 3).0);
    }

    #[test]
    fn detectable_effect_reproduces_old_kai() {
        same(
            detectable_effect(0.2, 10, 0.05, 2, 0.8).unwrap(),
            0.22568636217175092,
        );
        same(
            detectable_effect(0.381, 24, 0.05, 1, 0.8).unwrap(),
            0.22757337784522497,
        );
        same(
            detectable_effect(0.323, 12, 0.05, 1, 0.8).unwrap(),
            0.28686063517451005,
        );
        assert!(detectable_effect(0.2, 1, 0.05, 2, 0.8).is_none());
        assert!(detectable_effect(0.2, 10, 0.05, 0, 0.8).is_none());
    }

    #[test]
    fn detectable_effect_shrinks_with_tasks() {
        let ten = detectable_effect(0.2, 10, 0.05, 2, 0.8).unwrap();
        let forty = detectable_effect(0.2, 40, 0.05, 2, 0.8).unwrap();
        assert!(forty < ten / 1.9);
        assert!((ten - (2.685 + 0.8834) * 0.2 / 10f64.sqrt()).abs() < 2e-3);
    }

    #[test]
    fn std_dev_reproduces_old_kai() {
        assert_eq!(std_dev(&[1.0]), None);
        same(std_dev(&[1.0, 3.0]).unwrap(), std::f64::consts::SQRT_2);
        let eight = [0.5, -0.2, 0.9, 0.4, -0.1, 0.3, 0.8, 0.6];
        same(std_dev(&eight).unwrap(), 0.39279220242478635);
    }

    #[test]
    fn wilson_reproduces_old_kai() {
        let (lo, hi) = wilson(18, 18, 1.959_964).unwrap();
        same(lo, 0.8241207740667156);
        same(hi, 1.0);
        let (lo, hi) = wilson(5, 10, 1.959_964).unwrap();
        same(lo, 0.2365930890114794);
        same(hi, 0.7634069109885206);
        let (lo, hi) = wilson(3, 7, 1.96).unwrap();
        same(lo, 0.15821692226262685);
        same(hi, 0.7495457695909742);
        assert!(wilson(0, 0, 1.96).is_none());
    }

    #[test]
    fn holm_reproduces_old_kai() {
        assert_eq!(holm(&[0.01, 0.04, 0.03]), [0.03, 0.06, 0.06]);
        assert_eq!(holm(&[0.2, 0.001, 0.04, 0.5]), [0.4, 0.004, 0.12, 0.5]);
    }

    #[test]
    fn normal_cdf_is_close_to_tables() {
        for (x, want) in [
            (0.0, 0.5),
            (1.0, 0.841_344_746),
            (1.644_854, 0.95),
            (-2.0, 0.022_750_132),
            (2.857, 0.997_861),
        ] {
            assert!((normal_cdf(x) - want).abs() < 2e-6, "{x}");
        }
    }
}
