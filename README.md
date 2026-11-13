# kai - kombuse auto improve

kai runs the same coding task twice: once without additional instructions
and once with an extra prompt or a pinned add-on such as Ponytail. The two
Claude sessions run in separate microVMs, in parallel by default, and keep
their answers and generated files in separate result directories.

eval-reporter and Bench Board show both results with token usage, recorded
proxy costs and individual runtimes. There is no grader or automatic winner.
The repository also includes GitHub ticket import and qualification rules,
VM management, and dependency preparation for Rust repositories.

## What works today

```sh
cargo run                           # prints: hello kai
cargo run -- --help                 # the options and commands
cargo run -- --log-level debug      # log lines on standard error
cargo run -- github sync --board <owner/name>   # a board's issues into the store
cargo run -- ticket list            # the tickets in the store
cargo run -- ticket qualify         # whether kai may use each, and why not
cargo run -- store status           # per board: issues kept, when last synced, when last in full
cargo run -- store reset            # empties the store; the next sync fetches everything
cargo run -- vm doctor              # what running a microVM needs here, and what fixes each gap
cargo run -- vm fetch               # the pinned Firecracker and guest kernel into kai's cache
cargo run -- vm image build rust    # the image of kai's Rust recipe, with Docker
cargo run -- vm image list          # the recipes, and which images are built
cargo run -- vm run --image rust -- cargo --version   # one command in a fresh microVM
cargo run -- ticket deps <number>   # a ticket's repository and crates, fetched and built in microVMs
cargo run -- agent fetch            # the pinned Claude/SDK/Node runtime for a microVM
cargo run -- agent fetch --addon ponytail   # Ponytail's skill, pinned, as a read-only layer
cargo run -- agent compare --model <id> --extra-prompt "Use one HTML file." --output ./comparison -- "Build an HTML calculator."
cargo run -- run ponytail           # the named pair in .runs/ponytail/task.json
```

### One task, with and without an extra prompt

Save a comparison as `<name>/task.json`. In the first terminal, start the
proxy and leave it running:

```sh
./tools/start-proxy
```

In a second terminal in this repository:

```sh
cargo run -- run ponytail
```

The script builds the current binary, obtains the provider credential
through the existing `kai secrets exec` command on PATH, and starts
`prxy run` with `~/.prxy/spend.toml` on a free loopback port. It requires
that existing secrets command, a credential grant for prxy, and the
policy to be prepared. No token needs to be copied. Ctrl-C in the first
terminal stops the proxy. Each invocation of `run` still starts a fresh pair.

The private handoff defaults to `.kai/proxy.json`, configurable as
`[proxy] connection_file` relative to `kai.toml`. It holds only the
loopback endpoint and temporary proxy placeholder, never a provider key.
Kai accepts it only while its publishing process holds the file lock;
normal shutdown removes it, and a stale file after a crash is refused.
An explicit `ANTHROPIC_BASE_URL` selects the environment route and requires
its matching `ANTHROPIC_API_KEY`; remove old endpoint exports to use the
automatic connection. A key alone (for example from `.env`) is ignored
when using the handoff; it is never mixed with the published endpoint.
Mock mode always requires its explicit environment route.

The included [.runs/ponytail/task.json](.runs/ponytail/task.json) compares the same
HTML-calculator task on Sonnet 5.5, once plain and once with the pinned
Ponytail add-on activated by `/ponytail full`. Both run in parallel. The
runtime, image and add-on must already be prepared (`kai agent fetch`,
`kai vm image build rust`, `kai agent fetch --addon ponytail`), and the
external proxy route is supplied by the running script or the environment.

The lookup directory is configurable in `kai.toml`:

```toml
[runs]
directory = ".runs"
```

It is relative to that config file, so `kai run ponytail` also works from
a directory below it. Without a setting, `.runs` is the default. The name
is the task directory's name and may contain letters, digits, `-` and `_`.

A run file requires `model`, `prompt`, and exactly one of `extra_prompt`
or `extra_prompt_file`. Optional fields are `tree`, `max_turns`,
`sequential` (default false), `addon` and `arms`. File paths are relative to the
JSON file. With `addon`, the extra prompt is its activation slash command;
only the second VM mounts that add-on. Unknown fields are errors.
Each invocation writes to `<task>/results/<timestamp>/`; the timestamp
is Unix seconds followed by nine fractional digits. The `arms` object
names the two output directories, defaulting to `without` and `with`:

```json
"arms": {"without": "plain", "with": "ponytail"}
```

Names must be different, contain only letters, digits, `-` and `_`, and
must not be `input`, which is reserved for the starting files. For the
included task the result is:

```text
.runs/ponytail/
  task.json
  results/<timestamp>/
    pair.json
    input/
    plain/
      tree/
      answer.md
      status.txt
      run/
    ponytail/
      tree/
      answer.md
      status.txt
      run/
```

Generated results under `.runs` are gitignored. `--output DIR` overrides
the result directory. `--cache DIR` overrides the VM cache and
`--mock` selects an explicitly supplied local fake API.

For an ad-hoc comparison, pass the same inputs directly:

`kai agent compare` runs the same task twice: once as given, once with
the extra instructions before it. Both runs use the same model and
settings, each in a fresh microVM with its own home and work tree. They
run **in parallel** by default; `--sequential` runs one after the other
on a smaller machine. The default pair uses two CPUs and 4 GiB per VM.

```sh
kai agent compare --model <id> \
  --extra-prompt "Use a single HTML file with embedded CSS and JavaScript." \
  --output ./comparison -- "Build an HTML calculator."
```

Use `--extra-prompt-file instructions.txt` instead of `--extra-prompt` for
instructions in a file. `--tree ./starter` supplies starting files; they
are copied once before either run starts, and both VMs receive that copy.
Without `--tree`, both start empty. `--output` must be a new directory
outside the starting tree; when omitted, kai creates one in its VM cache.

The command prints the result paths. Open each generated application
separately, such as `comparison/without/tree/index.html` and
`comparison/with/tree/index.html` for the calculator:

```text
comparison/
  pair.json          frozen inputs, arm names and execution times
  input/             the shared starting files
  without/
    tree/            files produced without the extra prompt
    answer.md        Claude's final text, when it returned one
    status.txt       completed, incomplete, failed or interrupted
    run/             the normal agent and VM records
  with/
    tree/            files produced with the extra prompt
    answer.md
    status.txt
    run/
```

There is no grader or automatic winner. A failure on one side leaves the
other side running and keeps its result; the command exits 0 only when
both runs finish successfully. Ctrl-C or SIGTERM stops both VMs and keeps
the records already written; an interrupted run may have no exported tree.

As with `agent run`, first fetch the runtime and build the image, then
start `tools/start-proxy` or supply the external proxy's `ANTHROPIC_BASE_URL`
and placeholder `ANTHROPIC_API_KEY`. The proxy owns spending approval and limits. `--mock`
uses a local fake API instead. Parallel runs, separate generated files,
prompt delivery and partial failure are tested with that fake API in
real microVMs. A live HTML-calculator pair has also completed through the
external proxy with separate exported HTML files; this is not a quality score.

### View comparison metrics

New comparisons also write [pair.json](contracts/kai-pair-1.md), a frozen
record of the task, arm names and separate execution times. A terminal
[report-evidence.json](contracts/kai-report-evidence-1.md) binds the pair
and available agent files by SHA-256. Reporter 0.9 verifies this handoff
and shows its provenance in Bench Board's local run details; 0.8 can read
the pair without verifying the handoff. Collect one result directory:

```sh
eval-reporter collect kai-run .runs/ponytail/results/<timestamp> --codebase none --send
```

The reporter's existing watcher can discover these results automatically.
In its policy, add `[kai_run]` with `runs = ["/absolute/path/to/task/results"]`,
`codebase = "none"` and `auto_send = true`. Sending also requires the global
`auto_send` setting and a configured sink. Run `eval-reporter watch`.
Bench Board displays both answers, result paths, input/output/cache tokens,
settled proxy costs and individual runtimes. Unknown values stay unknown;
there is no grader or winner. Old results without `pair.json` are not imported.

### Tickets

A ticket is an issue on a board repository, made with an issue form whose
fields are Instruction, Repo, Ref (a full commit sha), Check and Hidden
tests, and labelled `kai`. `kai github sync` reads the board's issues
changed since the last sync into the store, `.kai/store.sqlite` beside
`kai.toml` (`--store` names another); `--full` reads all of them and
removes those GitHub no longer returns. `--actor-permissions` also asks
GitHub what access each account that put a label on has to the board now,
one request per account, and keeps the answer, or GitHub's words when it
does not tell; `kai store status` shows them, and the rule `trust-gate`
judges by them. It never writes to GitHub. The
token is `KAI_GITHUB_TOKEN`, else what `gh auth token` prints; a read-only
token is enough.

The store keeps the issues as GitHub said them, body and all, and kai
reads each body as the form when it uses the ticket, so a change to how
the form is read needs no sync. A store kai makes, and the directory it
makes for it, are readable by their owner only: they hold the issues'
text. A sync fetches at most 100 labels and 100 labeled events of an
issue, and the store records whether that was all of them.

`kai ticket qualify` judges each ticket from what GitHub said: refused when
a rule fails (no `kai` label, the label put on last by an account
without write access to the board, a closed issue, not the form, an empty
instruction, a code block in it, a repository or ref that is malformed, no
hidden tests), undecided while a rule is not checked (whether the
repository has the commit and the hidden tests, for every ticket yet; who
put the label on, while that account's access was not asked or GitHub did
not tell it), and qualified only when every rule was checked and met. So
today no ticket is qualified.

### microVMs

What a microVM needs on the host:

- Linux on x86_64, and read and write access to `/dev/kvm`; no root.
- e2fsprogs 1.47.1 or newer (`mke2fs`, `debugfs`, `e2fsck`),
  squashfs-tools and tar.
- Docker, only to build an image.
- fence 0.3.0 or newer on `PATH`, only for `--proxy`.

`kai vm doctor` checks each of them and names the fix for what is missing.
Once per machine, then a first run:

```sh
cargo run -- vm doctor              # exits with 1 while a need is not met
cargo run -- vm fetch               # downloads Firecracker and the guest kernel
cargo run -- vm image build rust    # builds the Rust image with Docker
cargo run -- vm run --image rust -- cargo --version
```

The last line boots a fresh VM on the image, prints what `cargo --version`
wrote there (`cargo 1.98.1 (…)`) and exits with its status.

`kai vm fetch` downloads Firecracker v1.18.0 from its GitHub release and
the guest kernel 6.1.155 from Firecracker's CI bucket, checks each against
the SHA-256 pinned in [crates/vm/src/pins.rs](crates/vm/src/pins.rs) and
keeps only what matches, in `~/.cache/kai/vm` unless `--cache` or
`vm.cache` says otherwise. Nothing else in kai downloads them.

Images come from recipes kai ships, one Dockerfile each in
[images/](images/): today `rust`, the official slim Rust image at a pinned
digest with git, pkg-config and CA certificates from a dated Debian
snapshot. `kai vm image build rust` builds it with Docker, packs its file
system as squashfs into the cache, keeps its environment beside it, checks
that it boots, and prints its id; building it again does nothing. Only
building needs Docker.

`kai vm run --image <recipe, image id or file> -- <command>` boots a fresh
VM on the image (read-only, with an overlay on top), runs the command there
as root with the image's environment (`--env` adds; an image file brings
only `PATH`), prints what it wrote to each stream, and exits with its
status; with 125 when it has none, such as at the time limit (`--timeout`,
600 s). The guest has no network device. Each run leaves a directory, by
default under `runs/` in the cache, with `result.json`
([kai-vm-run/1](contracts/kai-vm-run-1.md)), the output, the console and the
job drive.

`--tree <dir>` copies a directory in as the work tree, the command's
directory (`/job/tree`); the directory itself is never changed, and
`--tree-out <dir>` gets the tree as the command left it. `--file <host
path>=<path>` puts a file under `/job`, beside the tree. `--upper-out
<file>` keeps what the command wrote outside its tree as a layer, when it
exits with 0; `--layer <file>` lays such a layer over the image of a later
run, read-only, deletions included. `--user <uid>:<gid>` runs the command
as another user, who owns the tree and a home, `/job/home`, and nothing
else: `job.json` and the files of `--file` are then root's, readable as
their modes allow; `--disk` sizes the job drive.

The guest's only ways out are the ones a run names. `--port
<n>=unix:<path>` or `<n>=tcp:<host>:<port>` lets the command reach a socket
or address of this machine at the guest's `127.0.0.1:<n>` (over vsock).
`--proxy <host>:<port>` (repeatable) puts `fence proxy` behind the guest's
`127.0.0.1:3128` and sets `HTTPS_PROXY` and the rest, so that tools reach
the named hosts and nothing else; `--proxy-learn` lets every host through
and records those not named. Its ledger is `proxy.jsonl` in the run
directory, and `result.json` counts it.

`kai ticket deps <number>` (or `--repo owner/name --ref <sha>`, a public
repository) gets a ticket's code into microVMs without kai's host ever
running git or cargo on it. A fetch run, whose only way out is fence's
proxy to GitHub, `static.rust-lang.org` and crates.io, clones the commit
into `/src`, installs the toolchain `rust-toolchain.toml` names and runs
`cargo fetch --locked`; nothing of the tree runs there. A build run with no
way out then builds the dependencies (`cargo test --no-run`). What each
wrote is kept as a layer, keyed by image, script, repository and commit,
and a second call takes both from the cache. A run on the two layers works
in `/src`, offline, with the dependencies built:
`kai vm run --image rust --layer <fetch> --layer <build> --cwd /src -- cargo test`.
A refused ticket is not fetched.

`kai agent fetch` downloads Claude Code 2.1.323, Claude Agent SDK 0.3.323
and Node 22.22.0 for Linux x86_64, verifies the artifact and extracted-file
hashes in [the runtime pins](crates/agent/src/runtime.rs), and packs them
with the compiled-in wrapper under `/opt/kai/agents/session`. All files
are root-owned in a read-only layer. No package manager or installer runs
in the guest. The SDK's self-contained entry point is used directly; its
optional integrations are not part of this profile.

A second fetch revalidates the layer. An invalid entry is moved aside and
rebuilt; interrupted builds recover. The whole runtime tuple and wrapper
bytes determine the cache identity. `agent run` and `agent session` only
use a previously fetched layer and never download implicitly.

`kai agent fetch --addon ponytail` packs an add-on the same way, as a layer
of its own: Ponytail's `ponytail` skill and the repository's MIT licence,
each file pinned by its commit and SHA-256 in
[the add-on pins](crates/agent/src/addon.rs), under
`/opt/kai/addons/ponytail/`, root-owned with mode 0644. With a manifest kai
writes, `.claude-plugin/plugin.json`, the directory is a local plugin of
Claude Code, the skill at `skills/ponytail/SKILL.md`. Beside the layer,
its record (`kai-addon-layer/1`) names the pin, the digest of the layer's
inventory (what it holds) and the layer's own digest (its bytes); see
[kai-addon-layer/1](contracts/kai-addon-layer-1.md).

`kai agent run` and `kai agent session` take `--addon ponytail`, fetched
before, and `--activation "/ponytail full"`: the layer is mounted, its
directory given to Claude as a local plugin, and the activation put on a
line of its own before the first turn's text. `agent.json` keeps three
facts apart: the add-on installed, its skill offered in a turn
(`skills_offered`), and the slash command Claude took (`commands_invoked`).
Claude passes an unknown slash command to the model as plain text, so
`agent run` fails when the activation was not taken. Against a fake API,
Ponytail's text and the task went to the model in one request.

`kai agent run --model <id> --tree ./fixture --tree-out ./after -- "<prompt>"`
runs one turn. `kai agent session --model <id> --tree ./fixture` accepts
newline-delimited JSON commands on stdin and emits ordered JSON events on
stdout: `{"op":"send","text":"..."}`, `{"op":"interrupt"}`,
`{"op":"close"}`. Wait for a turn's result (and its interrupt acknowledgement
when requested) before sending another. A command the session cannot take
(not one of these, a send during a turn, an interrupt without one) is
answered with a `refused` line and the session goes on. EOF closes the
session; Ctrl-C or SIGTERM drops it and stops the VM. `agent session`
exits 0 when the session closed and its VM ended cleanly, whatever the
turns' outcomes; `agent run` exits 0 only for an answered turn. Sessions
continue within one live VM; resuming after a process or VM restart is
not implemented. `--max-turns <n>` bounds the model requests of each turn;
a turn that reaches it ends with the outcome `limit`, and its last tool
call may still have changed the tree. Without it, only the VM's time
limit (`--timeout`) bounds a turn.

The guest sets `CLAUDE_CODE_MAX_OUTPUT_TOKENS=32000` explicitly for each
model request. This keeps the output ceiling independent of model defaults;
the external proxy still owns spending approval and limits.

Both commands use the private proxy handoff or `ANTHROPIC_BASE_URL` naming
an explicit loopback HTTP port. Normal mode accepts only a proxy-issued
placeholder of the supported format; `--mock` discards the environment key and uses a fixed
dummy against an explicitly supplied local fake endpoint. Approval, spending
limits and provider credentials belong to the external proxy. A simple
HTML comparison has completed with a real model. Against a local fake API that calls
tools, Bash runs in the guest's tree as uid 1000, a Write and an Edit
come out with `--tree-out`, and a permission request Claude makes on its
own, such as for changing its settings in the tree, is denied and named
in the turn's result. Claude runs without bare mode, which would offer
only Bash, Read and Edit, and with no setting source, so a tree's
CLAUDE.md, AGENTS.md and hooks stay out. `agent.json` records the tools
asked for and, per turn, those Claude offered.

The unprivileged guest owns its work tree and fresh home. An explicit model,
a tool allowlist and VM limits configure the run. `events.jsonl`,
`prompt-<turn>.md`, `agent.json` and the VM's `result.json` describe what
happened; SDK cost estimates are not recorded. See [agent sessions](docs/agent.md),
the [session protocol](contracts/kai-agent-session-1.md) and
[run record](contracts/kai-agent-run-1.md).

Limits today:

- Linux on x86_64 with KVM only.
- One recipe, `rust`.
- `--tree-out` keeps neither hard links nor holes in files.
- `kai ticket deps` takes public repositories on GitHub with a Rust
  project and its `Cargo.lock`; no token enters a VM.
- A VM belongs to the kai that started it and is stopped when kai stops,
  on Ctrl-C or SIGTERM too. A kai killed outright cannot stop it; the VM
  then ends at the guest's own time limit.

### Settings

A `.env` file, the first from the current directory upward, may hold the
`KAI_*` variables; a variable already set wins over it.

kai reads the first `kai.toml` from the current directory upward, or the
file `--config` names. An option, or the environment variable beside it,
wins over the file, and the file over kai's default. A key kai does not
know is an error. [kai.toml](kai.toml) in this repository shows every key.

### Building

kai needs Rust (`rust-toolchain.toml` names the version and the target
x86_64-unknown-linux-musl, for kai's guest init; rustup installs both) and
a C compiler: SQLite is compiled into kai. Without the musl target kai is
built without its init, and `kai vm doctor` says so.

## Scope and remaining limits

The simple comparison MVP is implemented, including real-model execution
through the external proxy and reporting of both results. Whether a model
follows an add-on's instructions is for the owner to inspect; execution
completion is not a quality judgment.

The GitHub ticket workflow is separate from this MVP. Running a ticket's
check and hidden tests on the layers prepared by `kai ticket deps` is not
implemented. Automated grading, statistical verdicts and automatic variant
selection are outside the simple comparison MVP.

## Licence

Copyright 2026 Kombuse Labs. Apache-2.0, see [LICENSE](LICENSE) and
[NOTICE](NOTICE). Contributing: [CONTRIBUTING.md](CONTRIBUTING.md). Security
reports: [SECURITY.md](SECURITY.md).
