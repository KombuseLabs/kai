# kai-variant/1: what one arm adds to the baseline

A record, not a file of its own: the lock ([kai-lock/2](kai-lock-2.md))
holds one per arm, beside its [canonical](kai-canonical-json-1.md) id.
The shadow's variant is the empty one: no add-on, no activation. Example
(the ghost, with invented digests):
[kai-variant-1.example.json](kai-variant-1.example.json).

It is the minimal form of mvp-environments' variant record. The arm's
name (`shadow`, `ghost`) is the study's alias for it and is not part of
the record or its id.

| Field | Type | When | Meaning |
| --- | --- | --- | --- |
| `format` | string | always | `kai-variant/1` |
| `baseline` | string | always | the id of the [kai-environment/1](kai-environment-1.md) record it runs on |
| `addons` | array of objects | always | the add-ons in the order they are given to the agent; empty for the shadow |
| `activation` | string | when the arm has one | the line that precedes the task's instruction in the first input, such as `/ponytail full` |

An add-on:

| Field | Type | Meaning |
| --- | --- | --- |
| `kind` | string | how it reaches the agent: `plugin`, a local plugin directory |
| `name` | string | its name, as `kai agent fetch --addon` knows it |
| `key` | string | its build key (`sha256:`), the key of its `kai-addon-layer/1` entry: what it was made from |
| `inventory` | string | `sha256:`, the digest of what its layer holds |
| `layer` | string | `sha256:`, the digest of its layer's bytes |

Two variants with the same baseline, add-ons and activation have the same
id. A variant whose `baseline` is not the study's is refused.
