//! The local, frozen input and execution record; reporting is a separate reader.
use super::CompareArgs;
use helpers::{fs::write_whole, hash::sha256_bytes};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::Path;

#[derive(Serialize)]
pub(super) struct Arm {
    name: String,
    pub state: String,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
    pub elapsed_ms: Option<u64>,
}

impl Arm {
    fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            state: "pending".into(),
            started_at: None,
            finished_at: None,
            elapsed_ms: None,
        }
    }
}

#[derive(Serialize)]
pub(super) struct Record {
    format: &'static str,
    run_id: String,
    producer: &'static str,
    state: &'static str,
    started_at: u64,
    finished_at: Option<u64>,
    task: Value,
    profile: Value,
    treatment: Value,
    pub without: Arm,
    pub with: Arm,
}

pub(super) fn now() -> anyhow::Result<u64> {
    Ok(u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )?)
}

impl Record {
    pub fn new(args: &CompareArgs, extra: &str, output: &Path) -> anyhow::Result<Self> {
        let snapshot = helpers::tree_digest::tree(&output.join("input"))?;
        // Versioned length-delimited bytes, shared with the reporter reader.
        let identity = format!(
            "kai-task/1:{}:{}:{}",
            args.prompt.len(),
            args.prompt,
            snapshot
        );
        let p = &args.profile;
        Ok(Self {
            format: "kai-pair/1",
            run_id: helpers::hash::random_id()?,
            producer: env!("CARGO_PKG_VERSION"),
            state: "running",
            started_at: now()?,
            finished_at: None,
            task: json!({"name":args.name,"prompt":args.prompt,"snapshot":snapshot,"digest":sha256_bytes(identity.as_bytes())}),
            profile: json!({"model":p.model,"tools":p.tools,"image":p.image,"cpus":p.cpus,"memory":p.memory,"timeout":p.timeout,"max_turns":p.max_turns,"max_output_tokens":32000,"mock":p.mock,"agent_version":agent::runtime::CLAUDE.version,"sdk":agent::runtime::SDK_VERSION}),
            treatment: json!({"extra_prompt":extra,"digest":sha256_bytes(extra.as_bytes()),"addon":args.addon.as_deref().and_then(agent::addon::named).map(|a| json!({"name":a.name,"commit":a.commit}))}),
            without: Arm::new(&args.arms.without),
            with: Arm::new(&args.arms.with),
        })
    }

    pub fn finish(&mut self, interrupted: bool) {
        self.state = if interrupted {
            "interrupted"
        } else {
            "finished"
        };
        self.finished_at = now().ok();
        for arm in [&mut self.without, &mut self.with] {
            if matches!(arm.state.as_str(), "running" | "pending") {
                arm.state = if interrupted { "interrupted" } else { "failed" }.into();
                // Cancellation dropped the future before its monotonic measurement settled.
                // Keep elapsed unknown instead of substituting wall-clock subtraction.
                arm.finished_at = self.finished_at;
            }
        }
    }

    pub fn save(&self, output: &Path) -> anyhow::Result<()> {
        write_whole(&output.join("pair.json"), &serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}
