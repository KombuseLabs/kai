//! What GitHub says of an account's access to a board: the data the trust
//! gate is judged on. Judging it is `engine`'s.

/// An account's access to a repository, as GitHub answered when asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Access {
    /// GitHub said it.
    Known {
        /// GitHub's `permission`: `admin`, `write`, `read` or `none`, as
        /// GitHub wrote it.
        permission: String,
        /// GitHub's `role_name`, finer than `permission` (`maintain` and
        /// `triage` among them), or `None` when GitHub did not send one.
        role: Option<String>,
    },
    /// GitHub did not say, and why: its status and message, such as for
    /// an account that is no collaborator, or a token that may not ask.
    Unknown(String),
}

/// One account's access to a board, and when it was asked. It is the
/// access at that time, not when the account put a label on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActorAccess {
    /// The account's login.
    pub login: String,
    /// What GitHub said.
    pub access: Access,
    /// When it was asked (RFC 3339, UTC).
    pub observed_at: String,
}
