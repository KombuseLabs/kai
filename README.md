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
cargo run -- agent fetch            # the pinned Claude Code as a read-only layer for a microVM
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

`kai agent fetch` downloads Claude Code 2.1.322 for Linux from Anthropic's
release bucket, checks it against the SHA-256 pinned in
[crates/agent/src/lib.rs](crates/agent/src/lib.rs), and packs it with
squashfs-tools as a layer that holds `/opt/kai/agents/claude/2.1.322/claude`
and nothing else, every file root's. It prints the layer's path, under
`agents/` in the cache; a second call checks the layer afresh and makes
nothing. A layer that does not check out is moved aside and made again
from the checked download, and what a stopped fetch left is removed. A
run takes the layer with `--layer`:
`kai vm run --image rust --layer <layer> -- /opt/kai/agents/claude/2.1.322/claude --version`.
No agent works on a task yet, and nothing in kai calls a model.

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
- An agent at work in a microVM: Claude Code on a tree, through a model
  route on this machine, and the record of what it did.
- The variants of an agent, the one in use today among them, a run of
  each on the qualified tickets, and the verdict with what each cost.

## Licence

Copyright 2026 Kombuse Labs. Apache-2.0, see [LICENSE](LICENSE) and
[NOTICE](NOTICE). Contributing: [CONTRIBUTING.md](CONTRIBUTING.md). Security
reports: [SECURITY.md](SECURITY.md).
