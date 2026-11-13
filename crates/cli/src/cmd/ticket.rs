//! `kai ticket`: the tickets in the store, and whether kai may use them.

use std::path::Path;

use anyhow::Context as _;
use clap::Subcommand;
use types::{IssueState, Outcome, Ticket, Verdict};

use super::{Context, Failure};

#[derive(Subcommand)]
pub(crate) enum TicketCommand {
    /// Lists the tickets in the store: issue, state, labels, title
    List,
    /// Says for each ticket in the store whether kai may use it, and why not, judged from what GitHub said
    Qualify,
}

pub(crate) fn run(command: &TicketCommand, context: &Context<'_>) -> Result<(), Failure> {
    let tickets = tickets(&context.store)?;
    match command {
        TicketCommand::List => print_list(&tickets),
        TicketCommand::Qualify => print_qualify(&tickets),
    }
    Ok(())
}

/// Every ticket in the store; a hint on standard error when there is none.
fn tickets(store_path: &Path) -> anyhow::Result<Vec<Ticket>> {
    let tickets = super::store::open(store_path)?
        .tickets()
        .context("cannot read the store")?;
    if tickets.is_empty() {
        eprintln!(
            "kai: the store {} holds no tickets; `kai github sync` fetches them",
            store_path.display()
        );
    }
    Ok(tickets)
}

/// One line per ticket: issue, state, labels, title, in columns.
fn print_list(tickets: &[Ticket]) {
    let issues: Vec<String> = tickets.iter().map(|t| t.issue.to_string()).collect();
    let labels: Vec<String> = tickets
        .iter()
        .map(|t| {
            if t.labels.is_empty() {
                "-".to_owned()
            } else {
                t.labels.join(",")
            }
        })
        .collect();
    let issue_width = issues.iter().map(String::len).max().unwrap_or(0);
    let label_width = labels.iter().map(String::len).max().unwrap_or(0);
    for ((ticket, issue), labels) in tickets.iter().zip(&issues).zip(&labels) {
        let state = match ticket.state {
            IssueState::Open => "open",
            IssueState::Closed => "closed",
        };
        println!(
            "{issue:<issue_width$}  {state:<6}  {labels:<label_width$}  {}",
            ticket.title
        );
    }
}

/// Per ticket its verdict, and below it each rule that failed or was not
/// checked, with why; then the count of each verdict.
fn print_qualify(tickets: &[Ticket]) {
    let issue_width = tickets
        .iter()
        .map(|t| t.issue.to_string().len())
        .max()
        .unwrap_or(0);
    let (mut qualified, mut refused, mut undecided) = (0, 0, 0);
    for ticket in tickets {
        let qualification = ::engine::qualify::qualify(ticket);
        let verdict = qualification.verdict();
        match verdict {
            Verdict::Qualified => qualified += 1,
            Verdict::Refused => refused += 1,
            Verdict::Undecided => undecided += 1,
        }
        println!(
            "{:<issue_width$}  {:<9}  {}",
            ticket.issue.to_string(),
            verdict.to_string(),
            ticket.title
        );
        for finding in &qualification.findings {
            let (how, why) = match &finding.outcome {
                Outcome::Pass => continue,
                Outcome::Fail(why) => ("failed", why),
                Outcome::NotChecked(why) => ("not checked", why),
            };
            println!("    {how:<11}  {}: {why}", finding.rule);
        }
    }
    if !tickets.is_empty() {
        println!("{qualified} qualified, {refused} refused, {undecided} undecided");
    }
}
