# kai-report-definition/1

**P0 contract, not implemented.** This immutable local declaration is copied
into the result root before either arm starts. It describes the question and
intended comparison; it does not schedule work, prove treatment delivery,
grade outputs or authorize publication. Historical definitions are explicitly
`retrospective`.

The file is at most 64 KiB. Every field below is required; null is explicit.
Unknown keys, duplicate keys, malformed digests, non-finite numbers and
unsupported references are rejected.

| Field | Type and rule |
| --- | --- |
| `format` | Literal `kai-report-definition/1` |
| `declaration` | `before-execution` or `retrospective` |
| `declared_at` | Nonnegative UTC Unix milliseconds; before both starts for `before-execution` |
| `question` | Null or the object below |
| `cohort` | Null or `{id, manifest}` where `manifest` is a FileRef to `kai-report-cohort/1` |
| `assignment` | Null or `{cell_id, repetition}`; both null when `cohort` is null |

`question` has exactly `text`, `hypothesis`, `primary_metric`, `target`,
`quality_requirement`, `analysis_ref`:

- `text` and a non-null `hypothesis` are local prose, each at most 4,000
  characters. The hypothesis must be projected verbatim when present and
  remains a declaration, never a result.
- `primary_metric` is `usd`, `input_tokens`, `cache_read_tokens`,
  `cache_write_tokens`, `output_tokens` or `elapsed_ms`.
- `target` is null or `{effect: "ratio", operator: "at-most", value}` where
  `value` is positive and finite. It is a declared target, not a decision.
- `quality_requirement` is `none` or `external-evidence`; it expresses a
  requirement, not evidence that it was met.
- `analysis_ref` is null in v1. No analysis schema or estimator is implied.

`kai-report-cohort/1` has `{format, id, cells}`. `id` is 32 lowercase hex
characters. Every cell has exactly `id`, `task_digest`, `snapshot_digest`,
`baseline_digest`, `treatment_digest`, `execution_os`, and `repetitions`.
Cell ids are unique nonempty labels; digests are `sha256:` values; OS is
`linux`, `macos`, `windows`, `other` or null; repetitions is positive.
The assigned `cell_id` must exist, and `repetition` is positive and within
the cell's declared count. These declarations do not establish actual
comparability: Reporter also checks the observed task, snapshot, profile,
runtime and treatment pins.

The declaration and cohort file are hashed by exact bytes in the enclosing
`kai-report-evidence/2` handoff. All text, task membership and cohort details
remain local unless separately allowlisted for a portable report.

Example: [`definition.json`](fixtures/kai-evidence-p0/definition.json).
