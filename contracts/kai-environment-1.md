# kai-environment/1: the baseline every arm of a study shares

A record, not a file of its own: the lock ([kai-lock/2](kai-lock-2.md))
holds it under `baseline` beside its id, the
[canonical](kai-canonical-json-1.md) id of the record. Every arm of a
study runs on this one baseline; a comparison of arms with two baselines
is refused. Example (a ghost's baseline, with invented digests):
[kai-environment-1.example.json](kai-environment-1.example.json).

It is the minimal form of mvp-environments' environment record: what a
study needs to say that its arms ran on the same thing. It names no
host path and no friendly alias.

## Fields

| Field | Type | Meaning |
| --- | --- | --- |
| `format` | string | `kai-environment/1` |
| `image` | object | the VM image: `recipe` (its name, such as `rust`) and `id` (`sha256:`, the image's id as `kai vm image` gives it) |
| `vm` | object | `firecracker`, `kernel`, `initramfs`: each `sha256:` and the file's SHA-256, as [kai-vm-run/1](kai-vm-run-1.md) records them |
| `runtime` | object | the agent's runtime, as [kai-agent-run/1](kai-agent-run-1.md) records it: `agent` (`claude-code`), `claude`, `sdk`, `node` (exact versions), `layer` (`key`, the layer's build key; `sha256`, its bytes; `length`), `wrapper` (`sha256:` and the wrapper's SHA-256) and `session` (`kai-agent-session/1`) |
| `profile` | object | how the agent is started, the same for every arm: see below |

`profile`:

| Field | Type | Meaning |
| --- | --- | --- |
| `uid`, `gid` | integer | the agent's user in the guest |
| `bare` | boolean | whether Claude runs in bare mode; `false` |
| `setting_sources` | array of strings | the setting sources Claude loads; empty: nothing of the tree or the home configures it |
| `model` | string | the model id exactly as given to `--model` |
| `effort` | string | `claude-default`: kai sets no effort, and Claude's own default applies. It is written out, not left out, since it is part of what the arms share |
| `tools` | array of strings | the tools asked for, in the study file's order. What Claude offered in a trial is in that trial's record ([kai-trial/3](kai-trial-3.md)) |
| `cpus`, `memory_mib`, `timeout_s` | integer | the VM's processors, memory and time limit |
| `max_turns` | integer | the turn limit passed to the agent |
| `protocol` | string | `kai-trial-protocol/1`: how a trial runs and how its first input is made (the activation line, if the arm has one, a newline, then the task's instruction). A kai that runs trials another way refuses the study |

Every field is present; a record without one is refused, never filled in.
