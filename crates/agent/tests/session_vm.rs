//! Opt-in tests with the pinned SDK in a real VM and a local fake model only.
//! Set KAI_AGENT_TEST_CACHE and KAI_AGENT_TEST_MIMIC, then run this test with
//! --ignored --test-threads=1. These tests never fetch artifacts or call a model.
use agent::session::{Options, Route, Session};
use helpers::fs::PartDir;
use helpers::lines::BoundedLines;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Child, Command};

struct Fixture {
    root: PartDir,
    model: Child,
    url: String,
    cache: PathBuf,
}
impl Fixture {
    async fn new(stall_first: bool) -> Self {
        let cache = std::env::var_os("KAI_AGENT_TEST_CACHE")
            .expect("explicit test cache")
            .into();
        let binary = std::env::var_os("KAI_AGENT_TEST_MIMIC").expect("explicit local mimic binary");
        let root = PartDir::beside(&std::env::temp_dir().join("kai-session-vm-test")).unwrap();
        // Interactive server: keep its child and pipe rather than using run(),
        // which waits for completion and therefore cannot serve the VM.
        let mut command = Command::new(binary);
        command
            .env_clear()
            .args(["--text", "fixture reply", "--ledger"])
            .arg(root.path().join("model.jsonl"));
        if stall_first {
            command.args(["--answer", "stall:60s"]);
        } else {
            command.args(["--answer", "ok", "--answer", "stall:60s", "--answer", "ok"]);
        }
        let mut model = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut lines = BoundedLines::new(model.stdout.take().unwrap(), 1024);
        let bytes = tokio::time::timeout(Duration::from_secs(5), lines.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let url = String::from_utf8(bytes).unwrap().trim().to_owned();
        Self {
            root,
            model,
            url,
            cache,
        }
    }
    fn run_dir(&self) -> PathBuf {
        self.root.path().join("run")
    }
    async fn session(&self, timeout: u64) -> Session {
        let route = Route::new(&self.url, Some("synthetic-inherited-key-canary"), true).unwrap();
        let mut options = Options::new("test-model".into(), route, self.run_dir());
        options.timeout_s = timeout;
        options.tools.clear();
        Session::start(&self.cache, options).await.unwrap()
    }
    async fn stop(mut self) {
        self.model.kill().await.unwrap();
        self.model.wait().await.unwrap();
    }
}
async fn first_chunk(session: &mut Session) {
    loop {
        let event = session.next_event().await.unwrap();
        if event.payload["event"]["type"] == "message_start" {
            break;
        }
    }
}
fn record(dir: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(dir.join("agent.json")).unwrap()).unwrap()
}

#[tokio::test]
#[ignore = "requires KVM, a fetched runtime, explicit cache and local mimic"]
async fn live_session_interrupts_and_continues_with_one_id() {
    let fixture = Fixture::new(false).await;
    let mut session = fixture.session(30).await;
    session.send("first fixture").await.unwrap();
    loop {
        if session.next_event().await.unwrap().kind == "result" {
            break;
        }
    }
    session.send("second fixture").await.unwrap();
    first_chunk(&mut session).await;
    session.interrupt().await.unwrap();
    let mut ack = false;
    loop {
        let event = session.next_event().await.unwrap();
        if event.kind == "interrupted" {
            ack = true;
        }
        if event.kind == "result" {
            assert!(ack);
            break;
        }
    }
    session.send("third fixture").await.unwrap();
    loop {
        if session.next_event().await.unwrap().kind == "result" {
            break;
        }
    }
    assert_eq!(
        session
            .turns()
            .iter()
            .map(|t| t.outcome.as_str())
            .collect::<Vec<_>>(),
        ["answered", "interrupted", "answered"]
    );
    let result = session.close().await.unwrap();
    assert_eq!(result.exit_code, Some(0));
    let log = std::fs::read_to_string(fixture.run_dir().join("events.jsonl")).unwrap();
    let ready: Value = serde_json::from_str(log.lines().next().unwrap()).unwrap();
    let example: Value = serde_json::from_str(include_str!(
        "../../../contracts/kai-agent-session-1.example.json"
    ))
    .unwrap();
    assert_eq!(ready, example);
    let text = std::fs::read_to_string(fixture.run_dir().join("agent.json")).unwrap();
    assert!(!text.contains("synthetic-inherited-key-canary"));
    assert!(!text.contains("cost_usd") && !text.contains("costUSD"));
    let saved = record(&fixture.run_dir());
    assert!(saved["session_id"].is_string());
    assert_eq!(saved["outcome"], "closed");
    fixture.stop().await;
}

#[tokio::test]
#[ignore = "requires KVM, a fetched runtime, explicit cache and local mimic"]
async fn closing_during_a_turn_records_cancellation_and_ends_the_vm() {
    let fixture = Fixture::new(true).await;
    let mut session = fixture.session(30).await;
    session.send("fixture cancelled by close").await.unwrap();
    first_chunk(&mut session).await;
    let result = session.close().await.unwrap();
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(record(&fixture.run_dir())["cancelled_turn"], 1);
    fixture.stop().await;
}

#[tokio::test]
#[ignore = "requires KVM, a fetched runtime, explicit cache and local mimic"]
async fn dropping_a_session_closes_the_upstream_and_records_incomplete() {
    let fixture = Fixture::new(true).await;
    let mut session = fixture.session(30).await;
    session.send("fixture dropped by host").await.unwrap();
    first_chunk(&mut session).await;
    drop(session);
    assert_eq!(record(&fixture.run_dir())["outcome"], "dropped");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if std::fs::read(fixture.root.path().join("model.jsonl")).is_ok_and(|b| !b.is_empty()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    fixture.stop().await;
}

#[tokio::test]
#[ignore = "requires KVM, a fetched runtime, explicit cache and local mimic"]
async fn a_vm_timeout_is_infrastructure_failure_not_an_answer() {
    let fixture = Fixture::new(true).await;
    let mut session = fixture.session(2).await;
    session.send("fixture exceeds VM limit").await.unwrap();
    while session.next_event().await.is_ok() {}
    assert!(session.turns().is_empty());
    let saved = record(&fixture.run_dir());
    assert_eq!(saved["outcome"], "failed");
    assert_eq!(saved["vm"]["outcome"], "timeout");
    fixture.stop().await;
}
