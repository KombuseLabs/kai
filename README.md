# kai - kombuse auto improve

kai compares variants of a coding agent on real tickets and says for each
variant whether it is better than the one in use today, worse or not
distinguishable, and what it cost. Nothing of that is built yet. What is:
tickets from GitHub issues and the rules that say whether kai may use one,
and microVMs that run one command each, a ticket's code among them.

## What works today

```sh
cargo run                           # prints: hello kai
cargo run -- --help                 # the options and commands
cargo run -- --log-level debug      # log lines on standard error
cargo run -- github sync --board <owner/name>   # a board's issues into the store
cargo run -- ticket list            # the tickets in the store
cargo run -- ticket qualify         # whether kai may use each, and why not
cargo run -- store reset            # empties the store; the next sync fetches everything
cargo run -- vm doctor              # what running a microVM needs here, and what fixes each gap
cargo run -- vm fetch               # the pinned Firecracker and guest kernel into kai's cache
cargo run -- vm image build rust    # the image of kai's Rust recipe, with Docker
cargo run -- vm image list          # the recipes, and which images are built
cargo run -- vm run --image rust -- cargo --version   # one command in a fresh microVM
cargo run -- ticket deps <number>   # a ticket's repository and crates, fetched and built in microVMs
cargo run -- agent fetch            # the pinned Claude/SDK/Node runtime for a microVM
cargo run -- agent fetch --addon ponytail   # Ponytail's skill, pinned, as a read-only layer
```

### Tickets

A ticket is an issue on a board repository, made with an issue form whose
fields are Instruction, Repo, Ref (a full commit sha), Check and Hidden
tests, and labelled `kai`. `kai github sync` reads the board's issues
changed since the last sync into the store, `.kai/store.sqlite` beside
`kai.toml` (`--store` names another). It never writes to GitHub. The token
is `KAI_GITHUB_TOKEN`, else what `gh auth token` prints; a read-only token
is enough.

`kai ticket qualify` judges each ticket from what GitHub said: refused when
a rule fails (no `kai` label, a closed issue, not the form, an empty
instruction, a code block in it, a repository or ref that is malformed, no
hidden tests), undecided while a rule is not checked yet (who put the
label on; whether the repository has the commit and the hidden tests), and
qualified only when every rule was checked and met. So today no ticket is
qualified.

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
[kai-addon-layer/1](contracts/kai-addon-layer-1.md). No session gives an
add-on to Claude yet; `--layer` only puts it into the VM.

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
not implemented.

Both commands require `ANTHROPIC_BASE_URL` naming an explicit loopback HTTP
port. Normal mode accepts only a proxy-issued placeholder of the supported
format in `ANTHROPIC_API_KEY`; `--mock` discards that value and uses a fixed
dummy against an explicitly supplied local fake endpoint. Approval, spending
limits and provider credentials belong to the external proxy. No real
model workflow has been verified. Against a local fake API that calls
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

## Not built yet

- A ticket's check, its hidden tests, run in a microVM on the layers
  `kai ticket deps` makes.
- A real-model run through the external proxy. Sessions and tool calls
  are tested with a fake API only.
- An add-on given to the agent in a session: which skill it loads, how it
  is invoked, and evidence that it reached the model.
- The variants of an agent, the one in use today among them, a run of
  each on the qualified tickets, and the verdict with what each cost.

## Licence

Copyright 2026 Kombuse Labs. Apache-2.0, see [LICENSE](LICENSE) and
[NOTICE](NOTICE). Contributing: [CONTRIBUTING.md](CONTRIBUTING.md). Security
reports: [SECURITY.md](SECURITY.md).
