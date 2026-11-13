//! `kai study`: comparing variants of an agent on tickets. Only `estimate`
//! exists so far.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use engine::study::estimate::{self, Design, Spread};
use helpers::config;
use serde::Deserialize;
use types::study::Question;

use super::{Context, Failure};

#[derive(Subcommand)]
pub(crate) enum StudyCommand {
    /// Says from a study's size what it can see: trials, worst-case tokens, the smallest saving it detects, and the chance of `better` at a few true savings. Information only; it spends nothing
    Estimate(EstimateArgs),
}

#[derive(Args)]
pub(crate) struct EstimateArgs {
    /// The study file, TOML. Until the study file's format is fixed, only the keys of the estimate are read (see the docs of `StudyFile`); other keys are ignored
    #[arg(value_name = "FILE")]
    file: PathBuf,

    /// Instead of the assumed spread: the standard deviation of the per-task log ratios a pilot gave
    #[arg(long, value_name = "SD")]
    observed_sd: Option<f64>,
}

pub(crate) fn run(command: &StudyCommand, _context: &Context<'_>) -> Result<(), Failure> {
    match command {
        StudyCommand::Estimate(args) => estimate(args),
    }
}

/// What `kai study estimate` reads: the minimal inputs, as a TOML file.
/// The study file (`kai-study/2`) belongs to a later step; it will hold
/// these under the same names, so that one file serves both. Keys this
/// estimate does not use are ignored on purpose, for that reason.
///
/// ```toml
/// tasks = 24             # a count, or the list of the tasks' ids
/// arms = 2               # a count, or the list of the arms' names
/// repetitions = 2
/// max_turns = 40
/// context_tokens = 200000
/// max_output_tokens = 32000   # optional
///
/// [question]             # optional; these are the defaults
/// min_saving = 0.20
/// margin = 0.15
/// alpha = 0.05
///
/// [assumed]              # optional; these are the defaults
/// within_task_sd = 0.35
/// task_by_arm_sd = 0.15
/// ```
#[derive(Deserialize)]
struct StudyFile {
    tasks: Count,
    arms: Count,
    repetitions: u32,
    max_turns: u32,
    context_tokens: u64,
    max_output_tokens: Option<u64>,
    #[serde(default)]
    question: QuestionFile,
    assumed: Option<Assumed>,
}

/// A number, or the list whose length it is.
#[derive(Deserialize)]
#[serde(untagged)]
enum Count {
    Number(usize),
    List(Vec<String>),
}

impl Count {
    fn get(&self) -> usize {
        match self {
            Count::Number(n) => *n,
            Count::List(list) => list.len(),
        }
    }
}

/// The `[question]` table; a key left out takes the spec's default.
#[derive(Default, Deserialize)]
struct QuestionFile {
    min_saving: Option<f64>,
    margin: Option<f64>,
    alpha: Option<f64>,
}

#[derive(Deserialize)]
struct Assumed {
    within_task_sd: f64,
    task_by_arm_sd: f64,
}

fn read(path: &Path) -> Result<StudyFile, Failure> {
    config::load::<StudyFile>(path)
        .map(|loaded| loaded.value)
        .map_err(|err| {
            let mut message = err.to_string();
            let mut source = std::error::Error::source(&err);
            while let Some(cause) = source {
                message.push_str(&format!(": {cause}"));
                source = cause.source();
            }
            Failure::Usage(anyhow::anyhow!(message))
        })
}

fn estimate(args: &EstimateArgs) -> Result<(), Failure> {
    let file = read(&args.file)?;
    let defaults = Question::default();
    let spread = match (args.observed_sd, &file.assumed) {
        (Some(sd_per_task), _) => Spread::Observed { sd_per_task },
        (None, Some(a)) => Spread::Assumed {
            within_task_sd: a.within_task_sd,
            task_by_arm_sd: a.task_by_arm_sd,
        },
        (None, None) => Spread::default(),
    };
    let design = Design {
        tasks: file.tasks.get(),
        repetitions: file.repetitions,
        arms: file.arms.get(),
        max_turns: file.max_turns,
        context_tokens: file.context_tokens,
        max_output_tokens: file
            .max_output_tokens
            .unwrap_or(estimate::MAX_OUTPUT_TOKENS),
        spread,
        question: Question {
            min_saving: file.question.min_saving.unwrap_or(defaults.min_saving),
            margin: file.question.margin.unwrap_or(defaults.margin),
            alpha: file.question.alpha.unwrap_or(defaults.alpha),
        },
    };
    let e = estimate::estimate(&design).map_err(|err| {
        Failure::Usage(anyhow::Error::new(err).context(args.file.display().to_string()))
    })?;
    print(&design, &e);
    Ok(())
}

fn print(design: &Design, e: &estimate::Estimate) {
    let q = &design.question;
    println!(
        "trials: {} ({} tasks x {} repetitions x {} arms)",
        e.trials, design.tasks, design.repetitions, design.arms
    );
    match design.spread {
        Spread::Assumed {
            within_task_sd,
            task_by_arm_sd,
        } => println!(
            "spread per task: {:.3} (assumed: within a task and arm {within_task_sd}, task by arm {task_by_arm_sd}; not measured)",
            e.sd_per_task
        ),
        Spread::Observed { .. } => println!(
            "spread per task: {:.3} (observed in a pilot)",
            e.sd_per_task
        ),
    }
    println!(
        "detectable saving: {:.1} % at 80 % power, alpha {} (log ratio {:.3})",
        e.detectable_saving * 100.0,
        q.alpha,
        e.detectable_log_ratio
    );
    println!(
        "chance of `better` (saving of at least {:.0} %, interval excluding no saving), by the normal approximation:",
        q.min_saving * 100.0
    );
    for chance in &e.chance_of_better {
        println!(
            "  true saving {:>3.0} %: {:>5.1} %",
            chance.saving * 100.0,
            chance.better * 100.0
        );
    }
    println!(
        "worst-case tokens: {} per trial, {} in all ({} turns x ({} context + {} output) tokens x {} trials); a bound, not a forecast",
        e.tokens_per_trial_max,
        e.tokens_max,
        design.max_turns,
        design.context_tokens,
        design.max_output_tokens,
        e.trials
    );
    println!("no price is shown: prices are the proxy's, and an unknown price stays unknown");
}
