# kai-vm-run/1: how a microVM run went

`result.json` in a run's directory, which `kai vm run` makes (by default
under `runs/` in kai's cache). kai writes it last, once, under a temporary
name that it then renames, so a reader never sees half of it. Example:
[kai-vm-run-1.example.json](kai-vm-run-1.example.json).

The run directory beside it:

| File | What it is |
| --- | --- |
| `result.json` | this format |
| `out/stdout`, `out/stderr` | what the command wrote, read back from the job drive; `out/stderr` also holds kai's init's line when the command could not start |
| `console.log` | the VM's serial console: Firecracker's first line, the kernel's errors, the init's line when it failed; carriage returns removed |
| `firecracker.log` | Firecracker's own log, warnings and worse |
| `vm.json` | what Firecracker was started with |
| `job.ext4` | the job drive after the run: `job.json`, `tree/`, the files given with `--file`, `out/`, and `home/` for a command that did not run as root |
| `proxy.jsonl` | with `--proxy`: fence's ledger, one line per connection, host and port, never what was sent |

## Fields

| Field | Type | When | Meaning |
| --- | --- | --- | --- |
| `format` | string | always | `kai-vm-run/1` |
| `outcome` | string | always | how the run ended; see below |
| `exit_code` | integer | `exited` only | the command's exit status; 127 when the program was not found, 126 when it could not be run |
| `signal` | integer | `signalled` only | the number of the signal that ended the command |
| `reason` | string | `killed`, `guest-failed`, `launch-failed` | what went wrong, for people |
| `output_complete` | boolean | not for `launch-failed` | whether `out/` holds all the command wrote |
| `image` | object | always | the image: `recipe` and `id` for one made from a recipe (`kai vm image build`), and `file`, `sha256:` and the image file's SHA-256, when the file could be read |
| `kernel` | string | always | `sha256:` and the guest kernel's SHA-256 |
| `firecracker` | string | always | `sha256:` and the Firecracker program's SHA-256 |
| `initramfs` | string | always | `sha256:` and the initramfs's SHA-256; it holds kai's init and nothing else |
| `cpus` | integer | always | processors of the VM |
| `memory_mib` | integer | always | memory of the VM, in MiB |
| `timeout_s` | integer | always | the command's time limit, in seconds |
| `wall_ms` | integer | when Firecracker started | milliseconds from Firecracker's start to its end, on the host |
| `command_ms` | integer | `exited`, `signalled`, `timeout` | milliseconds the command ran, measured in the guest |
| `layers` | array of strings | when the run had layers | each layer file's SHA-256 as `sha256:<hex>`, bottom first |
| `upper_out` | boolean | when `--upper-out` was given | whether what the command wrote outside its tree was kept as a layer; only a command that exits with 0 leaves one |
| `tree_out` | boolean | when `--tree-out` was given | whether the tree was written there |
| `proxy` | object | when the run had `--proxy` | what went through it, counted from `proxy.jsonl` (fence's ledger): `allowed` and `refused` connections, `refused_targets` (`host:port` → count); in learn mode `would_deny` and `would_deny_targets`, what no rule named and was let through |

A value nobody measured is absent, never 0.

## Outcomes

| `outcome` | Meaning | Whose doing |
| --- | --- | --- |
| `exited` | the command ended by itself | the command |
| `signalled` | a signal ended the command | the command |
| `timeout` | the guest ended the command at its time limit; its output up to then is complete | the command |
| `killed` | the VM did not end in time after the limit and kai killed it; the output may lack its last seconds | the infrastructure |
| `guest-failed` | the VM ended without a status: kai's init or the kernel failed; `console.log` says more | the infrastructure |
| `launch-failed` | the VM did not run: Firecracker did not start, or ended with an error before the guest wrote a status | the infrastructure |

A failure of the infrastructure is never a result of the command.

## What a reader may rely on

- The command ran as root in the guest unless the run says otherwise.
  Root in the guest can change anything on the job drive, `out/` included:
  `out/` is what the command left there, not more.
- A command given another user (`--user`) owns `/job/tree`, everything in
  it, and `/job/home`, mode 0700, which kai's init makes when it is missing.
  Everything else on the job drive (its root, `job.json`, the files given
  with `--file`, `out/`) the init makes root's and takes group's and
  others' write, setuid and setgid away, whatever uids and modes the drive
  was made with, so that nothing given runs as root: the command reads
  what their modes let others read and writes none of it, and cannot
  make, remove or replace a name in `/job`. The init does both before the
  command starts, never following a link. The image and the layers are
  read-only.
- `out/` holds plain files only; kai reads them back by fixed names, never by
  names the guest chose.
