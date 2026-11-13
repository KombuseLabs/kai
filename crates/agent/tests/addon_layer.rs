//! `agent::addon::fetch` with an invented add-on, a skill and a licence
//! served by a local HTTP server that stands in for the publisher's
//! repository: no network, and no byte of a real add-on. They need
//! squashfs-tools, which CI does not install, so they are ignored unless
//! asked for: `cargo test -p agent --test addon_layer -- --ignored`.

use std::os::unix::fs::MetadataExt as _;
use std::path::Path;

use agent::addon::{ADDON_FORMAT, ADDONS_DIR, Addon, AddonFile, AddonInfo, Written, cached, fetch};
use agent::layer::{Error, INFO_FILE, LAYER_FILE};
use helpers::hash::sha256_bytes;
use helpers::testing::TestDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SKILL: &[u8] = b"---\nname: invented\ndescription: An invented skill for kai's tests.\n---\n\nSay MARKER-invented-6f1d when asked.\n";
const LICENCE: &[u8] = b"Invented licence: a fixture, not a grant.\n";
const MANIFEST: &[u8] = b"{\"name\":\"invented\"}\n";
const SKILL_PATH: &str = "/opt/kai/addons/invented/skills/invented/SKILL.md";
const LICENCE_PATH: &str = "/opt/kai/addons/invented/LICENSE";
const MANIFEST_PATH: &str = "/opt/kai/addons/invented/.claude-plugin/plugin.json";

/// A fresh directory of the test's own.
fn dir(test: &str) -> TestDir {
    TestDir::new(&format!("addon-layer-{test}"))
}

/// Pins are `&'static`, as in the tracked pins.
fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// Serves the skill, whose body is `skill`, and the licence, each as many
/// times as `times` says.
async fn serve(skill: &[u8], times: [u64; 2]) -> MockServer {
    let server = MockServer::start().await;
    for ((at, body), times) in [("/skill", skill), ("/licence", LICENCE)]
        .into_iter()
        .zip(times)
    {
        Mock::given(method("GET"))
            .and(path(at))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.to_vec()))
            .expect(times)
            .mount(&server)
            .await;
    }
    server
}

/// One pinned file of the invented add-on.
fn file(
    server: &MockServer,
    at: &str,
    source: &'static str,
    guest: &'static str,
    body: &[u8],
) -> AddonFile {
    let sha = leak(sha256_bytes(body));
    AddonFile {
        source,
        path: guest,
        artefact: vm::Artefact {
            name: "an invented add-on's file",
            file: leak(format!("invented-{}", at.trim_start_matches('/'))),
            url: leak(format!("{}{at}", server.uri())),
            download_sha256: sha,
            member: None,
            sha256: sha,
            executable: false,
        },
        length: body.len() as u64,
    }
}

/// The invented add-on, pinned to `SKILL` and `LICENCE`, downloaded from
/// `server`, and a plugin's manifest kai writes.
fn addon(server: &MockServer) -> Addon {
    addon_with(vec![
        file(
            server,
            "/skill",
            "skills/invented/SKILL.md",
            SKILL_PATH,
            SKILL,
        ),
        file(server, "/licence", "LICENSE", LICENCE_PATH, LICENCE),
    ])
}

fn addon_with(files: Vec<AddonFile>) -> Addon {
    Addon {
        name: "invented",
        repository: "https://example.invalid/invented",
        commit: "0123456789abcdef0123456789abcdef01234567",
        licence: "LicenseRef-invented",
        licence_file: LICENCE_PATH,
        files: Box::leak(files.into_boxed_slice()),
        written: &[Written {
            path: MANIFEST_PATH,
            bytes: MANIFEST,
        }],
    }
}

/// `unsquashfs -lln` of the layer: mode, owner and path of each line.
fn listing(layer: &Path) -> Vec<(String, String, String)> {
    let out = std::process::Command::new("unsquashfs")
        .args(["-lln", "-UTC"])
        .arg(layer)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let words: Vec<&str> = line.split_whitespace().collect();
            (
                words[0].to_owned(),
                words[1].to_owned(),
                words[5].to_owned(),
            )
        })
        .collect()
}

/// A file as the layer holds it.
fn file_in(layer: &Path, guest: &str) -> Vec<u8> {
    let out = std::process::Command::new("unsquashfs")
        .arg("-cat")
        .arg(layer)
        .arg(guest.trim_start_matches('/'))
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    out.stdout
}

/// The entries of the add-ons' directory, hidden ones aside.
fn entries(cache: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(cache.join(ADDONS_DIR))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort();
    names
}

#[tokio::test]
#[ignore = "needs squashfs-tools: cargo test -p agent --test addon_layer -- --ignored"]
async fn an_addon_is_its_pinned_files_alone_root_s_and_read_only() {
    let server = serve(SKILL, [2, 2]).await;
    let addon = addon(&server);
    let cache = dir("made");

    let first = fetch(&addon, &cache).await.unwrap();
    assert!(!first.reused && first.downloaded, "{first:?}");
    let hex = first.info.key.trim_start_matches("sha256:");
    assert_eq!(
        first.file,
        cache.join(ADDONS_DIR).join(hex).join(LAYER_FILE)
    );
    let dirs = |path: &str| ("drwxr-xr-x".to_owned(), "0/0".to_owned(), path.to_owned());
    let mut expected = vec![
        dirs("squashfs-root"),
        dirs("squashfs-root/opt"),
        dirs("squashfs-root/opt/kai"),
        dirs("squashfs-root/opt/kai/addons"),
        dirs("squashfs-root/opt/kai/addons/invented"),
        dirs("squashfs-root/opt/kai/addons/invented/.claude-plugin"),
        dirs("squashfs-root/opt/kai/addons/invented/skills"),
        dirs("squashfs-root/opt/kai/addons/invented/skills/invented"),
    ];
    for guest in [SKILL_PATH, LICENCE_PATH, MANIFEST_PATH] {
        expected.push((
            "-rw-r--r--".to_owned(),
            "0/0".to_owned(),
            format!("squashfs-root{guest}"),
        ));
    }
    let mut found = listing(&first.file);
    found.sort();
    expected.sort();
    assert_eq!(found, expected);
    assert_eq!(file_in(&first.file, SKILL_PATH), SKILL);
    assert_eq!(file_in(&first.file, LICENCE_PATH), LICENCE);
    assert_eq!(file_in(&first.file, MANIFEST_PATH), MANIFEST);

    // The record: the pin, the two digests apart, written whole.
    let info: AddonInfo =
        serde_json::from_slice(&std::fs::read(first.file.with_file_name(INFO_FILE)).unwrap())
            .unwrap();
    assert_eq!(info, first.info);
    assert_eq!(info.format, ADDON_FORMAT);
    assert_eq!(
        (info.name.as_str(), info.dir.as_str(), info.commit.as_str()),
        ("invented", "/opt/kai/addons/invented", addon.commit)
    );
    assert_eq!(info.licence_file, LICENCE_PATH);
    assert_eq!(info.files.len(), 2);
    assert_eq!(info.files[0].source, "skills/invented/SKILL.md");
    assert_eq!(
        info.files[0].sha256,
        format!("sha256:{}", sha256_bytes(SKILL))
    );
    assert_eq!(info.written.len(), 1);
    assert_eq!(
        (info.written[0].path.as_str(), info.written[0].length),
        (MANIFEST_PATH, MANIFEST.len() as u64)
    );
    assert_eq!(
        info.written[0].sha256,
        format!("sha256:{}", sha256_bytes(MANIFEST))
    );
    assert_eq!(
        info.layer,
        format!(
            "sha256:{}",
            sha256_bytes(&std::fs::read(&first.file).unwrap())
        )
    );
    assert!(info.inventory.starts_with("sha256:"));
    assert_ne!(info.inventory, info.layer);

    // Checked and taken again, the same file; found without a fetch.
    let before = std::fs::metadata(&first.file).unwrap();
    let second = fetch(&addon, &cache).await.unwrap();
    assert!(second.reused && !second.downloaded, "{second:?}");
    assert_eq!(second.info, first.info);
    assert_eq!(std::fs::metadata(&second.file).unwrap().ino(), before.ino());
    assert_eq!(cached(&addon, &cache).await.unwrap().info, first.info);

    // Packed again elsewhere: the same bytes, the same inventory.
    let other = fetch(&addon, &dir("made-other")).await.unwrap();
    assert_eq!(
        (other.info.layer.as_str(), other.info.inventory.as_str()),
        (first.info.layer.as_str(), first.info.inventory.as_str())
    );
}

#[tokio::test]
#[ignore = "needs squashfs-tools: cargo test -p agent --test addon_layer -- --ignored"]
async fn a_file_that_is_not_the_pinned_one_publishes_nothing() {
    // Pinned to SKILL, served something else: the licence is not asked for.
    let server = serve(b"not the invented skill", [1, 0]).await;
    let addon = addon(&server);
    let cache = dir("wrong");

    let err = fetch(&addon, &cache).await.unwrap_err();
    assert!(
        matches!(err, Error::Vm(vm::Error::Checksum { .. })),
        "{err}"
    );
    assert_eq!(entries(&cache), ["downloads"]);
    let err = cached(&addon, &cache).await.unwrap_err();
    assert!(matches!(err, Error::NotFetched(_)), "{err}");
    assert!(err.to_string().contains("kai agent fetch --addon invented"));
}

#[tokio::test]
#[ignore = "needs squashfs-tools: cargo test -p agent --test addon_layer -- --ignored"]
async fn an_entry_that_does_not_check_out_is_moved_aside_and_built_without_a_download() {
    // One download of each file: every rebuild takes them from the cache.
    let server = serve(SKILL, [1, 1]).await;
    let addon = addon(&server);
    let cache = dir("spoilt");
    let first = fetch(&addon, &cache).await.unwrap();
    let entry = first.file.parent().unwrap().to_path_buf();

    type Spoil = fn(&Path);
    let spoils: [(&str, Spoil); 3] = [
        ("a changed byte", |entry| {
            use std::io::{Seek as _, SeekFrom, Write as _};
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .open(entry.join(LAYER_FILE))
                .unwrap();
            file.seek(SeekFrom::Start(100)).unwrap();
            file.write_all(&[0x5a]).unwrap();
        }),
        ("another inventory", |entry| {
            let json = entry.join(INFO_FILE);
            let mut info: AddonInfo =
                serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
            info.inventory = format!("sha256:{}", "0".repeat(64));
            std::fs::write(&json, serde_json::to_vec(&info).unwrap()).unwrap();
        }),
        ("another commit", |entry| {
            let json = entry.join(INFO_FILE);
            let mut info: AddonInfo =
                serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
            info.commit = "f".repeat(40);
            std::fs::write(&json, serde_json::to_vec(&info).unwrap()).unwrap();
        }),
    ];
    for (what, spoil) in spoils {
        spoil(&entry);
        assert!(cached(&addon, &cache).await.is_err(), "{what}");
        let again = fetch(&addon, &cache).await.unwrap();
        assert!(!again.reused && !again.downloaded, "{what}: {again:?}");
        let (aside, why) = again.quarantined.clone().expect(what);
        assert!(aside.starts_with(cache.join(ADDONS_DIR)), "{what}");
        assert!(!why.is_empty(), "{what}");
        assert_eq!(again.info, first.info, "{what}");
        assert_eq!(file_in(&again.file, SKILL_PATH), SKILL, "{what}");
    }
}

#[tokio::test]
#[ignore = "needs squashfs-tools: cargo test -p agent --test addon_layer -- --ignored"]
async fn a_pin_outside_its_directory_or_without_its_licence_downloads_nothing() {
    let server = serve(SKILL, [0, 0]).await;
    let skill = file(
        &server,
        "/skill",
        "skills/invented/SKILL.md",
        SKILL_PATH,
        SKILL,
    );
    let licence = file(&server, "/licence", "LICENSE", LICENCE_PATH, LICENCE);
    let elsewhere = AddonFile {
        path: "/opt/kai/agents/invented/SKILL.md",
        ..skill
    };
    let cache = dir("refused");
    let outside = fetch(&addon_with(vec![elsewhere, licence]), &cache)
        .await
        .unwrap_err();
    assert!(matches!(outside, Error::Path { .. }), "{outside}");
    let unlicensed = fetch(&addon_with(vec![skill]), &cache).await.unwrap_err();
    assert!(matches!(unlicensed, Error::Pin(_)), "{unlicensed}");
    assert!(!cache.join(ADDONS_DIR).exists());
}
