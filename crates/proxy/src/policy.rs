//! What an egress lets through, and the program that does it.
//!
//! kai does not read the rules: they go to fence as they are, one
//! `--net=<rule>` each, and fence parses them and refuses one it cannot
//! read before it listens.

use std::path::PathBuf;
use std::time::Duration;

/// What an egress lets through, and the program that does it.
#[derive(Clone, Debug)]
pub struct Policy {
    /// The hosts a run may reach, in fence's `net.allow` syntax:
    /// `host:port`, `*.name:port` for every name below `name`. Empty lets
    /// nothing through. fence checks them and refuses one it cannot read.
    pub net: Vec<String>,
    /// fence's learn mode: every host is let through, and those outside
    /// [`Policy::net`] are recorded as `would-deny`. What such a run
    /// fetched must not be kept.
    pub learn: bool,
    /// The program: a name looked up on `PATH`, or a path.
    pub fence: PathBuf,
    /// How long fence may take to say it listens.
    pub ready_timeout: Duration,
    /// How long fence may take to end once its standard input is closed.
    pub close_grace: Duration,
}

impl Policy {
    /// A policy that lets through the hosts `net` names, with `fence` from
    /// `PATH` and the default times.
    pub fn new<I, S>(net: I) -> Policy
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Policy {
            net: net.into_iter().map(Into::into).collect(),
            ..Policy::default()
        }
    }
}

impl Default for Policy {
    /// Nothing let through, not in learn mode, `fence` from `PATH`, ten
    /// seconds to listen and ten to end.
    fn default() -> Policy {
        Policy {
            net: Vec::new(),
            learn: false,
            fence: PathBuf::from("fence"),
            ready_timeout: Duration::from_secs(10),
            close_grace: Duration::from_secs(10),
        }
    }
}
