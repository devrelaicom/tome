# Contract: `tome catalog update`

## Synopsis

```
tome catalog update [<name>] [--json] [-v|-vv]
```

## Arguments

| Arg / flag | Type | Default | Description |
|---|---|---|---|
| `<name>` (positional, optional) | string | (all catalogs) | If supplied, refresh only the named catalog. If omitted, refresh every registered catalog **sequentially** (FR-005, no parallel updates in Phase 1). |
| `--json` | flag | off | Emit one JSON record per refreshed catalog. |
| `-v` / `-vv` | flag | off | Increase diagnostic logging verbosity. |

## Behaviour (single catalog)

1. Look up the catalog. If not registered → `CatalogNotFound` (exit 3).
2. If `--ref` was originally a SHA (regex `^[0-9a-f]{7,40}$`): print an informational message ("catalog `<name>` is pinned to `<sha>`; use `tome catalog add --ref` to change") and **exit 0**. Do not invoke Git. (FR-008)
3. Otherwise: spawn `git fetch origin` in the catalog's cache directory.
4. Spawn `git reset --hard origin/<ref>`. If `<ref>` was a tag, use the tag's full ref name (`refs/tags/<ref>`).
5. Re-parse the catalog manifest from the new HEAD. On failure → `ManifestInvalid` (exit 5); the cache directory **does** reflect the new HEAD (Git already moved it), but `last_synced` is **not** updated and the registry entry's `plugin_count` is **not** updated. The user can re-run `update` after fixing upstream.
6. Atomically update `config.toml` setting `last_synced = now`.
7. Emit confirmation.

## Behaviour (refresh-all)

1. Load registry. Build the list of catalogs in alphabetical order.
2. For each catalog, run the single-catalog behaviour above **sequentially**.
3. **Fail fast** (FR-007): on the first failure, abort the remaining catalogs and exit with that failure's exit code. Already-refreshed catalogs are **not** rolled back (cross-catalog updates have no transactional semantics; this is documented in the spec's Edge Cases). The error names the catalog that failed.

## Stdout

**Human mode (single)**:
```
Refreshed `midnight-experts` (ref: main, plugins: 2, advanced 3 commits).
```

**Human mode (refresh-all)** — one line per catalog:
```
Refreshed `midnight-experts` (ref: main, plugins: 2, advanced 3 commits).
Refreshed `my-private` (ref: v1.0, plugins: 4, already up-to-date).
```

**Human mode (SHA-pinned, no-op)**:
```
Catalog `pinned-experiment` is pinned to a64f3c1; use `tome catalog add --ref` to change.
```

**JSON mode** — newline-delimited:
```jsonc
{"refreshed":{"name":"midnight-experts","ref":"main","plugin_count":2,"advanced_commits":3,"last_synced":"…"}}
{"pinned":{"name":"pinned-experiment","ref":"a64f3c1"}}
```

## Exit codes

| Code | Category | Conditions |
|---|---|---|
| 0 | success | every catalog refreshed or skipped (SHA-pinned) |
| 3 | catalog not found | named catalog not registered |
| 5 | manifest invalid | new HEAD's manifest fails validation |
| 6 | git failed | clone/fetch/reset error (stderr scrubbed and surfaced) |
| 7 | I/O failed | registry write or cache I/O failed |
| 8 | interrupted | SIGINT during fetch/reset |

## Atomicity guarantees

- For each catalog, the cache directory ends in either the pre-update state (on failure or interruption) or the fully-updated state. There is no intermediate state observable to a subsequent invocation.
- For refresh-all, individual catalogs are independently atomic; previously refreshed catalogs are not rolled back when a later catalog fails (documented).

## Interactions with credential scrubbing

`git fetch`'s stderr passes through `git::scrub_credentials` before any handling, identical to `tome catalog add`.
