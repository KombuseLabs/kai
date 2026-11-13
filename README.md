# kai - kombuse auto improve

## What works today

```sh
cargo run                           # prints: hello kai
cargo run -- --help                 # the options and commands
cargo run -- --log-level debug      # log lines on standard error
cargo run -- github sync --board <owner/name>   # a board's issues into the store
cargo run -- ticket list            # the tickets in the store
cargo run -- ticket qualify         # whether kai may use each, and why not
cargo run -- store reset            # empties the store; the next sync fetches everything
```

Tickets: an issue on a board repository, made with an issue form whose
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

A `.env` file, the first from the current directory upward, may hold the
`KAI_*` variables; a variable already set wins over it.

Settings: kai reads the first `kai.toml` from the current directory upward,
or the file `--config` names. An option, or the environment variable beside
it, wins over the file, and the file over kai's default. A key kai does not
know is an error. [kai.toml](kai.toml) in this repository shows every key.

kai needs Rust (`rust-toolchain.toml` names the version) and a C compiler:
SQLite is compiled into kai.

## Licence

Copyright 2026 Kombuse Labs. Apache-2.0, see [LICENSE](LICENSE) and
[NOTICE](NOTICE). Contributing: [CONTRIBUTING.md](CONTRIBUTING.md). Security
reports: [SECURITY.md](SECURITY.md).
