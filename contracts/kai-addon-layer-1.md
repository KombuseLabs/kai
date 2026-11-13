# kai-addon-layer/1

An add-on is a set of files of a publisher's repository at one commit,
each pinned by its SHA-256, and files kai writes beside them, laid
read-only under `/opt/kai/addons/<name>/` in a layer of their own.
`layer.json` lies beside `layer.sqfs` in an entry of kai's VM cache,
`addons/<key>/`, and is written last, when the layer was packed and read
back. `kai agent fetch --addon <name>` writes it; a session that is given
the add-on reads it back first and uses the layer only when the JSON names
the key kai expects and the layer has the length and SHA-256 the JSON
names, hashed afresh. `agent.json` ([kai-agent-run/1](kai-agent-run-1.md))
carries each add-on's record in `addons`.

| Field | Meaning |
| --- | --- |
| format | `kai-addon-layer/1` |
| key | `sha256:<hex>`: SHA-256 of the identity as canonical JSON, keys sorted and no white space. The identity is every field but `key`, `inventory`, `layer` and `layer_len` |
| name | The add-on's name: lower-case letters, digits and `-` |
| dir | Its directory in the guest, `/opt/kai/addons/<name>` |
| repository, commit | The publisher's repository and the full commit every pinned file is taken from |
| licence, licence_file | The licence the repository grants at that commit, as an SPDX identifier a person read in the text, and where that text lies in the guest |
| files | The pinned files, each with `source` (its path in the repository), `path` (in the guest), `sha256` (`sha256:<hex>`) and `length` |
| written | The files kai writes, compiled into kai, such as a plugin's manifest: `path`, `sha256` and `length`. Never a change of a pinned file |
| inventory | `sha256:<hex>`: what the layer holds. SHA-256 of one line per directory and file, in order of the path: `d 0755 0/0 - - <path>` for a directory, `f 0644 0/0 <length> <sha256> <path>` for a file |
| layer, layer_len | SHA-256 (`sha256:<hex>`) and length of `layer.sqfs`: its bytes |
| packing, mksquashfs | `mksquashfs`'s options and version, those of an agent's layer ([kai-agent-layer/1](kai-agent-layer-1.md)) |

The two digests answer different questions and are never merged. The
inventory names what the layer holds, so it is the same wherever the same
files are packed. The layer's digest names bytes, which another
`mksquashfs` may lay out otherwise; the key, which names the version, then
differs too.

Every file of the layer is root's and mode 0644, every directory 0755;
the layer holds the listed files and their directories, nothing else. A
file is downloaded from its URL at the commit, never from an archive made
on request, and refused unless its SHA-256 is the pin's.

An entry whose JSON does not read, names another key or another field of
the identity, or another inventory, or whose layer has other bytes is not
used: `kai agent fetch --addon` moves it aside and builds it again.

The [example](kai-addon-layer-1.example.json) is an invented add-on: a
skill, its licence and a plugin manifest kai writes; no value is a real
repository's. A writer test builds the JSON kai would write for those pins
and compares it with the example, and reads the example back as an
entry's JSON.
