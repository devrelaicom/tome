# Phase 3 Exit Codes — Contract

Phase 3 extends the closed `TomeError` enum with eight new variants per spec FR-201. Phase 1 codes (0–8) and Phase 2 codes (20+) are unchanged. Existing variants and their codes are NOT renumbered.

## New variants

| Code | Variant | Category | Trigger |
|---|---|---|---|
| 60 | `McpStartupFailed { reason }` | `mcp_startup` | Composite MCP pre-flight failure where no more specific Phase 1/2 code applies. The `reason` payload identifies the specific sub-condition for diagnostic purposes. |
| 61 | `McpProtocolIo { source }` | `mcp_io` | I/O failure on the MCP stdio transport (e.g., harness closed stdin mid-session). |
| 70 | `WorkspaceMalformed { path, reason }` | `workspace_malformed` | A `.tome/` marker is present but its config or DB is unreadable. |
| 71 | `WorkspaceNotFound { path }` | `workspace_not_found` | `--workspace <path>` or `TOME_WORKSPACE` named a path with no `.tome/`. |
| 72 | `WorkspaceConflict` | `workspace_conflict` | `--workspace` and `--global` passed on the same command. |
| 73 | `SchemaVersionTooNew { on_disk, expected }` | `schema_too_new` | Index DB records a schema version greater than this Tome's `target`. |
| 74 | `SchemaMigrationFailed { from, to, source }` | `schema_migration` | A registered forward migration's `apply` returned an error. |
| 75 | `DoctorFixNotSafe { subsystem }` | `doctor_fix_unsafe` | `--fix` was passed but the run completed with unfixable issues remaining (or a fixable class threw an error during repair). |

## Specific-over-generic preference

When the MCP pre-flight finds a Phase 1/2 condition with its own code, exit with that code, not `McpStartupFailed`:

- Index DB missing or unreadable mid-session → exit `51` (`IndexIntegrityCheckFailure`), not 60. (An earlier draft of this contract mis-typed the number as 35; 35 is `VectorExtensionInitFailure` in the Phase 2 closed enum. The variant name is canonical; the number comes from `src/error.rs::exit_code`.)
- Required model missing → exit `30` (`ModelMissing`), not 60.
- Model checksum mismatch → exit `32` (`ModelChecksumMismatch`), not 60.
- Embedder drift → exit `41` (`EmbedderNameDrift`) or `42` (`EmbedderVersionDrift`), not 60.
- Schema version too new → exit `73` (`SchemaVersionTooNew`), not 60.

`McpStartupFailed` is the residual: any pre-flight failure not matching any specific case (e.g., the `rmcp` SDK rejects an unparsable client handshake, or stdio binding fails on a system where stdin isn't usable as a transport). The `reason` payload string is taxonomy-controlled — see `src/error.rs` for the closed enum of `reason` strings.

## Display messages

Each variant has a `Display` impl that produces a user-facing diagnosis. Sample messages:

```rust
McpStartupFailed { reason: "rmcp handshake rejected: invalid initialize request" }
    → "MCP server failed to start: rmcp handshake rejected: invalid initialize request"

McpProtocolIo { source: io::Error("broken pipe") }
    → "MCP protocol I/O error: broken pipe"

WorkspaceMalformed { path, reason: "invalid TOML in .tome/config.toml at line 4" }
    → "workspace malformed at {path}: invalid TOML in .tome/config.toml at line 4
       hint: run `tome doctor` for a full diagnosis"

WorkspaceNotFound { path }
    → "workspace not found: {path} does not contain a .tome/ marker
       hint: run `tome workspace init {path}` to create one"

WorkspaceConflict
    → "workspace conflict: --workspace and --global cannot be combined"

SchemaVersionTooNew { on_disk, expected }
    → "schema version too new: on-disk schema is v{on_disk}, this Tome supports up to v{expected}
       hint: upgrade Tome to a version that supports schema v{on_disk}"

SchemaMigrationFailed { from, to, source }
    → "schema migration v{from} → v{to} failed: {source}
       hint: file the error against your installed Tome version"

DoctorFixNotSafe { subsystem }
    → "doctor: subsystem `{subsystem}` cannot be auto-fixed
       hint: see the report's `suggested fixes` section for the manual command"
```

## Category strings

Categories are returned by `TomeError::category()` and used by the JSON error envelope's `category` field. New strings in Phase 3:

- `mcp_startup`
- `mcp_io`
- `workspace_malformed`
- `workspace_not_found`
- `workspace_conflict`
- `schema_too_new`
- `schema_migration`
- `doctor_fix_unsafe`

Categories are namespaced by intent. No collisions with Phase 1/2 categories.

## Tests

`tests/exit_codes.rs` extends its exhaustive list to include all eight new variants. The exhaustive `match` arm in `_code_for` adds eight new lines; the compiler enforces that no variant is missed.

`tests/error_messages.rs` adds a Display assertion per new variant (a one-line check that the message contains the expected substrings — variant name, path, hints). This closes the Phase 10 deferral item "Phase 2 `TomeError` Display tests" for the new Phase 3 variants; the remaining Phase 2 Display gaps stay in the polish backlog.

## Cross-reference with the spec

Spec FR-201 names the eight new failure classes. This document is the exit-code resolution table for that list.

| Spec failure class | Variant | Exit code |
|---|---|---|
| MCP server startup pre-condition failure (composite) | `McpStartupFailed` | 60 |
| MCP protocol I/O failure | `McpProtocolIo` | 61 |
| Workspace marker malformed | `WorkspaceMalformed` | 70 |
| Workspace not found at an explicit path | `WorkspaceNotFound` | 71 |
| Workspace conflict | `WorkspaceConflict` | 72 |
| Schema-version-too-new | `SchemaVersionTooNew` | 73 |
| Schema migration failure | `SchemaMigrationFailed` | 74 |
| Doctor fix not safe | `DoctorFixNotSafe` | 75 |

## Notes on numbering

Phase 1 uses 0–8. Phase 2 uses 20–55 (block 1 plugins, block 2 models, block 3 query/strict, block 4 index/concurrency, block 5 frontmatter, block 6 catalog cascade, block 7 not-a-terminal). Phase 3 starts block 60 for MCP and block 70 for workspace/schema. The 56–59 gap is reserved for any further Phase 2 follow-ups; 76–79 is reserved for Phase 3 follow-ups.

Block boundaries make the exit-code table readable but are not load-bearing — the closed-set invariant is what matters.
