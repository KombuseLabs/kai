//! An egress from the runner's side, with the stand-in for `fence proxy`
//! (`src/bin/fence-stand-in.rs`) behind it: what fence is started with,
//! what the launcher and the guest get, connections allowed and refused and
//! counted from fence's ledger, and every way fence can misbehave turned
//! into an error. No network: the guest's tools are played by clients on
//! the run's socket, and the stand-in connects nowhere.

use std::collections::BTreeMap;
use std::fs;
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::FileTypeExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use proxy::{Egress, Error, Policy, Summary};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::UnixStream;

/// The name Firecracker connects to for vsock port 3128.
const SOCKET: &str = "v.sock_3128";

/// An empty directory of the test's own, standing in for a run's.
fn run_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("egress-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A policy with the stand-in as fence and short times.
fn policy(net: &[&str]) -> Policy {
    Policy {
        fence: env!("CARGO_BIN_EXE_fence-stand-in").into(),
        ready_timeout: Duration::from_secs(5),
        close_grace: Duration::from_secs(5),
        ..Policy::new(net.iter().copied())
    }
}

/// Makes the stand-in misbehave in `dir` (see its module documentation).
fn misbehave(dir: &Path, mode: &str) {
    fs::write(dir.join("stand-in.mode"), mode).unwrap();
}

/// Connects as Firecracker does, through a path short enough for a socket
/// address whatever the directory's length.
async fn connect(socket: &Path) -> UnixStream {
    let dir = fs::File::open(socket.parent().unwrap()).unwrap();
    let name = socket.file_name().unwrap().to_str().unwrap();
    UnixStream::connect(format!("/proc/self/fd/{}/{name}", dir.as_raw_fd()))
        .await
        .unwrap()
}

/// Sends a `CONNECT` and returns the status line of the answer.
async fn ask(egress: &Egress, target: &str) -> String {
    let mut client = connect(egress.port().1).await;
    let request = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n");
    client.write_all(request.as_bytes()).await.unwrap();
    let mut answer = Vec::new();
    client.read_to_end(&mut answer).await.unwrap();
    let answer = String::from_utf8(answer).unwrap();
    answer.lines().next().unwrap_or_default().to_owned()
}

#[tokio::test]
async fn fence_is_started_as_agreed_and_given_no_more_than_it_needs() {
    let dir = run_dir("args");
    let mut policy = policy(&["index.crates.io:443", "static.crates.io:443"]);
    policy.learn = true;
    let egress = Egress::open(&dir, SOCKET, &policy).await.unwrap();

    let args = fs::read_to_string(dir.join("stand-in.args")).unwrap();
    assert_eq!(
        args.lines().collect::<Vec<_>>(),
        [
            "proxy",
            "--listen=unix:v.sock_3128",
            "--net=index.crates.io:443",
            "--net=static.crates.io:443",
            "--ledger=proxy.jsonl",
            "--learn",
        ]
    );
    // Cargo gives this test many variables; fence gets two at most.
    let env = fs::read_to_string(dir.join("stand-in.env")).unwrap();
    for name in env.lines() {
        assert!(name == "PATH" || name == "HOME", "fence was given {name}");
    }
    egress.close().await.unwrap();
}

#[tokio::test]
async fn the_launcher_and_the_guest_get_what_they_need() {
    let dir = run_dir("open");
    let egress = Egress::open(&dir, SOCKET, &policy(&[])).await.unwrap();

    let (port, socket) = egress.port();
    assert_eq!(port, 3128);
    assert_eq!(socket, dir.join(SOCKET));
    assert!(
        fs::symlink_metadata(socket)
            .unwrap()
            .file_type()
            .is_socket()
    );
    assert_eq!(egress.ledger(), dir.join("proxy.jsonl"));

    let env: BTreeMap<_, _> = egress.guest_env().into_iter().collect();
    for name in [
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "ALL_PROXY",
        "https_proxy",
        "http_proxy",
        "all_proxy",
    ] {
        assert_eq!(env[name], "http://127.0.0.1:3128", "{name}");
    }
    assert_eq!(env["NO_PROXY"], "localhost,127.0.0.1,::1");
    assert_eq!(env["no_proxy"], "localhost,127.0.0.1,::1");

    assert_eq!(egress.close().await.unwrap(), Summary::default());
    assert!(!dir.join(SOCKET).exists(), "fence removed its socket");
}

#[tokio::test]
async fn connections_are_decided_by_fence_and_counted_from_its_ledger() {
    let dir = run_dir("count");
    let egress = Egress::open(&dir, SOCKET, &policy(&["index.crates.io:443"]))
        .await
        .unwrap();

    assert_eq!(
        ask(&egress, "index.crates.io:443").await,
        "HTTP/1.1 200 Connection Established"
    );
    assert_eq!(
        ask(&egress, "example.com:443").await,
        "HTTP/1.1 403 Forbidden"
    );
    assert_eq!(
        ask(&egress, "example.com:443").await,
        "HTTP/1.1 403 Forbidden"
    );
    assert_eq!(ask(&egress, "[::1]:8080").await, "HTTP/1.1 403 Forbidden");

    let summary = egress.close().await.unwrap();
    assert_eq!(summary.allowed, 1);
    assert_eq!(summary.refused, 3);
    assert_eq!(
        summary.refused_targets,
        BTreeMap::from([
            ("[::1]:8080".to_owned(), 1),
            ("example.com:443".to_owned(), 2)
        ])
    );
    assert_eq!(summary.would_deny, 0);
    let ledger = fs::read_to_string(dir.join("proxy.jsonl")).unwrap();
    assert_eq!(ledger.lines().count(), 4);
}

#[tokio::test]
async fn learn_mode_lets_through_and_counts_what_a_policy_would_need() {
    let dir = run_dir("learn");
    let mut policy = policy(&["index.crates.io:443"]);
    policy.learn = true;
    let egress = Egress::open(&dir, SOCKET, &policy).await.unwrap();

    assert_eq!(
        ask(&egress, "static.crates.io:443").await,
        "HTTP/1.1 200 Connection Established"
    );
    let summary = egress.close().await.unwrap();
    assert_eq!(
        (summary.allowed, summary.refused, summary.would_deny),
        (0, 0, 1)
    );
    assert_eq!(
        summary.would_deny_targets,
        BTreeMap::from([("static.crates.io:443".to_owned(), 1)])
    );
}

#[tokio::test]
async fn a_deep_run_directory_works() {
    let dir = run_dir(&"deep-".repeat(30));
    assert!(dir.join(SOCKET).as_os_str().len() >= 108);
    let egress = Egress::open(&dir, SOCKET, &policy(&[])).await.unwrap();
    assert_eq!(
        ask(&egress, "example.com:443").await,
        "HTTP/1.1 403 Forbidden"
    );
    assert_eq!(egress.close().await.unwrap().refused, 1);
}

#[tokio::test]
async fn a_command_line_fence_refuses_is_an_error_with_its_words() {
    let dir = run_dir("refuse");
    misbehave(&dir, "refuse");
    let refused = Egress::open(&dir, SOCKET, &policy(&[])).await.unwrap_err();
    let Error::Refused { how, stderr } = &refused else {
        panic!("{refused}");
    };
    assert_eq!(how, "exit status: 2");
    assert_eq!(stderr, "fence: --net nonsense is not host:port");
    assert!(!dir.join(SOCKET).exists());
}

#[tokio::test]
async fn a_fence_that_never_listens_is_killed() {
    let dir = run_dir("silent");
    misbehave(&dir, "silent");
    let mut policy = policy(&[]);
    policy.ready_timeout = Duration::from_millis(300);
    let refused = Egress::open(&dir, SOCKET, &policy).await.unwrap_err();
    assert!(
        matches!(refused, Error::NotReady { after, .. } if after == Duration::from_millis(300)),
        "{refused}"
    );
}

#[tokio::test]
async fn a_first_line_that_is_not_the_ready_line_is_an_error() {
    let dir = run_dir("chatty");
    misbehave(&dir, "chatty");
    let refused = Egress::open(&dir, SOCKET, &policy(&[])).await.unwrap_err();
    assert!(
        matches!(&refused, Error::UnexpectedLine { line } if line == "hello"),
        "{refused}"
    );
}

#[tokio::test]
async fn a_fence_that_does_not_end_is_killed_and_reported() {
    let dir = run_dir("stubborn");
    misbehave(&dir, "stubborn");
    let mut policy = policy(&[]);
    policy.close_grace = Duration::from_millis(300);
    let egress = Egress::open(&dir, SOCKET, &policy).await.unwrap();
    let failed = egress.close().await.unwrap_err();
    assert!(
        matches!(&failed, Error::Ended { how, .. } if how.contains("killed")),
        "{failed}"
    );
}

#[tokio::test]
async fn a_fence_that_ends_badly_is_reported_with_its_words() {
    let dir = run_dir("failing");
    misbehave(&dir, "failing");
    let egress = Egress::open(&dir, SOCKET, &policy(&[])).await.unwrap();
    let failed = egress.close().await.unwrap_err();
    assert!(
        matches!(&failed, Error::Ended { how, stderr }
            if how == "exit status: 3" && stderr == "fence: failing on purpose"),
        "{failed}"
    );
}

#[tokio::test]
async fn a_ledger_kai_cannot_read_is_an_error_not_a_zero() {
    let dir = run_dir("bad-ledger");
    misbehave(&dir, "bad-ledger");
    let egress = Egress::open(&dir, SOCKET, &policy(&[])).await.unwrap();
    let failed = egress.close().await.unwrap_err();
    assert!(
        matches!(&failed, Error::Ledger { line: 1, reason, .. } if reason.starts_with("not JSON")),
        "{failed}"
    );
}

#[tokio::test]
async fn dropping_the_egress_ends_fence() {
    let dir = run_dir("drop");
    let egress = Egress::open(&dir, SOCKET, &policy(&[])).await.unwrap();
    let socket = egress.port().1.to_owned();
    drop(egress);
    // fence is killed, not asked: its socket stays, and nobody listens.
    let short = |dir: &fs::File| format!("/proc/self/fd/{}/{SOCKET}", dir.as_raw_fd());
    let handle = fs::File::open(&dir).unwrap();
    for _ in 0..50 {
        if UnixStream::connect(short(&handle)).await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("something still listens on {}", socket.display());
}

#[tokio::test]
async fn a_missing_fence_names_the_doctor() {
    let dir = run_dir("missing-fence");
    let mut policy = policy(&[]);
    policy.fence = "kai-no-such-fence-program".into();
    let refused = Egress::open(&dir, SOCKET, &policy).await.unwrap_err();
    assert!(matches!(refused, Error::FenceMissing { .. }), "{refused}");
    assert!(refused.to_string().contains("kai vm doctor"), "{refused}");
}

#[tokio::test]
async fn files_in_the_way_are_left_alone_and_fence_is_not_started() {
    for name in [SOCKET, "proxy.jsonl"] {
        let dir = run_dir(&format!("taken-{name}"));
        fs::write(dir.join(name), "someone else's").unwrap();
        let refused = Egress::open(&dir, SOCKET, &policy(&[])).await.unwrap_err();
        assert!(
            matches!(&refused, Error::Exists { path } if path == &dir.join(name)),
            "{refused}"
        );
        assert_eq!(
            fs::read_to_string(dir.join(name)).unwrap(),
            "someone else's"
        );
        assert!(!dir.join("stand-in.args").exists(), "fence was started");
    }
}

#[tokio::test]
async fn a_missing_directory_is_an_error() {
    let dir = run_dir("missing").join("not-there");
    let refused = Egress::open(&dir, SOCKET, &policy(&[])).await.unwrap_err();
    assert!(matches!(refused, Error::Io { .. }), "{refused}");
}

#[tokio::test]
async fn the_socket_name_is_a_plain_file_name() {
    let dir = run_dir("names");
    for name in [
        "",
        ".",
        "..",
        "proxy.jsonl",
        "sub/v.sock_3128",
        "../v.sock_3128",
        "v\0sock",
    ] {
        let refused = Egress::open(&dir, name, &policy(&[])).await.unwrap_err();
        assert!(
            matches!(refused, Error::BadName { .. }),
            "{name:?}: {refused}"
        );
    }
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 0, "nothing made");
}
