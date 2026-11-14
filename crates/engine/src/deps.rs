//! A ticket's repository and its dependencies, from inside: a fetch run and
//! a build run, whose writes become two read-only layers.
//!
//! - **The fetch run** has one way out, the egress, to GitHub and the
//!   ecosystem's hosts. It clones the commit into `/src` and downloads what
//!   the build needs; no code of the tree or of a dependency runs in it. Its
//!   upper layer is layer A. It is kept only when the run exits with 0 and
//!   the commit it checked out is the one asked for.
//! - **The build run** has no way out. On layer A it builds the
//!   dependencies, so their build scripts and procedural macros run with no
//!   network; its upper layer is layer B. A build that needs the network
//!   fails here.
//! - **Runs on the two layers** work in `/src` and find the dependencies
//!   built and cargo offline.
//!
//! A layer is keyed by what made it: layer A by the image, the fetch
//! script, the repository and the commit's sha; layer B by layer A's key
//! and the build script. The same key is used as it is, from the cache's
//! `layers/`, and starts no VM. A run makes its layer under a name of its
//! own; only a layer the run kept, and found good, is put in place, over
//! whatever lies there without its JSON, and its JSON is written last. A
//! cache stopped half-way is so made again, never in the way.
//!
//! The URL is built from the repository's name, never taken from text, and
//! the repository must be public: the fetch run holds no token.

use std::path::{Path, PathBuf};

use helpers::fs::{Part, write_whole};
use helpers::hash::sha256_bytes;
use serde::{Deserialize, Serialize};
use types::RepoName;
use vm::image::ImageInfo;
use vm::{Launch, Outcome, RunSpec};

/// The format of a layer's JSON in the cache.
pub const LAYER_FORMAT: &str = "kai-layer/1";

/// The commit's line on the fetch run's standard output.
const HEAD_LINE: &str = "kai-head ";

/// What kai knows of one ecosystem: its recipe, the hosts its downloads
/// need, and its two scripts.
#[derive(Clone, Copy, Debug)]
pub struct Ecosystem {
    /// Its name.
    pub name: &'static str,
    /// The image recipe both runs use.
    pub recipe: &'static str,
    /// What the fetch run's egress lets through, `github.com` for the
    /// clone included.
    pub hosts: &'static [&'static str],
    /// The fetch run's script, for `sh -c`: `$1` is the URL, `$2` the sha.
    pub fetch: &'static str,
    /// The build run's script, for `sh -c`, in `/src`.
    pub build: &'static str,
}

/// Rust: the toolchain `rust-toolchain.toml` names, then `cargo fetch`;
/// the build is `cargo test --no-run` with the target directory outside the
/// tree. Its hosts were found with a learn run on 13 November 2026, for
/// kai at its commit of that day: GitHub for the clone,
/// `static.rust-lang.org` for the toolchain, the crates.io index and its
/// downloads.
pub const RUST: Ecosystem = Ecosystem {
    name: "rust",
    recipe: "rust",
    hosts: &[
        "github.com:443",
        "static.rust-lang.org:443",
        "index.crates.io:443",
        "static.crates.io:443",
    ],
    fetch: r#"set -eu
url=$1
sha=$2
git init -q /src
git -C /src -c transfer.fsckObjects=true -c core.hooksPath=/dev/null \
  -c protocol.allow=never -c protocol.https.allow=always \
  fetch -q --depth 1 --no-tags "$url" "$sha"
git -C /src -c core.hooksPath=/dev/null checkout -q --detach "$sha"
echo "kai-head $(git -C /src rev-parse HEAD) $(git -C /src rev-parse 'HEAD^{tree}')"
cd /src
for file in rust-toolchain.toml rust-toolchain; do
  if [ -f "$file" ] && grep -Eq '^[[:space:]]*path[[:space:]]*=' "$file"; then
    echo "kai: $file names a toolchain by its path; kai installs channels only" >&2
    exit 3
  fi
done
if [ -f rust-toolchain.toml ] || [ -f rust-toolchain ]; then
  rustup toolchain install --no-self-update
fi
# Started outside the tree, cargo reads neither its toolchain path nor its
# .cargo/config.toml.
cd /
cargo fetch --locked --manifest-path /src/Cargo.toml
"#,
    build: r#"set -eu
cargo_home=${CARGO_HOME:-$HOME/.cargo}
mkdir -p "$cargo_home"
printf '[build]\ntarget-dir = "/var/kai/target"\n\n[net]\noffline = true\n' >> "$cargo_home/config.toml"
cd /src
cargo test --no-run --locked
"#,
};

/// What to fetch and build, and where.
#[derive(Clone, Debug)]
pub struct Request<'a> {
    /// The ecosystem.
    pub ecosystem: &'a Ecosystem,
    /// The repository on GitHub; it must be public.
    pub repo: &'a RepoName,
    /// The commit, a full sha.
    pub sha: &'a str,
    /// The image file of the ecosystem's recipe, and what its JSON says.
    pub image: (&'a Path, &'a ImageInfo),
    /// kai's VM cache: `layers/` and `runs/` lie there.
    pub cache: &'a Path,
    /// The fence program for the egress; `None` takes `fence` from `PATH`.
    pub fence: Option<PathBuf>,
}

/// One of the two layers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    /// Its key, `sha256:<hex>`.
    pub key: String,
    /// The layer file.
    pub file: PathBuf,
    /// Whether it was in the cache already; then no VM ran for it.
    pub cached: bool,
}

/// Layer A, the fetch run's, and layer B, the build run's, which lies on
/// it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layers {
    /// The tree in `/src` and the downloads.
    pub fetch: Layer,
    /// The dependencies built.
    pub build: Layer,
}

/// A layer's JSON in the cache, beside its file, written last.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerInfo {
    /// [`LAYER_FORMAT`].
    pub format: String,
    /// `fetch` or `build`.
    pub kind: String,
    /// Its key.
    pub key: String,
    /// The ecosystem.
    pub ecosystem: String,
    /// The repository, `owner/name`.
    pub repo: String,
    /// The commit's sha.
    pub sha: String,
    /// The image's id.
    pub image: String,
    /// SHA-256 of the script that made it, `sha256:<hex>`.
    pub script: String,
    /// For layer B: the key of layer A, which it lies on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    /// For layer A: the tree the commit holds, as git named it in the guest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<String>,
    /// The run that made it.
    pub run_dir: PathBuf,
}

/// Why there are no layers.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A run did not end with 0; its standard error says why.
    #[error("the {kind} run ended {outcome}: {detail}; see {}", run_dir.display())]
    Run {
        /// `fetch` or `build`.
        kind: &'static str,
        /// How it ended, as `result.json` names it, with the exit status.
        outcome: String,
        /// The last lines of its standard error, or the reason of the run.
        detail: String,
        /// Its run directory.
        run_dir: PathBuf,
    },
    /// The fetch run checked out another commit than the one asked for, or
    /// said none.
    #[error("the fetch run checked out {found}, not {expected}; see {}", run_dir.display())]
    Head {
        /// The sha asked for.
        expected: String,
        /// What the guest said, or that it said nothing.
        found: String,
        /// Its run directory.
        run_dir: PathBuf,
    },
    /// A run ended with 0, but its layer was not kept: kai's side failed,
    /// not the run.
    #[error("the {kind} run ended with 0, but its layer was not kept; see {}", run_dir.display())]
    NotKept {
        /// `fetch` or `build`.
        kind: &'static str,
        /// Its run directory.
        run_dir: PathBuf,
    },
    /// A run could not be made.
    #[error(transparent)]
    Vm(#[from] vm::Error),
    /// Reading or writing the cache failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file.
        path: PathBuf,
        /// What the system said.
        source: std::io::Error,
    },
}

/// The repository's URL on GitHub, made from its name.
pub fn url(repo: &RepoName) -> String {
    format!("https://github.com/{}/{}", repo.owner(), repo.name())
}

/// Layer A's key: the image, the fetch script, the repository, the sha.
pub fn fetch_key(request: &Request<'_>) -> String {
    key(&[
        &request.image.1.id,
        &sha256_bytes(request.ecosystem.fetch.as_bytes()),
        &request.repo.to_string(),
        request.sha,
    ])
}

/// Layer B's key: layer A's and the build script.
pub fn build_key(fetch_key: &str, ecosystem: &Ecosystem) -> String {
    key(&[fetch_key, &sha256_bytes(ecosystem.build.as_bytes())])
}

fn key(parts: &[&str]) -> String {
    let mut input = String::from(LAYER_FORMAT);
    for part in parts {
        input.push('\0');
        input.push_str(part);
    }
    format!("sha256:{}", sha256_bytes(input.as_bytes()))
}

/// The layer file and its JSON for `key` in `cache`.
fn paths(cache: &Path, key: &str) -> (PathBuf, PathBuf) {
    let hex = key.trim_start_matches("sha256:");
    let dir = cache.join("layers");
    (
        dir.join(format!("{hex}.ext4")),
        dir.join(format!("{hex}.json")),
    )
}

/// The layer with `key` when the cache holds it: its file and its JSON.
fn cached(cache: &Path, key: &str) -> Option<(PathBuf, LayerInfo)> {
    let (file, json) = paths(cache, key);
    let info: LayerInfo = serde_json::from_slice(&std::fs::read(json).ok()?).ok()?;
    (info.format == LAYER_FORMAT && info.key == key && file.is_file()).then_some((file, info))
}

/// Whether `cache` holds a fetch layer of `repo` at `sha`, of any image
/// or script: a fetch run checked that commit out, so the repository has
/// it. A fetch run whose guest checked out another commit keeps no layer.
pub fn fetched(cache: &Path, repo: &RepoName, sha: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(cache.join("layers")) else {
        return false;
    };
    let repo = repo.to_string();
    entries.flatten().any(|entry| {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            return false;
        }
        let Some(info) = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<LayerInfo>(&bytes).ok())
        else {
            return false;
        };
        info.format == LAYER_FORMAT
            && info.kind == "fetch"
            && info.repo == repo
            && info.sha.eq_ignore_ascii_case(sha)
            && path.with_extension("ext4").is_file()
    })
}

/// Fetches and builds, or takes what the cache holds; returns the two
/// layers.
pub async fn deps(launch: &Launch, request: &Request<'_>) -> Result<Layers, Error> {
    let dir = request.cache.join("layers");
    std::fs::create_dir_all(&dir).map_err(|source| Error::Io {
        path: dir.clone(),
        source,
    })?;
    let fetch = fetch(launch, request).await?;
    let build = build(launch, request, &fetch).await?;
    Ok(Layers { fetch, build })
}

async fn fetch(launch: &Launch, request: &Request<'_>) -> Result<Layer, Error> {
    let key = fetch_key(request);
    if let Some((file, _)) = cached(request.cache, &key) {
        return Ok(Layer {
            key,
            file,
            cached: true,
        });
    }
    let (file, json) = paths(request.cache, &key);
    let run_dir = vm::new_run_dir(request.cache, Some("fetch"));
    let (image, info) = request.image;
    let mut spec = RunSpec::new(
        image.to_path_buf(),
        vec![
            "sh".to_owned(),
            "-c".to_owned(),
            request.ecosystem.fetch.to_owned(),
            "fetch".to_owned(),
            url(request.repo),
            request.sha.to_owned(),
        ],
        run_dir.clone(),
    );
    spec.origin = Some(info.origin());
    spec.env = info.environment();
    spec.cwd = "/".to_owned();
    spec.timeout_s = 1800;
    let part = Part::beside(&file);
    spec.upper_out = Some(part.path().to_path_buf());
    let mut policy = proxy::Policy::new(request.ecosystem.hosts.iter().copied());
    if let Some(fence) = &request.fence {
        policy.fence = fence.clone();
    }
    spec.proxy = Some(policy);
    tracing::info!(repo = %request.repo, sha = request.sha, run = %run_dir.display(), "the fetch run");
    let result = vm::run(launch, &spec).await?;
    kept("fetch", &result, &run_dir)?;
    let stdout = std::fs::read_to_string(run_dir.join("out/stdout")).unwrap_or_default();
    let head = stdout
        .lines()
        .find_map(|line| line.strip_prefix(HEAD_LINE))
        .map(|rest| {
            rest.split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        });
    let (commit, tree) = match head.as_deref() {
        Some([commit, tree]) => (commit.clone(), tree.clone()),
        _ => (String::new(), String::new()),
    };
    if commit != request.sha {
        return Err(Error::Head {
            expected: request.sha.to_owned(),
            found: if commit.is_empty() {
                "nothing".to_owned()
            } else {
                commit
            },
            run_dir,
        });
    }
    put_in_place(part, &file)?;
    write_info(
        &json,
        &LayerInfo {
            format: LAYER_FORMAT.to_owned(),
            kind: "fetch".to_owned(),
            key: key.clone(),
            ecosystem: request.ecosystem.name.to_owned(),
            repo: request.repo.to_string(),
            sha: request.sha.to_owned(),
            image: info.id.clone(),
            script: format!(
                "sha256:{}",
                sha256_bytes(request.ecosystem.fetch.as_bytes())
            ),
            on: None,
            tree: Some(tree),
            run_dir,
        },
    )?;
    Ok(Layer {
        key,
        file,
        cached: false,
    })
}

async fn build(launch: &Launch, request: &Request<'_>, fetch: &Layer) -> Result<Layer, Error> {
    let key = build_key(&fetch.key, request.ecosystem);
    if let Some((file, _)) = cached(request.cache, &key) {
        return Ok(Layer {
            key,
            file,
            cached: true,
        });
    }
    let (file, json) = paths(request.cache, &key);
    let run_dir = vm::new_run_dir(request.cache, Some("build"));
    let (image, info) = request.image;
    let mut spec = RunSpec::new(
        image.to_path_buf(),
        vec![
            "sh".to_owned(),
            "-c".to_owned(),
            request.ecosystem.build.to_owned(),
            "build".to_owned(),
        ],
        run_dir.clone(),
    );
    spec.origin = Some(info.origin());
    spec.env = info.environment();
    spec.cwd = "/".to_owned();
    spec.timeout_s = 3600;
    spec.layers = vec![fetch.file.clone()];
    let part = Part::beside(&file);
    spec.upper_out = Some(part.path().to_path_buf());
    tracing::info!(run = %run_dir.display(), "the build run");
    let result = vm::run(launch, &spec).await?;
    kept("build", &result, &run_dir)?;
    put_in_place(part, &file)?;
    write_info(
        &json,
        &LayerInfo {
            format: LAYER_FORMAT.to_owned(),
            kind: "build".to_owned(),
            key: key.clone(),
            ecosystem: request.ecosystem.name.to_owned(),
            repo: request.repo.to_string(),
            sha: request.sha.to_owned(),
            image: info.id.clone(),
            script: format!(
                "sha256:{}",
                sha256_bytes(request.ecosystem.build.as_bytes())
            ),
            on: Some(fetch.key.clone()),
            tree: None,
            run_dir,
        },
    )?;
    Ok(Layer {
        key,
        file,
        cached: false,
    })
}

/// Whether the run ended with 0 and kept its upper layer: a run that did
/// not is its failure, a layer not kept after 0 is kai's.
fn kept(kind: &'static str, result: &vm::RunResult, run_dir: &Path) -> Result<(), Error> {
    if result.outcome != Outcome::Exited || result.exit_code != Some(0) {
        return Err(failed(kind, result, run_dir.to_path_buf()));
    }
    if result.upper_out != Some(true) {
        return Err(Error::NotKept {
            kind,
            run_dir: run_dir.to_path_buf(),
        });
    }
    Ok(())
}

/// The layer a run kept under its own name, renamed to `file`, over a file
/// there without its JSON.
fn put_in_place(part: Part, file: &Path) -> Result<(), Error> {
    part.keep().map_err(|source| Error::Io {
        path: file.to_path_buf(),
        source,
    })
}

/// A run's failure, with the last lines of its standard error.
fn failed(kind: &'static str, result: &vm::RunResult, run_dir: PathBuf) -> Error {
    let stderr = std::fs::read_to_string(run_dir.join("out/stderr")).unwrap_or_default();
    let tail: Vec<&str> = stderr
        .lines()
        .rev()
        .filter(|line| !line.trim().is_empty())
        .take(3)
        .collect();
    let detail = if tail.is_empty() {
        result
            .reason
            .clone()
            .unwrap_or_else(|| "it said nothing on standard error".to_owned())
    } else {
        tail.into_iter().rev().collect::<Vec<_>>().join(" | ")
    };
    let outcome = match result.exit_code {
        Some(code) => format!("{} {code}", result.outcome.name()),
        None => result.outcome.name().to_owned(),
    };
    Error::Run {
        kind,
        outcome,
        detail,
        run_dir,
    }
}

fn write_info(path: &Path, info: &LayerInfo) -> Result<(), Error> {
    let io = |source| Error::Io {
        path: path.to_path_buf(),
        source,
    };
    let mut text = serde_json::to_vec_pretty(info).map_err(|err| io(err.into()))?;
    text.push(b'\n');
    write_whole(path, &text).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> ImageInfo {
        ImageInfo {
            format: vm::image::IMAGE_FORMAT.to_owned(),
            recipe: "rust".to_owned(),
            id: format!("sha256:{}", "1".repeat(64)),
            file: format!("sha256:{}", "2".repeat(64)),
            docker_image: format!("sha256:{}", "3".repeat(64)),
            env: vec![],
            working_dir: None,
            user: None,
        }
    }

    #[test]
    fn the_url_is_made_from_the_name() {
        let repo: RepoName = "example-org/example-code".parse().unwrap();
        assert_eq!(url(&repo), "https://github.com/example-org/example-code");
    }

    #[test]
    fn keys_follow_everything_that_made_a_layer() {
        let repo: RepoName = "example-org/example-code".parse().unwrap();
        let other: RepoName = "example-org/other".parse().unwrap();
        let info = info();
        let path = Path::new("/images/rust.sqfs");
        let request = |repo, sha| Request {
            ecosystem: &RUST,
            repo,
            sha,
            image: (path, &info),
            cache: Path::new("/cache"),
            fence: None,
        };
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let a = fetch_key(&request(&repo, sha));
        assert_eq!(a, fetch_key(&request(&repo, sha)));
        assert_ne!(a, fetch_key(&request(&other, sha)));
        assert_ne!(a, fetch_key(&request(&repo, &"f".repeat(40))));
        let b = build_key(&a, &RUST);
        assert_ne!(a, b);
        assert!(b.starts_with("sha256:"));
    }

    /// Two layers in the cache are taken as they are: no VM is started, so
    /// a launch whose files are not there does not matter.
    #[tokio::test]
    async fn two_cached_layers_start_no_vm() {
        let cache = helpers::testing::TestDir::new("deps-cached");
        std::fs::create_dir_all(cache.join("layers")).unwrap();
        let repo: RepoName = "example-org/example-code".parse().unwrap();
        let info = info();
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let request = Request {
            ecosystem: &RUST,
            repo: &repo,
            sha,
            image: (Path::new("/no/image.sqfs"), &info),
            cache: &cache,
            fence: None,
        };
        let a = fetch_key(&request);
        let b = build_key(&a, &RUST);
        for (key, kind) in [(&a, "fetch"), (&b, "build")] {
            let (file, json) = paths(&cache, key);
            std::fs::write(&file, b"a layer").unwrap();
            let layer = LayerInfo {
                format: LAYER_FORMAT.to_owned(),
                kind: kind.to_owned(),
                key: key.clone(),
                ecosystem: "rust".to_owned(),
                repo: repo.to_string(),
                sha: sha.to_owned(),
                image: info.id.clone(),
                script: "sha256:0".to_owned(),
                on: None,
                tree: None,
                run_dir: cache.join("runs/0"),
            };
            write_info(&json, &layer).unwrap();
        }
        let nowhere = PathBuf::from("/no/such/file");
        let launch = Launch {
            firecracker: nowhere.clone(),
            firecracker_sha256: String::new(),
            kernel: nowhere.clone(),
            kernel_sha256: String::new(),
            initramfs: nowhere,
            initramfs_sha256: String::new(),
        };
        let layers = deps(&launch, &request).await.unwrap();
        assert!(layers.fetch.cached && layers.build.cached);
        assert_eq!(layers.fetch.key, a);
        assert_eq!(layers.build.key, b);
        assert!(!cache.join("runs").exists());
        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[test]
    fn the_rust_hosts_are_the_clones_and_the_registries() {
        assert!(RUST.hosts.contains(&"github.com:443"));
        assert!(
            RUST.fetch
                .contains("cargo fetch --locked --manifest-path /src/Cargo.toml")
        );
        assert!(RUST.build.contains("cargo test --no-run --locked"));
    }
}
