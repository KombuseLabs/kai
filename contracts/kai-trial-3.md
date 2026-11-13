# kai-trial/3: one attempt of one slot of a study

`trials/<task>__<arm>__r<repetition>/attempt-<n>/trial.json` in a study's
run directory, written once per attempt by `engine::study`, after
everything it points at: created only if absent, under a temporary name
it then renames. An existing record is never written again, so a study
that is stopped, crashes or is refused and then resumed runs exactly the
slots that have no final record. Example (a ghost's first slot, with
invented digests and usage):
[kai-trial-3.example.json](kai-trial-3.example.json).

The attempt's directory beside it holds `agent/` (what `kai agent run`
writes: `agent.json`, `events.jsonl`, `prompt-*.md`, `result.json`),
`tree-after/` (the agent's tree, which only the grader reads) and `grade/`.
The record points at them by path, relative to its own directory, and
SHA-256; it copies nothing.

## Fields

| Field | Type | When | Meaning |
| --- | --- | --- | --- |
| `format` | string | always | `kai-trial/3` |
| `study` | string | always | the study's name |
| `lock` | string | always | `sha256:` and the SHA-256 of `lock.json`'s bytes |
| `slot` | object | always | `task`, `arm`, `repetition`, `position`, as in the lock's plan |
| `attempt` | integer | always | from 1; only an `infra` attempt is followed by another |
| `outcome` | string | always | see below |
| `reason` | string | not for `counted` | see below |
| `pins` | object | when the session started | what ran: `baseline` and `variant` (ids), `image`, `layer` (the runtime layer's `sha256:`), `addons` (each layer's `sha256:`, in order), `model_seen` (the model the responses named, when known). A pin that differs from the lock is the outcome `drift` |
| `agent` | object | when the session started | `path` and `sha256` of `agent.json`; `session_id`; `turn`, the turn's outcome there (`answered`, `interrupted`, `agent-error`); `tools_offered`, the tools Claude offered |
| `exposure` | object or null | when the session started | `null` for an arm without activation; otherwise `commands`, the commands the session shows as invoked, such as `ponytail:ponytail` (an empty list: the activation did not reach the agent) |
| `grade` | object | when the tree was graded | `path` and `sha256` of `grade.json` (`kai-grade/1`), and `pass` |
| `usage` | object | when the ledger was read | from the proxy's ledger lines of the session, by session id: `input_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`, `output_tokens`, `total_tokens` (their sum), `charged_usd`, `basis`, `lines` (the ledger lines' ids) and `complete` (whether every request of the session has a line). Each is absent when unknown |
| `time` | object | when known | `wall_ms`, the VM's, and `turns`, the agent's |
| `started`, `ended` | string | always | UTC, RFC 3339 |

Usage is never Claude's own cost estimate, and a record has no cost of its
own: the cost is the ledger's, and the record points at its lines. A trial
with no ledger lines has no `usage`, not one of 0.

## Outcomes

| `outcome` | `reason` | Meaning | In the primary analysis |
| --- | --- | --- | --- |
| `counted` | none | the agent answered and the tree was graded, pass or fail | yes |
| `limit` | `time` or `turns` | the trial's time or turn limit was reached, the VM healthy | no; the sensitivity analysis counts it as a failure |
| `infra` | `killed`, `guest-failed`, `launch-failed`, `no-result`, `grader-failed`, `grade-flaky` or `tree-missing` | something failed that says nothing about the agent | no |
| `refused` | the proxy's rule, such as `daily-limit` | the proxy answered 402; the run stops and the slot waits | no |
| `drift` | the first field that differs, such as `pins.layer` | what ran is not what the lock froze; the run stops | no |

Infrastructure is never a verdict on the agent's work.

A slot is done when its last record is final: `counted`, `limit`, or
`infra` on attempt `[limits] max_attempts`. A `refused` or `drift` record
is not final and does not count toward `max_attempts`: the slot runs
again when the study is resumed, as the next attempt, in a new directory.
