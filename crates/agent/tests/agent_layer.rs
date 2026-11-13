//! `agent::layer::fetch` with an invented agent, served by a local
//! HTTP server that stands in for the release bucket: no network. They
//! need squashfs-tools, which CI does not install, so they are ignored
//! unless asked for:
//! `cargo test -p agent --test agent_layer -- --ignored`.

use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use agent::Agent;
use agent::layer::{AGENTS_DIR, Error, INFO_FILE, LAYER_FILE, LayerInfo, fetch};
use helpers::hash::sha256_bytes;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A fresh directory of the test's own.
fn dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kai-agent-layer-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Pins are `&'static str`, as in the tracked pins.
fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// An invented program: bytes that do not compress to nothing.
fn program() -> Vec<u8> {
    let mut state = 0x2545_f491_u32;
    (0..300_000)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state.to_le_bytes()[0]
        })
        .collect()
}

/// Serves `body` at `/invented` exactly `times` times.
async fn serve(body: Vec<u8>, times: u64) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/invented"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .expect(times)
        .mount(&server)
        .await;
    server
}

/// The invented agent, pinned to `body`, downloaded from `server`.
fn agent(server: &MockServer, body: &[u8]) -> Agent {
    let sha = leak(sha256_bytes(body));
    Agent {
        name: "invented",
        version: "1.0",
        binary: vm::Artefact {
            name: "Invented 1.0",
            file: "invented-1.0",
            url: leak(format!("{}/invented", server.uri())),
            download_sha256: sha,
            member: None,
            sha256: sha,
            executable: true,
        },
        length: body.len() as u64,
        path: "/opt/kai/agents/invented/1.0/invented",
    }
}

/// The hidden names in `dir`: locks, build directories, quarantines,
/// parts.
fn hidden(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with('.'))
        .collect();
    names.sort();
    names
}

/// The program as the layer holds it.
fn program_in(layer: &Path) -> Vec<u8> {
    let out = std::process::Command::new("unsquashfs")
        .arg("-cat")
        .arg(layer)
        .arg("opt/kai/agents/invented/1.0/invented")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    out.stdout
}

#[tokio::test]
#[ignore = "needs squashfs-tools: cargo test -p agent --test agent_layer -- --ignored"]
async fn a_fetch_builds_once_and_the_next_reuses_after_checking() {
    let body = program();
    let server = serve(body.clone(), 2).await;
    let agent = agent(&server, &body);
    let cache = dir("reuse");

    let first = fetch(&agent, &cache).await.unwrap();
    assert!(!first.reused && first.downloaded, "{first:?}");
    assert_eq!((first.quarantined.clone(), first.removed), (None, 0));
    let hex = first.info.key.trim_start_matches("sha256:");
    assert_eq!(
        first.file,
        cache.join(AGENTS_DIR).join(hex).join(LAYER_FILE)
    );
    assert_eq!(program_in(&first.file), body);
    let info: LayerInfo =
        serde_json::from_slice(&std::fs::read(first.file.with_file_name(INFO_FILE)).unwrap())
            .unwrap();
    assert_eq!(info, first.info);
    assert_eq!(info.format, "kai-agent-layer/1");
    assert_eq!(info.binary, format!("sha256:{}", sha256_bytes(&body)));
    assert_eq!(info.binary_len, body.len() as u64);
    assert_eq!(
        info.layer,
        format!(
            "sha256:{}",
            sha256_bytes(&std::fs::read(&first.file).unwrap())
        )
    );
    let before = std::fs::metadata(&first.file).unwrap();

    let second = fetch(&agent, &cache).await.unwrap();
    assert!(second.reused && !second.downloaded, "{second:?}");
    assert_eq!(
        (second.file.clone(), second.info.clone()),
        (first.file.clone(), info)
    );
    let after = std::fs::metadata(&second.file).unwrap();
    assert_eq!((after.ino(), after.mtime()), (before.ino(), before.mtime()));
    // Nothing but the two locks is left beside the entry.
    assert_eq!(hidden(&cache.join(AGENTS_DIR)), [format!(".{hex}.lock")]);
    assert_eq!(
        hidden(&cache.join(AGENTS_DIR).join("downloads")),
        [".invented-1.0.lock"]
    );

    // Two packs of the same program are the same bytes.
    let other = fetch(&agent, &dir("reuse-other")).await.unwrap();
    assert_eq!(other.info.layer, second.info.layer);
}

#[tokio::test]
#[ignore = "needs squashfs-tools: cargo test -p agent --test agent_layer -- --ignored"]
async fn a_download_that_is_not_the_pinned_one_leaves_no_entry() {
    let body = program();
    let server = serve(b"not the program".to_vec(), 1).await;
    let agent = agent(&server, &body);
    let cache = dir("wrong");

    let err = fetch(&agent, &cache).await.unwrap_err();
    assert!(
        matches!(err, Error::Vm(vm::Error::Checksum { .. })),
        "{err}"
    );
    let agents = cache.join(AGENTS_DIR);
    let entries: Vec<String> = std::fs::read_dir(&agents)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.'))
        .collect();
    assert_eq!(entries, ["downloads"]);
    assert!(hidden(&agents).iter().all(|name| name.ends_with(".lock")));
    assert_eq!(
        std::fs::read_dir(agents.join("downloads"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        [".invented-1.0.lock"]
    );
}

#[tokio::test]
#[ignore = "needs squashfs-tools: cargo test -p agent --test agent_layer -- --ignored"]
async fn an_entry_that_does_not_check_out_is_moved_aside_and_built_again() {
    let body = program();
    // One download: every rebuild takes the program from the cache.
    let server = serve(body.clone(), 1).await;
    let agent = agent(&server, &body);
    let cache = dir("invalid");
    let first = fetch(&agent, &cache).await.unwrap();
    let agents = cache.join(AGENTS_DIR);

    type Spoil = fn(&Path);
    let spoils: [(&str, Spoil); 5] = [
        ("truncated JSON", |entry| {
            let json = entry.join(INFO_FILE);
            let text = std::fs::read(&json).unwrap();
            std::fs::write(&json, &text[..text.len() / 2]).unwrap();
        }),
        ("no JSON", |entry| {
            std::fs::remove_file(entry.join(INFO_FILE)).unwrap()
        }),
        ("another packing", |entry| {
            let json = entry.join(INFO_FILE);
            let text = String::from_utf8(std::fs::read(&json).unwrap()).unwrap();
            std::fs::write(&json, text.replace("\"-all-root\"", "\"-all-root-not\"")).unwrap();
        }),
        ("a changed byte", |entry| {
            use std::io::{Seek as _, SeekFrom, Write as _};
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .open(entry.join(LAYER_FILE))
                .unwrap();
            file.seek(SeekFrom::Start(2000)).unwrap();
            file.write_all(&[0x5a]).unwrap();
        }),
        ("no layer", |entry| {
            std::fs::remove_file(entry.join(LAYER_FILE)).unwrap()
        }),
    ];
    for (what, spoil) in spoils {
        let entry = first.file.parent().unwrap().to_path_buf();
        spoil(&entry);
        // A VM that has the layer open, as Firecracker has.
        let reader = std::fs::File::open(entry.join(LAYER_FILE)).ok();
        let read_before = reader
            .as_ref()
            .map(|_| std::fs::read(entry.join(LAYER_FILE)).unwrap());

        let again = fetch(&agent, &cache).await.unwrap();
        assert!(!again.reused && !again.downloaded, "{what}: {again:?}");
        let (aside, why) = again.quarantined.clone().expect(what);
        assert!(!why.is_empty(), "{what}");
        assert!(aside.is_dir() && aside.starts_with(&agents), "{what}");
        assert_eq!(again.file, first.file, "{what}");
        assert_eq!(program_in(&again.file), body, "{what}");
        if let (Some(reader), Some(before)) = (reader, read_before) {
            // The moved entry was not written: the reader reads on what it
            // read, through the same file.
            let mut read_now = Vec::new();
            std::io::Read::read_to_end(&mut &reader, &mut read_now).unwrap();
            assert_eq!(read_now, before, "{what}");
            let aside_meta = std::fs::metadata(aside.join(LAYER_FILE)).unwrap();
            assert_eq!(reader.metadata().unwrap().ino(), aside_meta.ino(), "{what}");
            assert_ne!(
                std::fs::metadata(&again.file).unwrap().ino(),
                aside_meta.ino(),
                "{what}"
            );
        }
        let checked = fetch(&agent, &cache).await.unwrap();
        assert!(checked.reused, "{what}: {checked:?}");
    }
    let quarantined = hidden(&agents)
        .into_iter()
        .filter(|name| name.contains(".quarantine-"))
        .count();
    assert_eq!(quarantined, 5);
}

#[tokio::test]
#[ignore = "needs squashfs-tools: cargo test -p agent --test agent_layer -- --ignored"]
async fn what_a_killed_fetch_left_is_removed_and_the_fetch_completes() {
    let body = program();
    let server = serve(body.clone(), 2).await;
    let agent = agent(&server, &body);
    // The key, from a fetch elsewhere.
    let key = fetch(&agent, &dir("killed-key")).await.unwrap().info.key;
    let hex = key.trim_start_matches("sha256:");

    let cache = dir("killed");
    let agents = cache.join(AGENTS_DIR);
    // Killed while packing: the entry's part, a directory with half a
    // layer.
    let build = agents.join(format!(".{hex}.999999-0.part"));
    std::fs::create_dir_all(build.join("stage/opt")).unwrap();
    std::fs::write(build.join(LAYER_FILE), b"half a layer").unwrap();
    // Killed while downloading: a part of the program.
    std::fs::create_dir_all(agents.join("downloads")).unwrap();
    std::fs::write(
        agents.join("downloads/.invented-1.0.999999-0.part"),
        &body[..1000],
    )
    .unwrap();

    let fetched = fetch(&agent, &cache).await.unwrap();
    assert!(fetched.downloaded && !fetched.reused, "{fetched:?}");
    assert_eq!((fetched.removed, fetched.quarantined.clone()), (2, None));
    assert!(!build.exists());
    assert_eq!(program_in(&fetched.file), body);
    assert_eq!(hidden(&agents), [format!(".{hex}.lock")]);
}

#[tokio::test]
#[ignore = "needs squashfs-tools: cargo test -p agent --test agent_layer -- --ignored"]
async fn two_fetches_at_once_download_and_build_once() {
    let body = program();
    let server = serve(body.clone(), 1).await;
    let agent = agent(&server, &body);
    let cache = dir("together");

    let (one, two) = tokio::join!(fetch(&agent, &cache), fetch(&agent, &cache));
    let (one, two) = (one.unwrap(), two.unwrap());
    assert_eq!(one.file, two.file);
    assert_eq!(one.info, two.info);
    let built = [&one, &two]
        .iter()
        .filter(|fetched| !fetched.reused)
        .count();
    assert_eq!(built, 1, "{one:?} {two:?}");
    assert_eq!(program_in(&one.file), body);
}
