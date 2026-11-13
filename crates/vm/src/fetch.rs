//! `kai vm fetch`: a pinned artefact into the cache, checked.
//!
//! A download goes to a temporary file beside its target and is hashed as
//! it arrives. Only a file whose SHA-256 is the pinned one is renamed into
//! place; any failure removes what was written, so the cache never holds a
//! file that is not the pinned one under the pinned name.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use helpers::fs::Part;
use helpers::{hash, http, process};
use tokio::io::AsyncWriteExt as _;
use tokio::process::Command;

use crate::{Artefact, Error};

/// Sent with every download.
const USER_AGENT: &str = concat!("kai/", env!("CARGO_PKG_VERSION"));

/// What [`fetch()`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fetched {
    /// The file was there already, with the pinned hash; nothing was
    /// downloaded.
    AlreadyThere,
    /// The file was downloaded, checked and put in place.
    Downloaded,
}

/// Puts `artefact` into the directory `cache`, which is made if it is
/// missing. A file that is there with the pinned hash is kept as it is; one
/// with another hash is replaced, but only by a download that matches.
pub async fn fetch(artefact: &Artefact, cache: &Path) -> Result<Fetched, Error> {
    let target = cache.join(artefact.file);
    if target.exists() {
        let found = hash_file(target.clone()).await?;
        if found == artefact.sha256 {
            return Ok(Fetched::AlreadyThere);
        }
        tracing::warn!(file = %target.display(), sha256 = %found, "not the pinned file; downloading it again");
    }
    if !http::allowed_url(artefact.url) {
        return Err(Error::Url(artefact.url.to_owned()));
    }
    std::fs::create_dir_all(cache).map_err(|source| Error::Io {
        path: cache.to_path_buf(),
        source,
    })?;

    let download = Part::beside(&target);
    tracing::info!(url = artefact.url, "downloading");
    let actual = download_to(artefact.url, download.path()).await?;
    if actual != artefact.download_sha256 {
        return Err(Error::Checksum {
            what: artefact.url.to_owned(),
            expected: artefact.download_sha256,
            actual,
        });
    }

    let file = match artefact.member {
        None => download,
        Some(member) => {
            let part = Part::beside(&target);
            unpack(download.path(), member, part.path()).await?;
            drop(download);
            let actual = hash_file(part.path().to_path_buf()).await?;
            if actual != artefact.sha256 {
                return Err(Error::Checksum {
                    what: member.to_owned(),
                    expected: artefact.sha256,
                    actual,
                });
            }
            part
        }
    };
    let mode = if artefact.executable { 0o755 } else { 0o644 };
    std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(mode)).map_err(
        |source| Error::Io {
            path: file.path().to_path_buf(),
            source,
        },
    )?;
    file.keep().map_err(|source| Error::Io {
        path: target.clone(),
        source,
    })?;
    tracing::info!(file = %target.display(), "fetched and checked");
    Ok(Fetched::Downloaded)
}

/// The SHA-256 of the file at `path`, off the runtime's threads.
pub(crate) async fn hash_file(path: PathBuf) -> Result<String, Error> {
    hash::sha256_file_async(path.clone())
        .await
        .map_err(|source| Error::Io { path, source })
}

/// Downloads `url` into the file `path` and returns the SHA-256 of what
/// arrived. Redirects are followed: a release on GitHub is served from
/// another host. The hash, not the transport, is what makes the bytes
/// trusted, and no credential is sent.
async fn download_to(url: &str, path: &Path) -> Result<String, Error> {
    let failed = |err: reqwest::Error| Error::Download {
        url: url.to_owned(),
        message: http::error_chain(&err),
    };
    let client = http::builder(USER_AGENT)
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .build()
        .map_err(failed)?;
    let mut response = client.get(url).send().await.map_err(failed)?;
    if !response.status().is_success() {
        return Err(Error::Status {
            url: url.to_owned(),
            status: response.status().as_u16(),
        });
    }
    let io_error = |source| Error::Io {
        path: path.to_path_buf(),
        source,
    };
    let mut file = tokio::fs::File::create(path).await.map_err(io_error)?;
    let mut hash = hash::Sha256::new();
    while let Some(chunk) = response.chunk().await.map_err(failed)? {
        hash.update(&chunk);
        file.write_all(&chunk).await.map_err(io_error)?;
    }
    file.sync_all().await.map_err(io_error)?;
    Ok(hash.finish())
}

/// Writes the member `member` of the gzipped tar archive `archive` to the
/// file `out`, with the system's `tar`. The archive was checked against its
/// pin before.
async fn unpack(archive: &Path, member: &str, out: &Path) -> Result<(), Error> {
    let mut command = Command::new("tar");
    command
        .arg("-xzOf")
        .arg(archive)
        .arg(member)
        .stdin(Stdio::null());
    // Failed reads "cannot run tar: …" or "tar exit status: 2: …".
    let bytes = process::run(&mut command)
        .await
        .map_err(|failed| Error::Unpack {
            member: member.to_owned(),
            message: failed.to_string(),
        })?;
    tokio::fs::write(out, bytes)
        .await
        .map_err(|source| Error::Io {
            path: out.to_path_buf(),
            source,
        })
}
