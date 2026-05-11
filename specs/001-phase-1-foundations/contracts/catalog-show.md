# Contract: `tome catalog show`

## Synopsis

```
tome catalog show <name> [--json] [-v|-vv]
```

## Arguments

| Arg / flag | Type | Default | Description |
|---|---|---|---|
| `<name>` (positional) | string | — | The display name of a registered catalog. |
| `--json` | flag | off | Emit the full manifest as a single JSON object on stdout. |
| `-v` / `-vv` | flag | off | Increase diagnostic logging verbosity. |

## Behaviour

1. Look up the catalog. If not registered → `CatalogNotFound` (exit 3).
2. Read `tome-catalog.toml` from the cache directory.
3. Parse it (full strict validation). On failure → `ManifestInvalid` (exit 5) — this can happen if the user manually edited the cache, which is unsupported but should produce a clear error.
4. Emit the manifest.

## Stdout

**Human mode**:
```
midnight-experts (v0.1.0)
  Expert plugins for working with the Midnight privacy chain
  Owner: Midnight Labs <plugins@midnight.network>
  Source: https://github.com/midnight/midnight-experts (ref: main)
  Last synced: 2026-05-11T14:23:00Z

Plugins:
  midnight-compact-expert        ./plugins/midnight-compact-expert
  midnight-dapp-expert           ./plugins/midnight-dapp-expert
```

**JSON mode**:
```json
{
  "name": "midnight-experts",
  "description": "Expert plugins for working with the Midnight privacy chain",
  "version": "0.1.0",
  "owner": { "name": "Midnight Labs", "email": "plugins@midnight.network" },
  "registered": {
    "url": "https://github.com/midnight/midnight-experts",
    "ref": "main",
    "last_synced": "2026-05-11T14:23:00Z"
  },
  "plugins": [
    { "name": "midnight-compact-expert", "source": "./plugins/midnight-compact-expert" },
    { "name": "midnight-dapp-expert", "source": "./plugins/midnight-dapp-expert" }
  ]
}
```

## Exit codes

| Code | Category | Conditions |
|---|---|---|
| 0 | success | manifest displayed |
| 3 | catalog not found | `<name>` not registered |
| 5 | manifest invalid | cached manifest fails validation (cache corrupted or hand-edited) |
| 7 | I/O failed | cache file unreadable |
