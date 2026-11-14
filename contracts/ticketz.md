# Ticket mirror over HTTP

kai uses ticketz 0.2+ through hatch. Both tools are installed separately;
the mirror service must be running before ticket commands are used.
The default base URL is `http://127.0.0.1:4287`. `--store URL`, `KAI_STORE`
or `[store] url` overrides it, in that order. HTTPS is required off loopback;
URLs containing credentials, a query or fragment are refused. No redirect
is followed and no GitHub credential is sent by kai.

## Service

Run a dedicated hatch instance under prxy with ticketz's `hatch.toml`
policy. It permits snapshot, sync, list, show, status and reset, and forbids
HTTP callers from supplying `db` or `api`. Configure `TICKETZ_DB`, the
GitHub route and secret on the server. `TICKETZ_TOKEN_ENV` may select the
existing secret variable that prxy replaces with a placeholder. Use the
proxy's fixed GitHub upstream, a read-only repository token and a private
cache. Hatch binds loopback and has no authentication; remote use requires
an authenticated gateway outside this contract.

Use hatch `--port 4287 --timeout 300 --max-output 16777216` for a
service with a five-minute sync limit and 16 MiB of tool output. kai's
request timeout is 310 seconds; it accepts at most 32 MiB of HTTP response,
including the JSON-escaped stdout. Larger boards need matching limits.

## Calls and envelope

All requests use POST and a JSON object. There are three calls:

| Path | Body | stdout record |
| --- | --- | --- |
| `/ticketz/snapshot` | `{}` | `ticketz-snapshot/1` |
| `/ticketz/sync` | `{"repo":"example-org/board","full":true,"actor-permissions":true,"json":true}` | `ticketz-sync/1` |
| `/ticketz/reset` | `{"json":true}` | `ticketz-reset/1` |

Hatch returns its process envelope, for example:

```json
{"exit_code":0,"stdout":"{\"version\":\"ticketz-snapshot/1\",\"issues\":[],\"boards\":[]}\n","stderr":"","truncated":false}
```

Non-success HTTP, absent/nonzero process status, truncated output, malformed
JSON or an incompatible record version is an error, never an empty cache.
Extra fields are permitted for additive protocol changes.

## Snapshot

`ticketz-snapshot/1` has `issues` and `boards`, read from one SQLite
transaction inside ticketz. Each issue is `ticketz-issue/1`: repository,
number, title, URL, state, optional author, created/updated timestamps,
original body, labels and labeled events, with completeness flags for both
collections. An event has label, optional actor and timestamp `at`.

Each board is `ticketz-status/1`: repository, issue count, last sync,
optional cursor/full-sync time, optional `actors` map and `board_files`.
An actor has `observed_at` plus either `permission` and optional `role`,
or `permission_unknown`. Files have branch, commit, paths, completeness
and observation time. Omitted observations stay unknown. The client
rejects duplicate boards/issues, invalid issue states or identities,
issues without a board and inconsistent issue counts.

kai interprets forms and applies qualification rules to this snapshot.
The database path and schema are private to ticketz. No SQLite dependency
or database migration exists in kai.

## Sync and reset

A sync summary contains repository, full flag, fetched/kept counts and
`removed` only for a full sync. Optional `board_files_unknown` explains why
a file listing could not refresh; the old observation remains. A failure
fetching issues or permission responses leaves the previous snapshot intact.

Reset reports `removed`. It clears the service's entire cache, which may
be shared by other clients. kai never deletes the legacy `.kai/store.sqlite`.
