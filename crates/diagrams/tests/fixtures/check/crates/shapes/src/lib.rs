//! An invented crate that `tests/check.rs` reads; it is never built.

pub mod store;

/// A ticket.
pub struct Ticket {
    /// Its title.
    pub title: String,
    pub labels: Vec<String>,
    pub form: Option<Form>,
    pub events: Vec<Event>,
    pub state: State,
    secret: u32,
}

pub struct Form {
    pub repo: Option<String>,
}

pub struct Event {
    pub at: String,
    pub by: Option<String>,
}

pub enum State {
    Open,
    Closed,
}

pub enum Outcome {
    Pass,
    Fail(String),
    Ended { how: String, code: i32 },
}

pub enum Error {
    BadName,
    Exists,
    Task,
}

impl Ticket {
    /// What qualifying it found.
    pub fn verdict(&self) -> Outcome {
        Outcome::Pass
    }

    pub(crate) fn close(&mut self) {
        self.state = State::Closed;
    }

    pub fn new(title: String) -> Ticket {
        Ticket {
            title,
            labels: Vec::new(),
            form: None,
            events: Vec::new(),
            state: State::Open,
            secret: 0,
        }
    }
}

pub trait Run {
    fn start(&mut self) -> bool;
    fn stop(&mut self) {}
}

impl Run for Ticket {
    fn start(&mut self) -> bool {
        true
    }
}

/// Judges a ticket.
pub fn qualify(ticket: &Ticket) -> Outcome {
    ticket.verdict()
}
