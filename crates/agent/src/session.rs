//! A live SDK session in one microVM. Inputs and events have explicit turn ids;
//! interrupt acknowledgement is separate from the terminal result. Dropping a
//! session aborts its VM task. Nothing here starts a model proxy or sets a budget.

use crate::runtime;
use helpers::fs::{PartDir, write_whole};
use helpers::lines::BoundedLines;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt as _;
use tokio::net::UnixListener;
use tokio::task::JoinHandle;

/// Maximum encoded event or command including newline.
pub const MAX_FRAME: usize = 2 * 1024 * 1024;
/// Maximum UTF-8 prompt length.
pub const MAX_PROMPT: usize = 64 * 1024;
/// Tool names the profile can allow. These are not an isolation boundary.
pub const TOOLS: &[&str] = &["Read", "Edit", "Write", "Bash", "Glob", "Grep"];
/// The highest turn limit a session takes.
pub const MAX_TURNS: u32 = 10_000;

/// Why a session could not start or continue. Payloads and credentials are not echoed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A caller supplied an unsupported value.
    #[error("invalid session setting: {0}")]
    Invalid(&'static str),
    /// A transport, file or VM operation failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// A VM operation failed.
    #[error(transparent)]
    Vm(#[from] vm::Error),
    /// The runtime layer is missing or invalid.
    #[error(transparent)]
    Layer(#[from] crate::layer::Error),
    /// The guest broke its protocol or exceeded a limit.
    #[error("session protocol failed: {0}")]
    Protocol(&'static str),
    /// A bounded operation did not finish.
    #[error("session timed out")]
    Timeout,
}

/// A validated loopback model route. Debug deliberately omits its credential.
pub struct Route {
    port: u16,
    credential: String,
    mock: bool,
}
impl Route {
    /// Accepts only an explicit loopback HTTP endpoint. Mock mode substitutes a
    /// fixed dummy regardless of the inherited key; normal mode requires exactly
    /// `prxy-` followed by 32 lowercase hex digits. This is not proxy approval.
    pub fn new(url: &str, key: Option<&str>, mock: bool) -> Result<Self, Error> {
        let host = url
            .strip_prefix("http://")
            .ok_or(Error::Invalid("model route must be loopback HTTP"))?;
        let host = host.strip_suffix('/').unwrap_or(host);
        let (name, port) = host
            .split_once(':')
            .ok_or(Error::Invalid("model route needs a port"))?;
        if !matches!(name, "127.0.0.1" | "localhost") {
            return Err(Error::Invalid("model route must be loopback HTTP"));
        }
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::Invalid("model route port"));
        }
        let port = port
            .parse::<u16>()
            .ok()
            .filter(|p| *p != 0)
            .ok_or(Error::Invalid("model route port"))?;
        let credential = if mock {
            "dummy-not-a-key".to_owned()
        } else {
            let key = key.ok_or(Error::Invalid("proxy placeholder is missing"))?;
            if key.len() != 37
                || !key.starts_with("prxy-")
                || !key[5..]
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            {
                return Err(Error::Invalid("proxy placeholder format"));
            }
            key.to_owned()
        };
        Ok(Self {
            port,
            credential,
            mock,
        })
    }
}

/// Options common to one-shot runs and live sessions.
pub struct Options {
    /// Model identifier, chosen explicitly by the caller.
    pub model: String,
    /// Validated external route.
    pub route: Route,
    /// Image recipe already built in the cache.
    pub image: String,
    /// Initial tree; copied, never modified on the host.
    pub tree: Option<PathBuf>,
    /// Optional exported tree, which must not exist.
    pub tree_out: Option<PathBuf>,
    /// Additional layers, below the runtime.
    pub layers: Vec<PathBuf>,
    /// VM processors.
    pub cpus: u32,
    /// VM memory in MiB.
    pub memory_mib: u32,
    /// Whole VM execution limit in seconds.
    pub timeout_s: u64,
    /// Tool allowlist, drawn from [`TOOLS`]. Empty disables tools.
    pub tools: Vec<String>,
    /// The most model requests Claude makes for one turn, 1 to
    /// [`MAX_TURNS`]; `None` is no limit but the VM's time. A turn that
    /// reaches it ends with the outcome `limit`.
    pub max_turns: Option<u32>,
    /// Add-ons, as [`crate::addon::cached`] found them: each layer is
    /// mounted above the runtime and its directory given to Claude as a
    /// local plugin. At most eight, each name once.
    pub addons: Vec<crate::layer::Fetched<crate::addon::AddonInfo>>,
    /// A slash command, with its arguments, put on a line of its own before
    /// the first turn's text, such as `/ponytail full`; never before a
    /// later turn's.
    pub activation: Option<String>,
    /// Run directory, which must be absent or empty.
    pub run_dir: PathBuf,
}
impl Options {
    /// Standard profile: Rust image, two CPUs, 4 GiB, ten minutes, all six tools.
    pub fn new(model: String, route: Route, run_dir: PathBuf) -> Self {
        Self {
            model,
            route,
            image: "rust".into(),
            tree: None,
            tree_out: None,
            layers: vec![],
            cpus: 2,
            memory_mib: 4096,
            timeout_s: 600,
            tools: TOOLS.iter().map(|s| (*s).into()).collect(),
            max_turns: None,
            addons: vec![],
            activation: None,
            run_dir,
        }
    }
    fn validate(&self) -> Result<(), Error> {
        if self.model.is_empty()
            || self.model.len() > 200
            || !self.model.as_bytes()[0].is_ascii_alphanumeric()
            || !self
                .model
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._:/-".contains(&b))
        {
            return Err(Error::Invalid("model identifier"));
        }
        if self.tools.iter().any(|t| !TOOLS.contains(&t.as_str())) {
            return Err(Error::Invalid("tool allowlist"));
        }
        if self.timeout_s == 0 {
            return Err(Error::Invalid("timeout must be positive"));
        }
        // The SDK takes 0 as no limit at all, not as none allowed.
        if self
            .max_turns
            .is_some_and(|turns| !(1..=MAX_TURNS).contains(&turns))
        {
            return Err(Error::Invalid("a turn limit is 1 to 10000"));
        }
        let mut names = std::collections::BTreeSet::new();
        if self.addons.len() > 8
            || !self
                .addons
                .iter()
                .all(|addon| names.insert(addon.info.name.as_str()))
        {
            return Err(Error::Invalid("at most eight add-ons, each once"));
        }
        if let Some(activation) = &self.activation
            && command(activation).is_none()
        {
            return Err(Error::Invalid(
                "an activation is one line: a slash command and its arguments",
            ));
        }
        Ok(())
    }
}

/// The slash command an activation starts with, such as `/ponytail` of
/// `/ponytail full`: `/` and a name of letters, digits, `:`, `_` and `-`,
/// then nothing or a space and arguments, on one line of at most 256
/// bytes. `None` when `activation` is not one.
pub fn command(activation: &str) -> Option<&str> {
    let name = activation.split(' ').next()?;
    let plain = name.len() > 1
        && name.starts_with('/')
        && name[1..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b":_-".contains(&b));
    (plain && activation.len() <= 256 && !activation.contains(['\n', '\r'])).then_some(name)
}

/// One ordered event from the guest. Payload contains SDK data without its cost
/// estimate. Never deserialize this as executable instructions on the host.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    /// [`runtime::FORMAT`].
    pub format: String,
    /// Monotonically increasing, starting at one.
    pub seq: u64,
    /// ready, event, result, interrupted, closed or error.
    pub kind: String,
    /// Turn id for turn-specific events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<u64>,
    /// Stable SDK session id when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Bounded data associated with the event.
    pub payload: Value,
}

/// A terminal turn, distinct from the state of the VM or session.
#[derive(Clone, Debug, Serialize)]
pub struct Turn {
    /// Correlated request id.
    pub id: u64,
    /// answered, limit (the turn reached [`Options::max_turns`]),
    /// interrupted or agent-error.
    pub outcome: String,
    /// Digest of the submitted prompt; not its text.
    pub prompt_sha256: String,
    /// The tools Claude offered the model in this turn, as its `system`
    /// `init` event names them; `None` when the turn brought no such event.
    /// They can differ from [`Options::tools`], the tools asked for.
    pub tools_offered: Option<Vec<String>>,
    /// The skills Claude offered in this turn, as the same `init` event
    /// names them, an add-on's as `<plugin>:<skill>`; `None` without one.
    pub skills_offered: Option<Vec<String>>,
    /// The slash commands Claude took in this turn, as the user messages
    /// it replays name them (`/ponytail`); empty when it took none, `None`
    /// when it replayed no user message.
    pub commands_invoked: Option<Vec<String>>,
    /// SDK terminal result without cost estimates.
    pub result: Value,
}

struct Active {
    id: u64,
    prompt_sha256: String,
    tools_offered: Option<Vec<String>>,
    skills_offered: Option<Vec<String>>,
    commands_invoked: Option<Vec<String>>,
    interrupt: bool,
    acknowledged: bool,
    terminal: bool,
}
struct VmTask(JoinHandle<Result<vm::RunResult, vm::Error>>);
impl Drop for VmTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// A running guest and its ordered transport. Call [`Session::close`] to save
/// the terminal run record; dropping kills the VM and records an incomplete
/// outcome without claiming a VM result.
pub struct Session {
    reader: BoundedLines<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
    vm: VmTask,
    _control: PartDir,
    seq: u64,
    next_turn: u64,
    active: Option<Active>,
    session_id: Option<String>,
    log: std::fs::File,
    turns: Vec<Turn>,
    options: Options,
    layer: crate::layer::LayerInfo,
    failed: bool,
    closing: bool,
    recorded_bytes: usize,
    record_written: bool,
    tools_warned: bool,
    failure: Option<String>,
}

impl Session {
    /// Starts a fresh VM from a previously fetched runtime. No download occurs.
    pub async fn start(cache: &Path, mut options: Options) -> Result<Self, Error> {
        options.validate()?;
        options.run_dir = std::path::absolute(&options.run_dir)?;
        let runtime_layer = runtime::cached(cache).await?;
        let recipe =
            vm::image::recipe(&options.image).ok_or(Error::Invalid("unknown image recipe"))?;
        let (image, info) =
            vm::image::built(recipe, cache)?.ok_or(Error::Invalid("image is not built"))?;
        let launch = vm::Launch::from_cache(cache).await?;
        let parent = options
            .run_dir
            .parent()
            .ok_or(Error::Invalid("run directory"))?;
        std::fs::create_dir_all(parent)?;
        let control = PartDir::beside(&parent.join("agent-control"))?;
        std::fs::set_permissions(control.path(), std::fs::Permissions::from_mode(0o700))?;
        let socket = control.path().join("socket");
        let listener = helpers::fs::bind_unix(&socket)?;
        listener.set_nonblocking(true)?;
        let listener = UnixListener::from_std(listener)?;
        let mut spec = vm::RunSpec::new(
            image,
            vec![runtime::NODE_PATH.into(), runtime::WRAPPER_PATH.into()],
            options.run_dir.clone(),
        );
        spec.origin = Some(vm::Origin {
            recipe: info.recipe.clone(),
            id: info.id.clone(),
        });
        spec.env = environment(info.environment(), &options.route);
        spec.uid = 1000;
        spec.gid = 1000;
        spec.cpus = options.cpus;
        spec.memory_mib = options.memory_mib;
        spec.timeout_s = options.timeout_s;
        spec.tree = options.tree.clone();
        spec.tree_out = options.tree_out.clone();
        spec.layers = options.layers.clone();
        spec.layers.push(runtime_layer.file);
        spec.layers
            .extend(options.addons.iter().map(|addon| addon.file.clone()));
        spec.ports = vec![
            vm::Port {
                guest: 8787,
                to: vm::PortTarget::Tcp(format!("127.0.0.1:{}", options.route.port)),
            },
            vm::Port {
                guest: 8788,
                to: vm::PortTarget::Unix(socket),
            },
        ];
        let mut vm = VmTask(tokio::spawn(async move { vm::run(&launch, &spec).await }));
        // From here on a VM was started: a start that fails says so in
        // agent.json, as a session that fails later does.
        let layer = &runtime_layer.info;
        let accepted = tokio::select! {
            result = listener.accept() => result.map(|(stream, _)| stream).map_err(Error::Io),
            ended = &mut vm.0 => {
                let result = match ended {
                    Ok(Ok(result)) => Some(result),
                    _ => None,
                };
                let error = Error::Protocol("VM ended before connecting");
                return Err(failed_start(&options, layer, result.as_ref(), error));
            }
            _ = tokio::time::sleep(Duration::from_secs(options.timeout_s.saturating_add(30))) => Err(Error::Timeout),
        };
        let accepted = match accepted {
            Ok(accepted) => accepted,
            Err(error) => {
                vm.0.abort();
                return Err(failed_start(&options, layer, None, error));
            }
        };
        let (reader, writer) = accepted.into_split();
        let log = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(options.run_dir.join("events.jsonl"))
        {
            Ok(log) => log,
            Err(error) => {
                vm.0.abort();
                return Err(failed_start(&options, layer, None, error.into()));
            }
        };
        let mut session = Self {
            reader: BoundedLines::new(reader, MAX_FRAME),
            writer,
            vm,
            _control: control,
            seq: 0,
            next_turn: 1,
            active: None,
            session_id: None,
            log,
            turns: vec![],
            options,
            layer: runtime_layer.info.clone(),
            failed: false,
            closing: false,
            recorded_bytes: 0,
            record_written: false,
            tools_warned: false,
            failure: None,
        };
        if let Err(error) = session.handshake().await {
            return Err(session.fail_start(error));
        }
        Ok(session)
    }

    /// A handshake that failed: the VM is stopped and the run record says
    /// `failed`, with the first reason recorded.
    fn fail_start(&mut self, error: Error) -> Error {
        self.failure.get_or_insert_with(|| error.to_string());
        self.failed = true;
        self.vm.0.abort();
        if let Err(record_error) = self.record("failed", None) {
            tracing::warn!(error = %record_error, "could not save the record of a failed agent start");
        }
        error
    }

    async fn command(&mut self, mut message: Value) -> Result<(), Error> {
        if self.failed {
            return Err(Error::Protocol("session failed"));
        }
        message["format"] = runtime::FORMAT.into();
        let mut bytes =
            serde_json::to_vec(&message).map_err(|_| Error::Protocol("command encoding"))?;
        bytes.push(b'\n');
        if bytes.len() > MAX_FRAME {
            return Err(Error::Invalid("command too large"));
        }
        let sent = tokio::time::timeout(Duration::from_secs(10), self.writer.write_all(&bytes))
            .await
            .map_err(|_| Error::Timeout)
            .and_then(|result| result.map_err(Error::Io));
        if let Err(error) = &sent {
            self.failure = Some(error.to_string());
            self.failed = true;
            self.vm.0.abort();
            self.record("failed", None)?;
        }
        sent
    }

    /// Submits one turn. Overlap is refused, including while interruption
    /// drains. The first turn's text follows the activation, if any, on a
    /// line of its own; what is sent is what `prompt-<turn>.md` holds.
    pub async fn send(&mut self, text: &str) -> Result<u64, Error> {
        if self.active.is_some() {
            return Err(Error::Invalid("a turn is still active"));
        }
        let id = self.next_turn;
        let text = match &self.options.activation {
            Some(activation) if id == 1 => format!("{activation}\n{text}"),
            _ => text.to_owned(),
        };
        if text.len() > MAX_PROMPT {
            return Err(Error::Invalid("prompt exceeds 64 KiB"));
        }
        write_whole(
            &self.options.run_dir.join(format!("prompt-{id}.md")),
            text.replace(&self.options.route.credential, "[redacted]")
                .as_bytes(),
        )?;
        self.command(json!({"op":"send", "turn":id, "text":text}))
            .await?;
        self.next_turn += 1;
        self.active = Some(Active {
            id,
            prompt_sha256: helpers::hash::sha256_bytes(text.as_bytes()),
            tools_offered: None,
            skills_offered: None,
            commands_invoked: None,
            interrupt: false,
            acknowledged: false,
            terminal: false,
        });
        Ok(id)
    }

    /// Requests interruption. Continue reading until the matching result, not
    /// merely the acknowledgement, before sending the next input.
    pub async fn interrupt(&mut self) -> Result<(), Error> {
        let active = self
            .active
            .as_ref()
            .ok_or(Error::Invalid("no active turn"))?;
        if active.interrupt {
            return Err(Error::Invalid("interrupt already requested"));
        }
        let id = active.id;
        self.command(json!({"op":"interrupt", "turn":id})).await?;
        self.active.as_mut().expect("active turn").interrupt = true;
        Ok(())
    }

    /// Reads, validates and records one event. A protocol/transport failure is
    /// terminal; subsequent calls cannot continue with a partially read frame.
    pub async fn next_event(&mut self) -> Result<Event, Error> {
        if self.failed {
            return Err(Error::Protocol("session failed"));
        }
        let result = self.receive().await;
        if let Err(error) = &result {
            self.failure = Some(error.to_string());
            self.failed = true;
            let vm_result = if matches!(error, Error::Protocol("unexpected EOF") | Error::Io(_)) {
                // The guest may have reached its own timeout. Let the existing
                // VM runner finish reading its status before aborting its task.
                match tokio::time::timeout(Duration::from_secs(5), &mut self.vm.0).await {
                    Ok(Ok(Ok(result))) => Some(result),
                    _ => None,
                }
            } else {
                None
            };
            self.vm.0.abort();
            self.record("failed", vm_result.as_ref())?;
        }
        result
    }

    async fn receive(&mut self) -> Result<Event, Error> {
        let bytes = tokio::time::timeout(
            Duration::from_secs(self.options.timeout_s.saturating_add(30)),
            self.reader.next(),
        )
        .await
        .map_err(|_| Error::Timeout)??;
        let bytes = bytes.ok_or(Error::Protocol("unexpected EOF"))?;
        self.recorded_bytes = self.recorded_bytes.saturating_add(bytes.len());
        if self.recorded_bytes > 64 * 1024 * 1024 {
            return Err(Error::Protocol("transcript exceeds 64 MiB"));
        }
        let text =
            std::str::from_utf8(&bytes).map_err(|_| Error::Protocol("event is not UTF-8"))?;
        let text = text.replace(&self.options.route.credential, "[redacted]");
        let mut event: Event =
            serde_json::from_str(&text).map_err(|_| Error::Protocol("invalid event JSON"))?;
        if event.format != runtime::FORMAT || event.seq != self.seq + 1 {
            return Err(Error::Protocol("event version or sequence"));
        }
        self.seq = event.seq;
        if let Some(id) = &event.session_id {
            if id.len() > 128 || self.session_id.as_ref().is_some_and(|old| old != id) {
                return Err(Error::Protocol("session id changed"));
            }
            self.session_id = Some(id.clone());
        }
        remove_cost(&mut event.payload);
        match event.kind.as_str() {
            "ready" if self.seq == 1 => {}
            "closed" if self.closing => {}
            "event" | "result" | "interrupted" => {
                let active = self
                    .active
                    .as_mut()
                    .ok_or(Error::Protocol("event without active turn"))?;
                if event.turn != Some(active.id) {
                    return Err(Error::Protocol("wrong turn id"));
                }
                if active.tools_offered.is_none()
                    && let Some(offered) = offered_tools(&event)?
                {
                    if !self.tools_warned && !same_tools(&offered, &self.options.tools) {
                        self.tools_warned = true;
                        tracing::warn!(
                            requested = ?self.options.tools,
                            offered = ?offered,
                            "Claude offers other tools than were asked for; agent.json records both"
                        );
                    }
                    active.tools_offered = Some(offered);
                }
                if active.skills_offered.is_none()
                    && let Some(skills) = offered_skills(&event)?
                {
                    active.skills_offered = Some(skills);
                }
                if let Some(commands) = replayed_commands(&event)? {
                    let invoked = active.commands_invoked.get_or_insert_with(Vec::new);
                    if invoked.len() + commands.len() > 64 {
                        return Err(Error::Protocol("too many replayed commands"));
                    }
                    invoked.extend(commands);
                }
                if event.kind == "interrupted" {
                    if !active.interrupt || active.acknowledged {
                        return Err(Error::Protocol("unexpected interrupt acknowledgement"));
                    }
                    active.acknowledged = true;
                }
                if event.kind == "result" {
                    if active.terminal {
                        return Err(Error::Protocol("duplicate terminal result"));
                    }
                    active.terminal = true;
                    let interrupted = event.payload["interrupted"] == true;
                    if interrupted && !active.interrupt {
                        return Err(Error::Protocol("unrequested interrupted result"));
                    }
                    // A successful SDK result is an answer whatever the flag
                    // says: an interrupt cannot take back a finished answer.
                    let succeeded =
                        event.payload["subtype"] == "success" && event.payload["is_error"] == false;
                    // The SDK's own word for a turn that reached max_turns;
                    // neither Claude's work nor kai's infrastructure.
                    let limited = event.payload["subtype"] == "error_max_turns";
                    let outcome = if succeeded {
                        "answered"
                    } else if limited {
                        "limit"
                    } else if interrupted {
                        "interrupted"
                    } else {
                        "agent-error"
                    };
                    let turn = Turn {
                        id: active.id,
                        outcome: outcome.into(),
                        prompt_sha256: active.prompt_sha256.clone(),
                        tools_offered: active.tools_offered.clone(),
                        skills_offered: active.skills_offered.clone(),
                        commands_invoked: active.commands_invoked.clone(),
                        result: event.payload.clone(),
                    };
                    if let Some(activation) = &self.options.activation
                        && turn.id == 1
                        && !invoked(&turn, activation)
                    {
                        tracing::warn!(
                            activation = activation.as_str(),
                            "Claude did not take the activation's command; the first turn went to the model as plain text"
                        );
                    }
                    self.turns.push(turn);
                }
                if active.terminal && (!active.interrupt || active.acknowledged) {
                    self.active = None;
                }
            }
            "error" => return Err(Error::Protocol("guest reported failure")),
            _ => return Err(Error::Protocol("unexpected event kind")),
        }
        let mut line = serde_json::to_vec(&event).map_err(|_| Error::Protocol("event encoding"))?;
        line.push(b'\n');
        self.log.write_all(&line)?;
        self.log.flush()?;
        Ok(event)
    }

    /// Completed turns so far, independent of the VM's eventual exit status.
    pub fn turns(&self) -> &[Turn] {
        &self.turns
    }

    /// Closes the SDK process, waits for the VM and atomically writes agent.json.
    /// Closing during a turn is cancellation, never an answer.
    pub async fn close(self) -> Result<vm::RunResult, Error> {
        self.close_with(|_| {}).await
    }

    /// Like [`Self::close`], forwarding each final event to the caller before
    /// the VM is joined. Useful for a CLI that prints the entire event stream.
    pub async fn close_with(
        mut self,
        mut event_received: impl FnMut(&Event),
    ) -> Result<vm::RunResult, Error> {
        self.closing = true;
        self.command(json!({"op":"close"})).await?;
        loop {
            let event = self.next_event().await?;
            event_received(&event);
            if event.kind == "closed" {
                break;
            }
        }
        let result = tokio::time::timeout(Duration::from_secs(30), &mut self.vm.0)
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|_| Error::Protocol("VM task failed"))??;
        self.log.sync_all()?;
        self.record("closed", Some(&result))?;
        Ok(result)
    }
    fn record(&mut self, outcome: &str, result: Option<&vm::RunResult>) -> Result<(), Error> {
        write_record(
            &self.options,
            &self.layer,
            &Record {
                session_id: self.session_id.as_deref(),
                turns: &self.turns,
                cancelled_turn: self.active.as_ref().map(|turn| turn.id),
                vm: result,
                outcome,
                reason: self.failure.as_deref(),
            },
        )?;
        self.record_written = true;
        Ok(())
    }

    /// The start's last steps, once a session exists: the wrapper is
    /// configured and must answer with the pinned versions.
    async fn handshake(&mut self) -> Result<(), Error> {
        let mut start =
            json!({"op":"start", "model":self.options.model, "tools":self.options.tools});
        // Without add-ons the command is what it was before they existed.
        if !self.options.addons.is_empty() {
            start["plugins"] = self
                .options
                .addons
                .iter()
                .map(|addon| addon.info.dir.clone())
                .collect();
        }
        if let Some(turns) = self.options.max_turns {
            start["max_turns"] = turns.into();
        }
        self.command(start).await?;
        let ready = self.receive().await?;
        if ready.kind != "ready"
            || ready.payload["sdk"] != runtime::SDK_VERSION
            || ready.payload["claude"] != runtime::CLAUDE.version
            || ready.payload["node"] != runtime::NODE_VERSION
        {
            return Err(Error::Protocol("runtime version mismatch"));
        }
        Ok(())
    }
}

/// What a run record holds besides the options and the layer.
struct Record<'a> {
    session_id: Option<&'a str>,
    turns: &'a [Turn],
    cancelled_turn: Option<u64>,
    vm: Option<&'a vm::RunResult>,
    outcome: &'a str,
    reason: Option<&'a str>,
}

/// Writes `agent.json` (kai-agent-run/1) into the run directory, whole.
fn write_record(
    options: &Options,
    layer: &crate::layer::LayerInfo,
    record: &Record<'_>,
) -> Result<(), Error> {
    let addons: Vec<&crate::addon::AddonInfo> =
        options.addons.iter().map(|addon| &addon.info).collect();
    let json = json!({"format":"kai-agent-run/1", "agent":"claude-code", "version":runtime::CLAUDE.version,
        "sdk":runtime::SDK_VERSION,"node":runtime::NODE_VERSION,"layer":layer,"model":options.model,
        "tools":options.tools,"max_turns":options.max_turns,"addons":addons,"activation":options.activation,
        "mock":options.route.mock,"route_port":options.route.port,
        "session_id":record.session_id,"turns":record.turns,"cancelled_turn":record.cancelled_turn,
        "vm":record.vm,"outcome":record.outcome,"reason":record.reason });
    write_whole(
        &options.run_dir.join("agent.json"),
        &serde_json::to_vec_pretty(&json).map_err(|_| Error::Protocol("record encoding"))?,
    )?;
    Ok(())
}

/// A start that failed after its VM was started: the run record says so,
/// with the VM's own record when the VM ended, and the start's error is
/// returned. A record that cannot be written is logged, not hidden behind
/// the start's error.
fn failed_start(
    options: &Options,
    layer: &crate::layer::LayerInfo,
    vm: Option<&vm::RunResult>,
    error: Error,
) -> Error {
    let reason = error.to_string();
    let written = std::fs::create_dir_all(&options.run_dir)
        .map_err(Error::Io)
        .and_then(|()| {
            write_record(
                options,
                layer,
                &Record {
                    session_id: None,
                    turns: &[],
                    cancelled_turn: None,
                    vm,
                    outcome: "failed",
                    reason: Some(&reason),
                },
            )
        });
    if let Err(record_error) = written {
        tracing::warn!(error = %record_error, "could not save the record of a failed agent start");
    }
    error
}

impl Drop for Session {
    fn drop(&mut self) {
        self.vm.0.abort();
        if !self.record_written && self.record("dropped", None).is_err() {
            tracing::warn!("could not save incomplete agent session record");
        }
    }
}

fn environment(mut env: BTreeMap<String, String>, route: &Route) -> BTreeMap<String, String> {
    // Image configuration is not allowed to inject alternate authentication.
    env.retain(|k, _| {
        !k.starts_with("ANTHROPIC_")
            && !k.starts_with("CLAUDE_")
            && !matches!(k.as_str(), "NODE_OPTIONS" | "NODE_PATH")
    });
    for (key, value) in [
        ("HOME", "/job/home"),
        ("CLAUDE_CONFIG_DIR", "/job/home/.claude"),
        ("TMPDIR", "/tmp"),
        ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1"),
        ("DISABLE_AUTOUPDATER", "1"),
        ("ANTHROPIC_BASE_URL", "http://127.0.0.1:8787"),
        ("ANTHROPIC_API_KEY", &route.credential),
    ] {
        env.insert(key.into(), value.into());
    }
    env
}

/// The tools a turn's `system` `init` event names, when `event` is one.
/// A list that is not a short array of short strings breaks the protocol.
fn offered_tools(event: &Event) -> Result<Option<Vec<String>>, Error> {
    init_list(
        event,
        "tools",
        Error::Protocol("init event without a valid tool list"),
    )
}

/// The skills a turn's `system` `init` event names, when `event` is one
/// that names them. A list that is not a short array of short strings
/// breaks the protocol.
fn offered_skills(event: &Event) -> Result<Option<Vec<String>>, Error> {
    if event.payload.get("skills").is_none() {
        return Ok(None);
    }
    init_list(
        event,
        "skills",
        Error::Protocol("init event without a valid skill list"),
    )
}

/// The list `field` of a `system` `init` event, or `None` for any other
/// event; `invalid` when it is not at most 256 strings of 1 to 128 bytes.
fn init_list(event: &Event, field: &str, invalid: Error) -> Result<Option<Vec<String>>, Error> {
    let payload = &event.payload;
    if event.kind != "event" || payload["type"] != "system" || payload["subtype"] != "init" {
        return Ok(None);
    }
    let Some(names) = payload[field].as_array().filter(|names| names.len() <= 256) else {
        return Err(invalid);
    };
    let mut list = Vec::with_capacity(names.len());
    for name in names {
        match name.as_str() {
            Some(name) if !name.is_empty() && name.len() <= 128 => list.push(name.to_owned()),
            _ => return Err(invalid),
        }
    }
    Ok(Some(list))
}

/// The slash commands a user message Claude replays names in its
/// `<command-name>` tags, when `event` is such a message: empty when it
/// names none. A tag that holds no plain command breaks the protocol.
fn replayed_commands(event: &Event) -> Result<Option<Vec<String>>, Error> {
    let payload = &event.payload;
    if event.kind != "event" || payload["type"] != "user" || payload["isReplay"] != true {
        return Ok(None);
    }
    let content = &payload["message"]["content"];
    let texts: Vec<&str> = match content {
        Value::String(text) => vec![text.as_str()],
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str())
            .collect(),
        _ => vec![],
    };
    const OPEN: &str = "<command-name>";
    let invalid = || Error::Protocol("replayed command without a plain name");
    let mut commands = vec![];
    for text in texts {
        let mut rest = text;
        while let Some(at) = rest.find(OPEN) {
            let (name, after) = rest[at + OPEN.len()..]
                .split_once("</command-name>")
                .ok_or_else(invalid)?;
            if name.len() > 128 || command(name) != Some(name) {
                return Err(invalid());
            }
            commands.push(name.to_owned());
            rest = after;
        }
    }
    Ok(Some(commands))
}

/// Whether Claude took the slash command `activation` starts with in
/// `turn`, as the user messages it replayed name it: that command, or the
/// plugin's skill a short one resolved to (`/ponytail` replayed as
/// `/ponytail:ponytail`).
pub fn invoked(turn: &Turn, activation: &str) -> bool {
    let Some(name) = command(activation) else {
        return false;
    };
    let resolved = |invoked: &str| {
        invoked
            .split_once(':')
            .is_some_and(|(_, skill)| format!("/{skill}") == name)
    };
    turn.commands_invoked.as_ref().is_some_and(|commands| {
        commands
            .iter()
            .any(|invoked| invoked == name || resolved(invoked))
    })
}

/// Whether two tool lists name the same tools, in any order.
fn same_tools(offered: &[String], requested: &[String]) -> bool {
    let mut offered: Vec<&str> = offered.iter().map(String::as_str).collect();
    let mut requested: Vec<&str> = requested.iter().map(String::as_str).collect();
    offered.sort_unstable();
    offered.dedup();
    requested.sort_unstable();
    requested.dedup();
    offered == requested
}

fn remove_cost(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("total_cost_usd");
            map.remove("cost_usd");
            map.remove("costUSD");
            map.remove("totalCostUSD");
            for value in map.values_mut() {
                remove_cost(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                remove_cost(value);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixStream;

    #[test]
    fn routes_reject_keys_and_urls_without_echoing_them() {
        let secret = "invented-secret-canary";
        for url in [
            "https://127.0.0.1:2",
            "http://remote:2",
            "http://localhost:2/path",
            "http://localhost:0",
            "http://localhost:2?x=1",
            "http://localhost:2//",
        ] {
            assert!(Route::new(url, Some(secret), true).is_err());
        }
        for key in [
            secret.to_owned(),
            format!("prxy-{}", "A".repeat(32)),
            format!("prxy-{} ", "a".repeat(32)),
        ] {
            let err = Route::new("http://localhost:2/", Some(&key), false)
                .err()
                .unwrap();
            assert!(!err.to_string().contains(&key));
        }
        let normal = Route::new(
            "http://localhost:2/",
            Some(&format!("prxy-{}", "a".repeat(32))),
            false,
        )
        .unwrap();
        assert!(!normal.mock);
        let mock = Route::new("http://localhost:2/", Some(secret), true).unwrap();
        assert_eq!(mock.credential, "dummy-not-a-key");
        let env = environment(
            BTreeMap::from([
                ("ANTHROPIC_AUTH_TOKEN".into(), secret.into()),
                ("CLAUDE_CODE_OAUTH_TOKEN".into(), secret.into()),
                ("NODE_OPTIONS".into(), secret.into()),
            ]),
            &mock,
        );
        assert!(!format!("{env:?}").contains(secret));
        assert_eq!(env["HOME"], "/job/home");
    }

    fn fixture() -> (Session, UnixStream) {
        let control = PartDir::beside(&std::env::temp_dir().join("kai-session-unit")).unwrap();
        let dir = control.path().to_path_buf();
        let options = Options::new(
            "fixture".into(),
            Route::new("http://localhost:1", None, true).unwrap(),
            dir.clone(),
        );
        let (one, two) = UnixStream::pair().unwrap();
        let (reader, writer) = one.into_split();
        let layer = crate::layer::LayerInfo {
            format: crate::layer::LAYER_FORMAT.into(),
            key: "fixture".into(),
            agent: "fixture".into(),
            version: "1".into(),
            binary: "fixture".into(),
            binary_len: 1,
            path: "/fixture".into(),
            layer: "fixture".into(),
            layer_len: 1,
            packing: vec![],
            mksquashfs: "fixture".into(),
            members: vec![],
        };
        (
            Session {
                reader: BoundedLines::new(reader, MAX_FRAME),
                writer,
                vm: VmTask(tokio::spawn(std::future::pending())),
                _control: control,
                seq: 1,
                next_turn: 1,
                active: None,
                session_id: None,
                log: std::fs::File::create(dir.join("events.jsonl")).unwrap(),
                turns: vec![],
                options,
                layer,
                failed: false,
                closing: false,
                recorded_bytes: 0,
                record_written: false,
                tools_warned: false,
                failure: None,
            },
            two,
        )
    }
    async fn event(peer: &mut UnixStream, seq: u64, kind: &str, payload: Value) {
        let mut text = json!({"format":runtime::FORMAT,"seq":seq,"kind":kind,"turn":1,"session_id":"fixture-session","payload":payload}).to_string();
        text.push('\n');
        peer.write_all(text.as_bytes()).await.unwrap();
    }
    #[tokio::test]
    async fn interrupt_receipt_does_not_free_the_turn_and_cost_is_not_recorded() {
        let (mut session, mut peer) = fixture();
        assert_eq!(session.send("fixture prompt").await.unwrap(), 1);
        assert!(session.send("overlap").await.is_err());
        session.interrupt().await.unwrap();
        event(&mut peer, 2, "interrupted", json!({})).await;
        session.next_event().await.unwrap();
        assert!(session.send("too early").await.is_err());
        event(
            &mut peer,
            3,
            "result",
            json!({"subtype":"error_during_execution","is_error":true,"interrupted":true,"total_cost_usd":2}),
        )
        .await;
        session.next_event().await.unwrap();
        assert_eq!(session.turns[0].outcome, "interrupted");
        assert!(session.turns[0].result.get("total_cost_usd").is_none());
        assert_eq!(session.send("next turn").await.unwrap(), 2);
    }
    #[tokio::test]
    async fn an_out_of_order_event_is_terminal_and_stops_the_vm() {
        let (mut session, mut peer) = fixture();
        session.send("fixture").await.unwrap();
        event(&mut peer, 99, "event", json!({})).await;
        assert!(session.next_event().await.is_err());
        assert!(session.failed);
        assert!(session.send("cannot continue").await.is_err());
        tokio::task::yield_now().await;
        assert!(session.vm.0.is_finished());
    }
    #[tokio::test]
    async fn a_changed_session_id_or_unrequested_ack_is_rejected() {
        let (mut session, mut peer) = fixture();
        session.send("fixture").await.unwrap();
        event(&mut peer, 2, "interrupted", json!({})).await;
        assert!(session.next_event().await.is_err());
        let (mut session, mut peer) = fixture();
        session.send("fixture").await.unwrap();
        session.session_id = Some("another-session".into());
        event(&mut peer, 2, "event", json!({})).await;
        assert!(session.next_event().await.is_err());
    }
    #[tokio::test]
    async fn a_completed_result_crossing_interrupt_waits_for_its_late_receipt() {
        let (mut session, mut peer) = fixture();
        session.send("fixture").await.unwrap();
        session.interrupt().await.unwrap();
        event(
            &mut peer,
            2,
            "result",
            json!({"subtype":"success","is_error":false,"interrupted":false}),
        )
        .await;
        session.next_event().await.unwrap();
        assert!(session.send("not drained yet").await.is_err());
        event(&mut peer, 3, "interrupted", json!({"completed":true})).await;
        session.next_event().await.unwrap();
        assert_eq!(session.turns()[0].outcome, "answered");
        assert_eq!(session.send("next").await.unwrap(), 2);
    }
    #[tokio::test]
    async fn a_successful_result_after_an_interrupt_request_is_an_answer() {
        let (mut session, mut peer) = fixture();
        session.send("fixture").await.unwrap();
        session.interrupt().await.unwrap();
        event(&mut peer, 2, "interrupted", json!({})).await;
        session.next_event().await.unwrap();
        // As an earlier wrapper sent it: the flag set because an interrupt
        // was requested, although the SDK finished the turn.
        event(
            &mut peer,
            3,
            "result",
            json!({"subtype":"success","is_error":false,"interrupted":true}),
        )
        .await;
        session.next_event().await.unwrap();
        assert_eq!(session.turns()[0].outcome, "answered");
        assert_eq!(session.send("next").await.unwrap(), 2);
    }
    #[tokio::test]
    async fn the_tools_claude_offers_are_recorded_beside_those_asked_for() {
        let (mut session, mut peer) = fixture();
        session.send("fixture").await.unwrap();
        event(
            &mut peer,
            2,
            "event",
            json!({"type":"system","subtype":"init","tools":["Bash","Edit","Read"]}),
        )
        .await;
        session.next_event().await.unwrap();
        // A second init in the same turn does not replace the first.
        event(
            &mut peer,
            3,
            "event",
            json!({"type":"system","subtype":"init","tools":["Fixture"]}),
        )
        .await;
        session.next_event().await.unwrap();
        event(
            &mut peer,
            4,
            "result",
            json!({"subtype":"success","is_error":false}),
        )
        .await;
        session.next_event().await.unwrap();
        assert!(session.tools_warned);
        session.send("fixture without init").await.unwrap();
        let mut text = json!({"format":runtime::FORMAT,"seq":5,"kind":"result","turn":2,
            "session_id":"fixture-session","payload":{"subtype":"success","is_error":false}})
        .to_string();
        text.push('\n');
        peer.write_all(text.as_bytes()).await.unwrap();
        session.next_event().await.unwrap();
        session.record("closed", None).unwrap();
        let saved = saved_record(&session);
        assert_eq!(saved["tools"].as_array().unwrap().len(), TOOLS.len());
        assert_eq!(
            saved["turns"][0]["tools_offered"],
            json!(["Bash", "Edit", "Read"])
        );
        assert_eq!(saved["turns"][1]["tools_offered"], Value::Null);
    }
    #[tokio::test]
    async fn an_init_event_without_a_tool_list_breaks_the_protocol() {
        for tools in [json!(null), json!("Bash"), json!([1]), json!([""])] {
            let (mut session, mut peer) = fixture();
            session.send("fixture").await.unwrap();
            event(
                &mut peer,
                2,
                "event",
                json!({"type":"system","subtype":"init","tools":tools}),
            )
            .await;
            assert!(session.next_event().await.is_err());
            assert!(session.failed);
        }
    }
    #[test]
    fn tool_lists_compare_as_sets() {
        let list = |names: &[&str]| names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
        assert!(same_tools(
            &list(&["Read", "Bash"]),
            &list(&["Bash", "Read"])
        ));
        assert!(!same_tools(&list(&["Bash", "Edit", "Read"]), &list(TOOLS)));
        assert!(same_tools(&[], &[]));
    }
    #[test]
    fn an_activation_is_one_line_that_starts_with_a_plain_slash_command() {
        assert_eq!(command("/ponytail full"), Some("/ponytail"));
        assert_eq!(command("/ponytail:ponytail"), Some("/ponytail:ponytail"));
        for bad in [
            "",
            "/",
            "ponytail full",
            "/pony tail",
            "/ponytail full\ninjected",
            "/pony$tail",
            " /ponytail",
        ] {
            assert_ne!(command(bad), Some("/ponytail"), "{bad:?}");
        }
        assert!(command("/pony$tail").is_none() && command("/ponytail\rfull").is_none());
        assert!(command(&format!("/ponytail {}", "x".repeat(300))).is_none());
        let route = Route::new("http://localhost:1", None, true).unwrap();
        let mut options = Options::new("fixture".into(), route, "/fixture".into());
        options.activation = Some("ponytail full".into());
        assert!(options.validate().is_err());
    }
    /// A user message Claude replays, as the wrapper forwards it.
    fn replayed(content: Value) -> Value {
        json!({"type":"user","isReplay":true,"message":{"role":"user","content":content}})
    }
    #[tokio::test]
    async fn the_activation_goes_before_the_first_turn_only_and_its_command_is_recorded() {
        let (mut session, mut peer) = fixture();
        session.options.activation = Some("/fixture full".into());
        session.send("fixture task").await.unwrap();
        let sent = std::fs::read_to_string(session.options.run_dir.join("prompt-1.md")).unwrap();
        assert_eq!(sent, "/fixture full\nfixture task");
        event(
            &mut peer,
            2,
            "event",
            json!({"type":"system","subtype":"init","tools":[],"skills":["fixture:fixture","other"]}),
        )
        .await;
        event(
            &mut peer,
            3,
            "event",
            replayed(json!(
                "<command-message>fixture</command-message>\n<command-name>/fixture</command-name>\n<command-args>full</command-args>"
            )),
        )
        .await;
        event(
            &mut peer,
            4,
            "result",
            json!({"subtype":"success","is_error":false}),
        )
        .await;
        for _ in 0..3 {
            session.next_event().await.unwrap();
        }
        let first = &session.turns()[0];
        assert_eq!(
            first.skills_offered,
            Some(vec!["fixture:fixture".into(), "other".into()])
        );
        assert_eq!(first.commands_invoked, Some(vec!["/fixture".into()]));
        assert!(invoked(first, "/fixture full"));
        assert!(!invoked(first, "/other"));
        // A short command Claude resolved to a plugin's skill.
        let resolved = Turn {
            commands_invoked: Some(vec!["/plugin:fixture".into()]),
            ..first.clone()
        };
        assert!(invoked(&resolved, "/fixture full"));
        assert!(invoked(&resolved, "/plugin:fixture"));
        assert!(!invoked(&resolved, "/plugin"));

        // The second turn: no activation, a replay that names no command.
        session.send("fixture follow-up").await.unwrap();
        let sent = std::fs::read_to_string(session.options.run_dir.join("prompt-2.md")).unwrap();
        assert_eq!(sent, "fixture follow-up");
        for (seq, kind, payload) in [
            (
                5,
                "event",
                replayed(json!([{"type":"text","text":"fixture follow-up"}])),
            ),
            (6, "result", json!({"subtype":"success","is_error":false})),
        ] {
            let mut text = json!({"format":runtime::FORMAT,"seq":seq,"kind":kind,"turn":2,
                "session_id":"fixture-session","payload":payload})
            .to_string();
            text.push('\n');
            peer.write_all(text.as_bytes()).await.unwrap();
            session.next_event().await.unwrap();
        }
        let second = &session.turns()[1];
        assert_eq!(second.commands_invoked, Some(vec![]));
        assert_eq!(second.skills_offered, None);
        session.record("closed", None).unwrap();
        let saved = saved_record(&session);
        assert_eq!(saved["activation"], "/fixture full");
        assert_eq!(saved["addons"], json!([]));
        assert_eq!(saved["turns"][0]["commands_invoked"], json!(["/fixture"]));
        assert_eq!(saved["turns"][1]["skills_offered"], Value::Null);
    }
    #[tokio::test]
    async fn a_replayed_command_that_is_no_plain_name_breaks_the_protocol() {
        for content in [
            json!("<command-name>/fixture full</command-name>"),
            json!("<command-name>/fixture"),
            json!([{"type":"text","text":"<command-name>fixture</command-name>"}]),
        ] {
            let (mut session, mut peer) = fixture();
            session.send("fixture").await.unwrap();
            event(&mut peer, 2, "event", replayed(content)).await;
            assert!(session.next_event().await.is_err());
            assert!(session.failed);
        }
        // A user message that is not a replay, such as a tool's result, is
        // not read for commands.
        let (mut session, mut peer) = fixture();
        session.send("fixture").await.unwrap();
        event(
            &mut peer,
            2,
            "event",
            json!({"type":"user","message":{"content":"<command-name>/x y</command-name>"}}),
        )
        .await;
        session.next_event().await.unwrap();
    }
    fn saved_record(session: &Session) -> Value {
        serde_json::from_slice(&std::fs::read(session.options.run_dir.join("agent.json")).unwrap())
            .unwrap()
    }
    #[tokio::test]
    async fn a_start_that_fails_after_the_vm_started_leaves_a_failed_record() {
        let (session, _peer) = fixture();
        let error = failed_start(&session.options, &session.layer, None, Error::Timeout);
        assert!(matches!(error, Error::Timeout));
        let saved = saved_record(&session);
        assert_eq!(saved["outcome"], "failed");
        assert_eq!(saved["reason"], "session timed out");
        assert_eq!(saved["turns"], json!([]));
        assert_eq!(saved["vm"], Value::Null);
    }
    #[tokio::test]
    async fn a_handshake_with_other_versions_fails_with_a_record() {
        let (mut session, mut peer) = fixture();
        session.seq = 0;
        let mut ready = json!({"format":runtime::FORMAT,"seq":1,"kind":"ready","payload":{"sdk":"0.0.0","claude":"0","node":"0"}}).to_string();
        ready.push('\n');
        peer.write_all(ready.as_bytes()).await.unwrap();
        let error = session.handshake().await.unwrap_err();
        let error = session.fail_start(error);
        assert!(matches!(error, Error::Protocol("runtime version mismatch")));
        assert!(session.record_written && session.failed);
        tokio::task::yield_now().await;
        assert!(session.vm.0.is_finished());
        let saved = saved_record(&session);
        assert_eq!(saved["outcome"], "failed");
        assert_eq!(
            saved["reason"],
            "session protocol failed: runtime version mismatch"
        );
    }
    /// The start command the handshake sends, read on the guest's side.
    async fn start_command(session: &mut Session, peer: &mut UnixStream) -> Value {
        use tokio::io::AsyncBufReadExt as _;
        session.seq = 0;
        let mut ready = json!({"format":runtime::FORMAT,"seq":1,"kind":"ready","payload":{
            "sdk":runtime::SDK_VERSION,"claude":runtime::CLAUDE.version,"node":runtime::NODE_VERSION}})
        .to_string();
        ready.push('\n');
        peer.write_all(ready.as_bytes()).await.unwrap();
        session.handshake().await.unwrap();
        let mut line = String::new();
        tokio::io::BufReader::new(peer)
            .read_line(&mut line)
            .await
            .unwrap();
        serde_json::from_str(&line).unwrap()
    }
    #[tokio::test]
    async fn a_turn_limit_goes_into_start_only_when_given() {
        let (mut session, mut peer) = fixture();
        let start = start_command(&mut session, &mut peer).await;
        assert!(start.get("max_turns").is_none() && start.get("plugins").is_none());
        let (mut session, mut peer) = fixture();
        session.options.max_turns = Some(3);
        let start = start_command(&mut session, &mut peer).await;
        assert_eq!(start["max_turns"], 3);
    }
    #[test]
    fn a_turn_limit_is_one_or_more() {
        let options = |max_turns| {
            let route = Route::new("http://localhost:1", None, true).unwrap();
            let mut options = Options::new("fixture".into(), route, "/fixture".into());
            options.max_turns = max_turns;
            options
        };
        // The SDK takes 0 as no limit at all.
        assert!(options(Some(0)).validate().is_err());
        assert!(options(Some(MAX_TURNS + 1)).validate().is_err());
        for fine in [None, Some(1), Some(40), Some(MAX_TURNS)] {
            assert!(options(fine).validate().is_ok(), "{fine:?}");
        }
    }
    #[tokio::test]
    async fn a_turn_that_reaches_the_limit_is_its_own_outcome() {
        let (mut session, mut peer) = fixture();
        session.options.max_turns = Some(1);
        session.send("fixture").await.unwrap();
        // As Claude Code 2.1.323 ends a turn at its limit.
        event(
            &mut peer,
            2,
            "result",
            json!({"subtype":"error_max_turns","is_error":true,"terminal_reason":"max_turns",
                "errors":["Reached maximum number of turns (1)"],"num_turns":2,"interrupted":false}),
        )
        .await;
        session.next_event().await.unwrap();
        assert_eq!(session.turns()[0].outcome, "limit");
        // The session goes on: the limit holds per turn.
        assert_eq!(session.send("next").await.unwrap(), 2);
        session.record("closed", None).unwrap();
        let saved = saved_record(&session);
        assert_eq!(saved["max_turns"], 1);
        assert_eq!(saved["turns"][0]["outcome"], "limit");
        let (mut session, _peer) = fixture();
        session.record("closed", None).unwrap();
        assert_eq!(saved_record(&session)["max_turns"], Value::Null);
    }
    #[tokio::test]
    async fn the_run_record_is_the_documented_example() {
        let (mut session, _peer) = fixture();
        session.record("dropped", None).unwrap();
        let saved: Value = serde_json::from_slice(
            &std::fs::read(session.options.run_dir.join("agent.json")).unwrap(),
        )
        .unwrap();
        let expected: Value = serde_json::from_str(include_str!(
            "../../../contracts/kai-agent-run-1.example.json"
        ))
        .unwrap();
        assert_eq!(saved, expected);
        let ready: Event = serde_json::from_str(include_str!(
            "../../../contracts/kai-agent-session-1.example.json"
        ))
        .unwrap();
        assert_eq!(ready.format, runtime::FORMAT);
        assert_eq!(ready.payload["sdk"], runtime::SDK_VERSION);
    }
    #[tokio::test]
    async fn known_route_credentials_are_redacted_even_from_guest_map_keys() {
        let (mut session, mut peer) = fixture();
        session.send("text dummy-not-a-key").await.unwrap();
        event(&mut peer, 2, "result", json!({"subtype":"success","is_error":false,"dummy-not-a-key":"dummy-not-a-key","modelUsage":{"fixture":{"costUSD":0.1}}})).await;
        let received = session.next_event().await.unwrap();
        let text = serde_json::to_string(&received).unwrap();
        assert!(!text.contains("dummy-not-a-key") && !text.contains("costUSD"));
        let prompt = std::fs::read_to_string(session.options.run_dir.join("prompt-1.md")).unwrap();
        assert!(!prompt.contains("dummy-not-a-key"));
    }
}
