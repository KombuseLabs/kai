//! A private, process-locked handoff from prxy to a separate kai terminal.
use super::{Context, Failure};
use crate::settings;
use agent::session::Route;
use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Connection {
    endpoint: String,
    placeholder: String,
}

fn from_env(mock: bool) -> anyhow::Result<Route> {
    // Do not let clap or serialization errors echo environment values.
    let endpoint = std::env::var("ANTHROPIC_BASE_URL")
        .map_err(|_| anyhow::anyhow!("ANTHROPIC_BASE_URL must name a loopback model route"))?;
    let key = std::env::var("ANTHROPIC_API_KEY").ok();
    Ok(Route::new(&endpoint, key.as_deref(), mock)?)
}

fn private(file: &File) -> anyhow::Result<()> {
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file()
            && metadata.mode() & 0o077 == 0
            && metadata.uid() == std::fs::metadata("/proc/self")?.uid(),
        "proxy connection must be a private file owned by the current user"
    );
    Ok(())
}

pub(super) fn route(context: &Context<'_>, mock: bool) -> anyhow::Result<Route> {
    // Never mix a handoff's token with an explicitly supplied endpoint.
    // An unrelated API key from .env must neither select the environment
    // route nor be mixed with the published endpoint.
    if mock || std::env::var_os("ANTHROPIC_BASE_URL").is_some() {
        return from_env(mock);
    }
    let path = settings::proxy_connection(context.config);
    read_connection(&path)
}

fn read_connection(path: &Path) -> anyhow::Result<Route> {
    let mut file = File::open(path)
        .context("proxy is not ready; start ./tools/start-proxy in another terminal")?;
    private(&file)?;
    match file.try_lock() {
        Err(TryLockError::WouldBlock) => {}
        Ok(()) => anyhow::bail!("proxy connection is stale; start ./tools/start-proxy"),
        Err(TryLockError::Error(err)) => return Err(err.into()),
    }
    let mut bytes = Vec::new();
    (&mut file).take(4097).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 4096, "invalid proxy connection file");
    let connection: Connection = serde_json::from_slice(&bytes).map_err(|_| {
        anyhow::anyhow!("invalid proxy connection file; restart ./tools/start-proxy")
    })?;
    Ok(Route::new(
        &connection.endpoint,
        Some(&connection.placeholder),
        false,
    )?)
}

struct Published {
    path: PathBuf,
    // Keep the lock until after removal, even when the command fails.
    file: File,
}

impl Drop for Published {
    fn drop(&mut self) {
        // Do not remove a replacement created by somebody else.
        if let (Ok(current), Ok(ours)) = (std::fs::metadata(&self.path), self.file.metadata())
            && current.ino() == ours.ino()
            && current.dev() == ours.dev()
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn write_connection(path: &Path, connection: &Connection) -> anyhow::Result<Published> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    private(&file)?;
    file.try_lock().map_err(|_| {
        anyhow::anyhow!("a proxy is already registered; use it or stop it with Ctrl-C")
    })?;
    file.set_len(0)?;
    let published = Published {
        path: path.to_owned(),
        file,
    };
    serde_json::to_writer(&published.file, connection)?;
    (&published.file).flush()?;
    Ok(published)
}

pub(super) fn publish(context: &Context<'_>) -> Result<(), Failure> {
    from_env(false).map_err(Failure::Usage)?;
    let connection = Connection {
        endpoint: std::env::var("ANTHROPIC_BASE_URL").expect("validated environment"),
        placeholder: std::env::var("ANTHROPIC_API_KEY").expect("validated environment"),
    };
    context.runtime.block_on(async {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let _published =
            write_connection(&settings::proxy_connection(context.config), &connection)?;
        println!("Proxy ready. In another terminal: cargo run -- run ponytail");
        println!("Keep this terminal open; Ctrl-C stops the proxy.");
        tokio::select! {
            result = tokio::signal::ctrl_c() => result?,
            _ = terminate.recv() => {},
        }
        Ok::<(), anyhow::Error>(())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    fn connection() -> Connection {
        Connection {
            endpoint: "http://127.0.0.1:1234".into(),
            placeholder: format!("prxy-{}", "a".repeat(32)),
        }
    }

    #[test]
    fn handoff_is_private_exclusive_and_removed_on_exit() {
        let dir = helpers::testing::TestDir::new("proxy-handoff");
        let path = dir.join("proxy.json");
        let publisher = write_connection(&path, &connection()).unwrap();
        assert!(read_connection(&path).is_ok());
        let original = std::fs::read(&path).unwrap();
        assert!(write_connection(&path, &connection()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_connection(&path).is_err());
        drop(publisher);
        assert!(!path.exists());
    }

    #[test]
    fn stale_or_invalid_handoffs_are_refused_without_echoing_credentials() {
        let dir = helpers::testing::TestDir::new("proxy-stale");
        let path = dir.join("proxy.json");
        let mut invalid = connection();
        invalid.placeholder = "invented-provider-secret".into();
        let publisher = write_connection(&path, &invalid).unwrap();
        let error = read_connection(&path).err().unwrap().to_string();
        assert!(!error.contains(&invalid.placeholder));
        assert!(error.contains("placeholder"));
        // A killed publisher leaves its file but the OS releases its lock.
        publisher.file.unlock().unwrap();
        assert!(
            read_connection(&path)
                .err()
                .unwrap()
                .to_string()
                .contains("stale")
        );
        drop(publisher);
    }
}
