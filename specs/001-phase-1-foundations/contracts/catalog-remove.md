# Contract: `tome catalog remove`

## Synopsis

```
tome catalog remove <name> [--force] [--json] [-v|-vv]
```

## Arguments

| Arg / flag | Type | Default | Description |
|---|---|---|---|
| `<name>` (positional) | string | — | The display name of a registered catalog. |
| `--force` | flag | off | Skip the confirmation prompt. **Required** when stdin is not a TTY. |
| `--json` | flag | off | Emit a JSON confirmation record. |
| `-v` / `-vv` | flag | off | Increase diagnostic logging verbosity. |

## Behaviour

1. Look up `<name>` in `config.toml`. If not registered → `CatalogNotFound` (exit 3).
2. If `--force` is not set and stdin is a TTY: prompt
   `Remove catalog 'midnight-experts' and its local cache at <path>? [y/N]`. Default no. On non-`y`/`yes` answer: exit 0 (no-op).
3. If `--force` is not set and stdin is **not** a TTY: exit with `Usage` (exit 2) and the message
   `'tome catalog remove' requires --force in non-interactive contexts`.
4. Atomically write a new `config.toml` without the entry (tempfile-and-rename).
5. Recursively remove the cache directory. Errors during cache removal are logged at `WARN` (stderr) but do **not** fail the command — the registry is the source of truth.
6. Emit confirmation.

## Stdout

**Human mode**:
```
Removed catalog `midnight-experts` (cache cleared at /Users/alice/.local/share/tome/catalogs/a3f9c1b2…).
```

**JSON mode**:
```json
{
  "removed": {
    "name": "midnight-experts",
    "url": "https://github.com/midnight/midnight-experts",
    "cache_path": "/Users/alice/.local/share/tome/catalogs/a3f9c1b2…"
  }
}
```

## Exit codes

| Code | Category | Conditions |
|---|---|---|
| 0 | success | catalog removed, or user declined the prompt |
| 2 | usage | missing `--force` in non-TTY context |
| 3 | catalog not found | `<name>` not registered |
| 7 | I/O failed | registry write failed (cache cleanup failures do **not** trigger this) |
| 8 | interrupted | SIGINT during the operation |

## Atomicity guarantees

- The registry mutation is atomic. Either the entry is fully present in `config.toml` or fully absent.
- Cache directory removal is best-effort; partial removal is acceptable because the next `add` re-creates the directory in a tempdir.

## Examples

```sh
# Interactive
tome catalog remove midnight-experts

# Scriptable
tome catalog remove midnight-experts --force --json
```
