# Contracts

The file formats kai reads and writes, described for anyone who reads those
files with another program. One page per format.

| Format | File | Written by |
| --- | --- | --- |
| [kai-agent-session/1](kai-agent-session-1.md) | `events.jsonl`, host/guest stream | `agent` and its guest wrapper |
| [kai-agent-run/1](kai-agent-run-1.md) | `agent.json` | `agent` |
| [kai-vm-run/1](kai-vm-run-1.md) | `result.json` of a microVM run | `kai vm run` (crates/vm) |

These rules hold for every format:

- **Every file names its format**, as a name and a number (`<name>/<number>`).
  A change that an existing reader could misread gets a new number, and the
  page changes in the same commit.
- **Every page comes with an example file.** The crate that writes the
  format has a test that it still writes the example, and a crate that
  reads it has a test that it reads the example.
- **Unknown is not zero.** A value nobody recorded is absent or `null`,
  never 0.
