//! Images from recipes. A recipe is one Dockerfile in this repository's
//! `images/`, compiled into kai, so that what an image is made of is kai's
//! own and reviewed; no ticket and no repository brings one. kai builds it
//! with Docker on a person's command (`kai vm image build`), with an empty
//! context, so the Dockerfile is the whole input, and packs Docker's file
//! system into a squashfs image.
//!
//! The cache keeps, per image:
//!
//! ```text
//! images/<id>.sqfs        the file system, owners and modes kept
//! images/<id>.json        what the export drops (Env, WorkingDir, User), the file's hash, Docker's id
//! images/<id>.build.log   Docker's output
//! ```
//!
//! The image is the file and the JSON beside it; the JSON is written last,
//! so an image without one is not built. The JSON is kai's own bookkeeping
//! in its cache and read by nothing else; it names its format so that a
//! cache from another kai is not misread. A run with a cached image needs
//! no Docker.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use helpers::fs::{Part, write_whole};
use helpers::hash::sha256_bytes;
use helpers::process::Failed;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::fetch::hash_file;
use crate::{DEFAULT_PATH, Error, Launch, Origin, Outcome, RunSpec};

/// The format of an image's JSON in the cache.
pub const IMAGE_FORMAT: &str = "kai-vm-image/1";

/// One recipe: a Dockerfile kai ships.
#[derive(Clone, Copy, Debug)]
pub struct Recipe {
    /// Its name, as `kai vm image build` and `--image` take it.
    pub name: &'static str,
    /// The Dockerfile.
    pub text: &'static str,
}

/// Every recipe kai has.
pub const RECIPES: &[Recipe] = &[Recipe {
    name: "rust",
    text: include_str!("../../../images/rust.Dockerfile"),
}];

/// The recipe named `name`.
pub fn recipe(name: &str) -> Option<&'static Recipe> {
    RECIPES.iter().find(|recipe| recipe.name == name)
}

/// The image id of `recipe`: the SHA-256 of `kai-image/1`, its name and its
/// text, as `sha256:<hex>`. The same recipe has the same id, and is built
/// once.
pub fn image_id(recipe: &Recipe) -> String {
    let input = format!("kai-image/1\0{}\0{}", recipe.name, recipe.text);
    format!("sha256:{}", sha256_bytes(input.as_bytes()))
}

/// The rules every recipe keeps, so that an image is the same when it is
/// built again:
///
/// - every `FROM` names its image by digest (`name@sha256:<64 hex>`);
/// - a recipe that installs Debian packages reads them from
///   `snapshot.debian.org`, Debian as it was on one day, and never from
///   `deb.debian.org`;
/// - a recipe that downloads with `curl`, `wget` or `ADD` checks what it
///   got with `sha256sum`.
///
/// ```
/// let recipe = "FROM debian:bookworm-slim\nRUN true\n";
/// assert!(vm::image::check_recipe(recipe).unwrap_err().contains("digest"));
/// ```
pub fn check_recipe(text: &str) -> Result<(), String> {
    let mut stages: Vec<String> = Vec::new();
    for line in logical_lines(text) {
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some(keyword) = words.first() else {
            continue;
        };
        if keyword.eq_ignore_ascii_case("FROM") {
            let operands: Vec<&str> = words[1..]
                .iter()
                .copied()
                .filter(|word| !word.starts_with("--"))
                .collect();
            let image = operands.first().copied().unwrap_or_default();
            if !pinned(image) && !stages.iter().any(|stage| stage.eq_ignore_ascii_case(image)) {
                return Err(format!(
                    "`FROM {image}` names no digest; write name@sha256:<64 hex>, or an earlier stage"
                ));
            }
            if let [_, r#as, name] = operands.as_slice()
                && r#as.eq_ignore_ascii_case("AS")
            {
                stages.push((*name).to_owned());
            }
            stages.push(String::new());
        }
    }
    if stages.is_empty() {
        return Err("the recipe has no FROM".to_owned());
    }
    let body = logical_lines(text)
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    let installs = body.contains("apt-get") || body.split_whitespace().any(|word| word == "apt");
    if installs {
        if !body.contains("snapshot.debian.org") {
            return Err("it installs Debian packages but not from snapshot.debian.org".to_owned());
        }
        if body.contains("deb.debian.org") {
            return Err("it reads deb.debian.org, which changes from day to day".to_owned());
        }
    }
    let downloads = body.contains("curl ")
        || body.contains("wget ")
        || body
            .lines()
            .any(|line| line.trim_start().starts_with("add "));
    if downloads && !body.contains("sha256sum") && !body.contains("--checksum=sha256:") {
        return Err("it downloads without checking a SHA-256".to_owned());
    }
    Ok(())
}

/// The Dockerfile's instructions, with continued lines joined and comments
/// left out.
fn logical_lines(text: &str) -> impl Iterator<Item = String> + '_ {
    let mut lines = text
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'));
    std::iter::from_fn(move || {
        let mut joined = String::new();
        for line in lines.by_ref() {
            match line.trim_end().strip_suffix('\\') {
                Some(head) => {
                    joined.push_str(head);
                    joined.push(' ');
                }
                None => {
                    joined.push_str(line);
                    return Some(joined);
                }
            }
        }
        (!joined.is_empty()).then_some(joined)
    })
}

/// `name@sha256:<64 lower-case hex>`.
fn pinned(image: &str) -> bool {
    image.split_once("@sha256:").is_some_and(|(name, digest)| {
        !name.is_empty()
            && digest.len() == 64
            && digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// An image in the cache: what `images/<id>.json` holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageInfo {
    /// [`IMAGE_FORMAT`].
    pub format: String,
    /// The recipe it was made from.
    pub recipe: String,
    /// Its id ([`image_id`]).
    pub id: String,
    /// SHA-256 of the squashfs file, as `sha256:<hex>`.
    pub file: String,
    /// The id Docker gave the image it built.
    pub docker_image: String,
    /// The image's environment, `NAME=value` each, which the export drops.
    pub env: Vec<String>,
    /// The image's working directory, if it names one. Runs start in the
    /// work tree all the same.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// The image's user, if it names one. Runs are root unless they say
    /// otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

impl ImageInfo {
    /// The image's environment as a map, with `PATH` set when the image
    /// names none.
    pub fn environment(&self) -> std::collections::BTreeMap<String, String> {
        let mut env: std::collections::BTreeMap<String, String> = self
            .env
            .iter()
            .filter_map(|entry| entry.split_once('='))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        env.entry("PATH".to_owned())
            .or_insert_with(|| DEFAULT_PATH.to_owned());
        env
    }

    /// Where the run records the image came from.
    pub fn origin(&self) -> Origin {
        Origin {
            recipe: self.recipe.clone(),
            id: self.id.clone(),
        }
    }
}

/// The image file and its JSON for `id` in `cache`.
fn paths(cache: &Path, id: &str) -> (PathBuf, PathBuf) {
    let hex = id.strip_prefix("sha256:").unwrap_or(id);
    let dir = cache.join("images");
    (
        dir.join(format!("{hex}.sqfs")),
        dir.join(format!("{hex}.json")),
    )
}

/// The built image of `recipe` in `cache`, or `None` while it is not built.
pub fn built(recipe: &Recipe, cache: &Path) -> Result<Option<(PathBuf, ImageInfo)>, Error> {
    by_id(&image_id(recipe), cache)
}

/// The image with the id `id` in `cache`, or `None` when there is none.
pub fn by_id(id: &str, cache: &Path) -> Result<Option<(PathBuf, ImageInfo)>, Error> {
    let (file, json) = paths(cache, id);
    let text = match std::fs::read(&json) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(Error::Io { path: json, source }),
    };
    let info: ImageInfo = serde_json::from_slice(&text).map_err(|err| Error::Io {
        path: json.clone(),
        source: err.into(),
    })?;
    if info.format != IMAGE_FORMAT || !file.is_file() {
        return Ok(None);
    }
    Ok(Some((file, info)))
}

/// What `--image` names: a recipe, whose image must be built; an image id
/// (`sha256:<hex>`) in the cache; else an image file, used as it is.
pub fn resolve(name: &str, cache: &Path) -> Result<(PathBuf, Option<ImageInfo>), Error> {
    if let Some(recipe) = recipe(name) {
        return match built(recipe, cache)? {
            Some((file, info)) => Ok((file, Some(info))),
            None => Err(Error::NotBuilt(recipe.name.to_owned())),
        };
    }
    if name.starts_with("sha256:") {
        return match by_id(name, cache)? {
            Some((file, info)) => Ok((file, Some(info))),
            None => Err(Error::Recipe(format!(
                "no image {name} in {}",
                cache.display()
            ))),
        };
    }
    Ok((PathBuf::from(name), None))
}

/// What [`build`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Built {
    /// The image was in the cache already; nothing was built.
    AlreadyThere,
    /// The image was built, packed, probed and put into the cache.
    Now,
}

/// Builds the image of `recipe` into `cache`, unless it is there already,
/// and returns its file, what its JSON says and whether it was built now.
/// Needs Docker, `mksquashfs` and, for the
/// probe run, what `launch` holds. Docker keeps its own copy of the image,
/// tagged `kai-image:<first 16 hex digits of the id>`.
pub async fn build(
    recipe: &Recipe,
    cache: &Path,
    launch: &Launch,
) -> Result<(PathBuf, ImageInfo, Built), Error> {
    check_recipe(recipe.text)
        .map_err(|why| Error::Recipe(format!("recipe {}: {why}", recipe.name)))?;
    if let Some((file, info)) = built(recipe, cache)? {
        return Ok((file, info, Built::AlreadyThere));
    }
    let id = image_id(recipe);
    let hex = id.trim_start_matches("sha256:").to_owned();
    let tag = format!("kai-image:{}", &hex[..16]);
    let dir = cache.join("images");
    std::fs::create_dir_all(&dir).map_err(|source| Error::Io {
        path: dir.clone(),
        source,
    })?;
    let (file, json) = paths(cache, &id);
    let log = dir.join(format!("{hex}.build.log"));

    docker_build(recipe, &tag, &log).await?;
    let (docker_image, config) = inspect(&tag).await?;
    let part = Part::beside(&file);
    export(&tag, part.path()).await?;
    let file_hash = hash_file(part.path().to_path_buf()).await?;

    let info = ImageInfo {
        format: IMAGE_FORMAT.to_owned(),
        recipe: recipe.name.to_owned(),
        id: id.clone(),
        file: format!("sha256:{file_hash}"),
        docker_image,
        env: config.env.unwrap_or_default(),
        working_dir: config.working_dir.filter(|dir| !dir.is_empty()),
        user: config.user.filter(|user| !user.is_empty()),
    };
    probe(part.path(), &info, &dir, &hex, launch).await?;

    part.keep().map_err(|source| Error::Io {
        path: file.clone(),
        source,
    })?;
    let mut text = serde_json::to_vec_pretty(&info).map_err(|err| Error::Io {
        path: json.clone(),
        source: err.into(),
    })?;
    text.push(b'\n');
    write_whole(&json, &text).map_err(|source| Error::Io { path: json, source })?;
    Ok((file, info, Built::Now))
}

/// `docker build` of the recipe, fed on standard input with no context;
/// Docker's output goes to `log`.
async fn docker_build(recipe: &Recipe, tag: &str, log: &Path) -> Result<(), Error> {
    let log_file = std::fs::File::create(log).map_err(|source| Error::Io {
        path: log.to_path_buf(),
        source,
    })?;
    let mut child = Command::new("docker")
        .args(["build", "--progress", "plain", "--tag", tag, "-"])
        .stdin(Stdio::piped())
        .stdout(log_file.try_clone().map_err(|source| Error::Io {
            path: log.to_path_buf(),
            source,
        })?)
        .stderr(log_file)
        .spawn()
        .map_err(|err| docker_missing("build", &err))?;
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt as _;
        stdin
            .write_all(recipe.text.as_bytes())
            .await
            .map_err(|err| Error::Docker {
                step: "build",
                message: err.to_string(),
            })?;
    }
    let status = child.wait().await.map_err(|err| Error::Docker {
        step: "build",
        message: err.to_string(),
    })?;
    if !status.success() {
        let text = std::fs::read_to_string(log).unwrap_or_default();
        let tail: Vec<&str> = text.lines().rev().take(5).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(Error::Docker {
            step: "build",
            message: format!("{status}; {}: {}", log.display(), tail.join(" | ")),
        });
    }
    Ok(())
}

/// The part of Docker's image configuration a run uses.
#[derive(Deserialize)]
struct Config {
    #[serde(rename = "Env")]
    env: Option<Vec<String>>,
    #[serde(rename = "WorkingDir")]
    working_dir: Option<String>,
    #[serde(rename = "User")]
    user: Option<String>,
}

/// Docker's id of the image tagged `tag`, and its configuration.
async fn inspect(tag: &str) -> Result<(String, Config), Error> {
    let output = docker(
        "inspect",
        &[
            "image",
            "inspect",
            "--format",
            "{{.Id}} {{json .Config}}",
            tag,
        ],
    )
    .await?;
    let (docker_id, config) = output.trim().split_once(' ').ok_or_else(|| Error::Docker {
        step: "inspect",
        message: format!("unexpected answer: {output}"),
    })?;
    let config: Config = serde_json::from_str(config).map_err(|err| Error::Docker {
        step: "inspect",
        message: err.to_string(),
    })?;
    Ok((docker_id.to_owned(), config))
}

/// The image's file system, through a container that never runs:
/// `docker export | mksquashfs - <out> -tar`, which takes owners and modes
/// from the tar headers and so needs no root.
async fn export(tag: &str, out: &Path) -> Result<(), Error> {
    let container = docker("create", &["create", tag]).await?.trim().to_owned();
    let packed = pack(&container, out).await;
    let _ = docker("rm", &["rm", &container]).await;
    packed
}

async fn pack(container: &str, out: &Path) -> Result<(), Error> {
    let mut export = Command::new("docker")
        .args(["export", container])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| docker_missing("export", &err))?;
    let stream: Stdio = export
        .stdout
        .take()
        .ok_or_else(|| Error::Docker {
            step: "export",
            message: "no output".to_owned(),
        })?
        .try_into()
        .map_err(|err: std::io::Error| Error::Docker {
            step: "export",
            message: err.to_string(),
        })?;
    let squash = Command::new("mksquashfs")
        .arg("-")
        .arg(out)
        .args([
            "-tar",
            "-comp",
            "zstd",
            "-quiet",
            "-no-progress",
            "-noappend",
        ])
        .stdin(stream)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    let (exported, squashed) = tokio::join!(export.wait_with_output(), squash);
    let squashed = squashed.map_err(|err| Error::Tool {
        program: "mksquashfs",
        message: format!("cannot run it: {err}"),
    })?;
    let exported = exported.map_err(|err| Error::Docker {
        step: "export",
        message: err.to_string(),
    })?;
    if !exported.status.success() {
        return Err(Error::Docker {
            step: "export",
            message: String::from_utf8_lossy(&exported.stderr).trim().to_owned(),
        });
    }
    if !squashed.status.success() {
        return Err(Error::Tool {
            program: "mksquashfs",
            message: String::from_utf8_lossy(&squashed.stderr).trim().to_owned(),
        });
    }
    Ok(())
}

/// `sh -c true` in a VM on the new image, before it is kept. Its run
/// directory is removed when it passed and kept when it did not.
async fn probe(
    file: &Path,
    info: &ImageInfo,
    dir: &Path,
    hex: &str,
    launch: &Launch,
) -> Result<(), Error> {
    let run_dir = dir.join(format!(".probe-{}-{}", &hex[..16], std::process::id()));
    let _ = std::fs::remove_dir_all(&run_dir);
    let mut spec = RunSpec::new(
        file.to_path_buf(),
        vec!["sh".to_owned(), "-c".to_owned(), "true".to_owned()],
        run_dir.clone(),
    );
    spec.origin = Some(info.origin());
    spec.env = info.environment();
    spec.cpus = 1;
    spec.memory_mib = 512;
    spec.timeout_s = 60;
    spec.disk_mib = Some(256);
    let result = crate::run(launch, &spec).await?;
    if result.outcome == Outcome::Exited && result.exit_code == Some(0) {
        let _ = std::fs::remove_dir_all(&run_dir);
        return Ok(());
    }
    let outcome = match result.exit_code {
        Some(code) => format!("{} {code}", result.outcome.name()),
        None => result.outcome.name().to_owned(),
    };
    Err(Error::Probe { outcome, run_dir })
}

/// `docker <args>`'s standard output, or what went wrong.
async fn docker(step: &'static str, args: &[&str]) -> Result<String, Error> {
    let mut command = Command::new("docker");
    command.args(args).stdin(Stdio::null());
    match helpers::process::run(&mut command).await {
        Ok(stdout) => Ok(String::from_utf8_lossy(&stdout).into_owned()),
        Err(Failed::Start { source, .. }) => Err(docker_missing(step, &source)),
        Err(Failed::Status { stderr, .. }) => Err(Error::Docker {
            step,
            message: stderr,
        }),
    }
}

fn docker_missing(step: &'static str, err: &std::io::Error) -> Error {
    let message = if err.kind() == std::io::ErrorKind::NotFound {
        "Docker is not installed or not on PATH; only `kai vm image build` needs it".to_owned()
    } else {
        format!("cannot run docker: {err}")
    };
    Error::Docker { step, message }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every recipe kai ships keeps the rules.
    #[test]
    fn every_recipe_keeps_the_rules() {
        for recipe in RECIPES {
            assert_eq!(check_recipe(recipe.text), Ok(()), "{}", recipe.name);
        }
    }

    #[test]
    fn a_from_without_a_digest_is_refused() {
        let digest = "a".repeat(64);
        assert!(check_recipe(&format!("FROM rust:1@sha256:{digest}\nRUN true\n")).is_ok());
        assert!(check_recipe("FROM rust:1-slim-bookworm\n").is_err());
        assert!(
            check_recipe(&format!(
                "FROM --platform=linux/amd64 rust@sha256:{digest}\n"
            ))
            .is_ok()
        );
        assert!(check_recipe(&format!("FROM rust@sha256:{}\n", "A".repeat(64))).is_err());
        assert!(check_recipe(&format!("FROM rust@sha256:{digest} AS base\nFROM base\n")).is_ok());
        assert!(check_recipe(&format!("FROM rust@sha256:{digest} AS base\nFROM other\n")).is_err());
        assert!(check_recipe("RUN true\n").is_err());
    }

    #[test]
    fn packages_come_from_a_dated_snapshot_and_downloads_are_checked() {
        let from = format!("FROM debian@sha256:{}\n", "b".repeat(64));
        let apt = "RUN apt-get update && apt-get install -y git\n";
        assert!(check_recipe(&format!("{from}{apt}")).is_err());
        let snapshot = "RUN echo 'deb http://snapshot.debian.org/archive/debian/20261112T000000Z bookworm main' > /etc/apt/sources.list\n";
        assert!(check_recipe(&format!("{from}{snapshot}{apt}")).is_ok());
        assert!(
            check_recipe(&format!("{from}RUN curl -fsSLo /x https://example.com/x\n")).is_err()
        );
        assert!(
            check_recipe(&format!(
                "{from}RUN curl -fsSLo /x https://example.com/x \\\n && echo 'abc  /x' | sha256sum -c -\n"
            ))
            .is_ok()
        );
    }

    #[test]
    fn the_id_follows_the_recipes_text() {
        let a = Recipe {
            name: "a",
            text: "FROM x\n",
        };
        let b = Recipe {
            name: "a",
            text: "FROM y\n",
        };
        assert_eq!(image_id(&a), image_id(&a));
        assert_ne!(image_id(&a), image_id(&b));
        assert!(image_id(&a).starts_with("sha256:"));
    }

    #[test]
    fn the_images_environment_keeps_its_path() {
        let info = ImageInfo {
            format: IMAGE_FORMAT.to_owned(),
            recipe: "rust".to_owned(),
            id: "sha256:1".to_owned(),
            file: "sha256:2".to_owned(),
            docker_image: "sha256:3".to_owned(),
            env: vec![
                "PATH=/usr/local/cargo/bin:/usr/bin".to_owned(),
                "CARGO_HOME=/usr/local/cargo".to_owned(),
            ],
            working_dir: None,
            user: None,
        };
        let env = info.environment();
        assert_eq!(env["PATH"], "/usr/local/cargo/bin:/usr/bin");
        assert_eq!(env["CARGO_HOME"], "/usr/local/cargo");
    }
}
