# Contract: `tome catalog add`

## Synopsis

```
tome catalog add <git-url|owner/repo|path> [--name <name>] [--ref <branch|tag|sha>] [--json] [-v|-vv]
```

## Arguments

| Arg / flag | Type | Default | Description |
|---|---|---|---|
| `<source>` (positional) | string | — | The catalog source. One of: `owner/repo` (expanded to `https://github.com/owner/repo`); a full URL (`https://…`, `http://…`, `git@…`, `file://…`); or a bare local filesystem path (treated as `file://` after canonicalisation). |
| `--name <name>` | string | (from manifest) | Override the catalog's display name. Takes precedence over the manifest's `name` field. |
| `--ref <ref>` | string | (upstream default branch) | A branch, tag, or commit SHA to track. SHAs (`^[0-9a-f]{7,40}$`) are pinned; branches/tags are tracking. |
| `--json` | flag | off | Emit a JSON record on stdout instead of human-readable output. |
| `-v` / `-vv` | flag | off | Increase diagnostic logging verbosity (stderr). Orthogonal to `--json`. |

## Behaviour

1. Resolve `<source>` to a canonical Git URL.
2. Compute `path = sha256(url)` and refuse if a catalog with that path already exists (`CatalogAlreadyExists`, exit 4).
3. `git clone --depth 1 [--branch <ref>] <url> <tempdir>` into a tempdir alongside the final cache directory (same filesystem).
4. If `--ref` is provided and is a SHA, `git fetch --depth 1 origin <sha> && git checkout <sha>` in the tempdir.
5. Parse `tome-catalog.toml` at the tempdir root. On any validation failure → `ManifestInvalid` (exit 5); tempdir is deleted.
6. Atomically rename tempdir → final cache directory.
7. Compute the display name (`--name` override > manifest `name`). If a catalog with that display name is already registered, → `CatalogAlreadyExists` (exit 4); the cache directory is deleted.
8. Write the new `CatalogEntry` into `config.toml` atomically (write-and-rename).
9. Emit confirmation output.

## Stdout

**Human mode**:
```
Added catalog `midnight-experts` from https://github.com/midnight/midnight-experts (ref: main, plugins: 2).
```

**JSON mode**:
```json
{
  "added": {
    "name": "midnight-experts",
    "url": "https://github.com/midnight/midnight-experts",
    "ref": "main",
    "plugin_count": 2,
    "last_synced": "2026-05-11T14:23:00Z"
  }
}
```

## Exit codes

| Code | Category | Conditions |
|---|---|---|
| 0 | success | catalog added |
| 2 | usage | invalid `<source>` form, conflicting flags, etc. |
| 4 | catalog already exists | display name collision OR cache path (URL hash) collision |
| 5 | manifest invalid | `tome-catalog.toml` missing, malformed, or fails any validation rule in `data-model.md` §3 |
| 6 | git failed | clone or fetch failed; stderr from `git` is scrubbed and surfaced |
| 7 | I/O failed | filesystem error (permission denied, disk full, etc.) |
| 8 | interrupted | SIGINT received during clone/fetch |

## Atomicity guarantees

- The cache directory either fully exists with a parsed-and-validated manifest, or does not exist at all. No partial cache is observable.
- `config.toml` is updated via tempfile-and-rename. A concurrent reader sees either the old or the new state, never a half-written file.
- On any failure path the tempdir is dropped (RAII) before the function returns.

## Interactions with credential scrubbing

`git clone`'s stderr is captured into a `Vec<u8>`, passed through `git::scrub_credentials`, and only the scrubbed bytes are stored on `TomeError::GitFailed.detail`. The unscrubbed bytes are dropped immediately and not logged.

## Examples

```sh
# Shorthand → github.com
tome catalog add midnight/midnight-experts

# Full URL with a custom display name and a pinned tag
tome catalog add https://github.com/midnight/midnight-experts --name me-private --ref v0.3.1

# Local path during catalog development
tome catalog add /Users/alice/work/my-catalog

# Scriptable form
tome catalog add midnight/midnight-experts --json | jq .added.plugin_count
```
