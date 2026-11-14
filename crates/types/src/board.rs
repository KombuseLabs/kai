//! The files of a board repository, where the hidden tests of its tickets
//! lie.

/// The files of a board repository at the head of its default branch, as
/// GitHub listed them. The hidden tests of a ticket lie here, in the board,
/// never in the code repository the agent works in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardFiles {
    /// The default branch's name.
    pub branch: String,
    /// The commit at its head, a full sha.
    pub commit: String,
    /// The path of every file, `/`-separated, in GitHub's order.
    pub paths: Vec<String>,
    /// Whether [`paths`](Self::paths) are all of them; GitHub cuts a long
    /// listing short.
    pub complete: bool,
}
