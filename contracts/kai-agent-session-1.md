# kai-agent-session/1

The host and pinned guest wrapper exchange newline-delimited UTF-8 JSON.
The VM's guest port 8788 forwards only to an owner-private Unix socket on
the host. Port 8787 is the separate model route. Neither is general egress.

Each command has `format: "kai-agent-session/1"` and `op`:

| op | Fields | Meaning |
| --- | --- | --- |
| start | model, tools; plugins | Configure one SDK session; sent once by the host. `plugins` may be absent |
| send | turn, text | Monotonic turn id starting at 1, text at most 64 KiB |
| interrupt | turn | Request interruption of the active turn |
| close | none | Stop the SDK subprocess and end the guest |

`plugins`, when present, lists at most eight distinct directories
`/opt/kai/addons/<name>`, each an add-on's
([kai-addon-layer/1](kai-addon-layer-1.md)) on a read-only layer; Claude
loads each as a local plugin, the one way a skill loads without a setting
source. Absent, the session has none, as before the field existed.
Claude replays every user message it takes, as an `event` whose SDK
message has `isReplay`; a message that ran a slash command names it in a
`<command-name>` tag, which the host reads.

The model is explicit. The tool list contains only Read, Edit, Write, Bash,
Glob and Grep; an empty list disables tools. Claude may offer the model
fewer: each turn's `system` `init` event names those it offers. The
wrapper starts Claude without bare mode, which would offer only Bash, Read
and Edit, and with no setting source: nothing of the tree (its CLAUDE.md,
AGENTS.md, settings or hooks) configures Claude, and the fresh home holds
nothing either. A permission request Claude makes, for an allowed tool
too (as before it changes its own settings in the tree), is denied, never
presented as a spending approval; the turn goes on, and the result's
`permission_denials` names the request.

Every event has `format`, monotonic `seq` starting at 1, `kind`, and
`payload`; `turn` and `session_id` appear when applicable. A session id,
once present, must remain unchanged.

| kind | Meaning |
| --- | --- |
| ready | Exact SDK, Claude and Node versions, checked by the host |
| event | Partial stream, assistant, user or system event from the SDK, during a turn |
| interrupted | Acknowledgement of the interrupt request |
| result | Terminal SDK result with an explicit interrupted flag |
| closed | Wrapper closing; the host must still join the VM |
| error | Safe reason code; no exception or environment dump |

Only one turn is outstanding. An interrupt receipt is not a result: drain
both before sending another turn. If a result crossed an interrupt in
transit, the wrapper acknowledges it with `completed: true` and the normal
result stays normal; interruption cannot retroactively change an answer.
The host rejects mismatched ids, sequence gaps and unexpected receipts.

Frames are limited to 2 MiB including newline; transcripts to 64 MiB. The
wrapper bounds queued commands and socket backpressure. A malformed frame,
unknown operation, unexpected EOF, timeout or version mismatch fails the
session. Framing preserves partial bytes when a caller cancels a pending
read. EOF/close stops the SDK process; dropping the Rust handle cancels the
VM task. The VM's own timeout is the final limit after abrupt host death.

SDK costs are removed recursively, including model-level cost estimates.
The route credential is redacted from guest event strings. These controls
do not make arbitrary tool output safe to execute: payloads remain data.
The host never evaluates code or shell expressions from events.

The CLI starts/configures the session itself. Its stdin accepts the smaller
commands `{"op":"send","text":"fixture"}`, `{"op":"interrupt"}` and
`{"op":"close"}`; it assigns turn ids. Its stdout carries the above events.
A command it does not forward (not one of these, a send while a turn is
active, an interrupt without an active turn or a second one) is answered
on stdout with `{"format":"kai-agent-session/1","kind":"refused","payload":
{"reason":"..."}}`, a line of the CLI's own without `seq`, and the session
goes on; the reason never echoes the command. An input line is limited to
the frame limit, 2 MiB; a longer one, or one cut off by the end of the
input, closes the session as `close` would. `kai agent session` exits 0
when the session closed and its VM ended cleanly, whatever its turns'
outcomes, which the result events and agent.json give; `kai agent run`
exits 0 only for an answered turn. Both take `--addon <name>` for each
add-on, whose `dir` goes into `plugins`, and `--activation`, which the
host puts on a line of its own before the first turn's text; the `send`
command carries the text as sent. `kai agent run` with an activation
exits 0 only when Claude replayed its command.

The [example](kai-agent-session-1.example.json) is a ready event. The VM
integration test checks the actual wrapper's first event against it.
