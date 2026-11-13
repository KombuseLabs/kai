//! What qualifying a ticket found: one finding per rule, and the verdict
//! they add up to.

use std::fmt;

/// A rule a ticket must meet before kai uses it. Each has a short name that
/// stays the same, for output and for scripts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rule {
    /// The issue carries the `kai` label.
    Label,
    /// The label was put there by someone with write access to the board.
    TrustGate,
    /// The issue is open.
    Open,
    /// The issue's body is the kai ticket form.
    Form,
    /// The instruction is not empty.
    Instruction,
    /// The instruction holds no code block, which could give the solution
    /// away.
    NoCodeBlock,
    /// The repository is a valid `owner/name`.
    Repo,
    /// The ref is a full commit sha.
    Ref,
    /// The ticket names its check, or the ecosystem's default is known.
    Check,
    /// The ticket names its hidden tests.
    HiddenTests,
    /// The repository has the commit, and the hidden tests are there.
    AtRef,
}

impl Rule {
    /// The rule's short name, such as `trust-gate`.
    pub fn name(self) -> &'static str {
        match self {
            Rule::Label => "label",
            Rule::TrustGate => "trust-gate",
            Rule::Open => "open",
            Rule::Form => "form",
            Rule::Instruction => "instruction",
            Rule::NoCodeBlock => "no-code-block",
            Rule::Repo => "repo",
            Rule::Ref => "ref",
            Rule::Check => "check",
            Rule::HiddenTests => "hidden-tests",
            Rule::AtRef => "at-ref",
        }
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// How a ticket fared against one rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The ticket meets the rule.
    Pass,
    /// The ticket does not meet the rule, and why.
    Fail(String),
    /// The rule was not checked, and why. Not a pass: unknown is not good.
    NotChecked(String),
}

/// One rule and how the ticket fared against it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// The rule.
    pub rule: Rule,
    /// How the ticket fared.
    pub outcome: Outcome,
}

impl Finding {
    /// The ticket meets `rule`.
    pub fn pass(rule: Rule) -> Finding {
        Finding {
            rule,
            outcome: Outcome::Pass,
        }
    }

    /// The ticket does not meet `rule`, because of `why`.
    pub fn fail(rule: Rule, why: impl Into<String>) -> Finding {
        Finding {
            rule,
            outcome: Outcome::Fail(why.into()),
        }
    }

    /// `rule` was not checked, because of `why`.
    pub fn not_checked(rule: Rule, why: impl Into<String>) -> Finding {
        Finding {
            rule,
            outcome: Outcome::NotChecked(why.into()),
        }
    }
}

/// What qualifying one ticket found, in the order the rules were applied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Qualification {
    /// One finding per rule that was applied.
    pub findings: Vec<Finding>,
}

impl Qualification {
    /// What the findings add up to: refused if any rule failed, else
    /// undecided if any rule was not checked, else qualified.
    ///
    /// ```
    /// use types::{Finding, Qualification, Rule, Verdict};
    ///
    /// let mut qualification = Qualification {
    ///     findings: vec![
    ///         Finding::pass(Rule::Label),
    ///         Finding::not_checked(Rule::TrustGate, "not built yet"),
    ///     ],
    /// };
    /// assert_eq!(qualification.verdict(), Verdict::Undecided);
    ///
    /// qualification.findings.push(Finding::fail(Rule::Open, "the issue is closed"));
    /// assert_eq!(qualification.verdict(), Verdict::Refused);
    /// ```
    pub fn verdict(&self) -> Verdict {
        let any = |pick: fn(&Outcome) -> bool| self.findings.iter().any(|f| pick(&f.outcome));
        if any(|outcome| matches!(outcome, Outcome::Fail(_))) {
            Verdict::Refused
        } else if any(|outcome| matches!(outcome, Outcome::NotChecked(_))) {
            Verdict::Undecided
        } else {
            Verdict::Qualified
        }
    }
}

/// Whether kai may use a ticket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Every rule was checked and met.
    Qualified,
    /// At least one rule failed.
    Refused,
    /// No rule failed, but at least one was not checked.
    Undecided,
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Verdict::Qualified => "qualified",
            Verdict::Refused => "refused",
            Verdict::Undecided => "undecided",
        })
    }
}
