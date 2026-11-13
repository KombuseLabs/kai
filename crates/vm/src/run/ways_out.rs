//! The ways out of a guest that has no network device. A guest's
//! connection to the host's vsock port N reaches the socket `v.sock_N` in
//! the run directory. Behind each port the run names, a relay passes the
//! connection on to a socket or address of the host; behind the proxy's
//! port stands the egress. All are open before the VM starts and closed
//! after it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{PortTarget, ProxyRecord, RunSpec};

/// Firecracker's vsock socket in the run directory.
pub(super) const VSOCK: &str = "v.sock";

/// The socket Firecracker connects to for guest port `port`.
fn socket_name(port: u16) -> String {
    format!("{VSOCK}_{port}")
}

/// The ways out of a run, open: the command's environment and the ports
/// the init listens on, and the relays and the egress behind them.
pub(super) struct WaysOut {
    /// The command's environment, the proxy's variables included.
    pub(super) env: BTreeMap<String, String>,
    /// The guest ports the init forwards to vsock, the proxy's included.
    pub(super) ports: Vec<u16>,
    relays: tokio::task::JoinSet<()>,
    egress: Option<proxy::Egress>,
    learn: bool,
}

/// Opens a relay for each of `spec`'s ports and, when it has a proxy
/// policy, the egress. `Err` says what could not be opened; the relays
/// opened before it end with it.
pub(super) async fn open(dir: &Path, spec: &RunSpec) -> Result<WaysOut, String> {
    let mut env = spec.env.clone();
    let mut ports: Vec<u16> = spec.ports.iter().map(|port| port.guest).collect();
    let mut relays = tokio::task::JoinSet::new();
    for port in &spec.ports {
        let listener = listen_in(dir, &socket_name(port.guest))
            .map_err(|err| format!("port {}: cannot listen: {err}", port.guest))?;
        relays.spawn(relay(listener, port.to.clone()));
    }
    let mut egress = None;
    if let Some(policy) = &spec.proxy {
        let opened = proxy::Egress::open(dir, &socket_name(proxy::GUEST_PORT), policy)
            .await
            .map_err(|err| format!("cannot open the egress: {err}"))?;
        env.extend(
            opened
                .guest_env()
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value)),
        );
        ports.push(proxy::GUEST_PORT);
        egress = Some(opened);
    }
    Ok(WaysOut {
        env,
        ports,
        relays,
        egress,
        learn: spec.proxy.as_ref().is_some_and(|policy| policy.learn),
    })
}

impl WaysOut {
    /// After the VM: the relays stop, and the egress ends and its ledger is
    /// counted. An egress that does not close cleanly leaves no record,
    /// never one of zeros.
    pub(super) async fn close(mut self) -> Option<ProxyRecord> {
        self.relays.abort_all();
        match self.egress?.close().await {
            Ok(summary) => Some(ProxyRecord {
                allowed: summary.allowed,
                refused: summary.refused,
                refused_targets: summary.refused_targets,
                would_deny: self.learn.then_some(summary.would_deny),
                would_deny_targets: summary.would_deny_targets,
            }),
            Err(err) => {
                tracing::warn!(%err, "the egress did not close cleanly");
                None
            }
        }
    }
}

/// A listener on the socket `name` in `dir`; through `/proc/self/fd` when
/// the path is too long for a socket address.
fn listen_in(dir: &Path, name: &str) -> std::io::Result<tokio::net::UnixListener> {
    let path = dir.join(name);
    let listener = if path.as_os_str().len() < 100 {
        std::os::unix::net::UnixListener::bind(&path)?
    } else {
        use std::os::fd::AsRawFd as _;
        let handle = std::fs::File::open(dir)?;
        std::os::unix::net::UnixListener::bind(format!(
            "/proc/self/fd/{}/{name}",
            handle.as_raw_fd()
        ))?
    };
    listener.set_nonblocking(true)?;
    tokio::net::UnixListener::from_std(listener)
}

/// Passes each connection of the guest's on to `to`, both ways, until the
/// relay is aborted after the VM.
async fn relay(listener: tokio::net::UnixListener, to: PortTarget) {
    while let Ok((mut guest, _)) = listener.accept().await {
        let to = to.clone();
        tokio::spawn(async move {
            let copied = match &to {
                PortTarget::Unix(path) => match connect_unix(path).await {
                    Ok(mut host) => tokio::io::copy_bidirectional(&mut guest, &mut host).await,
                    Err(err) => Err(err),
                },
                PortTarget::Tcp(address) => match tokio::net::TcpStream::connect(address).await {
                    Ok(mut host) => tokio::io::copy_bidirectional(&mut guest, &mut host).await,
                    Err(err) => Err(err),
                },
            };
            if let Err(err) = copied {
                tracing::debug!(%err, ?to, "a port's connection ended with an error");
            }
        });
    }
}

/// A stream to the Unix socket at `path`; through `/proc/self/fd` when the
/// path is too long for a socket address.
async fn connect_unix(path: &Path) -> std::io::Result<tokio::net::UnixStream> {
    if path.as_os_str().len() < 100 {
        return tokio::net::UnixStream::connect(path).await;
    }
    use std::os::fd::AsRawFd as _;
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(std::io::Error::other("no socket file named"));
    };
    let handle = std::fs::File::open(parent)?;
    let short = Path::new("/proc/self/fd")
        .join(handle.as_raw_fd().to_string())
        .join(name);
    tokio::net::UnixStream::connect(short).await
}

/// Removes the run's sockets when dropped, also when a run is interrupted.
pub(super) struct Sockets(pub(super) PathBuf);

impl Drop for Sockets {
    fn drop(&mut self) {
        remove_sockets(&self.0);
    }
}

/// The run's sockets, Firecracker's and the relays', once the VM is gone.
pub(super) fn remove_sockets(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if (name == VSOCK || name.starts_with(&format!("{VSOCK}_")))
            && entry.file_type().is_ok_and(|kind| {
                use std::os::unix::fs::FileTypeExt as _;
                kind.is_socket()
            })
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
