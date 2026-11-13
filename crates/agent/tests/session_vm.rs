//! Opt-in tests with the pinned SDK in a real VM and a local fake model only.
//! Set KAI_AGENT_TEST_CACHE and KAI_AGENT_TEST_MIMIC (a mimic that answers
//! `tool:`, 0.2.0 or newer), then run this test with --ignored
//! --test-threads=1. These tests never fetch artifacts or call a model.
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
        if stall_first {
            Self::answering(&["stall:60s"]).await
        } else {
            Self::answering(&["ok", "stall:60s", "ok"]).await
        }
    }
    /// mimic answers requests in this order, the last one repeating.
    async fn answering(answers: &[&str]) -> Self {
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
        for answer in answers {
            command.args(["--answer", answer]);
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
        self.session_with(|options| {
            options.timeout_s = timeout;
            options.tools.clear();
        })
        .await
    }
    /// A session of the standard profile (all of [`agent::session::TOOLS`]),
    /// changed by `change`.
    async fn session_with(&self, change: impl FnOnce(&mut Options)) -> Session {
        let route = Route::new(&self.url, Some("synthetic-inherited-key-canary"), true).unwrap();
        let mut options = Options::new("test-model".into(), route, self.run_dir());
        change(&mut options);
        Session::start(&self.cache, options).await.unwrap()
    }
    /// A tree of the given files under the fixture's root.
    fn tree(&self, files: &[(&str, &str)]) -> PathBuf {
        let tree = self.root.path().join("tree");
        for (path, text) in files {
            let path = tree.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        std::fs::create_dir_all(&tree).unwrap();
        tree
    }
    fn tree_out(&self) -> PathBuf {
        self.root.path().join("after")
    }
    fn ledger(&self) -> Vec<Value> {
        std::fs::read_to_string(self.root.path().join("model.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
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

/// Sends one prompt and reads its turn to the end: the tool results the
/// guest's Claude sent back to the model, in order, and the turn.
async fn one_turn(session: &mut Session, prompt: &str) -> (Vec<Value>, agent::session::Turn) {
    session.send(prompt).await.unwrap();
    let mut results = vec![];
    loop {
        let event = session.next_event().await.unwrap();
        if event.kind == "result" {
            break;
        }
        if event.payload["type"] == "user" {
            for block in event.payload["message"]["content"].as_array().unwrap() {
                if block["type"] == "tool_result" {
                    results.push(block.clone());
                }
            }
        }
    }
    (results, session.turns().last().unwrap().clone())
}

/// A tool result's text, whether given as a string or as text blocks.
fn text(result: &Value) -> String {
    match &result["content"] {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block["text"].as_str())
            .collect(),
        other => panic!("tool result without text: {other}"),
    }
}

#[tokio::test]
#[ignore = "requires KVM, a fetched runtime, explicit cache and local mimic"]
async fn bash_runs_in_the_tree_as_the_agent_user() {
    let fixture = Fixture::answering(&[
        r#"tool:Bash:{"command":"id -u && id -g && pwd && cargo --version"}"#,
        "ok",
    ])
    .await;
    let mut session = fixture.session_with(|options| options.timeout_s = 60).await;
    let (results, turn) = one_turn(&mut session, "fixture: run a command").await;
    assert_eq!(turn.outcome, "answered");
    let [result] = results.as_slice() else {
        panic!("one tool result expected: {results:?}");
    };
    assert_ne!(result["is_error"], true, "{result}");
    let output = text(result);
    assert!(
        output.starts_with("1000\n1000\n/job/tree\ncargo "),
        "{output}"
    );
    // The standard profile: Claude offers every tool asked for.
    let mut offered = turn.tools_offered.expect("the turn's init names its tools");
    offered.sort();
    let mut all = agent::session::TOOLS.to_vec();
    all.sort_unstable();
    assert_eq!(offered, all);
    assert_eq!(session.close().await.unwrap().exit_code, Some(0));
    // The tool's result went back to the model: the second request is
    // longer by the call and its result.
    let ledger = fixture.ledger();
    assert_eq!(ledger.len(), 2);
    assert!(ledger[1]["messages"].as_u64() > ledger[0]["messages"].as_u64());
    assert_eq!(ledger[0]["answer"]["spec"], "tool:Bash");
    fixture.stop().await;
}

#[tokio::test]
#[ignore = "requires KVM, a fetched runtime, explicit cache and local mimic"]
async fn an_edit_comes_out_in_the_tree() {
    let fixture = Fixture::answering(&[
        r#"tool:Edit:{"file_path":"/job/tree/seed.txt","old_string":"seed","new_string":"grown"}"#,
        "ok",
    ])
    .await;
    let tree = fixture.tree(&[("seed.txt", "seed\n")]);
    let tree_out = fixture.tree_out();
    let mut session = fixture
        .session_with(|options| {
            options.timeout_s = 60;
            options.tree = Some(tree.clone());
            options.tree_out = Some(tree_out.clone());
        })
        .await;
    let (results, turn) = one_turn(&mut session, "fixture: edit a file").await;
    assert_eq!(turn.outcome, "answered");
    assert_ne!(results[0]["is_error"], true, "{results:?}");
    let vm = session.close().await.unwrap();
    assert_eq!(vm.tree_out, Some(true));
    assert_eq!(
        std::fs::read_to_string(tree_out.join("seed.txt")).unwrap(),
        "grown\n"
    );
    assert_eq!(
        std::fs::read_to_string(tree.join("seed.txt")).unwrap(),
        "seed\n"
    );
    fixture.stop().await;
}

#[tokio::test]
#[ignore = "requires KVM, a fetched runtime, explicit cache and local mimic"]
async fn a_written_file_comes_out_in_the_tree() {
    let fixture = Fixture::answering(&[
        r#"tool:Write:{"file_path":"/job/tree/hello.txt","content":"hi\n"}"#,
        "ok",
    ])
    .await;
    let tree = fixture.tree(&[]);
    let tree_out = fixture.tree_out();
    let mut session = fixture
        .session_with(|options| {
            options.timeout_s = 60;
            options.tree = Some(tree.clone());
            options.tree_out = Some(tree_out.clone());
        })
        .await;
    let (results, turn) = one_turn(&mut session, "fixture: write a file").await;
    assert_eq!(turn.outcome, "answered");
    assert_ne!(results[0]["is_error"], true, "{results:?}");
    // mimic's `ok`, with the text the fixture gives it.
    assert_eq!(turn.result["result"], "fixture reply");
    assert_eq!(session.close().await.unwrap().tree_out, Some(true));
    assert_eq!(
        std::fs::read_to_string(tree_out.join("hello.txt")).unwrap(),
        "hi\n"
    );
    assert!(!tree.join("hello.txt").exists());
    fixture.stop().await;
}

#[tokio::test]
#[ignore = "requires KVM, a fetched runtime, explicit cache and local mimic"]
async fn nothing_of_the_tree_configures_claude() {
    // A tree's CLAUDE.md and AGENTS.md would be read into the first request
    // and its settings' hooks would run, were any setting source loaded.
    let marker = "fixture-context-marker ".repeat(2048);
    let hooks = r#"{"hooks":{
        "SessionStart":[{"hooks":[{"type":"command","command":"touch /job/tree/session-start-ran"}]}],
        "UserPromptSubmit":[{"hooks":[{"type":"command","command":"touch /job/tree/prompt-hook-ran"}]}]}}"#;
    let fixture = Fixture::answering(&["ok"]).await;
    let tree = fixture.tree(&[
        ("CLAUDE.md", &marker),
        ("AGENTS.md", &marker),
        (".claude/settings.json", hooks),
    ]);
    let tree_out = fixture.tree_out();
    let mut session = fixture
        .session_with(|options| {
            options.timeout_s = 60;
            options.tree = Some(tree.clone());
            options.tree_out = Some(tree_out.clone());
        })
        .await;
    let (_, turn) = one_turn(&mut session, "fixture: answer").await;
    assert_eq!(turn.outcome, "answered");
    assert_eq!(session.close().await.unwrap().tree_out, Some(true));
    assert!(tree_out.join("CLAUDE.md").exists());
    assert!(!tree_out.join("session-start-ran").exists());
    assert!(!tree_out.join("prompt-hook-ran").exists());
    let ledger = fixture.ledger();
    let request = &ledger[0];
    let context =
        request["system_bytes"].as_u64().unwrap() + request["messages_bytes"].as_u64().unwrap();
    assert!(
        context < marker.len() as u64,
        "the first request holds {context} bytes of system and messages"
    );
    fixture.stop().await;
}

#[tokio::test]
#[ignore = "requires KVM, a fetched runtime, explicit cache and local mimic"]
async fn a_permission_request_is_denied_recorded_and_the_turn_goes_on() {
    // Claude asks before it changes its own settings in the tree, even for
    // an allowed tool. The session has no way to answer, so it denies.
    let fixture = Fixture::answering(&[
        r#"tool:Edit:{"file_path":"/job/tree/.claude/settings.json","old_string":"","new_string":"{}"}"#,
        "ok",
    ])
    .await;
    let tree = fixture.tree(&[(".claude/keep", "")]);
    let tree_out = fixture.tree_out();
    let mut session = fixture
        .session_with(|options| {
            options.timeout_s = 60;
            options.tree = Some(tree.clone());
            options.tree_out = Some(tree_out.clone());
        })
        .await;
    let (results, turn) = one_turn(&mut session, "fixture: change the settings").await;
    assert_eq!(turn.outcome, "answered");
    assert_eq!(results[0]["is_error"], true, "{results:?}");
    assert!(text(&results[0]).contains("not enabled for this session"));
    let denials = turn.result["permission_denials"].as_array().unwrap();
    assert_eq!(denials.len(), 1, "{denials:?}");
    assert_eq!(denials[0]["tool_name"], "Edit");
    session.close().await.unwrap();
    assert!(!tree_out.join(".claude/settings.json").exists());
    let saved = record(&fixture.run_dir());
    assert_eq!(
        saved["turns"][0]["result"]["permission_denials"][0]["tool_name"],
        "Edit"
    );
    fixture.stop().await;
}

#[tokio::test]
#[ignore = "requires KVM, a fetched runtime, explicit cache and local mimic"]
async fn a_tool_outside_the_allowlist_is_not_offered_and_does_not_run() {
    let fixture =
        Fixture::answering(&[r#"tool:Bash:{"command":"touch /job/tree/ran"}"#, "ok"]).await;
    let tree = fixture.tree(&[]);
    let tree_out = fixture.tree_out();
    let mut session = fixture
        .session_with(|options| {
            options.timeout_s = 60;
            options.tools = vec!["Read".into()];
            options.tree = Some(tree.clone());
            options.tree_out = Some(tree_out.clone());
        })
        .await;
    let (results, turn) = one_turn(&mut session, "fixture: run what is not allowed").await;
    assert_eq!(turn.outcome, "answered");
    assert_eq!(results[0]["is_error"], true, "{results:?}");
    assert!(text(&results[0]).contains("No such tool available"));
    assert_eq!(
        turn.tools_offered.as_deref(),
        Some(&["Read".to_owned()][..])
    );
    assert_eq!(turn.result["permission_denials"], serde_json::json!([]));
    session.close().await.unwrap();
    assert!(tree_out.is_dir() && !tree_out.join("ran").exists());
    fixture.stop().await;
}
