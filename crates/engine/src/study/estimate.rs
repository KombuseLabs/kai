//! What a study of a given size can see, before anything is spent: the
//! trial count, the worst-case tokens, the smallest saving it detects and
//! the chance of `better` at a few true savings (spec 5.3, 5.4 and 11).
//!
//! The spread is assumed until a pilot has measured it: the standard
//! deviation of log tokens within a task and arm `s`, and a task-by-arm
//! interaction `t`. A task's value is the log of the ratio of the arms'
//! means over `r` repetitions, with standard deviation
//! `sqrt(2 s^2 / r + t^2)`. [`Spread::Observed`] takes the standard
//! deviation of the per-task log ratios a pilot gave
//! ([`super::verdict::Comparison::tokens`]) instead.
//!
//! Everything here is information for the owner. It is not an approval, and
//! it prints no price: prices are the proxy's.

use types::study::Question;

use super::stats;

/// Where the spread of the per-task values comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Spread {
    /// Assumed, not measured (the spec's 0.35 and 0.15).
    Assumed {
        /// Standard deviation of log tokens of one task and arm, `s`.
        within_task_sd: f64,
        /// The task-by-arm interaction, `t`.
        task_by_arm_sd: f64,
    },
    /// Measured: the standard deviation of the per-task log ratios, at the
    /// repetitions the pilot had.
    Observed {
        /// The standard deviation.
        sd_per_task: f64,
    },
}

impl Default for Spread {
    fn default() -> Spread {
        Spread::Assumed {
            within_task_sd: 0.35,
            task_by_arm_sd: 0.15,
        }
    }
}

/// A study's size and limits, the minimal inputs of an estimate. The study
/// file of S4 will hold them; until then `kai study estimate` reads exactly
/// these.
#[derive(Clone, Debug, PartialEq)]
pub struct Design {
    /// How many tasks.
    pub tasks: usize,
    /// How many repetitions of each task and arm.
    pub repetitions: u32,
    /// How many arms, the shadow included.
    pub arms: usize,
    /// The turn limit of a trial.
    pub max_turns: u32,
    /// The model's context window in tokens.
    pub context_tokens: u64,
    /// The bound a request has on its output, 32,000 at the proxy.
    pub max_output_tokens: u64,
    /// Where the spread comes from.
    pub spread: Spread,
    /// The thresholds the verdict will use.
    pub question: Question,
}

/// The default output bound of a request, the proxy's.
pub const MAX_OUTPUT_TOKENS: u64 = 32_000;

/// The true savings the chance of `better` is given for.
pub const SAVINGS: [f64; 5] = [0.0, 0.15, 0.20, 0.25, 0.30];

/// Why a design cannot be estimated.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EstimateError {
    /// The paired test needs two tasks at the least.
    #[error("a study needs at least 2 tasks to have an interval, got {0}")]
    TooFewTasks(usize),
    /// A comparison needs two arms.
    #[error("a study needs at least 2 arms, got {0}")]
    TooFewArms(usize),
    /// No repetition, no trial.
    #[error("a study needs at least 1 repetition")]
    NoRepetitions,
    /// A number the formulas cannot use.
    #[error("{0}")]
    Invalid(String),
    /// The worst case does not fit in 64 bits.
    #[error("the worst-case token count is too large to compute")]
    Overflow,
}

/// The chance of `better` at one true saving.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chance {
    /// The true saving, 0.20 for 20 %.
    pub saving: f64,
    /// The chance that the study says `better`.
    pub better: f64,
}

/// What a design gives.
#[derive(Clone, Debug, PartialEq)]
pub struct Estimate {
    /// Tasks times repetitions times arms.
    pub trials: u64,
    /// `max_turns x (context_tokens + max_output_tokens)`: a bound, absurdly
    /// high on purpose.
    pub tokens_per_trial_max: u64,
    /// That times the trials.
    pub tokens_max: u64,
    /// The standard deviation of a task's log ratio.
    pub sd_per_task: f64,
    /// The smallest true log ratio seen with 80 % power, two-sided at alpha.
    pub detectable_log_ratio: f64,
    /// That as a saving, `1 - exp(-log ratio)`.
    pub detectable_saving: f64,
    /// The chance of `better` at [`SAVINGS`], by the normal approximation.
    pub chance_of_better: Vec<Chance>,
}

/// The standard deviation of a task's log ratio under `spread` at
/// `repetitions`.
pub fn sd_per_task(spread: Spread, repetitions: u32) -> f64 {
    match spread {
        Spread::Assumed {
            within_task_sd: s,
            task_by_arm_sd: t,
        } => (2.0 * s * s / repetitions as f64 + t * t).sqrt(),
        Spread::Observed { sd_per_task } => sd_per_task,
    }
}

/// Estimates what `design` can see.
pub fn estimate(design: &Design) -> Result<Estimate, EstimateError> {
    if design.tasks < 2 {
        return Err(EstimateError::TooFewTasks(design.tasks));
    }
    if design.arms < 2 {
        return Err(EstimateError::TooFewArms(design.arms));
    }
    if design.repetitions == 0 {
        return Err(EstimateError::NoRepetitions);
    }
    let q = &design.question;
    if !(q.alpha > 0.0 && q.alpha < 1.0) {
        return Err(EstimateError::Invalid(format!(
            "alpha must be between 0 and 1, got {}",
            q.alpha
        )));
    }
    if !(q.min_saving > 0.0 && q.min_saving < 1.0) {
        return Err(EstimateError::Invalid(format!(
            "min_saving must be between 0 and 1, got {}",
            q.min_saving
        )));
    }
    if !(q.margin > 0.0 && q.margin <= 1.0) {
        return Err(EstimateError::Invalid(format!(
            "margin must be above 0 and at most 1, got {}",
            q.margin
        )));
    }
    let sd = sd_per_task(design.spread, design.repetitions);
    if !(sd.is_finite() && sd > 0.0) {
        return Err(EstimateError::Invalid(format!(
            "the spread per task must be a positive number, got {sd}"
        )));
    }

    let trials = design.tasks as u64 * design.repetitions as u64 * design.arms as u64;
    let per_trial = design
        .context_tokens
        .checked_add(design.max_output_tokens)
        .and_then(|window| window.checked_mul(design.max_turns as u64))
        .ok_or(EstimateError::Overflow)?;
    let tokens_max = per_trial
        .checked_mul(trials)
        .ok_or(EstimateError::Overflow)?;

    // One comparison per ghost: the arms but the shadow.
    let comparisons = design.arms - 1;
    let detectable_log_ratio = stats::detectable_effect(
        sd,
        design.tasks,
        q.alpha,
        comparisons,
        super::verdict::POWER,
    )
    .ok_or_else(|| EstimateError::Invalid("no detectable effect".to_owned()))?;

    // `better` needs the interval below 1 and the point at or below the
    // bar; the point estimate is about normal with this standard error.
    let se = sd / (design.tasks as f64).sqrt();
    let t = stats::t_quantile(1.0 - q.alpha / 2.0, (design.tasks - 1) as f64);
    let needed = q.bar().ln().min(-t * se);
    let chance_of_better = SAVINGS
        .iter()
        .map(|&saving| Chance {
            saving,
            better: stats::normal_cdf((needed - (1.0 - saving).ln()) / se),
        })
        .collect();

    Ok(Estimate {
        trials,
        tokens_per_trial_max: per_trial,
        tokens_max,
        sd_per_task: sd,
        detectable_log_ratio,
        detectable_saving: 1.0 - (-detectable_log_ratio).exp(),
        chance_of_better,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn design(tasks: usize, repetitions: u32) -> Design {
        Design {
            tasks,
            repetitions,
            arms: 2,
            max_turns: 40,
            context_tokens: 200_000,
            max_output_tokens: MAX_OUTPUT_TOKENS,
            spread: Spread::default(),
            question: Question::default(),
        }
    }

    // The table of spec 5.3 (arithmetic on assumed spreads). Its row for 24
    // tasks and 3 repetitions says 0.191; the formula gives 0.193.
    #[test]
    fn the_size_table_of_the_spec_is_reproduced() {
        for (tasks, reps, trials, log_ratio, saving_percent) in [
            (12, 3, 72, 0.287, 25.0),
            (18, 3, 108, 0.226, 20.0),
            (24, 1, 48, 0.309, 27.0),
            (24, 2, 96, 0.228, 20.0),
            (24, 3, 144, 0.191, 17.0),
            (36, 2, 144, 0.183, 17.0),
        ] {
            let e = estimate(&design(tasks, reps)).unwrap();
            assert_eq!(e.trials, trials);
            assert!(
                (e.detectable_log_ratio - log_ratio).abs() < 0.002,
                "{tasks}x{reps}: {}",
                e.detectable_log_ratio
            );
            assert!(
                (e.detectable_saving * 100.0 - saving_percent).abs() < 0.8,
                "{tasks}x{reps}: {}",
                e.detectable_saving
            );
        }
    }

    #[test]
    fn the_spread_per_task_is_the_specs() {
        for (reps, sd) in [(1, 0.517), (2, 0.381), (3, 0.323), (5, 0.267)] {
            let got = sd_per_task(Spread::default(), reps);
            assert!((got - sd).abs() < 0.0006, "r = {reps}: {got}");
        }
    }

    #[test]
    fn the_chance_of_better_is_the_specs() {
        let e = estimate(&design(24, 2)).unwrap();
        for (chance, want) in e
            .chance_of_better
            .iter()
            .zip([0.002, 0.22, 0.50, 0.80, 0.96])
        {
            assert!(
                (chance.better - want).abs() < 0.012,
                "{}: {} against {want}",
                chance.saving,
                chance.better
            );
        }
    }

    #[test]
    fn the_worst_case_is_the_specs() {
        let e = estimate(&design(24, 2)).unwrap();
        assert_eq!(e.tokens_per_trial_max, 40 * 232_000);
        assert_eq!(e.tokens_max, 96 * 40 * 232_000);
        // The spec rounds this to 891,000,000.
        assert_eq!(e.tokens_max, 890_880_000);
    }

    #[test]
    fn an_observed_spread_replaces_the_assumed_one() {
        let mut d = design(8, 2);
        let assumed = estimate(&d).unwrap();
        d.spread = Spread::Observed { sd_per_task: 0.1 };
        let observed = estimate(&d).unwrap();
        assert_eq!(observed.sd_per_task, 0.1);
        assert!(observed.detectable_saving < assumed.detectable_saving);
    }

    #[test]
    fn thresholds_are_parameters() {
        let mut d = design(24, 2);
        d.question.min_saving = 0.10;
        let easy = estimate(&d).unwrap();
        let hard = estimate(&design(24, 2)).unwrap();
        assert!(easy.chance_of_better[1].better > hard.chance_of_better[1].better);
    }

    #[test]
    fn a_design_that_cannot_be_estimated_is_refused() {
        assert_eq!(estimate(&design(1, 2)), Err(EstimateError::TooFewTasks(1)));
        assert_eq!(estimate(&design(5, 0)), Err(EstimateError::NoRepetitions));
        let mut d = design(5, 2);
        d.arms = 1;
        assert_eq!(estimate(&d), Err(EstimateError::TooFewArms(1)));
        let mut d = design(5, 2);
        d.context_tokens = u64::MAX;
        assert_eq!(estimate(&d), Err(EstimateError::Overflow));
        let mut d = design(5, 2);
        d.question.alpha = 0.0;
        assert!(matches!(estimate(&d), Err(EstimateError::Invalid(_))));
        let mut d = design(5, 2);
        d.spread = Spread::Observed { sd_per_task: 0.0 };
        assert!(matches!(estimate(&d), Err(EstimateError::Invalid(_))));
    }
}
