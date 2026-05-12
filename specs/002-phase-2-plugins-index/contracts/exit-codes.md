# Exit Codes — Phase 2 Extension

Every code listed here corresponds to exactly one variant of the closed `TomeError` enum in `src/error.rs`. The `ExitCode` impl is the single source of truth for the integer mapping; this table is the spec the impl is checked against. Codes 0–8 are Phase 1 and are unchanged.

## Phase 1 codes (unchanged; for reference)

| Code | Meaning |
|---|---|
| 0 | Success |
| 2 | Usage error (clap parse failure or unknown subcommand) |
| 3 | Catalog not found |
| 4 | Catalog already exists |
| 5 | Manifest invalid (Phase 1 catalog manifest) |
| 6 | Git operation failed |
| 7 | I/O error (read/write under tome's data or config dirs) |
| 8 | Interrupted (SIGINT during a long-running operation) |

## Phase 2 codes

| Code | Variant name | Meaning |
|---|---|---|
| 20 | `PluginNotFound` | The supplied `<catalog>/<plugin>` identifier did not resolve to a known plugin |
| 21 | `PluginAlreadyInState` | The plugin is already in the requested state (enable on enabled / disable on disabled) |
| 22 | `PluginManifestParseError` | The plugin's `plugin.json` exists but is syntactically invalid or missing required identity fields |
| 23 | `SkillFrontmatterParseError` | A skill's metadata header is present but syntactically invalid (FR-013c) |
| 30 | `ModelMissing` | An operation requires a model file that is not present on disk |
| 31 | `ModelCorrupt` | A model file is present but unreadable, truncated, or fails to load |
| 32 | `ModelChecksumMismatch` | A downloaded model's digest disagrees with the published value |
| 33 | `ModelRegistrationParseError` | A `models/<name>/manifest.json` is unreadable or has unknown fields |
| 34 | `InferenceRuntimeInitFailure` | The ONNX runtime refused to start (missing OS dependency, ABI mismatch, etc.) |
| 35 | `VectorExtensionInitFailure` | The vector-search extension failed to load into the database engine |
| 36 | `EmbeddingGenerationFailure` | The embedder ran and returned an error for a specific input |
| 37 | `RerankingFailure` | The reranker ran and returned an error |
| 40 | `QueryNoResultsStrict` | `--strict` was passed and no candidate cleared the configured threshold |
| 41 | `EmbedderNameDrift` | Stored vectors were produced by a different embedder name |
| 42 | `EmbedderVersionDrift` | Stored vectors were produced by a different embedder version |
| 50 | `IndexBusy` | Another Tome process held the index write lock past the configured wait timeout |
| 51 | `IndexIntegrityCheckFailure` | The database engine's integrity check reported corruption |
| 52 | `SchemaTooNew` | The on-disk schema is newer than the running tool understands |
| 53 | `CatalogHasEnabledPlugins` | `catalog remove` refused because the catalog has enabled plugins and `--force` was not passed |
| 54 | `NotATerminal` | An interactive-only command was invoked without a connected terminal and without the non-interactive equivalent |

## Allocation rules

- Codes are reserved as documented. Once a code ships, its meaning is frozen (constitution principle II, NON-NEGOTIABLE).
- New failure classes get new codes. Old codes are never repurposed.
- Code ranges are loose groupings for human convenience: 20-29 plugin lifecycle, 30-39 models / inference, 40-49 query / model drift, 50-59 index / catalog interaction.
- The constitution permits future Phase-2 patches to extend this list; they must add a new code and a new enum variant in lockstep.

## Verification

`tests/exit_codes.rs` asserts every variant maps to its documented integer. The `TomeError` enum is `#[non_exhaustive]` on the consumer side but exhaustively matched in `impl ExitCode for TomeError`, so adding a new variant without updating the mapping is a compile error.
