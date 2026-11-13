//! An egress with the real `fence proxy` behind it: a tunnel to an allowed
//! host carries bytes both ways, a host no rule names is refused, and an
//! allowed name that leads to loopback is refused by fence's address check.
//! No network: the allowed host is an echo server on this machine, and the
//! refused ones are refused before any name is looked up.
//!
//! Ignored by default, since it needs fence 0.3.0 or newer. Run it with
//!
//! ```sh
//! cargo test -p proxy --test fence -- --ignored
//! ```
//!
//! with `fence` on `PATH`, or `KAI_FENCE` naming the program.

use std::collections::BTreeMap;
use std::fs;
use std::os::fd::AsRawFd as _;
use std::path::{Path, PathBuf};

use proxy::{Egress, Policy};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, UnixStream};

const SOCKET: &str = "v.sock_3128";

fn run_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("fence-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// An echo server on this machine's loopback; returns its port.
async fn echo_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut read, mut write) = stream.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
        }
    });
    port
}

/// Opens a `CONNECT` through the egress; returns the status line and the
/// stream, past the answer's head.
async fn connect(egress: &Egress, target: &str) -> (String, BufReader<UnixStream>) {
    let socket = egress.port().1;
    let dir = fs::File::open(socket.parent().unwrap()).unwrap();
    let name = socket.file_name().unwrap().to_str().unwrap();
    let mut stream = UnixStream::connect(format!("/proc/self/fd/{}/{name}", dir.as_raw_fd()))
        .await
        .unwrap();
    let request = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status).await.unwrap();
    let mut line = String::new();
    while reader.read_line(&mut line).await.unwrap() > 0 && line != "\r\n" {
        line.clear();
    }
    (status.trim_end().to_owned(), reader)
}

/// The fence program: `KAI_FENCE`, or `fence` from `PATH`.
fn real(net: Vec<String>) -> Policy {
    Policy {
        fence: std::env::var_os("KAI_FENCE").map_or_else(|| "fence".into(), PathBuf::from),
        ..Policy::new(net)
    }
}

#[tokio::test]
#[ignore = "needs fence 0.3.0 or newer: cargo test -p proxy --test fence -- --ignored"]
async fn the_real_fence_tunnels_what_the_policy_allows_and_refuses_the_rest() {
    let port = echo_server().await;
    let dir = run_dir("tunnel");
    let egress = Egress::open(&dir, SOCKET, &real(vec![format!("127.0.0.1:{port}")]))
        .await
        .unwrap();

    let (status, mut tunnel) = connect(&egress, &format!("127.0.0.1:{port}")).await;
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    tunnel
        .get_mut()
        .write_all(b"ping through fence")
        .await
        .unwrap();
    let mut echoed = [0u8; 18];
    tunnel.read_exact(&mut echoed).await.unwrap();
    assert_eq!(&echoed, b"ping through fence");
    drop(tunnel);

    let (status, _) = connect(&egress, "example.com:443").await;
    assert!(status.starts_with("HTTP/1.1 403"), "{status}");

    let summary = egress.close().await.unwrap();
    assert_eq!((summary.allowed, summary.refused), (1, 1));
    assert_eq!(
        summary.refused_targets,
        BTreeMap::from([("example.com:443".to_owned(), 1)])
    );
    assert!(!dir.join(SOCKET).exists(), "fence removed its socket");
}

#[tokio::test]
#[ignore = "needs fence 0.3.0 or newer: cargo test -p proxy --test fence -- --ignored"]
async fn the_real_fence_refuses_an_allowed_name_that_leads_to_loopback() {
    let port = echo_server().await;
    let dir = run_dir("address");
    let egress = Egress::open(&dir, SOCKET, &real(vec![format!("localhost:{port}")]))
        .await
        .unwrap();

    let (status, _) = connect(&egress, &format!("localhost:{port}")).await;
    assert!(status.starts_with("HTTP/1.1 403"), "{status}");

    let ledger = egress.ledger().to_owned();
    let summary = egress.close().await.unwrap();
    assert_eq!((summary.allowed, summary.refused), (0, 1));
    let text = fs::read_to_string(ledger).unwrap();
    assert!(text.contains("non-public address 127.0.0.1"), "{text}");
}
