# kai - kombuse auto improve

## What works today

```sh
cargo run                           # prints: hello kai
cargo run -- --help                 # the options
cargo run -- --log-level debug      # log lines on standard error
```

Settings: kai reads the first `kai.toml` from the current directory upward,
or the file `--config` names. An option, or the environment variable beside
it, wins over the file, and the file over kai's default. A key kai does not
know is an error. [kai.toml](kai.toml) in this repository shows every key.

kai needs Rust; `rust-toolchain.toml` names the version.

## Licence

Copyright 2026 Kombuse Labs. Apache-2.0, see [LICENSE](LICENSE) and
[NOTICE](NOTICE). Contributing: [CONTRIBUTING.md](CONTRIBUTING.md). Security
reports: [SECURITY.md](SECURITY.md).
