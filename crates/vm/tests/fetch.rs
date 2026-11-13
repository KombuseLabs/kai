//! `vm::fetch` against a local HTTP server that stands in for GitHub's
//! releases and the kernel's bucket: no network. The artefacts are
//! invented; their pins are computed here.

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use helpers::hash::sha256_file;
use helpers::testing::TestDir;
use vm::{Artefact, Error, Fetched, fetch};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A fresh directory of the test's own.
fn dir(test: &str) -> TestDir {
    TestDir::new(&format!("vm-fetch-{test}"))
}

/// Pins are `&'static str`, as in the tracked pins; a test's live as long
/// as the test.
fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// Through a file of its own, since the tests run at once.
fn sha256(bytes: &[u8]) -> String {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("kai-vm-fetch-{}-hash-{n}", std::process::id()));
    std::fs::write(&file, bytes).unwrap();
    let hash = sha256_file(&file).unwrap();
    std::fs::remove_file(&file).unwrap();
    hash
}

/// What is in `dir`, hidden files included.
fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// Serves `body` at `/<name>` exactly `times` times.
async fn serve(name: &str, body: Vec<u8>, times: u64) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/{name}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .expect(times)
        .mount(&server)
        .await;
    server
}

/// A file served as it is, as the kernel is.
fn plain(server: &MockServer, body: &[u8], pin: &str) -> Artefact {
    Artefact {
        name: "invented kernel",
        file: "vmlinux-invented",
        url: leak(format!("{}/vmlinux-invented", server.uri())),
        download_sha256: leak(pin.to_owned()),
        member: None,
        sha256: leak(sha256(body)),
        executable: false,
    }
}

/// An invented release: `release/tool` in a gzipped tar archive.
fn archive(test: &str) -> (Vec<u8>, Vec<u8>) {
    let work = dir(&format!("{test}-archive"));
    std::fs::create_dir(work.join("release")).unwrap();
    let program = b"#!/bin/sh\necho invented\n".to_vec();
    std::fs::write(work.join("release/tool"), &program).unwrap();
    let status = Command::new("tar")
        .args(["-czf", "release.tgz", "release"])
        .current_dir(&work)
        .status()
        .unwrap();
    assert!(status.success());
    let archive = std::fs::read(work.join("release.tgz")).unwrap();
    std::fs::remove_dir_all(&work).unwrap();
    (archive, program)
}

#[tokio::test]
async fn a_plain_file_is_downloaded_checked_and_kept() {
    let body = b"an invented kernel".to_vec();
    let server = serve("vmlinux-invented", body.clone(), 1).await;
    let artefact = plain(&server, &body, &sha256(&body));
    let cache = dir("plain").join("cache");

    assert_eq!(fetch(&artefact, &cache).await.unwrap(), Fetched::Downloaded);
    assert_eq!(std::fs::read(cache.join("vmlinux-invented")).unwrap(), body);
    assert_eq!(mode(&cache.join("vmlinux-invented")), 0o644);
    assert_eq!(listing(&cache), ["vmlinux-invented"]);

    // The second time it is there: no request (the server expects one).
    assert_eq!(
        fetch(&artefact, &cache).await.unwrap(),
        Fetched::AlreadyThere
    );
}

#[tokio::test]
async fn a_download_with_another_hash_is_refused_and_leaves_nothing() {
    let body = b"an invented kernel".to_vec();
    let server = serve("vmlinux-invented", body.clone(), 1).await;
    let artefact = plain(&server, &body, &"0".repeat(64));
    let cache = dir("wrong-hash");

    let err = fetch(&artefact, &cache).await.unwrap_err();
    assert!(matches!(err, Error::Checksum { .. }), "{err}");
    assert!(err.to_string().contains("nothing was kept"), "{err}");
    assert!(listing(&cache).is_empty(), "{:?}", listing(&cache));
}

#[tokio::test]
async fn a_file_that_is_not_the_pinned_one_is_replaced() {
    let body = b"an invented kernel".to_vec();
    let server = serve("vmlinux-invented", body.clone(), 1).await;
    let artefact = plain(&server, &body, &sha256(&body));
    let cache = dir("replaced");
    std::fs::write(cache.join("vmlinux-invented"), b"something else").unwrap();

    assert_eq!(fetch(&artefact, &cache).await.unwrap(), Fetched::Downloaded);
    assert_eq!(std::fs::read(cache.join("vmlinux-invented")).unwrap(), body);
}

#[tokio::test]
async fn a_member_of_an_archive_is_unpacked_checked_and_made_executable() {
    let (archive, program) = archive("member");
    let server = serve("release.tgz", archive.clone(), 1).await;
    let artefact = Artefact {
        name: "invented tool",
        file: "tool-1.0",
        url: leak(format!("{}/release.tgz", server.uri())),
        download_sha256: leak(sha256(&archive)),
        member: Some("release/tool"),
        sha256: leak(sha256(&program)),
        executable: true,
    };
    let cache = dir("member");

    assert_eq!(fetch(&artefact, &cache).await.unwrap(), Fetched::Downloaded);
    assert_eq!(std::fs::read(cache.join("tool-1.0")).unwrap(), program);
    assert_eq!(mode(&cache.join("tool-1.0")), 0o755);
    assert_eq!(listing(&cache), ["tool-1.0"]);
}

#[tokio::test]
async fn a_member_with_another_hash_is_refused_and_leaves_nothing() {
    let (archive, _) = archive("member-hash");
    let server = serve("release.tgz", archive.clone(), 1).await;
    let artefact = Artefact {
        name: "invented tool",
        file: "tool-1.0",
        url: leak(format!("{}/release.tgz", server.uri())),
        download_sha256: leak(sha256(&archive)),
        member: Some("release/tool"),
        sha256: leak("0".repeat(64)),
        executable: true,
    };
    let cache = dir("member-hash");

    let err = fetch(&artefact, &cache).await.unwrap_err();
    assert!(
        matches!(err, Error::Checksum { ref what, .. } if what == "release/tool"),
        "{err}"
    );
    assert!(listing(&cache).is_empty(), "{:?}", listing(&cache));
}

#[tokio::test]
async fn a_member_the_archive_lacks_is_an_unpack_error_and_leaves_nothing() {
    let (archive, program) = archive("no-member");
    let server = serve("release.tgz", archive.clone(), 1).await;
    let artefact = Artefact {
        name: "invented tool",
        file: "tool-1.0",
        url: leak(format!("{}/release.tgz", server.uri())),
        download_sha256: leak(sha256(&archive)),
        member: Some("release/other"),
        sha256: leak(sha256(&program)),
        executable: true,
    };
    let cache = dir("no-member");

    let err = fetch(&artefact, &cache).await.unwrap_err();
    assert!(matches!(err, Error::Unpack { .. }), "{err}");
    assert!(listing(&cache).is_empty(), "{:?}", listing(&cache));
}

#[tokio::test]
async fn an_http_error_is_reported_and_leaves_nothing() {
    let server = MockServer::start().await;
    let body = b"never served".to_vec();
    let artefact = plain(&server, &body, &sha256(&body));
    let cache = dir("missing");

    let err = fetch(&artefact, &cache).await.unwrap_err();
    assert!(matches!(err, Error::Status { status: 404, .. }), "{err}");
    assert!(listing(&cache).is_empty(), "{:?}", listing(&cache));
}

#[tokio::test]
async fn plain_http_to_another_machine_is_refused_before_a_request() {
    let artefact = Artefact {
        name: "invented kernel",
        file: "vmlinux-invented",
        url: "http://example.com/vmlinux-invented",
        download_sha256: "0000000000000000000000000000000000000000000000000000000000000000",
        member: None,
        sha256: "0000000000000000000000000000000000000000000000000000000000000000",
        executable: false,
    };
    let cache = dir("plain-http").join("cache");

    let err = fetch(&artefact, &cache).await.unwrap_err();
    assert!(matches!(err, Error::Url(_)), "{err}");
    assert!(!cache.exists());
}
