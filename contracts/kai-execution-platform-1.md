# kai-execution-platform/1

**P0 contract, not implemented.** This file records the guest platform
observed by Kai's launcher for one pair arm. It is not remote attestation and
does not describe the machine running Reporter.

The JSON object has exactly these required members:

| Field | Rule |
| --- | --- |
| `format` | Literal `kai-execution-platform/1` |
| `run_id` | Exact pair run id, 32 lowercase hex |
| `role` | `without` or `with` |
| `family` | `linux` in the Firecracker v1 implementation |
| `architecture` | Observed architecture string or null when not recorded |
| `observed_by` | Literal `launcher` |
| `backend` | Literal `firecracker` |
| `kernel` | Tagged SHA-256 equal to this arm's `agent.vm.kernel` |
| `firecracker` | Tagged SHA-256 equal to this arm's `agent.vm.firecracker` |

The enclosing `kai-report-evidence/2` binds this file by a FileRef and role.
Reporter also verifies run id, role and both embedded VM digests. It must not
fill the value from the collector host, requested profile or image display
name. No macOS or Windows execution is claimed by this backend contract.

Example: [`platform/without.json`](fixtures/kai-evidence-p0/platform/without.json).
