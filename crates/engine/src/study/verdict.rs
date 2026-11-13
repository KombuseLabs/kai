//! The verdict: one ghost against the shadow, by the rule of the study
//! spec's section 4.4, on per-task values.
//!
//! Method (spec 9.1). The unit is the task: each task gives one value per
//! arm, the mean over the repetitions both arms have, and one paired
//! difference. Quality is the mean over tasks of the difference of pass
//! rates; tokens are the geometric-mean ratio, on the log scale. **The
//! t-interval of the per-task values decides.** The sign-flip p-value is a
//! check: when the interval would give `worse` or `better` and the p-value
//! does not agree (it is not below alpha), the verdict is
//! `not-distinguishable`, reason `methods-disagree`. The check is made only
//! where it could change a verdict: a p-value below alpha beside an interval
//! that holds the null stays `not-distinguishable` for its own reason.
//!
//! What is not a trial of the primary analysis (spec 4.5): only `counted`
//! trials enter. A trial without a score stays out of the quality
//! comparison, one without tokens out of the token comparison, and each is
//! named in the result: neither is ever 0, and the metric never turns into
//! another one. The sensitivity analysis counts a `limit` trial as a
//! failure, with its tokens as recorded; if it gives another verdict the
//! verdict is `not-distinguishable`, reason `sensitive-to-limits`.
//!
//! Input contract: at most one trial per (task, arm, repetition), the final
//! attempt of its slot. Trials of arms other than the two compared are
//! ignored.
//!
//! Holm across several ghosts (a placebo arm) is [`super::stats::holm`]; the
//! verdicts here are per ghost and apply no correction. Study 1 has one
//! ghost.

use std::collections::BTreeMap;

use types::study::{Interval, Question, Reason, StudyVerdict, Trial, TrialOutcome};

use super::stats::{self, PairedTest};

/// The power the detectable saving is stated for (the spec's 80 %).
pub const POWER: f64 = 0.80;

/// How the sign-flip check is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sampling {
    /// Random sign assignments above [`stats::EXACT_LIMIT`] tasks.
    pub resamples: u32,
    /// The seed of those, so the same trials give the same p-value.
    pub seed: u64,
}

impl Default for Sampling {
    fn default() -> Sampling {
        Sampling {
            resamples: 10_000,
            seed: 0x6B61_6931,
        }
    }
}

/// Why a trial, or a pair of them, is not in a comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropReason {
    /// The arm has no trial in that repetition.
    Missing,
    /// The trial's outcome is not one the analysis counts.
    NotCounted(TrialOutcome),
    /// A counted trial with no grade: no score, so no quality value.
    NotScored,
    /// A trial with no tokens known: no token value.
    NoTokens,
    /// A task whose mean tokens are 0 on an arm: the ratio is undefined.
    ZeroTokens,
}

/// A pair (task, repetition) that one metric left out, or a task, when the
/// repetition is `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dropped {
    /// The task.
    pub task: String,
    /// The repetition; `None` when the whole task is meant.
    pub repetition: Option<u32>,
    /// The arm that lacks the value.
    pub arm: String,
    /// What it lacks.
    pub why: DropReason,
}

/// One task's value on one metric.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskValue {
    /// The task.
    pub task: String,
    /// How many repetitions the means are over.
    pub repetitions: u32,
    /// The shadow's mean (pass rate or tokens).
    pub shadow: f64,
    /// The ghost's mean.
    pub ghost: f64,
    /// The per-task value the test runs on: the difference of pass rates,
    /// or the natural log of the token ratio ghost over shadow.
    pub value: f64,
}

/// One metric of a comparison.
#[derive(Clone, Debug, PartialEq)]
pub struct Metric {
    /// One value per task that has one on both arms.
    pub tasks: Vec<TaskValue>,
    /// What was left out, and why.
    pub dropped: Vec<Dropped>,
    /// The test on `tasks`; `None` with fewer than two.
    pub test: Option<PairedTest>,
}

impl Metric {
    fn new(tasks: Vec<TaskValue>, dropped: Vec<Dropped>, alpha: f64, sampling: Sampling) -> Metric {
        let values: Vec<f64> = tasks.iter().map(|t| t.value).collect();
        let test = stats::paired_test(&values, alpha, sampling.resamples, sampling.seed);
        Metric {
            tasks,
            dropped,
            test,
        }
    }
}

/// The token ratio ghost over shadow, as a ratio (the exponential of the
/// log-scale test).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ratio {
    /// The geometric mean of the per-task ratios.
    pub point: f64,
    /// Its interval at `1 - alpha`.
    pub interval: Interval,
}

/// One comparison, by the rule alone.
#[derive(Clone, Debug, PartialEq)]
pub struct Comparison {
    /// The pass-rate difference, ghost minus shadow, per task.
    pub quality: Metric,
    /// The log of the token ratio, per task.
    pub tokens: Metric,
    /// The token ratio and its interval; `None` without a token test.
    pub ratio: Option<Ratio>,
    /// The saving the interval allows: `1 - ratio`, its ends swapped. A
    /// negative end is a cost.
    pub saving_allowed: Option<Interval>,
    /// Whether the interval excludes a saving of `min_saving`: its lower
    /// bound is above the bar.
    pub excludes_bar: Option<bool>,
    /// The smallest saving this many tasks detect with 80 % power at the
    /// observed spread of the log ratios; `None` below two tasks.
    pub detectable_saving: Option<f64>,
    /// Tasks every trial of which passed or every trial failed, on both
    /// arms: they carry no information on quality. Named, not removed.
    pub no_signal_tasks: Vec<String>,
    /// What the rule says.
    pub verdict: StudyVerdict,
    /// Why.
    pub reason: Reason,
}

/// The answer: the primary comparison, the sensitivity analysis and the
/// verdict that stands.
#[derive(Clone, Debug, PartialEq)]
pub struct Assessment {
    /// `counted` trials only.
    pub primary: Comparison,
    /// `limit` trials counted as failures, with their tokens. The same as
    /// `primary` when no `limit` trial is among the two arms'.
    pub sensitivity: Comparison,
    /// The verdict that stands.
    pub verdict: StudyVerdict,
    /// Why.
    pub reason: Reason,
}

/// Why the trials cannot be analysed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VerdictError {
    /// A slot with two trials; only its final attempt belongs here.
    #[error("two trials for task `{task}`, arm `{arm}`, repetition {repetition}")]
    DuplicateTrial {
        /// The task.
        task: String,
        /// The arm.
        arm: String,
        /// The repetition.
        repetition: u32,
    },
    /// The ghost is the shadow.
    #[error("the shadow and the ghost are both `{0}`")]
    SameArm(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Primary,
    Sensitivity,
}

#[derive(Clone, Copy)]
enum Which {
    Quality,
    Tokens,
}

type Pairs<'a> = BTreeMap<(&'a str, u32), (Option<&'a Trial>, Option<&'a Trial>)>;

fn pair_up<'a>(trials: &'a [Trial], shadow: &str, ghost: &str) -> Result<Pairs<'a>, VerdictError> {
    let mut pairs: Pairs<'a> = BTreeMap::new();
    for trial in trials {
        let is_shadow = trial.arm == shadow;
        if !is_shadow && trial.arm != ghost {
            continue;
        }
        let entry = pairs
            .entry((trial.task.as_str(), trial.repetition))
            .or_default();
        let side = if is_shadow {
            &mut entry.0
        } else {
            &mut entry.1
        };
        if side.is_some() {
            return Err(VerdictError::DuplicateTrial {
                task: trial.task.clone(),
                arm: trial.arm.clone(),
                repetition: trial.repetition,
            });
        }
        *side = Some(trial);
    }
    Ok(pairs)
}

/// One arm's value for one metric in one repetition, or why it has none.
fn side_value(trial: Option<&Trial>, mode: Mode, which: Which) -> Result<f64, DropReason> {
    let trial = trial.ok_or(DropReason::Missing)?;
    let counted = match trial.outcome {
        TrialOutcome::Counted => true,
        TrialOutcome::Limit if mode == Mode::Sensitivity => false,
        other => return Err(DropReason::NotCounted(other)),
    };
    match which {
        // A limit trial of the sensitivity analysis is a failure whatever
        // else is known of it; every other score is a grade or nothing.
        Which::Quality if !counted => Ok(0.0),
        Which::Quality => match trial.pass {
            Some(true) => Ok(1.0),
            Some(false) => Ok(0.0),
            None => Err(DropReason::NotScored),
        },
        Which::Tokens => trial
            .total_tokens
            .map(|tokens| tokens as f64)
            .ok_or(DropReason::NoTokens),
    }
}

fn metric(
    pairs: &Pairs<'_>,
    shadow: &str,
    ghost: &str,
    mode: Mode,
    which: Which,
) -> (Vec<TaskValue>, Vec<Dropped>) {
    let mut eligible: BTreeMap<&str, Vec<(f64, f64)>> = BTreeMap::new();
    let mut dropped = Vec::new();
    for (&(task, repetition), &(s, g)) in pairs {
        match (side_value(s, mode, which), side_value(g, mode, which)) {
            (Ok(s), Ok(g)) => eligible.entry(task).or_default().push((s, g)),
            (Err(why), _) => dropped.push(Dropped {
                task: task.to_owned(),
                repetition: Some(repetition),
                arm: shadow.to_owned(),
                why,
            }),
            (_, Err(why)) => dropped.push(Dropped {
                task: task.to_owned(),
                repetition: Some(repetition),
                arm: ghost.to_owned(),
                why,
            }),
        }
    }
    let mut values = Vec::new();
    for (task, reps) in eligible {
        let shadow_values: Vec<f64> = reps.iter().map(|r| r.0).collect();
        let ghost_values: Vec<f64> = reps.iter().map(|r| r.1).collect();
        let (ms, mg) = (stats::mean(&shadow_values), stats::mean(&ghost_values));
        let value = match which {
            Which::Quality => mg - ms,
            Which::Tokens => {
                // Tokens are positive; a mean of 0 has no ratio and is
                // never shown as one.
                let zero_arm = if ms <= 0.0 {
                    Some(shadow)
                } else if mg <= 0.0 {
                    Some(ghost)
                } else {
                    None
                };
                if let Some(arm) = zero_arm {
                    dropped.push(Dropped {
                        task: task.to_owned(),
                        repetition: None,
                        arm: arm.to_owned(),
                        why: DropReason::ZeroTokens,
                    });
                    continue;
                }
                (mg / ms).ln()
            }
        };
        values.push(TaskValue {
            task: task.to_owned(),
            repetitions: reps.len() as u32,
            shadow: ms,
            ghost: mg,
            value,
        });
    }
    (values, dropped)
}

/// The rule of 4.4 on the two tests. Rows 1 and 2 need the quality test,
/// rows 3 to 5 the token test; without one the verdict is
/// `not-distinguishable` for want of tasks.
fn rule(
    quality: Option<&PairedTest>,
    tokens: Option<&PairedTest>,
    question: &Question,
) -> (StudyVerdict, Reason) {
    let Some(quality) = quality else {
        return (StudyVerdict::NotDistinguishable, Reason::TooFewTasks);
    };
    let alpha = question.alpha;
    // Row 1.
    if quality.ci_high < 0.0 {
        return agree(quality, alpha, StudyVerdict::Worse, Reason::Quality);
    }
    // Row 2.
    if quality.lower_bound <= -question.margin {
        return (StudyVerdict::NotDistinguishable, Reason::QualityUnresolved);
    }
    let Some(tokens) = tokens else {
        return (StudyVerdict::NotDistinguishable, Reason::TooFewTasks);
    };
    // Row 3: the ratio's interval wholly above 1, that is the log's above 0.
    if tokens.ci_low > 0.0 {
        return agree(tokens, alpha, StudyVerdict::Worse, Reason::Costlier);
    }
    // Row 4. The point estimate is compared as a ratio with a hair of room,
    // so that a saving of exactly the bar meets it.
    if tokens.ci_high < 0.0 {
        if tokens.mean.exp() <= question.bar() + 1e-12 {
            return agree(tokens, alpha, StudyVerdict::Better, Reason::Cheaper);
        }
        return (StudyVerdict::NotDistinguishable, Reason::BelowBar);
    }
    // Row 5.
    (StudyVerdict::NotDistinguishable, Reason::NoEffectShown)
}

/// The sign-flip check: a verdict the interval gives stands only where the
/// p-value is below alpha as well.
fn agree(
    test: &PairedTest,
    alpha: f64,
    verdict: StudyVerdict,
    reason: Reason,
) -> (StudyVerdict, Reason) {
    if test.p_value < alpha {
        (verdict, reason)
    } else {
        (StudyVerdict::NotDistinguishable, Reason::MethodsDisagree)
    }
}

fn no_signal(pairs: &Pairs<'_>, mode: Mode) -> Vec<String> {
    let mut by_task: BTreeMap<&str, (u32, u32)> = BTreeMap::new();
    for (&(task, _), &(s, g)) in pairs {
        if let (Ok(s), Ok(g)) = (
            side_value(s, mode, Which::Quality),
            side_value(g, mode, Which::Quality),
        ) {
            let entry = by_task.entry(task).or_default();
            entry.0 += 2;
            entry.1 += (s + g) as u32;
        }
    }
    by_task
        .into_iter()
        .filter(|&(_, (n, passed))| passed == 0 || passed == n)
        .map(|(task, _)| task.to_owned())
        .collect()
}

fn compare(
    pairs: &Pairs<'_>,
    shadow: &str,
    ghost: &str,
    mode: Mode,
    question: &Question,
    sampling: Sampling,
) -> Comparison {
    let alpha = question.alpha;
    let (q_values, q_dropped) = metric(pairs, shadow, ghost, mode, Which::Quality);
    let (t_values, t_dropped) = metric(pairs, shadow, ghost, mode, Which::Tokens);
    let quality = Metric::new(q_values, q_dropped, alpha, sampling);
    let tokens = Metric::new(t_values, t_dropped, alpha, sampling);
    let (verdict, reason) = rule(quality.test.as_ref(), tokens.test.as_ref(), question);

    let ratio = tokens.test.as_ref().map(|t| Ratio {
        point: t.mean.exp(),
        interval: Interval {
            low: t.ci_low.exp(),
            high: t.ci_high.exp(),
        },
    });
    let saving_allowed = ratio.as_ref().map(|r| Interval {
        low: 1.0 - r.interval.high,
        high: 1.0 - r.interval.low,
    });
    let excludes_bar = ratio.as_ref().map(|r| r.interval.low > question.bar());
    let token_values: Vec<f64> = tokens.tasks.iter().map(|t| t.value).collect();
    let detectable_saving = stats::std_dev(&token_values)
        .and_then(|sd| stats::detectable_effect(sd, token_values.len(), alpha, 1, POWER))
        .map(|log_ratio| 1.0 - (-log_ratio).exp());
    Comparison {
        quality,
        tokens,
        ratio,
        saving_allowed,
        excludes_bar,
        detectable_saving,
        no_signal_tasks: no_signal(pairs, mode),
        verdict,
        reason,
    }
}

/// Decides one ghost against the shadow from the trials of a study.
pub fn decide(
    trials: &[Trial],
    shadow: &str,
    ghost: &str,
    question: &Question,
    sampling: Sampling,
) -> Result<Assessment, VerdictError> {
    if shadow == ghost {
        return Err(VerdictError::SameArm(shadow.to_owned()));
    }
    let pairs = pair_up(trials, shadow, ghost)?;
    let primary = compare(&pairs, shadow, ghost, Mode::Primary, question, sampling);
    let has_limit = pairs.values().any(|&(s, g)| {
        [s, g]
            .iter()
            .flatten()
            .any(|t| t.outcome == TrialOutcome::Limit)
    });
    let sensitivity = if has_limit {
        compare(&pairs, shadow, ghost, Mode::Sensitivity, question, sampling)
    } else {
        primary.clone()
    };
    let (verdict, reason) = if primary.verdict == sensitivity.verdict {
        (primary.verdict, primary.reason)
    } else {
        (StudyVerdict::NotDistinguishable, Reason::SensitiveToLimits)
    };
    Ok(Assessment {
        primary,
        sensitivity,
        verdict,
        reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The synthetic study: 24 invented tasks, two repetitions, a shadow and
    /// a ghost. Token counts are invented, not measured; the ghost's are the
    /// shadow's times `ratio` times a small, fixed wobble that differs by
    /// task and repetition, so that intervals have a width. Quality: every
    /// task passes on both arms unless a test says otherwise.
    struct Study {
        trials: Vec<Trial>,
    }

    const TASKS: usize = 24;
    const REPS: u32 = 2;

    /// A fixed wobble in `[-spread, spread]`, from the task and repetition.
    fn wobble(task: usize, rep: u32, spread: f64) -> f64 {
        let h = (task as u64 * 2654435761 + rep as u64 * 40503) % 1000;
        (h as f64 / 999.0 * 2.0 - 1.0) * spread
    }

    fn study(ratio: f64, spread: f64) -> Study {
        let mut trials = Vec::new();
        for task in 0..TASKS {
            // Tasks differ a lot in size, as real ones do.
            let base = 40_000.0 * (1.0 + (task % 7) as f64 * 0.8);
            for rep in 1..=REPS {
                for (arm, factor) in [
                    ("shadow", 1.0 + wobble(task, rep, 0.10)),
                    (
                        "ghost",
                        ratio
                            * (1.0 + wobble(task + 100, rep, 0.10))
                            * (1.0 + wobble(task, 0, spread)),
                    ),
                ] {
                    trials.push(Trial {
                        task: format!("task-{task:02}"),
                        arm: arm.to_owned(),
                        repetition: rep,
                        outcome: TrialOutcome::Counted,
                        pass: Some(true),
                        total_tokens: Some((base * factor).round() as u64),
                    });
                }
            }
        }
        Study { trials }
    }

    fn decide_default(trials: &[Trial]) -> Assessment {
        decide(
            trials,
            "shadow",
            "ghost",
            &Question::default(),
            Sampling::default(),
        )
        .unwrap()
    }

    #[test]
    fn a_clear_saving_is_better() {
        let a = decide_default(&study(0.60, 0.15).trials);
        assert_eq!(
            (a.verdict, a.reason),
            (StudyVerdict::Better, Reason::Cheaper)
        );
        let ratio = a.primary.ratio.unwrap();
        assert!((ratio.point - 0.60).abs() < 0.05, "{ratio:?}");
        assert!(ratio.interval.high < 1.0);
        assert_eq!(
            a.primary.excludes_bar,
            Some(false).map(|_| ratio.interval.low > 0.80)
        );
        // No tasks without signal here: all pass on both arms.
        assert_eq!(a.primary.no_signal_tasks.len(), TASKS);
    }

    #[test]
    fn equal_arms_are_not_distinguishable() {
        let a = decide_default(&study(1.0, 0.15).trials);
        assert_eq!(a.verdict, StudyVerdict::NotDistinguishable);
        assert_eq!(a.reason, Reason::NoEffectShown);
        let ratio = a.primary.ratio.unwrap();
        assert!(ratio.interval.low < 1.0 && ratio.interval.high > 1.0);
    }

    #[test]
    fn a_twelve_percent_saving_is_not_distinguishable_with_its_interval() {
        // A true saving of 12 % is under the bar: even when the interval
        // shows a saving, the point estimate misses 0.80.
        let a = decide_default(&study(0.88, 0.10).trials);
        assert_eq!(a.verdict, StudyVerdict::NotDistinguishable);
        let ratio = a.primary.ratio.unwrap();
        assert!(ratio.point > 0.80, "{ratio:?}");
        // The interval is printed: it holds what saving it allows.
        let allowed = a.primary.saving_allowed.unwrap();
        assert!(allowed.low < 0.12 && allowed.high > 0.12, "{allowed:?}");
        // Whether the interval excludes a 20 % saving is said, and is
        // consistent with its lower bound.
        assert_eq!(a.primary.excludes_bar, Some(ratio.interval.low > 0.80));
        assert!(a.primary.detectable_saving.is_some());
    }

    #[test]
    fn a_detected_saving_under_the_bar_is_below_bar() {
        // Little spread: the interval excludes 1, the point is 0.88.
        let a = decide_default(&study(0.88, 0.0).trials);
        assert_eq!(
            (a.verdict, a.reason),
            (StudyVerdict::NotDistinguishable, Reason::BelowBar)
        );
    }

    #[test]
    fn a_detectable_cost_is_worse() {
        let a = decide_default(&study(1.5, 0.10).trials);
        assert_eq!(
            (a.verdict, a.reason),
            (StudyVerdict::Worse, Reason::Costlier)
        );
    }

    #[test]
    fn worse_quality_is_worse_whatever_the_tokens() {
        let mut trials = study(0.6, 0.10).trials;
        // The ghost fails every task in both repetitions.
        for t in trials.iter_mut().filter(|t| t.arm == "ghost") {
            t.pass = Some(false);
        }
        let a = decide_default(&trials);
        assert_eq!(
            (a.verdict, a.reason),
            (StudyVerdict::Worse, Reason::Quality)
        );
    }

    #[test]
    fn quality_that_may_be_worse_by_the_margin_is_unresolved() {
        let mut trials = study(0.6, 0.10).trials;
        // The ghost fails 3 of 24 tasks entirely: a mean drop of 1/8 whose
        // one-sided lower bound is beyond the 15 points margin, while the
        // interval still reaches 0. (With 4 failing tasks the t-interval
        // excludes 0 but 4 differing tasks cannot give a sign-flip p below
        // 0.05: methods-disagree.)
        for t in trials
            .iter_mut()
            .filter(|t| t.arm == "ghost" && t.task.as_str() < "task-03")
        {
            t.pass = Some(false);
        }
        let a = decide_default(&trials);
        assert_eq!(a.verdict, StudyVerdict::NotDistinguishable);
        assert_eq!(a.reason, Reason::QualityUnresolved);
        // A wider margin lets the same trials through: thresholds are
        // parameters, not constants.
        let wide = Question {
            margin: 0.5,
            ..Question::default()
        };
        let b = decide(&trials, "shadow", "ghost", &wide, Sampling::default()).unwrap();
        assert_ne!(b.reason, Reason::QualityUnresolved);
    }

    #[test]
    fn the_bar_is_a_parameter() {
        let trials = study(0.88, 0.0).trials;
        let relaxed = Question {
            min_saving: 0.10,
            ..Question::default()
        };
        let a = decide(&trials, "shadow", "ghost", &relaxed, Sampling::default()).unwrap();
        assert_eq!(
            (a.verdict, a.reason),
            (StudyVerdict::Better, Reason::Cheaper)
        );
    }

    #[test]
    fn limit_sensitivity_flips_a_verdict_to_not_distinguishable() {
        let mut trials = study(0.6, 0.10).trials;
        let a = decide_default(&trials);
        assert_eq!(a.verdict, StudyVerdict::Better);
        // The ghost hits the limit on 8 tasks, in both repetitions, and the
        // limit trials leave the primary analysis. Counted as failures they
        // make the ghost worse on quality.
        for t in trials
            .iter_mut()
            .filter(|t| t.arm == "ghost" && t.task.as_str() < "task-08")
        {
            t.outcome = TrialOutcome::Limit;
            t.pass = None;
        }
        let a = decide_default(&trials);
        // The primary analysis drops those pairs: still better.
        assert_eq!(a.primary.verdict, StudyVerdict::Better);
        assert_ne!(a.sensitivity.verdict, StudyVerdict::Better);
        assert_eq!(a.verdict, StudyVerdict::NotDistinguishable);
        assert_eq!(a.reason, Reason::SensitiveToLimits);
        // The dropped pairs are named, and none is a score of 0.
        assert!(
            a.primary
                .quality
                .dropped
                .iter()
                .all(|d| d.why == DropReason::NotCounted(TrialOutcome::Limit) && d.arm == "ghost")
        );
        assert_eq!(a.primary.quality.dropped.len(), 16);
        assert_eq!(a.primary.quality.tasks.len(), TASKS - 8);
        assert_eq!(a.sensitivity.quality.tasks.len(), TASKS);
    }

    #[test]
    fn limit_trials_that_change_nothing_leave_the_verdict() {
        let mut trials = study(0.6, 0.10).trials;
        // One ghost trial hits the limit, and the other repetition of the
        // task still passes: the sensitivity analysis agrees.
        let first = trials.iter_mut().find(|t| t.arm == "ghost").unwrap();
        first.outcome = TrialOutcome::Limit;
        first.pass = None;
        let a = decide_default(&trials);
        assert_eq!(
            (a.verdict, a.reason),
            (StudyVerdict::Better, Reason::Cheaper)
        );
    }

    #[test]
    fn no_tokens_is_never_zero() {
        let mut trials = study(0.6, 0.10).trials;
        // The ledger knows no line of 6 of the ghost's trials.
        let mut unknown = 0;
        for t in trials
            .iter_mut()
            .filter(|t| t.arm == "ghost" && t.repetition == 1)
        {
            if unknown < 6 {
                t.total_tokens = None;
                unknown += 1;
            }
        }
        let a = decide_default(&trials);
        // Those pairs left the token comparison, named, and the other
        // repetition of each task still gives its value.
        let dropped = &a.primary.tokens.dropped;
        assert_eq!(dropped.len(), 6);
        assert!(dropped.iter().all(|d| d.why == DropReason::NoTokens));
        assert_eq!(a.primary.tokens.tasks.len(), TASKS);
        assert_eq!(
            a.primary
                .tokens
                .tasks
                .iter()
                .filter(|t| t.repetitions == 1)
                .count(),
            6
        );
        // Quality did not lose them: they were graded.
        assert!(a.primary.quality.dropped.is_empty());
        // And the ratio is about what it was; a 0 would have given 0.3.
        assert!((a.primary.ratio.unwrap().point - 0.6).abs() < 0.06);
    }

    #[test]
    fn no_token_at_all_gives_no_ratio_and_no_verdict_by_cost() {
        let mut trials = study(0.6, 0.10).trials;
        for t in trials.iter_mut() {
            t.total_tokens = None;
        }
        let a = decide_default(&trials);
        assert!(a.primary.ratio.is_none());
        assert!(a.primary.saving_allowed.is_none());
        assert!(a.primary.detectable_saving.is_none());
        assert!(a.primary.tokens.test.is_none());
        assert_eq!(
            (a.verdict, a.reason),
            (StudyVerdict::NotDistinguishable, Reason::TooFewTasks)
        );
        // The metric did not turn into wall time or anything else.
        assert_eq!(a.primary.tokens.dropped.len(), TASKS * REPS as usize);
    }

    #[test]
    fn a_counted_trial_without_a_grade_is_not_scored() {
        let mut trials = study(0.6, 0.10).trials;
        trials[1].pass = None; // the ghost of the first task, repetition 1
        let a = decide_default(&trials);
        assert_eq!(a.primary.quality.dropped.len(), 1);
        assert_eq!(a.primary.quality.dropped[0].why, DropReason::NotScored);
        // Its tokens still count: it was counted, only unscored.
        assert!(a.primary.tokens.dropped.is_empty());
    }

    #[test]
    fn infra_and_refused_trials_are_never_failures() {
        let mut trials = study(0.6, 0.10).trials;
        for t in trials
            .iter_mut()
            .filter(|t| t.arm == "ghost" && t.task.as_str() < "task-06")
        {
            t.outcome = TrialOutcome::Infra;
            t.pass = None;
            t.total_tokens = None;
        }
        trials
            .iter_mut()
            .filter(|t| t.arm == "shadow" && t.task.as_str() == "task-07")
            .for_each(|t| t.outcome = TrialOutcome::Refused);
        let a = decide_default(&trials);
        // Infrastructure is not a limit: the sensitivity analysis is the
        // primary one.
        assert_eq!(a.primary, a.sensitivity);
        assert_eq!(a.verdict, StudyVerdict::Better);
        assert_eq!(a.primary.quality.tasks.len(), TASKS - 7);
    }

    #[test]
    fn one_task_is_not_enough_for_an_interval() {
        let trials: Vec<Trial> = study(0.5, 0.0)
            .trials
            .into_iter()
            .filter(|t| t.task == "task-00")
            .collect();
        let a = decide_default(&trials);
        assert_eq!(
            (a.verdict, a.reason),
            (StudyVerdict::NotDistinguishable, Reason::TooFewTasks)
        );
    }

    #[test]
    fn the_interval_decides_and_the_sign_flip_is_a_check() {
        // Four tasks with a saving of 40 %: the t-interval excludes 1, but
        // four tasks cannot give a sign-flip p below 0.05 (the least is
        // 2/16), so the two methods disagree.
        let trials: Vec<Trial> = study(0.6, 0.02)
            .trials
            .into_iter()
            .filter(|t| t.task.as_str() < "task-04")
            .collect();
        let a = decide_default(&trials);
        let t = a.primary.tokens.test.unwrap();
        assert!(t.ci_high < 0.0, "the interval excludes no effect");
        assert!(t.p_value >= 0.05);
        assert_eq!(
            (a.verdict, a.reason),
            (StudyVerdict::NotDistinguishable, Reason::MethodsDisagree)
        );
    }

    #[test]
    fn repetitions_are_averaged_inside_a_task() {
        // Two repetitions are one value, not two: 24 tasks, not 48.
        let a = decide_default(&study(0.6, 0.10).trials);
        assert_eq!(a.primary.tokens.tasks.len(), TASKS);
        assert!(a.primary.tokens.tasks.iter().all(|t| t.repetitions == 2));
        // A repetition one arm lacks leaves the pair, not the task.
        let mut trials = study(0.6, 0.10).trials;
        trials.retain(|t| !(t.arm == "shadow" && t.task == "task-03" && t.repetition == 2));
        let a = decide_default(&trials);
        let task = a
            .primary
            .tokens
            .tasks
            .iter()
            .find(|t| t.task == "task-03")
            .unwrap();
        assert_eq!(task.repetitions, 1);
        assert_eq!(a.primary.tokens.dropped.len(), 1);
        assert_eq!(a.primary.tokens.dropped[0].why, DropReason::Missing);
    }

    #[test]
    fn zero_tokens_have_no_ratio() {
        let mut trials = study(0.6, 0.10).trials;
        for t in trials.iter_mut().filter(|t| t.task == "task-00") {
            t.total_tokens = Some(0);
        }
        let a = decide_default(&trials);
        assert_eq!(a.primary.tokens.tasks.len(), TASKS - 1);
        assert!(a.primary.tokens.dropped.iter().any(|d| {
            d.task == "task-00" && d.repetition.is_none() && d.why == DropReason::ZeroTokens
        }));
        assert!(a.primary.ratio.unwrap().point.is_finite());
    }

    #[test]
    fn tasks_without_signal_are_named_and_kept() {
        let mut trials = study(0.6, 0.10).trials;
        // Task 0 differs between arms: it has signal.
        for t in trials
            .iter_mut()
            .filter(|t| t.arm == "ghost" && t.task == "task-00")
        {
            t.pass = Some(false);
        }
        let a = decide_default(&trials);
        assert!(!a.primary.no_signal_tasks.contains(&"task-00".to_owned()));
        assert_eq!(a.primary.no_signal_tasks.len(), TASKS - 1);
        assert_eq!(a.primary.tokens.tasks.len(), TASKS);
    }

    #[test]
    fn a_double_trial_or_the_same_arm_is_refused() {
        let mut trials = study(0.6, 0.10).trials;
        trials.push(trials[0].clone());
        let err = decide(
            &trials,
            "shadow",
            "ghost",
            &Question::default(),
            Sampling::default(),
        );
        assert!(matches!(err, Err(VerdictError::DuplicateTrial { .. })));
        let err = decide(&[], "a", "a", &Question::default(), Sampling::default());
        assert_eq!(err, Err(VerdictError::SameArm("a".to_owned())));
    }

    #[test]
    fn the_same_trials_give_the_same_answer() {
        let trials = study(0.7, 0.2).trials;
        assert_eq!(decide_default(&trials), decide_default(&trials));
    }

    #[test]
    fn the_exact_bar_is_met() {
        // Every task saves exactly 20 %: the point estimate is the bar, and
        // that meets it ("at most 0.80").
        let mut trials = Vec::new();
        for task in 0..10 {
            for (arm, tokens) in [("shadow", 100_000u64), ("ghost", 80_000)] {
                trials.push(Trial {
                    task: format!("t{task}"),
                    arm: arm.to_owned(),
                    repetition: 1,
                    outcome: TrialOutcome::Counted,
                    pass: Some(true),
                    total_tokens: Some(tokens + task * 1000 * if arm == "ghost" { 8 } else { 10 }),
                });
            }
        }
        let a = decide_default(&trials);
        assert_eq!(
            (a.verdict, a.reason),
            (StudyVerdict::Better, Reason::Cheaper)
        );
    }
}
