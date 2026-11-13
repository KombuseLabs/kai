# kai-pair/1

`agent compare` and `run <name>` write `pair.json` beside the two arm
folders. This is a local execution record, not a grade or a submission.
It contains task text and must not be published.

The file is written atomically before execution (`state: running`) and
again after both executions settle (`finished`), including failed arms.
SIGINT/SIGTERM publish `interrupted`. A hard crash may leave `running`;
readers must not infer completion from file age.

| Field | Meaning |
| --- | --- |
| `format` | `kai-pair/1` |
| `run_id` | 128 random bits, lowercase hex; independent of location and outcomes |
| `producer` | Kai package version |
| `state` | `running`, `finished`, `interrupted` |
| `started_at`, `finished_at` | Unix milliseconds in UTC; end is null while running |
| `task` | Optional local `name`, exact `prompt`, `snapshot` digest and task `digest` |
| `profile` | Model, tools, image, CPUs, memory, timeout, max turns, output-token ceiling, mock flag, agent and SDK versions |
| `treatment` | Exact `extra_prompt`, its SHA-256 `digest`, optional add-on name and pinned commit |
| `without`, `with` | Role records, regardless of their directory names |

Each role contains `name` (its direct child directory), `state`,
`started_at`, `finished_at` and `elapsed_ms`. Arm states are `pending`,
`running`, `completed`, `incomplete`, `failed`, `interrupted`.
`completed` says that execution and export finished, never that its answer
is correct. Duration uses a monotonic clock around execution and export,
excluding time waiting for the other arm. If interruption prevents the
measurement from settling, it remains null.

The input tree is snapshotted once in `input/`. Its digest matches
`helpers::tree_digest` and eval-reporter's tree digest: sorted depth-first
file paths, NUL, executable marker plus content SHA-256, newline; symlinks
record `link:` plus the escaped target and are never followed. Empty
directories, timestamps and ownership do not affect identity.

Task SHA-256 hashes the UTF-8 bytes
`kai-task/1:<prompt byte length>:<exact prompt>:<snapshot hex>`.
Treatment SHA-256 hashes the exact extra text. For add-ons, the reader
also includes their pinned identity in the treatment key. A plain extra
prompt is separated from the task by two newlines; an add-on activation
uses the session protocol's single newline.

All previous arm files remain in place: `status.txt`, `answer.md`,
`run/agent.json`, `run/events.jsonl`, and exported `tree/`.
The external eval-reporter reads terminal pairs with:

```sh
eval-reporter collect kai-run .runs/ponytail/results/<timestamp> --codebase none
```

No reporter is required to execute a comparison. Old runs without this
record remain inspectable as files; they are not silently reconstructed
from an editable task definition.

After terminal publication, Kai writes the immutable
[report-evidence.json](kai-report-evidence-1.md) handoff. It hashes this
pair and available agent records without changing their contents.
