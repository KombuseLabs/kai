# kai-agent-layer/1

`layer.json` lies beside `layer.sqfs` in an entry of kai's VM cache,
`agents/<key>/`, and is written last, when the layer was packed and read
back. It says what the layer holds and how it was made. `kai agent fetch`
writes it; the session runtime reads it back before every use and uses the
layer only when the JSON names the key kai expects and the layer has the
length and SHA-256 the JSON names, hashed afresh. `agent.json`
([kai-agent-run/1](kai-agent-run-1.md)) carries it as its `layer`.

| Field | Meaning |
| --- | --- |
| format | `kai-agent-layer/1` |
| key | `sha256:<hex>`: the entry's key. It hashes the program's pin, its path, the packing options and `mksquashfs`'s version, and, when there are members, the agent's name and version and the members |
| agent, version | The agent's name in kai's records and its version, as the program reports it |
| binary, binary_len | SHA-256 (`sha256:<hex>`) and length of the program |
| path | Where the program lies in the guest, under `/opt/kai/agents/` |
| layer, layer_len | SHA-256 (`sha256:<hex>`) and length of `layer.sqfs`: its bytes |
| packing | `mksquashfs`'s options: every file root's, zstd, no extended attributes, every time stamp 0 |
| mksquashfs | `mksquashfs`'s version |
| members | Absent for a single program. Otherwise the runtime's other files, each with path, SHA-256 (bare hex), length and version: a pinned download, or a file compiled into kai such as the session wrapper, whose version is its protocol's |

Every file of the layer is root's and mode 0755, every directory 0755;
the layer holds the listed files and their directories, nothing else.
Two packs with the same key are the same bytes on one `mksquashfs`.

An entry whose JSON does not read, names another key or identity, or
whose layer has other bytes is not used: `kai agent fetch` moves it aside
and builds it again, and a session refuses to start until then.

The [example](kai-agent-layer-1.example.json) is an invented runtime of a
program, a supporting file and a wrapper; no value is a real artefact's.
A writer test packs nothing: it builds the JSON kai would write for those
pins and compares it with the example, and reads the example back as an
entry's JSON.
