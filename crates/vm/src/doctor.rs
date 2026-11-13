//! `kai vm doctor`: what running a microVM needs on this machine, whether
//! each need is met, and what fixes it when it is not.
//!
//! The checks look and do not change anything: they open `/dev/kvm`, hash
//! the files in the cache and ask each program for its version.

use std::path::Path;
use std::process::Command;

use helpers::hash::sha256_file;

use crate::{ARTEFACTS, Artefact, initramfs};

/// The oldest `mke2fs` that builds a file system from a directory or tar
/// stream the way the runs will need (`-d`).
const MKE2FS_MIN: [u32; 3] = [1, 47, 1];

/// One need, and how it stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    /// What is needed, for people: `/dev/kvm, readable and writable`.
    pub need: String,
    /// Whether this machine has it.
    pub met: bool,
    /// Whether only some commands need it, so that a run works without it.
    pub optional: bool,
    /// When met, what was found; otherwise what fixes it.
    pub detail: String,
}

impl Check {
    fn met(need: impl Into<String>, detail: impl Into<String>) -> Check {
        Check {
            need: need.into(),
            met: true,
            optional: false,
            detail: detail.into(),
        }
    }

    fn unmet(need: impl Into<String>, fix: impl Into<String>) -> Check {
        Check {
            need: need.into(),
            met: false,
            optional: false,
            detail: fix.into(),
        }
    }

    fn optional(mut self) -> Check {
        self.optional = true;
        self
    }
}

/// Every check, in the order they are best read: the machine, the
/// artefacts in `cache`, the programs.
pub fn check(cache: &Path) -> Vec<Check> {
    let mut checks = vec![architecture(), kvm(), init()];
    checks.extend(
        ARTEFACTS
            .iter()
            .map(|artefact| self::artefact(artefact, cache)),
    );
    checks.push(e2fsprogs("mke2fs", Some(MKE2FS_MIN)));
    checks.push(e2fsprogs("debugfs", None));
    checks.push(e2fsprogs("e2fsck", None));
    checks.push(program(
        "mksquashfs",
        "-version",
        "squashfs-tools, which packs images",
    ));
    checks.push(program(
        "tar",
        "--version",
        "tar, which unpacks Firecracker's release",
    ));
    checks.push(docker());
    checks.push(fence());
    checks
}

/// The oldest fence with `fence proxy`.
const FENCE_MIN: [u32; 3] = [0, 3, 0];

/// fence runs the proxy behind `--proxy`; a run without one does without.
fn fence() -> Check {
    let need = "fence 0.3.0 or newer, for `kai vm run --proxy` only";
    match version_line("fence", "--version") {
        None => Check::unmet(
            need,
            "fence not found: install it from the toolbox (cargo install --path fence)",
        )
        .optional(),
        Some(line) => match version(&line) {
            Some(found) if found.as_slice() >= FENCE_MIN.as_slice() => Check::met(need, line),
            _ => Check::unmet(need, format!("found {line}; `fence proxy` needs 0.3.0")).optional(),
        },
    }
}

/// Docker builds images from recipes; a run on a built image does without.
fn docker() -> Check {
    let need = "Docker, for `kai vm image build` only";
    let output = Command::new("docker")
        .args(["version", "--format", "{{.Server.Version}}"])
        .output();
    match output {
        Ok(output) if output.status.success() => Check::met(
            need,
            format!("Docker {}", String::from_utf8_lossy(&output.stdout).trim()),
        ),
        Ok(output) => Check::unmet(
            need,
            format!(
                "the Docker daemon does not answer: {}",
                String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
            ),
        )
        .optional(),
        Err(_) => Check::unmet(need, "docker not found: install Docker to build images").optional(),
    }
}

/// Firecracker and the pinned kernel are built for x86_64.
fn architecture() -> Check {
    let need = "an x86_64 machine";
    match std::env::consts::ARCH {
        "x86_64" => Check::met(need, "x86_64"),
        other => Check::unmet(
            need,
            format!("this is {other}; kai runs VMs on x86_64 only"),
        ),
    }
}

/// kai's init, which its build puts into kai.
fn init() -> Check {
    let need = "kai's guest init, built into kai";
    if initramfs::INIT.is_empty() {
        Check::unmet(
            need,
            format!(
                "this kai was built without it ({}); build kai again with the target x86_64-unknown-linux-musl installed (rustup target add x86_64-unknown-linux-musl)",
                initramfs::INIT_ERROR
            ),
        )
    } else {
        Check::met(need, format!("{} bytes", initramfs::INIT.len()))
    }
}

/// Opening the device is the test: access may come from a group or from
/// an access control list, and only the open says which applies.
fn kvm() -> Check {
    let need = "/dev/kvm, readable and writable";
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
    {
        Ok(_) => Check::met(need, "opened"),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Check::unmet(
            need,
            "there is no /dev/kvm: turn on virtualization in the firmware, or load the kvm module",
        ),
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => Check::unmet(
            need,
            "you may not open it: add yourself to the group that owns it (usually kvm), then log in again",
        ),
        Err(err) => Check::unmet(need, format!("cannot open it: {err}")),
    }
}

/// Whether `artefact` is in `cache` with its pinned hash.
pub fn artefact(artefact: &Artefact, cache: &Path) -> Check {
    let path = cache.join(artefact.file);
    let need = artefact.name;
    match sha256_file(&path) {
        Ok(found) if found == artefact.sha256 => Check::met(need, path.display().to_string()),
        Ok(_) => Check::unmet(
            need,
            format!(
                "{} is not the pinned file; `kai vm fetch` replaces it",
                path.display()
            ),
        ),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Check::unmet(
            need,
            format!("not in {}; `kai vm fetch` downloads it", cache.display()),
        ),
        Err(err) => Check::unmet(need, format!("cannot read {}: {err}", path.display())),
    }
}

/// A program of e2fsprogs, at least `min` when that is given.
fn e2fsprogs(name: &str, min: Option<[u32; 3]>) -> Check {
    let need = match min {
        Some(min) => format!("{name} {}.{}.{} or newer", min[0], min[1], min[2]),
        None => name.to_owned(),
    };
    let fix = format!(
        "install e2fsprogs{}, or put the directory that holds {name} (often /usr/sbin) on PATH",
        min.map(|min| format!(" {}.{}.{} or newer", min[0], min[1], min[2]))
            .unwrap_or_default()
    );
    let Some(line) = version_line(name, "-V") else {
        return Check::unmet(need, format!("{name} not found: {fix}"));
    };
    match (min, version(&line)) {
        (None, _) => Check::met(need, line),
        (Some(min), Some(found)) if found.as_slice() >= min.as_slice() => Check::met(need, line),
        (Some(_), Some(_)) => Check::unmet(need, format!("found {line}: {fix}")),
        (Some(_), None) => Check::unmet(
            need,
            format!("cannot read the version of {name} from `{line}`: {fix}"),
        ),
    }
}

/// A program that has to be there; `what` names it and what kai needs it
/// for.
fn program(name: &str, flag: &str, what: &str) -> Check {
    match version_line(name, flag) {
        Some(line) => Check::met(name, line),
        None => Check::unmet(name, format!("{name} not found: install {what}")),
    }
}

/// The first line `name flag` prints on standard output or standard error,
/// or `None` when the program does not start.
fn version_line(name: &str, flag: &str) -> Option<String> {
    let output = Command::new(name).arg(flag).output().ok()?;
    let text = if output.stdout.is_empty() {
        output.stderr
    } else {
        output.stdout
    };
    let text = String::from_utf8_lossy(&text);
    Some(text.lines().next().unwrap_or("").trim().to_owned())
}

/// The first word of `line` that is a version, `1.47.2` in
/// `mke2fs 1.47.2 (1-Jan-2025)`, as its numbers.
fn version(line: &str) -> Option<Vec<u32>> {
    line.split_whitespace().find_map(|word| {
        let numbers: Option<Vec<u32>> = word.split('.').map(|part| part.parse().ok()).collect();
        numbers.filter(|numbers| numbers.len() >= 2)
    })
}

#[cfg(test)]
mod tests {
    use super::version;

    #[test]
    fn versions_are_read_from_the_programs_first_lines() {
        assert_eq!(version("mke2fs 1.47.2 (1-Jan-2025)"), Some(vec![1, 47, 2]));
        assert_eq!(
            version("mksquashfs version 4.7.5 (2026/03/01)"),
            Some(vec![4, 7, 5])
        );
        assert_eq!(version("tar (GNU tar) 1.35"), Some(vec![1, 35]));
        assert_eq!(version("no version here 7"), None);
    }

    #[test]
    fn versions_compare_number_by_number() {
        let min: &[u32] = &[1, 47, 1];
        assert!(version("mke2fs 1.47.2 (1-Jan-2025)").unwrap().as_slice() >= min);
        assert!(version("mke2fs 1.47.1 (20-May-2024)").unwrap().as_slice() >= min);
        assert!(version("mke2fs 1.46.5 (30-Dec-2021)").unwrap().as_slice() < min);
        assert!(version("mke2fs 1.100.0 (1-Jan-2030)").unwrap().as_slice() >= min);
    }
}
