# Contributing

kai is at its beginning (the README says what works), and what it will
consist of is still being decided. Small fixes are welcome as pull
requests. For anything larger, please open an issue first.

## What every change needs

- **A GitHub noreply address** as author and committer:
  `git config user.email <id>+<login>@users.noreply.github.com` (GitHub
  shows yours under Settings → Emails). CI refuses any other address.
- **Nothing private** in tracked files: no absolute paths of your machine,
  no e-mail addresses, no real transcripts or data. Fixtures are invented.
  `tools/check-public.sh` checks it.
- **Docs with the change.** Whatever a crate makes public has a doc
  comment, and the README's "What works today" stays true.
- **A reason beside every new dependency** in `Cargo.toml`.
- **The checks pass.**

## Checks

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

`.github/workflows/ci.yml` names the version of each tool. None of them
calls a model or costs money.

## If you push to this repository

`main` takes signed commits only. Once per clone:

```sh
git config core.hooksPath .githooks
git config gpg.format ssh
git config user.signingkey ~/.ssh/<your key>.pub
git config commit.gpgsign true
git config gpg.ssh.allowedSignersFile .githooks/allowed_signers
```

The hooks then refuse a commit or a push that is unsigned, by another
address, or that holds something private. Your address and public key go
into `.githooks/allowed_signers`, in a commit signed by someone already
listed. Never pass `--no-verify` or `--no-gpg-sign`.

## Licence

Contributions are taken under the Apache License 2.0, as section 5 of
[LICENSE](LICENSE) says.
