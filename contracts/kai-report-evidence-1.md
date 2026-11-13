# kai-report-evidence/1

`agent compare` and `run <name>` publish `report-evidence.json` after the
terminal [pair record](kai-pair-1.md). It binds that pair and the available
agent records to their exact bytes. It contains no copied metrics, grade,
analysis or permission to publish. The handoff and its source files stay local.

Publication writes and flushes a temporary file, then creates the destination
atomically without replacing an existing file. A retry reuses the original
capture time and verifies the existing handoff; changed sources are an error.
A reporting failure writes `report-evidence-error.txt` when possible and emits
a diagnostic. It does not rerun either arm or change its execution status.
A hard crash that leaves the pair running produces no terminal handoff.

All fields below are required, including explicit nulls. Unknown members,
duplicate JSON members, invalid types and unsupported formats are rejected
by the reader. Integers are nonnegative; timestamps are UTC Unix milliseconds.

| Field | Meaning |
| --- | --- |
| `format` | `kai-report-evidence/1` |
| `producer` | Exactly `{name: "kai", version: <CLI package version>}` |
| `run_id` | The terminal pair's 32 lowercase hexadecimal digits |
| `capture` | Writer uses `producer-terminal`; reader also accepts explicitly imported `retrospective` evidence |
| `recorded_at` | Capture time, at or after the pair's finish time |
| `pair` | File reference to exactly `pair.json` |
| `definition` | Currently null; nonnull definitions require a future reader capability |
| `arms` | Exactly `without` and `with`, independent of their directory labels |

Each arm contains exactly `agent`, `agent_missing`, `execution_os`, `quality`.
`agent` references `<pair role name>/run/agent.json`, or is null.
`agent_missing` is null for a present record, otherwise `not-started` when
that arm never started or `not-written` when it started without leaving a
record. `unavailable-in-history` is reserved for retrospective imports.
A completed arm must have an agent record. The current writer and reader
use `execution_os: null` and `quality: "ungraded"`. Successful execution
is not evidence of correctness or a measured operating system.

A file reference is exactly `{path, sha256}`: a relative path to a regular
file under the result root, with no symlinks or traversal, and `sha256:`
followed by 64 lowercase hexadecimal digits over its exact file bytes.
This is not a canonical-JSON hash. The reporter hashes the same bytes it
parses and verifies the existing profile, prompt and add-on relationships.
Reporter 0.9 limits the handoff to 256 KiB and each pair/agent record to
16 MiB. Source-record paths are not additional file-read permissions.

Example shape (invented hashes, not a usable fixture):

```json
{
  "format": "kai-report-evidence/1",
  "producer": {"name": "kai", "version": "0.1.0"},
  "run_id": "0123456789abcdef0123456789abcdef",
  "capture": "producer-terminal",
  "recorded_at": 1794605000000,
  "pair": {"path": "pair.json", "sha256": "sha256:0000000000000000000000000000000000000000000000000000000000000000"},
  "definition": null,
  "arms": {
    "without": {
      "agent": {"path": "plain/run/agent.json", "sha256": "sha256:1111111111111111111111111111111111111111111111111111111111111111"},
      "agent_missing": null,
      "execution_os": null,
      "quality": "ungraded"
    },
    "with": {
      "agent": null,
      "agent_missing": "not-written",
      "execution_os": null,
      "quality": "ungraded"
    }
  }
}
```

Reporter 0.9 reads this file through the existing `collect kai-run` and
`[kai_run]` watcher. Old pairs without it remain readable. A handoff never
changes the sample id or the first collected metric snapshot. Late evidence
can add local provenance when the measured content is unchanged; changed
metrics are refused. No new execution is counted. The handoff checksum,
pair checksum, capture time and path appear only in `eval-local/2`; the
portable `eval-report/2` remains unchanged. Neither this writer nor this
reader implements experiment definitions, platform observations or grading.
