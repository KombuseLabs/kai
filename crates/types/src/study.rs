//! A study's data: where each trial runs in the plan, what became of it,
//! and the verdict the rule gives. Data only; the plan, the statistics and
//! the rule are `engine::study`'s.
//!
//! These names live in their own module, `types::study`. The study's verdict
//! is `StudyVerdict` and a trial's outcome `TrialOutcome` because
//! `types::Verdict` and `types::Outcome` are the ticket qualifying's, and
//! the diagrams name types by their bare names.
//!
//! A value nobody recorded is `None`; it is never a default. A trial without
//! tokens has no tokens, not 0 of them.

use std::fmt;

/// One place of the plan: an arm's attempt at a task, in one repetition,
/// and when it runs. The identity of a trial is (task, arm, repetition);
/// the position only says when.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Slot {
    /// The task's id.
    pub task: String,
    /// The arm's name, such as `shadow` or `ghost`.
    pub arm: String,
    /// The repetition, counted from 1. A repetition is a block: every task
    /// and every arm once.
    pub repetition: u32,
    /// Where the slot stands in the whole plan, counted from 1.
    pub position: u32,
}

/// What became of a trial (the study spec's section 4.5). Infrastructure
/// is never a verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrialOutcome {
    /// The agent answered and the grade is a pass or a fail.
    Counted,
    /// A turn or time limit was reached with the VM healthy. Left out of
    /// the primary analysis; the sensitivity analysis counts it as a
    /// failure, with its tokens.
    Limit,
    /// The run or its grading failed for a reason that says nothing about
    /// the agent.
    Infra,
    /// The proxy refused the run (402).
    Refused,
    /// A pin of the trial differs from the lock.
    Drift,
}

impl TrialOutcome {
    /// The outcome's short name, such as `counted`.
    pub fn name(self) -> &'static str {
        match self {
            TrialOutcome::Counted => "counted",
            TrialOutcome::Limit => "limit",
            TrialOutcome::Infra => "infra",
            TrialOutcome::Refused => "refused",
            TrialOutcome::Drift => "drift",
        }
    }
}

impl fmt::Display for TrialOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// What the statistics need of one trial. The trial record of S4 holds
/// more; this is the part a verdict reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trial {
    /// The task's id.
    pub task: String,
    /// The arm's name.
    pub arm: String,
    /// The repetition, from 1.
    pub repetition: u32,
    /// What became of it.
    pub outcome: TrialOutcome,
    /// Whether it passed the hidden tests and the check; `None` when it was
    /// not graded. A `Counted` trial without a grade is not scored, and
    /// stays out of the quality comparison.
    pub pass: Option<bool>,
    /// Total tokens of the session, as the ledger counted them; `None` when
    /// no line is known. Never 0 for "unknown".
    pub total_tokens: Option<u64>,
}

/// The decision rule's thresholds, written in the study file before the
/// first trial. They are parameters with the spec's defaults, since the
/// owner has not fixed them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Question {
    /// The saving the ghost must show: the ratio of its tokens to the
    /// shadow's must be at most `1 - min_saving`. Default 0.20.
    pub min_saving: f64,
    /// How much worse than the shadow the ghost's pass rate may be shown to
    /// be, in points of pass rate (0.15 is 15 points). Default 0.15.
    pub margin: f64,
    /// The error level: intervals are at `1 - alpha`. Default 0.05.
    pub alpha: f64,
}

impl Default for Question {
    fn default() -> Question {
        Question {
            min_saving: 0.20,
            margin: 0.15,
            alpha: 0.05,
        }
    }
}

impl Question {
    /// The ratio of ghost tokens to shadow tokens that the point estimate
    /// must not exceed for `better`: `1 - min_saving`, 0.80 by default.
    pub fn bar(&self) -> f64 {
        1.0 - self.min_saving
    }
}

/// A ghost's answer against the shadow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StudyVerdict {
    /// Quality not shown worse and a token saving of at least the bar, with
    /// an interval that excludes no saving.
    Better,
    /// Quality detectably lower, or tokens detectably higher.
    Worse,
    /// Everything else, and the default.
    NotDistinguishable,
}

impl StudyVerdict {
    /// The verdict's short name, such as `not-distinguishable`.
    pub fn name(self) -> &'static str {
        match self {
            StudyVerdict::Better => "better",
            StudyVerdict::Worse => "worse",
            StudyVerdict::NotDistinguishable => "not-distinguishable",
        }
    }
}

impl fmt::Display for StudyVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Why a verdict is what it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reason {
    /// `worse`: the pass-rate difference is detectably below 0.
    Quality,
    /// `not-distinguishable`: the pass rate may be worse by the margin or
    /// more.
    QualityUnresolved,
    /// `worse`: the token ratio's interval lies wholly above 1.
    Costlier,
    /// `better`: the token ratio's interval lies wholly below 1 and its
    /// point estimate is at or below the bar.
    Cheaper,
    /// `not-distinguishable`: the sensitivity analysis, with `limit` counted
    /// as a failure, gives another verdict.
    SensitiveToLimits,
    /// `not-distinguishable`: the sign-flip test and the interval stand on
    /// opposite sides of the null.
    MethodsDisagree,
    /// `not-distinguishable`: fewer than two tasks have a value on both
    /// arms, so there is no interval.
    TooFewTasks,
    /// `not-distinguishable`: the token interval lies wholly below 1, but
    /// the point estimate is above the bar.
    BelowBar,
    /// `not-distinguishable`: the token interval holds 1; no effect is
    /// shown either way.
    NoEffectShown,
}

impl Reason {
    /// The reason's short name, such as `sensitive-to-limits`.
    pub fn name(self) -> &'static str {
        match self {
            Reason::Quality => "quality",
            Reason::QualityUnresolved => "quality-unresolved",
            Reason::Costlier => "costlier",
            Reason::Cheaper => "cheaper",
            Reason::SensitiveToLimits => "sensitive-to-limits",
            Reason::MethodsDisagree => "methods-disagree",
            Reason::TooFewTasks => "too-few-tasks",
            Reason::BelowBar => "below-bar",
            Reason::NoEffectShown => "no-effect-shown",
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A closed interval of numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interval {
    /// The lower end.
    pub low: f64,
    /// The upper end.
    pub high: f64,
}
