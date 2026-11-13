# kai-report-cohort/1

An immutable, local declaration of intended comparison cells. It is
referenced by `kai-report-definition/1`; it does not prove that any cell ran
or that observed executions are comparable.

The object has exactly `format`, `id`, `cells`. `format` is the literal
`kai-report-cohort/1`. `id` is 128 random bits encoded as 32 lowercase hex
characters. `cells` is an array of unique cell records with exactly:

| Field | Rule |
| --- | --- |
| `id` | Unique nonempty local label |
| `task_digest` | `sha256:` plus 64 lowercase hex |
| `snapshot_digest` | `sha256:` plus 64 lowercase hex |
| `baseline_digest` | Tagged SHA-256 of the declared baseline identity |
| `treatment_digest` | Tagged SHA-256 of the declared treatment identity |
| `execution_os` | `linux`, `macos`, `windows`, `other` or null |
| `repetitions` | Positive integer |

Kai's pair adapter derives the baseline digest from the exact `pair.profile`
and the treatment digest from `pair.treatment` using the preimages documented
in the integration specification. This identifies requested inputs only;
Reporter checks observed runtime pins before matching cells. Null OS means no
OS was planned. It does not match an arbitrary observed OS.

Example: [`cohort.json`](fixtures/kai-evidence-p0/cohort.json).
