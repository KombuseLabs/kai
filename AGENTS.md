# kai: notes for coding agents

kai compares variants of a coding agent on real tickets and says for each
variant whether it is better than the shadow (the variant in use today),
worse or not distinguishable, and what it cost. Nothing of that is built
yet. What is: tickets come in from GitHub issues (`kai github sync`) into
a local SQLite store, and `kai ticket qualify` says from what GitHub said
whether kai may use each one. No ticket is run yet.

## Where things are

| Path | What it is |
| --- | --- |
| `crates/cli/` | the command line (clap): arguments, output, exit status; builds the program `kai`. `main.rs` starts up, `settings.rs` reads `kai.toml`, `src/cmd/` has a module per command group |
| `crates/types/` | the domain types (`Ticket`, `Qualification`); data only, depends on nothing of kai's |
| `crates/engine/` | what kai does, as a library: today the static qualify rules; never depends on `cli` |
| `crates/store/` | the store: a SQLite file of the tickets kai fetched, and where each board's last sync stopped |
| `crates/github/` | GitHub issues into tickets, one way, never writing; async, on reqwest |
| `crates/helpers/` | infrastructure the other crates share, nothing about what kai does: `logging`; `config`, which finds and reads settings files; `http`, where every HTTP client starts |
| `kai.toml` | kai's settings when it runs in this repository; shows every key kai knows |
| `docs/` | how parts of kai work, with diagrams (Mermaid, which GitHub renders); `qualify.md` |
| `contracts/` | the file formats kai reads and writes, one page each; none is defined yet |
| `deny.toml` | which licences and sources dependencies may have |
| `CONTRIBUTING.md` | what a change needs, the checks, and the settings of a clone that pushes here |
| `tools/check-public.sh` | refuses what must not be in a public repository |
| `.githooks/` | `pre-commit` and `pre-push`: no commit and no push that is unsigned or by an address that is not a GitHub noreply address; `allowed_signers` lists who may sign |
| `board/` | gitignored; exists only in a maintainer's checkout |

## Rules

- **Logging.** Code logs through `tracing`'s macros. Only a program's
  `main` calls `helpers::logging::init` and decides where lines go; a
  library crate never does. Results go to standard output, log lines never.
  clippy refuses `println!`, `eprintln!` and `dbg!` outside `cli`.
  A log line is a diagnostic, not a record: nothing a run found may exist
  only in a log, since a full queue drops lines.
- **Async.** kai is async, on tokio: what waits on the network or on a VM
  is an `async fn`. As with logging, only a program's `main` starts the
  runtime; a library crate never does.
- **Docs.** Whatever a crate makes public has a doc comment (the
  `missing_docs` lint, an error in CI); an example in a doc comment is a
  test.
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
```

CI (`.github/workflows/ci.yml`) names the version of each tool. A check
whose tool the machine does not have is said to be not run, never passed.
