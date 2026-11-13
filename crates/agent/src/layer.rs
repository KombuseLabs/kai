//! An agent's program as a read-only layer: a squashfs file system that
//! holds the program at its path under `/opt/kai/agents/` and nothing
//! else, every file root's, so that the layer lies on any image and the
//! agent, which runs as another user, cannot change its own program.
//!
//! [`fetch`] is what `kai agent fetch` calls. The pinned program is
//! downloaded and checked (`vm::fetch`), copied into a stage and packed
//! with `mksquashfs`; the packed layer is read back with `unsquashfs` and
//! checked, its JSON is written, and the entry is published by one rename.
//! The cache keeps, under `agents/` in kai's VM cache:
//!
//! ```text
//! downloads/<file>                  the program as downloaded, checked against its pin
//! downloads/.<file>.lock            held while it is downloaded
//! <key>/layer.sqfs                  a published entry: the layer,
//! <key>/layer.json                  and what it holds and how it was made (kai-agent-layer/1)
//! .<key>.lock                       held while the entry is checked, built or moved aside
//! .<key>.<pid>-<n>.part/            an entry being built; never used
//! .<key>.quarantine-<s>-<pid>-<n>/  an entry found invalid, moved aside
//! ```
//!
//! - **The key** is the SHA-256 of the program's pin, its path, the
//!   packing's options and `mksquashfs`'s version: a change of any of them
//!   is another entry.
//! - **An entry is used only when it checks out:** its JSON names what the
//!   key stands for, and its layer has the length and the SHA-256 the JSON
//!   names, hashed afresh each time. Two files being there is no evidence.
//! - **Published whole, once.** An entry is built in a directory of its
//!   own beside it, checked as a published one would be, flushed to the
//!   disk and renamed into place, all under the entry's lock, which a
//!   second fetch waits for and then finds the entry made.
//! - **Never rewritten.** An entry found invalid is moved aside, not
//!   changed, so that a VM which has its layer open reads on what it read;
//!   a person removes it.
//! - **A killed fetch is fetched again.** It leaves a part of an entry, or
//!   of a download, that the next fetch removes (`helpers::fs::remove_parts`):
//!   holding the lock, it knows that no builder is at work.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use helpers::fs::{Lock, PartDir, remove_parts, sync_dir};
use helpers::hash::{sha256_bytes, sha256_file_async};
use helpers::process::{self, Failed};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use super::{AGENTS_PREFIX, Agent};

/// The format of a layer's JSON, and the first part of its key.
pub const LAYER_FORMAT: &str = "kai-agent-layer/1";

/// The directory of the agents' entries in kai's VM cache.
pub const AGENTS_DIR: &str = "agents";

/// The layer's file in an entry.
pub const LAYER_FILE: &str = "layer.sqfs";

/// The layer's JSON in an entry.
pub const INFO_FILE: &str = "layer.json";

/// `mksquashfs`'s options that make the layer what it is: every file
/// root's, zstd, a new file, no extended attributes, every time stamp 0 so
/// that two packs of the same program are the same bytes. Part of the key.
pub const PACKING: [&str; 7] = [
    "-all-root",
    "-comp",
    "zstd",
    "-noappend",
    "-no-xattrs",
    "-repro-time",
    "0",
];

/// The longest JSON of an entry that is read.
const MAX_INFO: u64 = 64 * 1024;

/// What a layer is and how it was made: its JSON, beside it, written last.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerInfo {
    /// [`LAYER_FORMAT`].
    pub format: String,
    /// The entry's key, `sha256:<hex>`.
    pub key: String,
    /// The agent, as [`Agent::name`].
    pub agent: String,
    /// Its version.
    pub version: String,
    /// The program's SHA-256, `sha256:<hex>`.
    pub binary: String,
    /// The program's length in bytes.
    pub binary_len: u64,
    /// Where the program lies in the guest.
    pub path: String,
    /// The layer's SHA-256, `sha256:<hex>`.
    pub layer: String,
    /// The layer's length in bytes.
    pub layer_len: u64,
    /// `mksquashfs`'s options, [`PACKING`].
    pub packing: Vec<String>,
    /// `mksquashfs`'s version.
    pub mksquashfs: String,
}

/// What [`fetch`] did, and where the layer is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetched {
    /// The layer, for `--layer`.
    pub file: PathBuf,
    /// Its JSON.
    pub info: LayerInfo,
    /// Whether the entry was there and checked out; then nothing was
    /// downloaded or built.
    pub reused: bool,
    /// Whether the program was downloaded now; `false` when it was in the
    /// cache and checked out.
    pub downloaded: bool,
    /// An entry under the same key that did not check out, where it was
    /// moved, and why.
    pub quarantined: Option<(PathBuf, String)>,
    /// Build directories and parts of downloads that killed fetches left,
    /// now removed.
    pub removed: usize,
}

/// Where [`fetch_with`] stands, each reported once it is reached and
/// before fetch goes on: for a caller that shows progress, and for the
/// tests that stop a fetch at a known point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The entry's lock is held; the entry is not checked yet.
    Locked,
    /// The download's lock is held; the program is about to be checked in
    /// the cache or downloaded.
    Download,
    /// The program is on the stage in the entry's part; not packed yet.
    Staged,
    /// `mksquashfs` made the layer in the part; not read back yet.
    Packed,
    /// The layer was read back and its JSON written and flushed, in the
    /// part; not renamed yet.
    Written,
    /// The part was renamed to the entry; the cache's directory is not
    /// flushed yet.
    Published,
}

/// Why there is no layer.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The agent's path is not one a layer can hold.
    #[error("the agent's path `{0}` is not an absolute path under {AGENTS_PREFIX} of plain names")]
    Path(String),
    /// The download failed or did not match its pin.
    #[error(transparent)]
    Vm(#[from] vm::Error),
    /// A program kai runs on the host failed.
    #[error("{program}: {message}")]
    Tool {
        /// The program.
        program: &'static str,
        /// Its words, or why it did not start.
        message: String,
    },
    /// The layer `mksquashfs` made is not what it should be; nothing was
    /// published.
    #[error(
        "the packed layer is not the agent's program alone, root's: {0}; nothing was published"
    )]
    Packed(String),
    /// Reading or writing the cache failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// What the system said.
        source: std::io::Error,
    },
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// The names of `path` below `/`: an absolute path under
/// [`AGENTS_PREFIX`], each name letters, digits, `.`, `_` or `-`, and not
/// `.` or `..`.
pub fn components(path: &str) -> Result<Vec<&str>, Error> {
    let bad = || Error::Path(path.to_owned());
    if !path.starts_with(AGENTS_PREFIX) {
        return Err(bad());
    }
    let names: Vec<&str> = path[1..].split('/').collect();
    let plain = |name: &&str| {
        !name.is_empty()
            && *name != "."
            && *name != ".."
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    // The prefix ends with `/`, so a path that is the prefix alone ends
    // with an empty name.
    if !names.iter().all(plain) {
        return Err(bad());
    }
    Ok(names)
}

/// What an entry's JSON must say, but for the layer's hash and length.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    key: String,
    agent: String,
    version: String,
    binary: String,
    binary_len: u64,
    path: String,
    packing: Vec<String>,
    mksquashfs: String,
}

impl Identity {
    fn new(agent: &Agent, mksquashfs: &str) -> Identity {
        let binary = format!("sha256:{}", agent.binary.sha256);
        let packing: Vec<String> = PACKING.iter().map(|&option| option.to_owned()).collect();
        let mut input = String::from(LAYER_FORMAT);
        for part in [
            binary.as_str(),
            &agent.length.to_string(),
            agent.path,
            &packing.join(" "),
            mksquashfs,
        ] {
            input.push('\0');
            input.push_str(part);
        }
        Identity {
            key: format!("sha256:{}", sha256_bytes(input.as_bytes())),
            agent: agent.name.to_owned(),
            version: agent.version.to_owned(),
            binary,
            binary_len: agent.length,
            path: agent.path.to_owned(),
            packing,
            mksquashfs: mksquashfs.to_owned(),
        }
    }

    fn hex(&self) -> &str {
        self.key.trim_start_matches("sha256:")
    }

    /// Why `info` is not this identity's, or `None` when it is.
    fn mismatch(&self, info: &LayerInfo) -> Option<String> {
        let differs = |what: &str, found: &dyn std::fmt::Debug, expected: &dyn std::fmt::Debug| {
            Some(format!("its JSON names {what} {found:?}, not {expected:?}"))
        };
        if info.format != LAYER_FORMAT {
            return differs("the format", &info.format, &LAYER_FORMAT);
        }
        if info.key != self.key {
            return differs("the key", &info.key, &self.key);
        }
        if info.agent != self.agent || info.version != self.version {
            return differs(
                "the agent",
                &(&info.agent, &info.version),
                &(&self.agent, &self.version),
            );
        }
        if info.binary != self.binary || info.binary_len != self.binary_len {
            return differs(
                "the program",
                &(&info.binary, info.binary_len),
                &(&self.binary, self.binary_len),
            );
        }
        if info.path != self.path {
            return differs("the path", &info.path, &self.path);
        }
        if info.packing != self.packing || info.mksquashfs != self.mksquashfs {
            return differs(
                "the packing",
                &(&info.packing, &info.mksquashfs),
                &(&self.packing, &self.mksquashfs),
            );
        }
        None
    }
}

/// Puts `agent` into the cache under `cache` as a layer, unless an entry
/// there checks out, and says what it did. Downloads only when the program
/// is not in the cache already, checked. Needs `mksquashfs` and
/// `unsquashfs` (squashfs-tools).
pub async fn fetch(agent: &Agent, cache: &Path) -> Result<Fetched, Error> {
    fetch_with(agent, cache, |_| {}).await
}

/// [`fetch`], calling `at` with each [`Step`] as it is reached.
pub async fn fetch_with(
    agent: &Agent,
    cache: &Path,
    mut at: impl FnMut(Step),
) -> Result<Fetched, Error> {
    let names = components(agent.path)?;
    let id = Identity::new(agent, &mksquashfs_version().await?);
    let dir = cache.join(AGENTS_DIR);
    let downloads = dir.join("downloads");
    std::fs::create_dir_all(&downloads).map_err(io(&downloads))?;
    let hex = id.hex().to_owned();
    let lock_file = dir.join(format!(".{hex}.lock"));
    let _lock = Lock::acquire(&lock_file).await.map_err(io(&lock_file))?;
    at(Step::Locked);

    let entry = dir.join(&hex);
    let mut removed = remove_parts(&entry).map_err(io(&dir))?;
    let mut quarantined = None;
    match check_entry(&entry, &id).await {
        Entry::Valid(info) => {
            return Ok(Fetched {
                file: entry.join(LAYER_FILE),
                info: *info,
                reused: true,
                downloaded: false,
                quarantined: None,
                removed,
            });
        }
        Entry::Missing => {}
        Entry::Invalid(why) => {
            let aside = dir.join(format!(".{hex}.quarantine-{}", unique()));
            std::fs::rename(&entry, &aside).map_err(io(&entry))?;
            sync_dir(&dir).map_err(io(&dir))?;
            tracing::warn!(entry = %entry.display(), aside = %aside.display(), why, "an invalid entry was moved aside");
            quarantined = Some((aside, why));
        }
    }

    let download_lock = downloads.join(format!(".{}.lock", agent.binary.file));
    let downloaded = {
        let _lock = Lock::acquire(&download_lock)
            .await
            .map_err(io(&download_lock))?;
        at(Step::Download);
        removed += remove_parts(&downloads.join(agent.binary.file)).map_err(io(&downloads))?;
        vm::fetch(&agent.binary, &downloads).await? == vm::Fetched::Downloaded
    };

    let build = PartDir::beside(&entry).map_err(io(&dir))?;
    let stage = build.path().join("stage");
    let layer = build.path().join(LAYER_FILE);
    put_on_stage(&downloads.join(agent.binary.file), &stage, &names).await?;
    at(Step::Staged);
    let mut command = Command::new("mksquashfs");
    command
        .arg(&stage)
        .arg(&layer)
        .args(PACKING)
        .args(["-quiet", "-no-progress"]);
    process::run(&mut command)
        .await
        .map_err(tool("mksquashfs"))?;
    at(Step::Packed);
    std::fs::remove_dir_all(&stage).map_err(io(&stage))?;
    check_packed(&layer, agent, &names).await?;

    let hashed = sha256_file_async(layer.clone()).await.map_err(io(&layer))?;
    let layer_len = std::fs::metadata(&layer).map_err(io(&layer))?.len();
    let info = LayerInfo {
        format: LAYER_FORMAT.to_owned(),
        key: id.key.clone(),
        agent: id.agent.clone(),
        version: id.version.clone(),
        binary: id.binary.clone(),
        binary_len: id.binary_len,
        path: id.path.clone(),
        layer: format!("sha256:{hashed}"),
        layer_len,
        packing: id.packing.clone(),
        mksquashfs: id.mksquashfs.clone(),
    };
    let json = build.path().join(INFO_FILE);
    let mut text = serde_json::to_vec_pretty(&info).map_err(|err| io(&json)(err.into()))?;
    text.push(b'\n');
    std::fs::write(&json, text).map_err(io(&json))?;
    for file in [&layer, &json] {
        std::fs::File::open(file)
            .and_then(|file| file.sync_all())
            .map_err(io(file))?;
    }
    sync_dir(build.path()).map_err(io(build.path()))?;
    match check_entry(build.path(), &id).await {
        Entry::Valid(_) => {}
        Entry::Missing => return Err(Error::Packed("the build directory went away".to_owned())),
        Entry::Invalid(why) => return Err(Error::Packed(why)),
    }
    at(Step::Written);
    // The entry is not there: the lock is held, and what was there was
    // moved aside.
    build.keep().map_err(io(&entry))?;
    at(Step::Published);
    sync_dir(&dir).map_err(io(&dir))?;
    tracing::info!(entry = %entry.display(), "the agent's layer was built and published");
    Ok(Fetched {
        file: entry.join(LAYER_FILE),
        info,
        reused: false,
        downloaded,
        quarantined,
        removed,
    })
}

/// `mksquashfs`'s version, the word after `version` in the first line it
/// prints.
async fn mksquashfs_version() -> Result<String, Error> {
    let out = process::run(Command::new("mksquashfs").arg("-version"))
        .await
        .map_err(tool("mksquashfs"))?;
    let text = String::from_utf8_lossy(&out);
    let line = text.lines().next().unwrap_or_default();
    let mut words = line.split_whitespace();
    words
        .by_ref()
        .find(|&word| word == "version")
        .and(words.next())
        .map(str::to_owned)
        .ok_or_else(|| Error::Tool {
            program: "mksquashfs",
            message: format!("no version in `{line}`"),
        })
}

fn tool(program: &'static str) -> impl FnOnce(Failed) -> Error {
    move |failed| Error::Tool {
        program,
        message: failed.to_string(),
    }
}

/// A name of its own among processes and within one.
fn unique() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    format!("{secs}-{}-{n}", std::process::id())
}

/// What an entry's directory holds.
enum Entry {
    /// Nothing at its name.
    Missing,
    /// A layer whose JSON is the identity's and whose bytes are the ones
    /// the JSON names.
    Valid(Box<LayerInfo>),
    /// Something else, and why it is not an entry.
    Invalid(String),
}

/// Checks the entry at `dir` against `id`, hashing its layer afresh.
async fn check_entry(dir: &Path, id: &Identity) -> Entry {
    match std::fs::symlink_metadata(dir) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Entry::Missing,
        Err(err) => return Entry::Invalid(format!("cannot read it: {err}")),
        Ok(meta) if !meta.is_dir() => return Entry::Invalid("not a directory".to_owned()),
        Ok(_) => {}
    }
    let json = dir.join(INFO_FILE);
    let info: LayerInfo = match std::fs::symlink_metadata(&json) {
        Err(err) => return Entry::Invalid(format!("no {INFO_FILE}: {err}")),
        Ok(meta) if !meta.is_file() => return Entry::Invalid(format!("{INFO_FILE} is not a file")),
        Ok(meta) if meta.len() > MAX_INFO => {
            return Entry::Invalid(format!("{INFO_FILE} has {} bytes", meta.len()));
        }
        Ok(_) => match std::fs::read(&json).map(|text| serde_json::from_slice(&text)) {
            Ok(Ok(info)) => info,
            Ok(Err(err)) => return Entry::Invalid(format!("{INFO_FILE} does not read: {err}")),
            Err(err) => return Entry::Invalid(format!("cannot read {INFO_FILE}: {err}")),
        },
    };
    if let Some(why) = id.mismatch(&info) {
        return Entry::Invalid(why);
    }
    let layer = dir.join(LAYER_FILE);
    match std::fs::symlink_metadata(&layer) {
        Err(err) => return Entry::Invalid(format!("no {LAYER_FILE}: {err}")),
        Ok(meta) if !meta.is_file() => {
            return Entry::Invalid(format!("{LAYER_FILE} is not a file"));
        }
        Ok(meta) if meta.len() != info.layer_len => {
            return Entry::Invalid(format!(
                "{LAYER_FILE} has {} bytes, its JSON names {}",
                meta.len(),
                info.layer_len
            ));
        }
        Ok(_) => {}
    }
    match sha256_file_async(layer).await {
        Ok(hash) if format!("sha256:{hash}") == info.layer => Entry::Valid(Box::new(info)),
        Ok(hash) => Entry::Invalid(format!(
            "{LAYER_FILE} has SHA-256 {hash}, its JSON names {}",
            info.layer
        )),
        Err(err) => Entry::Invalid(format!("cannot read {LAYER_FILE}: {err}")),
    }
}

/// Copies the program `from` to its path under `stage`, the directories
/// and the program mode 0755, whatever the umask.
async fn put_on_stage(from: &Path, stage: &Path, names: &[&str]) -> Result<(), Error> {
    let mut dir = stage.to_path_buf();
    let (program, dirs) = names
        .split_last()
        .ok_or_else(|| Error::Path(String::new()))?;
    let mut made = vec![dir.clone()];
    for name in dirs {
        dir.push(name);
        made.push(dir.clone());
    }
    std::fs::create_dir_all(&dir).map_err(io(&dir))?;
    let to = dir.join(program);
    let (copy_from, copy_to) = (from.to_path_buf(), to.clone());
    tokio::task::spawn_blocking(move || std::fs::copy(copy_from, copy_to))
        .await
        .unwrap_or_else(|join| Err(std::io::Error::other(join)))
        .map_err(io(&to))?;
    made.push(to);
    for path in &made {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(io(path))?;
    }
    Ok(())
}

/// Reads the packed layer back: it lists the program's directories and
/// the program, nothing else, every one root's and mode 0755, and the
/// program's bytes are the pinned ones.
async fn check_packed(layer: &Path, agent: &Agent, names: &[&str]) -> Result<(), Error> {
    let listing = process::run(Command::new("unsquashfs").args(["-lln", "-UTC"]).arg(layer))
        .await
        .map_err(tool("unsquashfs"))?;
    let listing = String::from_utf8_lossy(&listing);
    let found: Vec<&str> = listing.lines().filter(|line| !line.is_empty()).collect();
    let mut expected = vec![("drwxr-xr-x", "squashfs-root".to_owned(), None)];
    for depth in 1..=names.len() {
        let path = format!("squashfs-root/{}", names[..depth].join("/"));
        if depth == names.len() {
            expected.push(("-rwxr-xr-x", path, Some(agent.length)));
        } else {
            expected.push(("drwxr-xr-x", path, None));
        }
    }
    if found.len() != expected.len() {
        return Err(Error::Packed(format!(
            "it lists {} entries, not {}",
            found.len(),
            expected.len()
        )));
    }
    for (line, (mode, path, size)) in found.iter().zip(&expected) {
        // `<mode> <uid>/<gid> <size> <date> <time> <path>`; the paths are
        // plain names, so the path is the sixth word.
        let words: Vec<&str> = line.split_whitespace().collect();
        let fits = words.len() == 6
            && words[0] == *mode
            && words[1] == "0/0"
            && words[5] == path
            && size.is_none_or(|size| words[2] == size.to_string());
        if !fits {
            return Err(Error::Packed(format!(
                "`{line}` where `{mode} 0/0 … {path}` belongs"
            )));
        }
    }
    let mut command = Command::new("unsquashfs");
    command.arg("-cat").arg(layer).arg(names.join("/"));
    let hashed = process::run_hashed(&mut command)
        .await
        .map_err(tool("unsquashfs"))?;
    if hashed.sha256 != agent.binary.sha256 || hashed.len != agent.length {
        return Err(Error::Packed(format!(
            "the program in it has SHA-256 {} and {} bytes, not the pinned {} and {}",
            hashed.sha256, hashed.len, agent.binary.sha256, agent.length
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(path: &'static str) -> Agent {
        Agent {
            path,
            ..super::super::CLAUDE
        }
    }

    #[test]
    fn a_path_is_absolute_under_the_prefix_and_of_plain_names() {
        assert_eq!(
            components("/opt/kai/agents/claude/1.0/claude").unwrap(),
            ["opt", "kai", "agents", "claude", "1.0", "claude"]
        );
        for bad in [
            "opt/kai/agents/claude",
            "/opt/kai/agents/",
            "/opt/kai/agents",
            "/opt/kai/other/claude",
            "/opt/kai/agents/../claude",
            "/opt/kai/agents/./claude",
            "/opt/kai/agents//claude",
            "/opt/kai/agents/cla ude",
            "/opt/kai/agents/claude/",
        ] {
            assert!(components(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_key_changes_with_every_part_of_the_identity() {
        let base = Identity::new(&super::super::CLAUDE, "4.7.5");
        assert_eq!(base, Identity::new(&super::super::CLAUDE, "4.7.5"));
        let mut other = super::super::CLAUDE;
        other.binary.sha256 = "0000";
        for changed in [
            Identity::new(&super::super::CLAUDE, "4.7.6"),
            Identity::new(&agent("/opt/kai/agents/claude/other/claude"), "4.7.5"),
            Identity::new(&other, "4.7.5"),
            Identity::new(
                &Agent {
                    length: 1,
                    ..super::super::CLAUDE
                },
                "4.7.5",
            ),
        ] {
            assert_ne!(changed.key, base.key);
        }
    }

    #[test]
    fn a_json_of_another_identity_is_named_by_what_differs() {
        let id = Identity::new(&super::super::CLAUDE, "4.7.5");
        let info = LayerInfo {
            format: LAYER_FORMAT.to_owned(),
            key: id.key.clone(),
            agent: id.agent.clone(),
            version: id.version.clone(),
            binary: id.binary.clone(),
            binary_len: id.binary_len,
            path: id.path.clone(),
            layer: "sha256:00".to_owned(),
            layer_len: 1,
            packing: id.packing.clone(),
            mksquashfs: id.mksquashfs.clone(),
        };
        assert_eq!(id.mismatch(&info), None);
        let older = Identity::new(&super::super::CLAUDE, "4.6.1");
        assert!(older.mismatch(&info).unwrap().contains("the key"));
        let mut moved = info.clone();
        moved.path = "/opt/kai/agents/elsewhere".to_owned();
        assert!(id.mismatch(&moved).unwrap().contains("the path"));
    }
}
