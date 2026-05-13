# `tome models` — Command Contracts

Covers `tome models download | list | remove`.

---

## `tome models download`

```
tome models download
                    [--force]
                    [--json]
```

### Behaviour

For each known model in `MODEL_REGISTRY` (currently `bge-small-en-v1.5` and `bge-reranker-base`):

1. If `models/<name>/manifest.json` exists and references files that all exist with the recorded sizes — skip unless `--force`.
2. Else:
   - Create `models/<name>/.partial/` (clean if it already exists).
   - GET the upstream URL via `reqwest::blocking`. Stream into the partial directory. Hash with SHA-256 in parallel.
   - On size mismatch / hash mismatch → exit 32 (`ModelChecksumMismatch`). Partial directory deleted.
   - On success → `fsync` files, `rename(.partial, .)`, then atomically write `manifest.json` via `tempfile::persist`.
3. Report each model on its own line: name, version, size, "downloaded" / "skipped" / "redownloaded".

### Output (human)

```
bge-small-en-v1.5 (1.5) — 45.0 MB
⠋ downloading…    8.2s
✓ downloaded
bge-reranker-base (base) — 280.0 MB
⠋ downloading…   41.8s
✓ downloaded
```

The model-download progress is rendered as an `indicatif` **indeterminate spinner**, not a byte-progress bar — `embedding::download::download_model` does not currently expose a byte-progress callback, so the CLI cannot drive a determinate bar. The byte-progress refactor is tracked as TD-010 in `.sdd/codebase/CONCERNS.md` and is a candidate for a post-v0.2.0 polish pass. The spinner is suppressed when stderr is not a TTY.

### Output (`--json`)

```json
{
  "models": [
    {
      "name": "bge-small-en-v1.5",
      "version": "1.5",
      "kind": "embedder",
      "action": "downloaded",
      "size_bytes": 47185920,
      "sha256_verified": true,
      "duration_ms": 8214
    },
    {
      "name": "bge-reranker-base",
      "version": "base",
      "kind": "reranker",
      "action": "downloaded",
      "size_bytes": 293601280,
      "sha256_verified": true,
      "duration_ms": 41801
    }
  ]
}
```

### Errors

| Trigger | Exit |
|---|---|
| Network failure | 7 (I/O) |
| Checksum mismatch | 32 |
| Manifest parse failure (when re-reading a pre-existing manifest) | 33 |

---

## `tome models list`

```
tome models list
                [--verify]
                [--json]
```

### Behaviour

For each model in `MODEL_REGISTRY`:

- `state = "ok"` when manifest + files + sizes are consistent.
- `state = "missing"` when no manifest.
- `state = "corrupt"` when files referenced by manifest are missing or have wrong sizes.
- `state = "checksum_mismatched"` when (and only when) `--verify` is passed and the SHA-256 disagrees.

The JSON `state` field is the lowercased / snake-case form shown above (`"checksum_mismatched"`, not `"ChecksumMismatched"`).

Without `--verify`, the check is cheap (existence + size). With `--verify`, the check rehashes; this can take several seconds for the reranker.

### Output (human)

```
Name                  Version  Kind      Size     State  Path                                              Licence
────────────────────  ───────  ────────  ───────  ─────  ────────────────────────────────────────────────  ───────
bge-small-en-v1.5     1.5      embedder  45.0 MB  ok     ~/.local/share/tome/models/bge-small-en-v1.5      MIT
bge-reranker-base     base     reranker  280 MB   ok     ~/.local/share/tome/models/bge-reranker-base      MIT
```

State colour: green (ok), red (missing/corrupt/checksum-mismatched).

### Output (`--json`)

Array of `ModelManifest`-derived JSON records plus a `state` field per record.

### Errors

| Trigger | Exit |
|---|---|
| Manifest unparsable | 33 |

---

## `tome models remove <name>`

```
tome models remove <name>
                  [--force]
                  [--json]
```

### Behaviour

1. If `<name>` is not in the registry → exit 2 (usage error: unknown model).
2. If the model is not installed → exit 30 (`ModelMissing`).
3. Unless `--force`, prompt for confirmation. Non-TTY without `--force` → exit 54.
4. Delete `models/<name>/` and its `manifest.json`. Operation is best-effort atomic: delete the manifest first to make the install state observable, then delete the files.
5. Report.

If the removed model is the embedder, a follow-up `tome query` will see vectors that can no longer be produced — `tome status` will report this as an unhealthy state. The next operation that needs the embedder will prompt to re-download (TTY) or exit 30 (non-TTY). FR-023 covers this explicitly.

### Errors

Same as above plus exit 7 for filesystem failures.
