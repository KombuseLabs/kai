# Contracts

The file formats kai reads and writes, described for anyone who reads those
files with another program. One page per format.

| Format | File | Written by |
| --- | --- | --- |
| [kai-agent-session/1](kai-agent-session-1.md) | `events.jsonl`, host/guest stream | `agent` and its guest wrapper |
| [kai-agent-run/1](kai-agent-run-1.md) | `agent.json` | `agent` |
| [kai-agent-layer/1](kai-agent-layer-1.md) | `layer.json` of an agent's layer in the VM cache | `kai agent fetch` (crates/agent) |
| [kai-addon-layer/1](kai-addon-layer-1.md) | `layer.json` of an add-on's layer in the VM cache | `kai agent fetch --addon` (crates/agent) |
| [kai-vm-run/1](kai-vm-run-1.md) | `result.json` of a microVM run | `kai vm run` (crates/vm) |
| [kai-study/2](kai-study-2.md) | `study.toml` of a study | a person, or `kai study new` |
| [kai-lock/2](kai-lock-2.md) | `lock.json` of a study | `kai study lock` (engine::study) |
| [kai-trial/3](kai-trial-3.md) | `trial.json` of one attempt of a study's slot | `kai study run` (engine::study) |
| [kai-environment/1](kai-environment-1.md) | a record in `lock.json`: the baseline every arm shares | `kai study lock` |
| [kai-variant/1](kai-variant-1.md) | a record in `lock.json`: what one arm adds | `kai study lock` |
| [kai-canonical-json/1](kai-canonical-json-1.md) | no file: a JSON value's one text and its id | `helpers::canonical` |

These rules hold for every format:

- **Every file names its format**, as a name and a number (`<name>/<number>`).
  A change that an existing reader could misread gets a new number, and the
  page changes in the same commit.
- **Every page comes with an example file.** The crate that writes the
  format has a test that it still writes the example, and a crate that
  reads it has a test that it reads the example.
- **Unknown is not zero.** A value nobody recorded is absent or `null`,
  never 0.
