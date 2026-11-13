# kai-report-evidence/2

**P0 contract, not implemented.** This version adds explicit event-stream
binding and enables frozen definition and launcher-backed platform evidence.
It does not change `kai-pair/1`, `kai-agent-run/1`, or the deployed v1 reader.
Readers must continue to accept v1 exactly as written; a v1 sidecar has no
`events` member, a null `definition`, and null `execution_os` values.

The pair writer creates this immutable local handoff only after a terminal
pair record. It writes atomically, never replaces existing bytes, and never
reruns an arm after a handoff error. A missing sidecar remains legacy evidence
coverage. Source records can contain task content and stay local.

Every member below is required. Unknown members, duplicate JSON keys, wrong
types, unsupported nested formats, non-finite values, hash mismatches and
contradictory identity are errors. Integers are nonnegative. File hashes are
`sha256:` plus 64 lowercase hexadecimal digits over the exact bytes read.
The handoff is limited to 256 KiB, `pair.json` and `agent.json` to 16 MiB,
each definition to 64 KiB, each platform record to 16 KiB, and each referenced
`events.jsonl` stream to the 64 MiB limit in `kai-agent-session/1`.

`FileRef` is exactly `{path, sha256}`. Paths are relative to the result root,
resolve to regular files below that root, and contain no symlink or traversal.
The reader opens and hashes the same bytes it parses. A reference grants no
permission to follow another path found in the referenced file.

| Field | Rule |
| --- | --- |
| `format` | Literal `kai-report-evidence/2` |
| `producer` | Exactly `{name: "kai", version: <pair producer>}` |
| `run_id` | Exact 32 lowercase hex `pair.json.run_id` |
| `capture` | `producer-terminal` or explicit `retrospective` import |
| `recorded_at` | UTC Unix milliseconds at or after terminal `pair.finished_at` |
| `pair` | FileRef to `pair.json`; format `kai-pair/1`; state `finished` or `interrupted` |
| `definition` | Null or FileRef to `kai-report-definition/1` |
| `arms` | Exactly `without` and `with`, keyed by role rather than directory label |

Each arm has exactly `agent`, `agent_missing`, `execution_os`, `events` and
`quality`:

| Field | Rule |
| --- | --- |
| `agent` | Null or FileRef to `<pair role name>/run/agent.json` (`kai-agent-run/1`) |
| `agent_missing` | Null iff agent is present; otherwise `not-started`, `not-written`, or retrospective-only `unavailable-in-history` |
| `execution_os` | Null or `{family, architecture, evidence}`; evidence is a FileRef to `kai-execution-platform/1` |
| `events` | Null or FileRef to `<pair role name>/run/events.jsonl`, the exact `kai-agent-session/1` stream bytes |
| `quality` | Literal `ungraded`; result status, exit code and successful execution never imply a grade |

`events` is independent of whether the final `agent.json` exists: a crashed
session may leave only a partial stream. When both files exist, all non-null
session ids must agree. Each event's `seq` is checked in file order. A gap,
malformed line, missing terminal `closed` event or source-format/version
mismatch yields partial or unknown source coverage with diagnostics; it does
not become a complete trace. These events have no required timestamp. Readers
must preserve valid untimed counts where the adapter supports them and leave
time metrics unknown. They must never fabricate timestamps or native
transcript metadata.

The `agent` result usage remains the existing primary per-arm token record.
Grindstone-derived request/tool/file metrics are separate descriptive facts;
they are not added to terminal usage. Exact trace contributor references use
the events FileRef hash plus source line and byte interval (end exclusive).
Raw event bytes and contributor references remain local.

`definition` and `execution_os` are parsed according to their linked contract
pages. For a `before-execution` definition, `declared_at` must precede both
arm start times. A retrospective declaration cannot be presented as
preregistered. A platform record is accepted only when its run id, role,
kernel and Firecracker hashes match the same arm's `agent.vm`; host OS and an
image label are not evidence.

External Sieve and prxy files are deliberately not nested under this handoff.
Reporter receives them as explicit local inputs and joins them under the
session/request rules in the shared P0 field/source matrix. It must not
discover them from a home-directory path or treat a missing file as an empty,
complete ledger.

The v2 fixture corpus and expected conformance vectors are duplicated byte
for byte at Kai `contracts/fixtures/kai-evidence-p0/` and Reporter
`tests/fixtures/kai-evidence-p0/`. Their `SHA256SUMS` files bind all source
fixtures. The fixtures contain invented data only; they do not claim that a
v2 writer or reader exists yet.

Example: [`report-evidence.json`](fixtures/kai-evidence-p0/report-evidence.json).
