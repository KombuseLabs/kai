# kai: notes for coding agents

kai compares variants of a coding agent on real tickets and says for each
variant whether it is better than the shadow (the variant in use today),
worse or not distinguishable, and what it cost. Nothing of that is built
yet. What is: tickets come in from GitHub issues (`kai github sync`) into
a local SQLite store, and `kai ticket qualify` says from what GitHub said
whether kai may use each one. `kai vm run` runs one command in a fresh
Firecracker microVM, and `kai ticket deps` fetches and builds a ticket's
repository and dependencies in microVMs. `kai agent fetch` packs a pinned
Claude/SDK/Node runtime as a read-only layer, and with `--addon` a pinned
skill as one of its own. `kai agent run` sends one
prompt, and `kai agent session` exchanges interactive commands/events with
that runtime in a microVM. These paths are tested against a local fake
model; no ticket's check or real-model workflow has been verified yet.

## Where things are

| Path | What it is |
| --- | --- |
| `crates/cli/` | the command line (clap): arguments, output, exit status; builds the program `kai`. `main.rs` starts up, `settings.rs` reads `kai.toml`, `src/cmd/` has a module per command group |
| `crates/types/` | the domain types (`Ticket`, `Qualification`); data only, depends on nothing of kai's |
| `crates/engine/` | what kai does, as a library: the static qualify rules; `deps`, a ticket's repository and dependencies fetched and built in microVMs as two layers; never depends on `cli` |
| `crates/agent/` | pinned agent runtimes and interactive sessions; `runtime` pins Claude/SDK/Node, `addon` pins add-ons (Ponytail's skill), `layer` checks, publishes and recovers the cache of both, `session` owns the host protocol, `wrapper.mjs` is its guest peer; depends on `vm` and `helpers`, never on `engine` or `cli` |
| `crates/store/` | the store: a SQLite file of the tickets kai fetched, and where each board's last sync stopped |
| `crates/github/` | GitHub issues into tickets, one way, never writing; async, on reqwest |
| `crates/proxy/` | a run's way out of its microVM (`Egress`): starts `fence proxy` on the socket the launcher's port ends at, ends it after the run, and counts its ledger; `src/bin/fence-stand-in.rs` stands in for fence in the tests |
| `crates/vm/` | microVMs on Firecracker: one command in a fresh VM (`kai vm run`; a tree, layers, ports, a way out through `proxy`'s egress), images from the recipes in `images/` (`kai vm image`), the pinned Firecracker and guest kernel (`kai vm fetch`), what a run needs (`kai vm doctor`); `src/run.rs` is a run step by step, `src/run/` its parts; the build script builds `guest` for musl and puts the init into kai |
| `crates/guest/` | kai's init, PID 1 of every VM (`kai-init`, static, musl), and `job.json` and `status.json`, which it shares with `vm`; the example `probe` is the VM tests' program |
| `crates/helpers/` | infrastructure the other crates share, nothing about what kai does: `logging`; `config`, which finds and reads settings files; `http`, where every HTTP client starts; `process`, how a program is run, waited for and its output kept or hashed; `fs`, files and directories put in place whole, what a killed process left of them removed, a lock among processes, a directory flushed; `hash`, SHA-256; `lines`, bounded cancellation-safe protocol frames; `testing` (tests only: the feature `test-support`, which the other crates turn on as a dev-dependency), a test's own directory under `kai-tests` in the temporary directory, removed when the test ends |
| `crates/diagrams/` | keeps the diagrams in Markdown true to the code (`tests/docs.rs`), writes the crate graph of `docs/crates.md` (`tests/crates.rs`), and the maintainer's tool `diagram-stubs`, which prints Rust stubs from a planned class diagram; reads source text with syn and builds no crate of kai's; nothing depends on it |
| `images/` | the recipes of kai's VM images, one Dockerfile each, compiled into kai; their rules are in `crates/vm/src/image.rs` |
| `kai.toml` | kai's settings when it runs in this repository; shows every key kai knows |
| `docs/` | how parts of kai work, with diagrams (Mermaid, which GitHub renders), each kept true by `crates/diagrams`: `qualify.md`, `proxy.md`, `agent.md`, `crates.md` (the crate graph, generated) |
| `contracts/` | the file formats kai reads and writes, one page each, with an example: `kai-vm-run/1`, `kai-agent-session/1`, `kai-agent-run/1` |
| `deny.toml` | which licences and sources dependencies may have |
| `CONTRIBUTING.md` | what a change needs, the checks, and the settings of a clone that pushes here |
| `tools/check-public.sh` | refuses what must not be in a public repository |
| `.githooks/` | `pre-commit` and `pre-push`: no commit and no push that is unsigned or by an address that is not a GitHub noreply address; `allowed_signers` lists who may sign |
| `board/` | gitignored; exists only in a maintainer's checkout |

## Rules

- **Logging.** Code logs through `tracing`'s macros. Only a program's
  `main` calls `helpers::logging::init` and decides where lines go; a
  library crate never does. Results go to standard output, log lines never.
  clippy refuses `println!`, `eprintln!` and `dbg!` outside `cli`, but for
  four programs whose `allow` says why: kai-init, PID 1 without a logger,
  writes to the VM's console; vm's build script talks to cargo; the test
  program `probe` answers on its streams; `diagram-stubs` prints the stubs
  it makes.
  A log line is a diagnostic, not a record: nothing a run found may exist
  only in a log, since a full queue drops lines.
- **Async.** kai is async, on tokio: what waits on the network or on a VM
  is an `async fn`. As with logging, only a program's `main` starts the
  runtime; a library crate never does.
- **Docs.** Whatever a crate makes public has a doc comment (the
  `missing_docs` lint, an error in CI); an example in a doc comment is a
  test.
- **Diagrams.** Every Mermaid block in a Markdown file starts with how it
  stays true to the code: `%% generated by …` (never edit it; regenerate
  it), `%% checked against …` (a test compares it with the code),
  `%% drawn from …#<stamp>` (a test fails when what it was drawn from
  changes: look at the diagram, fix it, set the new stamp), or
  `%% shows every variant of …` (a state diagram). A change to something
  a diagram shows changes the diagram in the same commit. `docs/` shows
  what is built; `board/` shows only plans. A `todo!()` is denied
  (clippy), so stubs stay in the working tree until their bodies are
  written.
- **The README's "What works today" stays true.** Nothing unbuilt is
  described as built.
- **This repository is public.** No absolute host paths, tracker ids,
  private project names, e-mail addresses or real transcripts in tracked
  files; fixtures are invented. `tools/check-public.sh` must pass.
  Commits are signed and by a GitHub noreply address; the hooks refuse
  anything else, once a clone is set up as `CONTRIBUTING.md` says. Never
  pass `--no-verify` or `--no-gpg-sign`.
- **Unknown is not zero.** A value nobody recorded stays absent in what
  kai writes and prints; it is never shown as 0.
- **Nothing here calls a model or spends money.** Never approve spending.
- **No new dependency** without its reason beside it in `Cargo.toml`, and
  `cargo deny` must still pass.
- **Linux first.** Tests run without network and without a model.
- Docs and comments are in English. Commit messages say what changed and
  why. Do not commit and do not push unless the owner asks for it.

## Commands

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo nextest run --locked && cargo test --doc --locked   # or: cargo test --locked
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --locked
cargo deny --locked --workspace check
git ls-files -z | xargs -0 typos
git ls-files -z '*.md' | xargs -0 lychee --offline --no-progress
tools/check-public.sh
git ls-files -z '*.md' | xargs -0 -n1 merman-cli lint         # the Mermaid of every diagram
UPDATE_DIAGRAMS=1 cargo nextest run -p diagrams                # rewrite the generated diagrams
cargo run -p diagrams --bin diagram-stubs -- <file.md>:<line>  # Rust stubs from a planned class diagram; writes no file
```

CI (`.github/workflows/ci.yml`) names the version of each tool. A check
whose tool the machine does not have is said to be not run, never passed.
