# kai-study/2: a study, as a person writes it

`study.toml` in a study's run directory, `.kai/studies/<name>/`. A person
writes it, or `kai study new` writes it with every default spelled out;
`kai study lock` freezes it, and from then on it is read only. Example:
[kai-study-2.example.toml](kai-study-2.example.toml).

The numbers 1 of this name, and of `kai-lock`, `kai-trial` and
`kai-report` up to their old numbers, belonged to an earlier kai; a reader
of those refuses these files rather than misreading them.

## Keys

A key that is not here is an error, and so is a value of another type.
"Default" is what a key left out means; the lock records the study with
every default filled in.

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `format` | string | required | `kai-study/2` |
| `name` | string | required | the study's name: letters, digits, `-` and `_`, as its directory is named |
| `image` | string | `rust` | the VM image recipe every trial runs on |
| `seed` | integer | required | the seed of the plan, 0 to 2^64 - 1; the same seed gives the same plan on any build |
| `repetitions` | integer | required | how often each arm runs each task, at least 1 |
| `[model]` `id` | string | required | the model id given to the agent, exactly; there is no default model |
| `[model]` `tools` | array of strings | `Read`, `Edit`, `Write`, `Bash`, `Glob`, `Grep` | the tools asked for; a subset of these six, each once |
| `[model]` `context_tokens` | integer | required | the model's context window in tokens, for the worst-case estimate only |
| `[model]` `max_output_tokens` | integer | 32000 | the bound of one request's output at the proxy, for the estimate only |
| `[limits]` `cpus` | integer | 2 | the VM's processors |
| `[limits]` `memory_mib` | integer | 4096 | the VM's memory |
| `[limits]` `timeout_s` | integer | 600 | a trial's time limit; reaching it is the outcome `limit` |
| `[limits]` `max_turns` | integer | 40 | the agent's turn limit; reaching it is the outcome `limit` |
| `[limits]` `max_attempts` | integer | 2 | how often a slot whose attempt ended `infra` is tried, at most |
| `[question]` `metric` | string | `total_tokens` | the primary cost; the only value today |
| `[question]` `quality_gate` | string | `pass_rate` | the primary quality; the only value today |
| `[question]` `min_saving` | float | 0.20 | the saving a ghost must show: its tokens at most `1 - min_saving` of the shadow's |
| `[question]` `margin` | float | 0.15 | how far below the shadow's pass rate the ghost's may be shown to be, in points of pass rate |
| `[question]` `alpha` | float | 0.05 | the error level; intervals are at `1 - alpha` |
| `[assumed]` `within_task_sd` | float | 0.35 | the assumed spread of log tokens within a task and arm, until a pilot measured it |
| `[assumed]` `task_by_arm_sd` | float | 0.15 | the assumed task-by-arm spread |
| `[[arms]]` | array of tables | required | two arms or more, see below |
| `[[tasks]]` | array of tables | required | one task or more, see below |

A limit is an execution input, the same for every arm. None of them is a
budget: approval and money are the proxy's.

An arm:

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `name` | string | required | the arm's name in plans and records, unique in the study |
| `role` | string | required | `shadow` (exactly one), `ghost` or `placebo` |
| `addons` | array of strings | empty | the add-ons the arm adds, by name, in order; the shadow adds none |
| `activation` | string | none | the line put before the task's instruction in the arm's first input; none for the shadow |

A task:

| Key | Type | Meaning |
| --- | --- | --- |
| `id` | string | the task's id, unique in the study |
| `dir` | string | the task's directory (`kai-task/1`), relative to this file. `kai study lock` copies it to `tasks/<id>/`; from then on only the copy is read |

The study's id in the lock is the [canonical](kai-canonical-json-1.md) id
of this file read as JSON, every default filled in: a table is an object,
an array of tables an array, a TOML integer an integer and a TOML float a
float. TOML's dates, NaN and infinities are refused. A comment, a blank
line or a default written out where it was left out is no change; a
changed value is.
