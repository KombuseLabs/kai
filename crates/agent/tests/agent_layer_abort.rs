//! `agent::layer::fetch`, killed while it runs, then fetched again.
//! A child process (this test program, run again for the one test
//! `child_fetch_until_killed`) fetches an invented agent B into a cache
//! that holds a valid entry of another, A, and is stopped at a known point:
//!
//! - in the middle of the download: a local server sends half the program
//!   and holds the connection;
//! - in the middle of packing: `mksquashfs` is a script that runs the real
//!   one, cuts the layer in half, as a kill would leave it, and waits;
//! - at a [`Step`]: the child's callback says where it is and waits.
//!
//! The test waits until the child is there, kills its whole process group
//! with SIGKILL, so that nothing of it runs on, and checks: every lock is
//! free, nothing incomplete is an entry, A's entry is the same file and is
//! taken as it is, and a fetch of B recovers. No network: the servers are
//! on this machine. They need squashfs-tools, so they are ignored unless
//! asked for: `cargo test -p agent --test agent_layer_abort -- --ignored`.

use std::io::{Read as _, Write as _};
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use agent::Agent;
use agent::layer::{AGENTS_DIR, Fetched, INFO_FILE, LAYER_FILE, Step, fetch, fetch_with};
use helpers::hash::sha256_bytes;
use helpers::testing::TestDir;
use serde::{Deserialize, Serialize};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// What the child fetches, and where it stops.
#[derive(Serialize, Deserialize)]
struct ChildJob {
    cache: PathBuf,
    url: String,
    version: String,
    sha: String,
    length: u64,
    /// A [`Step`] by its name, or none: then something else holds it.
    stop: Option<String>,
    /// Written when the child is at `stop`.
    marker: PathBuf,
}

/// The variable that names the child's job; without it the child does
/// nothing.
const CHILD_JOB: &str = "KAI_AGENT_LAYER_CHILD_JOB";

/// A fresh directory of the test's own.
fn dir(test: &str) -> TestDir {
    TestDir::new(&format!("agent-abort-{test}"))
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// An invented program; `seed` makes another.
fn program(seed: u32) -> Vec<u8> {
    let mut state = 0x2545_f491_u32 ^ seed;
    (0..300_000)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state.to_le_bytes()[0]
        })
        .collect()
}

/// The invented agent at `version`, pinned to the program with `sha` and
/// `length`, downloaded from `url`.
fn agent(url: &str, version: &str, sha: &str, length: u64) -> Agent {
    let sha = leak(sha.to_owned());
    Agent {
        name: "invented",
        version: leak(version.to_owned()),
        binary: vm::Artefact {
            name: "Invented",
            file: leak(format!("invented-{version}")),
            url: leak(url.to_owned()),
            download_sha256: sha,
            member: None,
            sha256: sha,
            executable: true,
        },
        length,
        path: leak(format!("/opt/kai/agents/invented/{version}/invented")),
    }
}

/// The program as the layer holds it.
fn program_in(layer: &Path, version: &str) -> Vec<u8> {
    let out = std::process::Command::new("unsquashfs")
        .arg("-cat")
        .arg(layer)
        .arg(format!("opt/kai/agents/invented/{version}/invented"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    out.stdout
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Waits until `done` holds: the child writes what it checks once it is
/// at its point, so how long this takes changes nothing but the time.
async fn until(what: &str, done: impl Fn() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "never: {what}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The child: fetches what its job names and stops where it says. Run
/// alone, without the variable, it does nothing.
#[tokio::test]
#[ignore = "the child of the tests below; run by them"]
async fn child_fetch_until_killed() {
    let Some(job) = std::env::var_os(CHILD_JOB) else {
        return;
    };
    let job: ChildJob = serde_json::from_slice(&std::fs::read(job).unwrap()).unwrap();
    let agent = agent(&job.url, &job.version, &job.sha, job.length);
    let stop = job.stop.clone();
    let marker = job.marker.clone();
    let _ = fetch_with(&agent, &job.cache, move |step| {
        if stop.as_deref() == Some(format!("{step:?}").as_str()) {
            std::fs::write(&marker, b"").unwrap();
            // Here until killed.
            loop {
                std::thread::park();
            }
        }
    })
    .await;
    // Ended by itself: the test that started it fails, as no marker came.
}

/// A child and the process group it leads; dropped, the group is killed,
/// so that a test that fails leaves nothing running.
struct Group(Option<std::process::Child>);

impl Group {
    /// SIGKILL to the child and everything it started, then reaped.
    fn kill(&mut self) {
        let Some(mut child) = self.0.take() else {
            return;
        };
        let killed = kill_group(child.id());
        let ended = child.wait().unwrap();
        assert!(killed, "kill failed");
        assert_eq!(
            std::os::unix::process::ExitStatusExt::signal(&ended),
            Some(9)
        );
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            kill_group(child.id());
            let _ = child.wait();
        }
    }
}

/// SIGKILL to the process group `id`. bash's kill takes a group; dash's
/// does not.
fn kill_group(id: u32) -> bool {
    std::process::Command::new("bash")
        .args(["-c", &format!("kill -KILL -- -{id}")])
        .status()
        .is_ok_and(|status| status.success())
}

/// A child fetching `job`, in a process group of its own, with `path`
/// before the `PATH` it inherits.
fn start_child(work: &Path, job: &ChildJob, path: Option<&Path>) -> Group {
    let file = work.join("job.json");
    std::fs::write(&file, serde_json::to_vec(job).unwrap()).unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "child_fetch_until_killed",
            "--ignored",
            "--test-threads=1",
        ])
        .env(CHILD_JOB, &file)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0);
    if let Some(path) = path {
        let inherited = std::env::var("PATH").unwrap_or_default();
        command.env("PATH", format!("{}:{inherited}", path.display()));
    }
    Group(Some(command.spawn().unwrap()))
}

/// Every lock file of the cache can be taken at once: no dead fetch holds
/// one.
fn locks_are_free(cache: &Path) {
    let agents = cache.join(AGENTS_DIR);
    for dir in [agents.clone(), agents.join("downloads")] {
        for name in names(&dir) {
            if name.ends_with(".lock") {
                let file = std::fs::File::open(dir.join(&name)).unwrap();
                file.try_lock()
                    .unwrap_or_else(|err| panic!("{name} is still held: {err:?}"));
            }
        }
    }
}

/// A local server that answers the first request with the headers and the
/// first half of `body`, then holds the connection until it is dropped.
struct Stalling {
    url: String,
    _hold: std::sync::mpsc::Sender<()>,
}

fn stalling(body: Vec<u8>) -> Stalling {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/b", listener.local_addr().unwrap());
    let (hold, held) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8];
        while !head.ends_with(b"\r\n\r\n") {
            if stream.read(&mut byte).unwrap() == 0 {
                return;
            }
            head.push(byte[0]);
        }
        let half = body.len() / 2;
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(&body[..half]);
        let _ = stream.flush();
        // Holds the connection until the test drops its sender.
        let _ = held.recv();
    });
    Stalling { url, _hold: hold }
}

/// Where a case stops the child.
#[derive(Clone, Copy, Debug)]
enum Stop {
    /// Half the program has arrived.
    MidDownload,
    /// `mksquashfs` has written half the layer.
    MidPacking,
    /// The fetch reached this step.
    At(Step),
}

/// One case: A built, B's fetch killed at `stop`, the cache checked, A
/// taken again and B fetched again; returns B's recovering fetch.
async fn interrupted(test: &str, stop: Stop) -> Fetched {
    let work = dir(test);
    let cache = work.join("cache");
    let (body_a, body_b) = (program(1), program(2));
    let server = MockServer::start().await;
    for (route, body) in [("/a", &body_a), ("/b", &body_b)] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
            .mount(&server)
            .await;
    }
    let sha_a = sha256_bytes(&body_a);
    let sha_b = sha256_bytes(&body_b);
    let a = agent(
        &format!("{}/a", server.uri()),
        "1.0",
        &sha_a,
        body_a.len() as u64,
    );
    let b = agent(
        &format!("{}/b", server.uri()),
        "2.0",
        &sha_b,
        body_b.len() as u64,
    );
    let kept = fetch(&a, &cache).await.unwrap();
    let kept_inode = std::fs::metadata(&kept.file).unwrap().ino();
    let agents = cache.join(AGENTS_DIR);
    let downloads = agents.join("downloads");

    let marker = work.join("marker");
    let mut job = ChildJob {
        cache: cache.clone(),
        url: format!("{}/b", server.uri()),
        version: "2.0".to_owned(),
        sha: sha_b.clone(),
        length: body_b.len() as u64,
        stop: None,
        marker: marker.clone(),
    };
    let mut stall = None;
    let mut bin = None;
    match stop {
        Stop::MidDownload => {
            let server = stalling(body_b.clone());
            job.url = server.url.clone();
            stall = Some(server);
        }
        Stop::MidPacking => {
            let real = std::process::Command::new("sh")
                .args(["-c", "command -v mksquashfs"])
                .output()
                .unwrap();
            let real = String::from_utf8(real.stdout).unwrap().trim().to_owned();
            assert!(!real.is_empty(), "no mksquashfs");
            let dir = work.join("bin");
            std::fs::create_dir(&dir).unwrap();
            let script = format!(
                "#!/bin/sh\n\
                 # mksquashfs for a test: the real one packs, half the layer is\n\
                 # cut away, as a kill in the middle leaves it, and this waits.\n\
                 case \"$1\" in -version) exec {real} \"$@\";; esac\n\
                 {real} \"$@\" || exit 1\n\
                 size=$(stat -c %s \"$2\")\n\
                 truncate -s $((size / 2)) \"$2\"\n\
                 : > {marker}\n\
                 exec sleep 3600\n",
                marker = marker.display()
            );
            let wrapper = dir.join("mksquashfs");
            std::fs::write(&wrapper, script).unwrap();
            std::fs::set_permissions(
                &wrapper,
                std::os::unix::fs::PermissionsExt::from_mode(0o755),
            )
            .unwrap();
            bin = Some(dir);
        }
        Stop::At(step) => job.stop = Some(format!("{step:?}")),
    }
    if let Stop::At(Step::Locked) = stop {
        // A's own fetch, killed while it holds A's lock.
        job.url = format!("{}/a", server.uri());
        job.version = "1.0".to_owned();
        job.sha = sha_a.clone();
        job.length = body_a.len() as u64;
    }

    let mut child = start_child(&work, &job, bin.as_deref());
    match stop {
        Stop::MidDownload => {
            let half = body_b.len() as u64 / 2;
            until("half the program in a part of the download", || {
                names(&downloads).iter().any(|name| {
                    name.ends_with(".part")
                        && std::fs::metadata(downloads.join(name)).is_ok_and(|m| m.len() >= half)
                })
            })
            .await;
        }
        _ => until("the child at its point", || marker.exists()).await,
    }
    child.kill();
    drop(stall);

    // What the kill left.
    locks_are_free(&cache);
    let parts: Vec<String> = names(&agents)
        .into_iter()
        .filter(|name| name.ends_with(".part"))
        .collect();
    let entries: Vec<String> = names(&agents)
        .into_iter()
        .filter(|name| !name.starts_with('.') && name != "downloads")
        .collect();
    let kept_hex = kept.info.key.trim_start_matches("sha256:").to_owned();
    match stop {
        Stop::MidDownload => {
            assert!(names(&downloads).iter().any(|name| name.ends_with(".part")));
            assert!(parts.is_empty(), "{parts:?}");
            assert_eq!(entries, std::slice::from_ref(&kept_hex));
        }
        Stop::At(Step::Locked) => {
            assert!(parts.is_empty(), "{parts:?}");
            assert_eq!(entries, std::slice::from_ref(&kept_hex));
        }
        Stop::At(Step::Published) => {
            assert!(parts.is_empty(), "{parts:?}");
            assert_eq!(entries.len(), 2, "{entries:?}");
        }
        Stop::MidPacking | Stop::At(_) => {
            // The entry's part, never its entry.
            assert_eq!(parts.len(), 1, "{parts:?}");
            assert_eq!(entries, std::slice::from_ref(&kept_hex));
            let part = agents.join(&parts[0]);
            let has = |name: &str| part.join(name).exists();
            match stop {
                Stop::At(Step::Staged) => assert!(!has(LAYER_FILE) && !has(INFO_FILE)),
                Stop::MidPacking | Stop::At(Step::Packed) => {
                    assert!(has(LAYER_FILE) && !has(INFO_FILE));
                }
                Stop::At(Step::Written) => assert!(has(LAYER_FILE) && has(INFO_FILE)),
                _ => unreachable!(),
            }
        }
    }

    // A is as it was, and taken as it is.
    let again = fetch(&a, &cache).await.unwrap();
    assert!(again.reused, "{again:?}");
    assert_eq!(std::fs::metadata(&again.file).unwrap().ino(), kept_inode);
    assert_eq!(program_in(&again.file, "1.0"), body_a);

    // B recovers: a whole layer of the whole program, checked.
    let recovered = fetch(&b, &cache).await.unwrap();
    assert_eq!(program_in(&recovered.file, "2.0"), body_b);
    assert_eq!(
        recovered.info.layer,
        format!(
            "sha256:{}",
            sha256_bytes(&std::fs::read(&recovered.file).unwrap())
        )
    );
    assert!(
        names(&agents).iter().all(|name| !name.ends_with(".part")),
        "{:?}",
        names(&agents)
    );
    assert!(
        names(&downloads)
            .iter()
            .all(|name| !name.ends_with(".part")),
        "{:?}",
        names(&downloads)
    );
    locks_are_free(&cache);
    recovered
}

#[tokio::test]
#[ignore = "needs squashfs-tools: cargo test -p agent --test agent_layer_abort -- --ignored"]
async fn killed_in_the_middle_of_the_download() {
    let recovered = interrupted("download", Stop::MidDownload).await;
    assert!(recovered.downloaded && !recovered.reused, "{recovered:?}");
    assert_eq!(recovered.removed, 1, "the part of the download");
}

#[tokio::test]
#[ignore = "needs squashfs-tools: cargo test -p agent --test agent_layer_abort -- --ignored"]
async fn killed_while_holding_the_lock_of_a_valid_entry() {
    let recovered = interrupted("locked", Stop::At(Step::Locked)).await;
    assert!(recovered.downloaded && !recovered.reused, "{recovered:?}");
    assert_eq!(recovered.removed, 0);
}

#[tokio::test]
#[ignore = "needs squashfs-tools: cargo test -p agent --test agent_layer_abort -- --ignored"]
async fn killed_with_the_program_on_the_stage() {
    let recovered = interrupted("staged", Stop::At(Step::Staged)).await;
    assert!(!recovered.downloaded && !recovered.reused, "{recovered:?}");
    assert_eq!(recovered.removed, 1, "the entry's part");
}

#[tokio::test]
#[ignore = "needs squashfs-tools: cargo test -p agent --test agent_layer_abort -- --ignored"]
async fn killed_in_the_middle_of_packing() {
    let recovered = interrupted("packing", Stop::MidPacking).await;
    assert!(!recovered.downloaded && !recovered.reused, "{recovered:?}");
    assert_eq!(recovered.removed, 1, "the entry's part with half a layer");
}

#[tokio::test]
#[ignore = "needs squashfs-tools: cargo test -p agent --test agent_layer_abort -- --ignored"]
async fn killed_with_the_layer_packed_and_not_checked() {
    let recovered = interrupted("packed", Stop::At(Step::Packed)).await;
    assert!(!recovered.downloaded && !recovered.reused, "{recovered:?}");
    assert_eq!(recovered.removed, 1);
}

#[tokio::test]
#[ignore = "needs squashfs-tools: cargo test -p agent --test agent_layer_abort -- --ignored"]
async fn killed_with_a_whole_entry_written_and_not_renamed() {
    // The part is complete and would check out; it is still not taken.
    let recovered = interrupted("written", Stop::At(Step::Written)).await;
    assert!(!recovered.downloaded && !recovered.reused, "{recovered:?}");
    assert_eq!(recovered.removed, 1);
}

#[tokio::test]
#[ignore = "needs squashfs-tools: cargo test -p agent --test agent_layer_abort -- --ignored"]
async fn killed_after_the_rename_before_the_flush() {
    let recovered = interrupted("published", Stop::At(Step::Published)).await;
    assert!(recovered.reused, "{recovered:?}");
    assert_eq!(recovered.removed, 0);
}
