# Contract: `tome catalog list`

## Synopsis

```
tome catalog list [--json] [-v|-vv]
```

## Arguments

| Arg / flag | Type | Default | Description |
|---|---|---|---|
| `--json` | flag | off | Emit one JSON object per line (newline-delimited JSON) on stdout. |
| `-v` / `-vv` | flag | off | Increase diagnostic logging verbosity. |

## Behaviour

1. Load `config.toml`.
2. For each catalog (deterministic order: alphabetical by `name` — `BTreeMap` natural order), print one row.
3. If no catalogs registered, print a single human-readable message in human mode or an empty stream in `--json` mode.

## Stdout

**Human mode** — a fixed-width table on stdout. Width auto-adapts to terminal width; truncation indicated with `…`.

```
NAME              URL                                                   REF   PLUGINS  LAST SYNCED
midnight-experts  https://github.com/midnight/midnight-experts          main  2        2026-05-11T14:23:00Z
my-private        https://github.com/me/private-plugins                 v1.0  4        2026-05-10T09:15:42Z
```

When zero catalogs are registered, in human mode:
```
No catalogs registered. Use `tome catalog add <source>` to add one.
```

**JSON mode** — newline-delimited JSON (one record per line, no enclosing array, suitable for `jq -c`):

```jsonc
{"name":"midnight-experts","url":"https://…","ref":"main","plugin_count":2,"last_synced":"2026-05-11T14:23:00Z"}
{"name":"my-private","url":"https://…","ref":"v1.0","plugin_count":4,"last_synced":"2026-05-10T09:15:42Z"}
```

When zero catalogs are registered, in `--json` mode: no output on stdout, exit 0.

## Exit codes

| Code | Category | Conditions |
|---|---|---|
| 0 | success | including the zero-catalogs case |
| 1 | internal | unexpected error |
| 7 | I/O failed | config file unreadable |

## Notes

- `plugin_count` is derived from the cached manifest, **not** by re-running `git`. If the cache is stale (no `tome catalog update` since upstream changes), the count is the count from the last successful sync. This is documented; users wanting a fresh count run `tome catalog update <name>` first.
- The `URL` column truncates from the middle when necessary, preserving the scheme prefix and the path suffix.
- `LAST SYNCED` is rendered in the local timezone in human mode and as RFC 3339 UTC in `--json` mode.
