# `tome status` — Command Contract

```
tome status
          [--verify]
          [--json]
```

Reports the health of every Phase 2 subsystem independently. Per FR-056 this is the doctor / pre-flight command — a single place users can look when something seems wrong before filing a bug. Exit code is non-zero if any subsystem is unhealthy.

## Behaviour

1. Resolve Tome version (`env!("CARGO_PKG_VERSION")`).
2. Check the embedder: manifest present, file existence, size match. With `--verify`, full SHA-256.
3. Check the reranker: same.
4. Check the index database: open read-only, run `PRAGMA integrity_check`, count enabled plugins and skill rows. If absent — report Missing (not an error, just informational).
5. Check `meta` for embedder/reranker drift.
6. Aggregate to `OverallHealth`: `ok` / `degraded` (reranker-only drift) / `unhealthy` (anything else).

## Output (human)

```
Tome:               0.2.0
Embedder:           bge-small-en-v1.5 (1.5)   ✓ ok
Reranker:           bge-reranker-base (base)  ✓ ok
Index database:     ✓ ok (12 plugins enabled, 156 skills indexed, 4.2 MB)
Schema version:     1
Drift:              none
Overall:            ✓ healthy
```

In a non-TTY context: no colour, no Unicode glyphs (use ASCII `[ok]` / `[fail]` markers instead).

Degraded example (reranker drift):

```
Tome:               0.2.0
Embedder:           bge-small-en-v1.5 (1.5)   ✓ ok
Reranker:           bge-reranker-large (large) ✓ ok
Index database:     ✓ ok (12 plugins enabled, 156 skills indexed)
Schema version:     1
Drift:              reranker name drift (stored: bge-reranker-base, configured: bge-reranker-large)
                    — queries still serve; consider `tome reindex --force` for consistency
Overall:            ⚠ degraded
```

Exit non-zero in degraded and unhealthy cases.

## Output (`--json`)

The full `StatusReport` struct from data-model §11.

## Exit codes

`tome status` is itself an introspection command; it does not return the per-subsystem error codes (those would prevent reporting in the first place). Instead:

| Overall health | Exit |
|---|---|
| Ok | 0 |
| Degraded | 1 (chosen for non-zero without claiming a specific failure mode) |
| Unhealthy | 1 |

The structured output identifies the failing subsystem so callers can react programmatically.

## Notes

- Status NEVER triggers a model download. If models are missing it reports Missing and exits non-zero.
- Status NEVER initiates a write. The integrity check is read-only.
- Status NEVER takes the advisory lock. If a writer holds it, the report includes a `"writer_pid"` field.
- `tome status --verify` is the supported pre-flight before filing a bug report. Users are nudged toward it from every error message that can be recovered with `tome status`.
