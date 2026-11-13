//! CLI validation and opt-in end-to-end pairs against a local fake API.
//! The VM tests need only KAI_AGENT_TEST_CACHE (already fetched and built).
use helpers::testing::TestDir;
use serde_json::{Value, json};
use std::path::Path;
use std::process::{Command, Output};

fn command(dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kai"));
    command.current_dir(dir);
    for (name, _) in std::env::vars() {
        if name.starts_with("KAI_") {
            command.env_remove(name);
        }
    }
    command.env("ANTHROPIC_BASE_URL", "http://127.0.0.1:1");
    command.env("ANTHROPIC_API_KEY", "invented-key-not-used-in-mock-mode");
    command
}

#[test]
fn compare_requires_one_extra_prompt_and_preserves_existing_output() {
    let dir = TestDir::new("compare-cli-validation");
    for args in [
        vec!["--", "task"],
        vec![
            "--extra-prompt",
            "one",
            "--extra-prompt-file",
            "two",
            "--",
            "task",
        ],
        vec!["--extra-prompt", " ", "--", "task"],
    ] {
        let out = command(&dir)
            .args(["agent", "compare", "--model", "fixture", "--mock"])
            .args(args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{out:?}");
    }
    std::fs::create_dir(dir.join("result")).unwrap();
    std::fs::write(dir.join("result/keep"), "untouched").unwrap();
    let out = command(&dir)
        .args([
            "agent",
            "compare",
            "--model",
            "fixture",
            "--mock",
            "--output",
            "result",
            "--extra-prompt",
            "extra",
            "--",
            "task",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(
        std::fs::read_to_string(dir.join("result/keep")).unwrap(),
        "untouched"
    );
    assert!(!dir.join("result/without").exists());
}

/// The streamed Messages response consumed by the actual pinned SDK.
fn reply(with: bool, tool: bool) -> String {
    let label = if with { "with" } else { "without" };
    let block = if tool {
        json!({"type":"tool_use","id":format!("toolu_{label}"),"name":"Write","input":{}})
    } else {
        json!({"type":"text","text":""})
    };
    let delta = if tool {
        json!({"type":"input_json_delta", "partial_json":json!({
            "file_path":"/job/tree/index.html", "content":format!("<!doctype html><title>{label}</title><p>{label}</p>")
        }).to_string()})
    } else {
        json!({"type":"text_delta", "text":format!("Created {label} result.")})
    };
    [
        json!({"type":"message_start","message":{"id":format!("msg_{label}"),"type":"message","role":"assistant","model":"fixture","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1}}}),
        json!({"type":"content_block_start","index":0,"content_block":block}),
        json!({"type":"content_block_delta","index":0,"delta":delta}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":if tool {"tool_use"} else {"end_turn"},"stop_sequence":null},"usage":{"output_tokens":1}}),
        json!({"type":"message_stop"}),
    ].iter().map(|value| format!("event: {}\ndata: {value}\n\n", value["type"].as_str().unwrap())).collect()
}

#[derive(Clone, Copy)]
enum Entry {
    Direct,
    Named,
    Ponytail,
}

async fn vm_pair(sequential: bool, fail_with: bool, entry: Entry) {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _};

    let cache = std::env::var_os("KAI_AGENT_TEST_CACHE").expect("explicit prepared VM cache");
    let dir = TestDir::new("compare-vm");
    std::fs::create_dir(dir.join("source")).unwrap();
    std::fs::write(dir.join("source/seed.txt"), "same input").unwrap();
    std::fs::write(dir.join("extra.txt"), "EXTRA_FIXTURE_MARKER").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    // Neither initial request gets a response until both VMs are waiting.
    // A sequential implementation of the default path therefore times out.
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let captured = requests.clone();
    let server = tokio::spawn(async move {
        let mut handlers = tokio::task::JoinSet::new();
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let captured = captured.clone();
            let barrier = barrier.clone();
            handlers.spawn(async move {
                let mut reader = tokio::io::BufReader::new(socket);
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    if line == "\r\n" { break; }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).await.unwrap();
                let body: Value = serde_json::from_slice(&bytes).unwrap();
                let messages = body["messages"].to_string();
                let with = messages.contains("EXTRA_FIXTURE_MARKER") || messages.to_ascii_lowercase().contains("ponytail");
                let followup = messages.contains("tool_result");
                assert!(messages.contains("TASK_FIXTURE: build a calculator"));
                captured.lock().unwrap().push(body);
                if !followup && !sequential {
                    tokio::time::timeout(Duration::from_secs(45), barrier.wait()).await.unwrap();
                }
                let (status, kind, content) = if with && fail_with {
                    ("402 Payment Required", "application/json", json!({"type":"error","error":{"type":"api_error","message":"prxy: not-approved: fixture"}}).to_string())
                } else {
                    ("200 OK", "text/event-stream", reply(with, !followup))
                };
                let response = format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{content}", content.len());
                reader.get_mut().write_all(response.as_bytes()).await.unwrap();
            });
        }
    });
    let mut cmd = tokio::process::Command::from(command(&dir));
    cmd.env("ANTHROPIC_BASE_URL", endpoint).kill_on_drop(true);
    if matches!(entry, Entry::Direct) {
        cmd.args(["agent", "compare", "--cache"]).arg(&cache).args([
            "--model",
            "fixture",
            "--mock",
            "--tree",
            "source",
            "--output",
            "results",
            "--extra-prompt-file",
            "extra.txt",
            "--max-turns",
            "3",
            "--timeout",
            "90",
        ]);
        if sequential {
            cmd.arg("--sequential");
        }
        cmd.args(["--", "TASK_FIXTURE: build a calculator"]);
    } else {
        std::fs::create_dir_all(dir.join("presets/fixture")).unwrap();
        std::fs::create_dir(dir.join("below")).unwrap();
        std::fs::write(dir.join("kai.toml"), "[runs]\ndirectory = 'presets'\n").unwrap();
        let mut definition = json!({"model":"fixture", "prompt":"TASK_FIXTURE: build a calculator",
            "extra_prompt_file":"../../extra.txt", "tree":"../../source", "max_turns":3, "sequential":sequential,
            "arms":{"without":"plain", "with":"ponytail"}});
        if matches!(entry, Entry::Ponytail) {
            definition
                .as_object_mut()
                .unwrap()
                .remove("extra_prompt_file");
            definition["extra_prompt"] = "/ponytail full".into();
            definition["addon"] = "ponytail".into();
        }
        std::fs::write(
            dir.join("presets/fixture/task.json"),
            definition.to_string(),
        )
        .unwrap();
        cmd.current_dir(dir.join("below"))
            .args(["run", "fixture", "--cache"])
            .arg(&cache)
            .arg("--mock");
    }
    let out: Output = tokio::time::timeout(Duration::from_secs(120), cmd.output())
        .await
        .unwrap()
        .unwrap();
    server.abort();
    assert_eq!(out.status.success(), !fail_with, "{out:?}");
    let output = if matches!(entry, Entry::Direct) {
        dir.join("results")
    } else {
        let results: Vec<_> = std::fs::read_dir(dir.join("presets/fixture/results"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(results.len(), 1);
        let timestamp = results[0].file_name().unwrap().to_str().unwrap();
        let (seconds, nanos) = timestamp.split_once('.').unwrap();
        assert!(seconds.parse::<u64>().unwrap() > 0);
        assert_eq!(nanos.len(), 9);
        assert!(nanos.parse::<u32>().unwrap() < 1_000_000_000);
        results[0].clone()
    };
    let mut sessions = vec![];
    for name in ["without", "with"] {
        let arm = if matches!(entry, Entry::Direct) {
            name
        } else if name == "without" {
            "plain"
        } else {
            "ponytail"
        };
        let result = output.join(arm);
        let record: Value =
            serde_json::from_slice(&std::fs::read(result.join("run/agent.json")).unwrap()).unwrap();
        sessions.push(record["session_id"].clone());
        let status = std::fs::read_to_string(result.join("status.txt")).unwrap();
        if name == "with" && fail_with {
            assert_ne!(status.trim(), "completed");
            continue;
        }
        assert_eq!(status, "completed\n");
        assert_eq!(
            std::fs::read_to_string(result.join("tree/seed.txt")).unwrap(),
            "same input"
        );
        assert!(
            std::fs::read_to_string(result.join("tree/index.html"))
                .unwrap()
                .contains(&format!("<title>{name}</title>"))
        );
        assert_eq!(
            std::fs::read_to_string(result.join("answer.md")).unwrap(),
            format!("Created {name} result.")
        );
        let prompt = std::fs::read_to_string(result.join("run/prompt-1.md")).unwrap();
        if matches!(entry, Entry::Ponytail) {
            assert_eq!(prompt.starts_with("/ponytail full\n"), name == "with");
            assert_eq!(
                record["addons"].as_array().unwrap().len(),
                usize::from(name == "with")
            );
            assert_eq!(
                record["turns"][0]["commands_invoked"]
                    .to_string()
                    .contains("ponytail"),
                name == "with"
            );
        } else {
            assert_eq!(prompt.contains("EXTRA_FIXTURE_MARKER"), name == "with");
        }
    }
    assert!(sessions.iter().all(Value::is_string));
    assert_ne!(sessions[0], sessions[1]);
    assert_eq!(
        std::fs::read_to_string(dir.join("source/seed.txt")).unwrap(),
        "same input"
    );
    assert!(!dir.join("source/index.html").exists());
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), if fail_with { 3 } else { 4 });
}

#[tokio::test]
#[ignore = "requires KVM and KAI_AGENT_TEST_CACHE; local fake API only"]
async fn both_vms_run_in_parallel_and_keep_different_outputs() {
    vm_pair(false, false, Entry::Direct).await;
}

#[tokio::test]
#[ignore = "requires KVM and KAI_AGENT_TEST_CACHE; local fake API only"]
async fn sequential_mode_keeps_the_same_two_outputs() {
    vm_pair(true, false, Entry::Direct).await;
}

#[tokio::test]
#[ignore = "requires KVM and KAI_AGENT_TEST_CACHE; local fake API only"]
async fn a_failed_arm_does_not_discard_the_other_result() {
    vm_pair(false, true, Entry::Direct).await;
}

#[tokio::test]
#[ignore = "requires KVM and KAI_AGENT_TEST_CACHE; local fake API only"]
async fn a_named_run_resolves_config_and_file_paths_from_a_subdirectory() {
    vm_pair(false, false, Entry::Named).await;
}

#[tokio::test]
#[ignore = "requires KVM, KAI_AGENT_TEST_CACHE and cached Ponytail; local fake API only"]
async fn a_named_ponytail_run_activates_the_addon_only_on_one_side() {
    vm_pair(false, false, Entry::Ponytail).await;
}

#[test]
fn named_runs_reject_bad_names_and_invalid_definitions() {
    let dir = TestDir::new("named-run-errors");
    std::fs::create_dir_all(dir.join("presets/fixture")).unwrap();
    std::fs::create_dir(dir.join("below")).unwrap();
    std::fs::write(dir.join("kai.toml"), "[runs]\ndirectory = 'presets'\n").unwrap();
    for (name, expected) in [
        ("../outside", "run name"),
        ("missing", "presets/missing/task.json"),
    ] {
        let out = command(&dir.join("below"))
            .args(["run", name, "--mock"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(expected),
            "{out:?}"
        );
    }
    for (definition, expected) in [
        (
            json!({"model":"fixture", "prompt":"task", "extra_prompt":"extra", "typo":true}),
            "unknown field",
        ),
        (json!({"model":"fixture", "prompt":"task"}), "exactly one"),
        (
            json!({"model":"fixture", "prompt":"task", "extra_prompt":"extra", "extra_prompt_file":"extra.txt"}),
            "exactly one",
        ),
        (
            json!({"model":"fixture", "prompt":"task", "extra_prompt":"extra", "max_turns":0}),
            "max_turns",
        ),
        (
            json!({"model":"fixture", "prompt":"task", "extra_prompt":"extra", "arms":{"without":"same", "with":"same"}}),
            "arm names must be different",
        ),
        (
            json!({"model":"fixture", "prompt":"task", "extra_prompt":"extra", "arms":{"without":"../outside"}}),
            "arm names must contain",
        ),
        (
            json!({"model":"fixture", "prompt":"task", "extra_prompt":"extra", "arms":{"with":"input"}}),
            "reserved",
        ),
    ] {
        std::fs::write(
            dir.join("presets/fixture/task.json"),
            definition.to_string(),
        )
        .unwrap();
        let out = command(&dir.join("below"))
            .args(["run", "fixture", "--mock"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(expected),
            "{out:?}"
        );
    }
}
