# kai-lock/2: what a study froze before its first trial

`lock.json` in a study's run directory. `kai study lock` writes it once,
after `kai study check` passed, under a temporary name it then renames;
an existing lock is never written again. Example (the study of
[kai-study-2.example.toml](kai-study-2.example.toml), with invented
digests): [kai-lock-2.example.json](kai-lock-2.example.json).

`kai study run` reads the lock, recomputes everything below from what is
on disk and what it would run, and refuses a study where anything differs,
naming the first field that does (such as `study.content.model.id`,
`tasks[fixture-add].files[instruction.md]` or `plan.seed`). A changed
study is a new study with a new name.

## Fields

| Field | Type | Meaning |
| --- | --- | --- |
| `format` | string | `kai-lock/2` |
| `canonical` | string | `kai-canonical-json/1`, the rule every id here is made by |
| `study` | object | `name`; `content`, the study file ([kai-study/2](kai-study-2.md)) as JSON with every default filled in; `id`, the canonical id of `content` |
| `baseline` | object | `record`, the [kai-environment/1](kai-environment-1.md) every arm runs on, its pins resolved; `id`, its canonical id |
| `arms` | array of objects | in the study file's order: `name`, `role` and `variant`, which holds `record` ([kai-variant/1](kai-variant-1.md)) and `id`. Every variant names `baseline.id` |
| `tasks` | array of objects | in the study file's order: `id`; `files`, every file of the task's copy under `tasks/<id>/`, sorted by path, each with `path` (relative, `/`-separated), `sha256` (`sha256:` and its SHA-256) and `mode` (four octal digits, such as `0644`); `inventory`, the canonical id of `files` |
| `plan` | object | `seed`, `repetitions`, `arms` and `tasks` (the names in the order the plan was drawn with) and `slots`: every trial as `task`, `arm`, `repetition` (from 1) and `position` (from 1, the order trials run in) |
| `kai` | object | `version`, the kai that locked the study |
| `locked_at` | string | when, in UTC, RFC 3339 |

A task's `files` cover every byte the task brings: its instruction, its
hidden tests, its reference and its `task.toml` (`kai-task/1`, written
with S5, will name further digests here, such as what qualified it; until
then the inventory is all a lock says of a task). A symbolic link or a
special file in a task is refused at the lock.

Nothing in a lock is a budget, an approval or a price. The model id is
the string given; which model answered is in each trial's record.
