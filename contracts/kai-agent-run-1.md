# kai-agent-run/1

`agent.json` is atomically written in the VM's run directory. It records a
session, not proof that a task was solved. `result.json` remains the VM's
own [run record](kai-vm-run-1.md). A start that fails once its VM was
started (the VM ends or times out before the wrapper connects, the event
log cannot be made, the wrapper's handshake fails) is recorded too, with
`outcome: failed`, its reason, no turns, and the VM's record when the VM
ended.

| Field | Meaning |
| --- | --- |
| format | `kai-agent-run/1` |
| agent, version, sdk, node | Exact runtime versions |
| layer | Validated layer identity, hashes, length, packing and runtime members |
| model, tools | Explicitly selected model and the tools asked for; Claude may offer fewer (a turn's `tools_offered`) |
| mock, route_port | Fake mode and loopback port; never the credential |
| session_id | SDK session id, or null before one was observed |
| reason | Safe failure reason, or null |
| outcome | `closed`, `failed` or `dropped`; independent of turn outcomes |
| turns | Completed turns, ordered: id, outcome, prompt_sha256, tools_offered and SDK result |
| cancelled_turn | Outstanding turn id at closure, or null |
| vm | VM run record when obtained; null if unavailable |

A turn is `answered` only for a successful SDK result, `interrupted` only
for an actual interrupted result following the host request, otherwise
`agent-error`. A closed session can contain failed turns. A timeout or
transport failure is infrastructure failure and never an answer.

A turn's `tools_offered` lists the tools Claude offered the model in that
turn, as the turn's `system` `init` event names them, or is null when the
turn brought no such event. It is what the model could call, which `tools`
alone does not say: Claude 2.1.323 started with `--bare`, which kai does
not do, offered only Bash, Read and Edit of those asked for. A tool the
model called that was not offered came back to it as an error. A
permission request Claude made, for an allowed tool too, was denied and
is in the SDK result's `permission_denials`.

Abrupt process death may leave no final agent.json; the event log then
remains incomplete evidence, not success.

`events.jsonl` contains validated [session events](kai-agent-session-1.md).
`prompt-<id>.md` stores each submitted input. These files may contain private
task content and stay local to the run; they are not safe public fixtures.
No SDK dollar estimates are stored. Usage, if present, is exactly what the
SDK reported; with a fake API it is synthetic, not real billing.

The [example record](kai-agent-run-1.example.json) is an invented incomplete
session with deliberately labelled fixture layer identities. A writer test
compares the serialized record with this file. No value is production evidence.
