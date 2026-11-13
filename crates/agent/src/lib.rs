//! Claude Code in a microVM: a pinned read-only [`runtime`], built through
//! [`layer`], and an interactive [`session`] over a bounded guest protocol.
//! [`CLAUDE`] retains the original single-program pin for existing layer users.
//! An [`addon`], such as a skill, is a layer of pinned files of its own.
//!
//! A new version is a change of the pin and nothing else, as for
//! Firecracker in `vm`'s pins: no `latest`, no installer, no update in a
//! VM.

pub mod addon;
pub mod layer;
pub mod runtime;
pub mod session;

/// An agent kai can put into a VM: one program, pinned, and where it lies
/// in the guest.
#[derive(Clone, Copy, Debug)]
pub struct Agent {
    /// The agent's name in kai's records: `claude-code`.
    pub name: &'static str,
    /// Its version, as the program reports it.
    pub version: &'static str,
    /// The program as it is downloaded, with its SHA-256; its `name`
    /// (`Claude Code 2.1.322`) is the agent's for people.
    pub binary: vm::Artefact,
    /// The program's length in bytes, as its release names it.
    pub length: u64,
    /// Where the program lies in the guest: an absolute path under
    /// [`AGENTS_PREFIX`], of plain names.
    pub path: &'static str,
}

/// The directory of the guest under which every agent lies, so that an
/// agent's layer adds files there and nowhere else.
pub const AGENTS_PREFIX: &str = "/opt/kai/agents/";

/// Claude Code 2.1.322 for Linux on x86_64 (glibc), from Anthropic's
/// release bucket. The hash and the length are those the release's
/// `manifest.json` names for `linux-x64`.
pub const CLAUDE: Agent = Agent {
    name: "claude-code",
    version: "2.1.322",
    binary: vm::Artefact {
        name: "Claude Code 2.1.322",
        file: "claude-2.1.322",
        url: "https://storage.googleapis.com/claude-code-dist-86c565f3-f756-42ad-8dfa-d59b1c096819/claude-code-releases/2.1.322/linux-x64/claude",
        download_sha256: "bfb3703ad5380242328459bc62ada4bde8ae7ba57c6e1eeadb9c00e08874c478",
        member: None,
        sha256: "bfb3703ad5380242328459bc62ada4bde8ae7ba57c6e1eeadb9c00e08874c478",
        executable: true,
    },
    length: 341_914_128,
    path: "/opt/kai/agents/claude/2.1.322/claude",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claudes_pin_is_one_file_under_the_agents_prefix() {
        assert!(CLAUDE.path.starts_with(AGENTS_PREFIX));
        assert!(CLAUDE.path.contains(CLAUDE.version));
        assert!(CLAUDE.binary.url.starts_with("https://"));
        assert!(CLAUDE.binary.url.contains(CLAUDE.version));
        assert_eq!(CLAUDE.binary.download_sha256, CLAUDE.binary.sha256);
        assert!(CLAUDE.binary.member.is_none() && CLAUDE.binary.executable);
        assert!(layer::components(CLAUDE.path).is_ok());
    }
}
