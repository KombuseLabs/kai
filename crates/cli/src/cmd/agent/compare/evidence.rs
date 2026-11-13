//! Immutable terminal handoff. Measurements stay in their owning records.
use anyhow::{Context as _, ensure};
use helpers::{fs::Part, hash::sha256_bytes};
use serde_json::{Value, json};
use std::path::Path;

fn reference(root: &Path, relative: &str) -> anyhow::Result<Value> {
    let mut path = root.to_path_buf();
    for component in Path::new(relative).components() {
        ensure!(
            matches!(component, std::path::Component::Normal(_)),
            "invalid evidence path"
        );
        path.push(component);
        ensure!(
            !std::fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "symlink in evidence path"
        );
    }
    ensure!(path.is_file(), "evidence source is not a file");
    let bytes = std::fs::read(path)?;
    Ok(json!({"path": relative, "sha256": format!("sha256:{}", sha256_bytes(&bytes))}))
}

pub(super) fn write(root: &Path, record: &super::record::Record) -> anyhow::Result<()> {
    let pair = serde_json::to_value(record)?;
    ensure!(
        matches!(pair["state"].as_str(), Some("finished" | "interrupted")),
        "pair is not terminal"
    );
    let finished = pair["finished_at"]
        .as_u64()
        .context("pair has no finish time")?;
    // Refuse to bind a different or unfinished pair on disk.
    ensure!(
        std::fs::read(root.join("pair.json"))? == serde_json::to_vec_pretty(record)?,
        "pair changed before evidence capture"
    );
    let mut arms = serde_json::Map::new();
    for role in ["without", "with"] {
        let arm = &pair[role];
        let name = arm["name"].as_str().context("missing arm name")?;
        let relative = format!("{name}/run/agent.json");
        let (agent, missing) = match reference(root, &relative) {
            Ok(reference) => (reference, Value::Null),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                ensure!(
                    arm["state"] != "completed",
                    "completed arm has no agent record"
                );
                (
                    Value::Null,
                    json!(if arm["started_at"].is_null() {
                        "not-started"
                    } else {
                        "not-written"
                    }),
                )
            }
            Err(error) => return Err(error),
        };
        arms.insert(
            role.into(),
            json!({"agent":agent,"agent_missing":missing,"execution_os":null,"quality":"ungraded"}),
        );
    }
    let mut evidence = json!({
        "format":"kai-report-evidence/1", "producer":{"name":"kai","version":env!("CARGO_PKG_VERSION")},
        "run_id":pair["run_id"], "capture":"producer-terminal", "recorded_at":super::record::now()?.max(finished),
        "pair":reference(root,"pair.json")?, "definition":null,"arms":arms
    });
    let target = root.join("report-evidence.json");
    match std::fs::symlink_metadata(&target) {
        Ok(metadata) => {
            ensure!(
                metadata.is_file(),
                "existing evidence is not a regular file"
            );
            let old_bytes = std::fs::read(&target)?;
            let old: Value = serde_json::from_slice(&old_bytes)?;
            ensure!(
                old["recorded_at"].as_u64().is_some_and(|t| t >= finished),
                "invalid existing capture time"
            );
            evidence["recorded_at"] = old["recorded_at"].clone();
            ensure!(
                old == evidence && old_bytes == serde_json::to_vec_pretty(&evidence)?,
                "existing evidence differs; never overwrite a handoff"
            );
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let part = Part::beside(&target);
    std::fs::write(part.path(), serde_json::to_vec_pretty(&evidence)?)?;
    std::fs::File::open(part.path())?.sync_all()?;
    // Atomic create-if-absent: even a competing writer cannot replace a handoff.
    std::fs::hard_link(part.path(), &target)?;
    drop(part);
    helpers::fs::sync_dir(root)?;
    Ok(())
}

pub(super) fn publish(root: &Path, record: &super::record::Record) {
    if let Err(error) = write(root, record) {
        let message = format!("reporting evidence incomplete: {error:#}\n");
        let _ =
            helpers::fs::write_whole(&root.join("report-evidence-error.txt"), message.as_bytes());
        eprintln!("kai: {message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use helpers::testing::TestDir;

    #[derive(Parser)]
    struct Args {
        #[command(flatten)]
        compare: super::super::CompareArgs,
    }

    fn record(root: &Path) -> super::super::record::Record {
        std::fs::create_dir(root.join("input")).unwrap();
        for arm in ["without", "with"] {
            std::fs::create_dir_all(root.join(arm).join("run")).unwrap();
        }
        let args = Args::parse_from([
            "test",
            "--model",
            "fixture",
            "--mock",
            "--extra-prompt",
            "extra",
            "--",
            "task",
        ]);
        super::super::record::Record::new(&args.compare, "extra", root).unwrap()
    }

    #[test]
    fn terminal_handoff_is_immutable_and_absent_for_running_pairs() {
        let dir = TestDir::new("evidence-terminal");
        let mut r = record(&dir);
        r.save(&dir).unwrap();
        assert!(write(&dir, &r).is_err());
        assert!(!dir.join("report-evidence.json").exists());
        r.without.started_at = Some(1);
        r.without.state = "completed".into();
        std::fs::write(
            dir.join("without/run/agent.json"),
            b"{\"format\":\"kai-agent-run/1\"}",
        )
        .unwrap();
        r.finish(true);
        r.save(&dir).unwrap();
        write(&dir, &r).unwrap();
        let first = std::fs::read(dir.join("report-evidence.json")).unwrap();
        let e: Value = serde_json::from_slice(&first).unwrap();
        assert_eq!(e["arms"]["with"]["agent_missing"], "not-started");
        assert!(e["arms"]["without"]["agent_missing"].is_null());
        assert_eq!(e["arms"]["with"]["quality"], "ungraded");
        write(&dir, &r).unwrap();
        assert_eq!(
            first,
            std::fs::read(dir.join("report-evidence.json")).unwrap()
        );
        std::fs::write(dir.join("without/run/agent.json"), b"changed").unwrap();
        assert!(write(&dir, &r).is_err());
        assert_eq!(
            first,
            std::fs::read(dir.join("report-evidence.json")).unwrap()
        );
    }

    #[test]
    fn absent_record_after_start_is_not_written_and_publication_failure_keeps_status() {
        let dir = TestDir::new("evidence-missing");
        let mut r = record(&dir);
        r.without.started_at = Some(1);
        r.finish(false);
        r.save(&dir).unwrap();
        write(&dir, &r).unwrap();
        let e: Value =
            serde_json::from_slice(&std::fs::read(dir.join("report-evidence.json")).unwrap())
                .unwrap();
        assert_eq!(e["arms"]["without"]["agent_missing"], "not-written");
        let pair = std::fs::read(dir.join("pair.json")).unwrap();
        std::fs::write(dir.join("without/run/agent.json"), b"later").unwrap();
        publish(&dir, &r);
        assert!(dir.join("report-evidence-error.txt").is_file());
        assert_eq!(pair, std::fs::read(dir.join("pair.json")).unwrap());
    }

    #[test]
    fn symlink_sources_are_refused() {
        let dir = TestDir::new("evidence-link");
        let mut r = record(&dir);
        std::os::unix::fs::symlink("../../pair.json", dir.join("without/run/agent.json")).unwrap();
        r.finish(false);
        r.save(&dir).unwrap();
        assert!(write(&dir, &r).is_err());
    }
}
