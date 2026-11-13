//! Names of repositories and issues.

use std::fmt;
use std::str::FromStr;

/// A repository on GitHub, as `owner/name`.
///
/// Each part is ASCII letters, digits, `-`, `_` and `.`, and neither is `.`
/// or `..`, so that the name is safe to use as a path.
///
/// ```
/// use types::RepoName;
///
/// let repo: RepoName = "example-org/example-code".parse()?;
/// assert_eq!(repo.owner(), "example-org");
/// assert_eq!(repo.to_string(), "example-org/example-code");
/// assert!("example-org/../etc".parse::<RepoName>().is_err());
/// # Ok::<(), types::ParseRepoNameError>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepoName {
    owner: String,
    name: String,
}

impl RepoName {
    /// The account or organisation that holds the repository.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// The repository's name within its owner.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl FromStr for RepoName {
    type Err = ParseRepoNameError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let error = || ParseRepoNameError {
            text: text.to_owned(),
        };
        let (owner, name) = text.split_once('/').ok_or_else(error)?;
        if !is_part(owner) || !is_part(name) {
            return Err(error());
        }
        Ok(RepoName {
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }
}

/// Whether `part` may be an owner or a repository name.
fn is_part(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

impl fmt::Display for RepoName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

/// Text that is not a repository name as [`RepoName`] describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseRepoNameError {
    text: String,
}

impl fmt::Display for ParseRepoNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}` is not a repository as owner/name", self.text)
    }
}

impl std::error::Error for ParseRepoNameError {}

/// An issue: its repository and its number there. Shown as
/// `owner/name#number`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IssueId {
    /// The repository the issue is in.
    pub repo: RepoName,
    /// The issue's number in that repository.
    pub number: u64,
}

impl fmt::Display for IssueId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.repo, self.number)
    }
}
