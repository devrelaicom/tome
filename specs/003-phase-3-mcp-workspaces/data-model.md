# Phase 3 Data Model

**Branch**: `003-phase-3-mcp-workspaces` | **Date**: 2026-05-14 | **Plan**: [plan.md](./plan.md)

Documents the new types, on-disk shapes, and structured outputs introduced in Phase 3. Phase 1 and Phase 2 types continue unchanged unless explicitly noted.

## 1. `Scope` — the workspace-resolution type

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Global,
    Workspace(PathBuf),  // absolute path to the workspace root (the directory CONTAINING `.tome/`)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeSource {
    Flag,          // --workspace <path> on the CLI
    GlobalFlag,    // --global on the CLI
    Env,           // TOME_WORKSPACE env var
    CwdWalk,       // workspace found by walking parents from CWD
    GlobalFallback // no workspace found, defaulted to global
}

#[derive(Debug, Clone)]
pub struct ResolvedScope {
    pub scope: Scope,
    pub source: ScopeSource,
}
```

**Invariants**:
- `Workspace(path)` carries an absolute, canonicalised path. `Paths` builders rely on the absoluteness.
- `ScopeSource::GlobalFlag` and `ScopeSource::Flag` are mutually exclusive at the CLI; the parser rejects both being set with `TomeError::WorkspaceConflict`.

## 2. `Paths` — extended

Existing Phase 2 `Paths` carries fixed XDG-derived paths. Phase 3 parameterises the index/catalog paths over `Scope`:

```rust
pub struct Paths {
    pub config_dir: PathBuf,         // unchanged — global config dir
    pub data_dir: PathBuf,           // unchanged
    pub catalogs_dir: PathBuf,       // unchanged — shared on-disk catalog clones
    pub models_dir: PathBuf,         // unchanged — shared model artefacts
    pub state_dir: PathBuf,          // NEW — ${XDG_STATE_HOME}/tome/
    pub global_config_file: PathBuf, // renamed from `config_file` (global config.toml)
    pub global_index_db: PathBuf,    // renamed from `index_db`
    pub global_index_lock: PathBuf,  // renamed from `index_lock`
    pub mcp_log: PathBuf,            // NEW — ${state_dir}/mcp.log
    pub mcp_log_prev: PathBuf,       // NEW — ${state_dir}/mcp.log.1
    pub workspace_registry: PathBuf, // NEW — ${state_dir}/workspaces.txt (opt-in best-effort)
}

impl Paths {
    pub fn config_file(&self, scope: &Scope) -> PathBuf {
        match scope {
            Scope::Global => self.global_config_file.clone(),
            Scope::Workspace(root) => root.join(".tome/config.toml"),
        }
    }
    pub fn index_db(&self, scope: &Scope) -> PathBuf { /* analogous */ }
    pub fn index_lock(&self, scope: &Scope) -> PathBuf { /* analogous */ }
    pub fn workspace_marker_dir(&self, root: &Path) -> PathBuf { root.join(".tome") }
}
```

**Backward compatibility note**: every Phase 1/2 call site that touched `paths.config_file` is updated to `paths.config_file(&scope)`. The Phase 3 task list calls this out as one of the largest mechanical refactors of US3.

## 3. Workspace on-disk shape

```text
<workspace-root>/
└── .tome/
    ├── config.toml       # workspace-scoped config (same schema as global)
    ├── index.db          # workspace-scoped SQLite skill index
    └── index.lock        # workspace-scoped advisory lockfile (created on first write)
```

`config.toml` schema is identical to the global config (Phase 1 `Config` struct). No new fields; the workspace just owns its own `[catalogs]` map. `#[serde(deny_unknown_fields)]` continues to apply.

`index.db` schema is identical to the global DB (Phase 2 + Phase 3 `meta.schema_version` row). Bootstrap on first write is identical. WAL mode + PRAGMA hardening identical.

## 4. `WorkspaceInfo` — `tome workspace info` output record

```rust
#[derive(Debug, Serialize)]
pub struct WorkspaceInfo {
    pub scope: ScopeKind,                  // "global" | "workspace"
    pub path: Option<PathBuf>,             // absolute path; None when scope is "global"
    pub source: ScopeSource,
    pub catalogs: u32,
    pub plugins_total: u32,
    pub plugins_enabled: u32,
    pub skills_indexed: u32,
    pub schema_version: Option<u32>,       // None when the DB hasn't been bootstrapped yet
    pub embedder: Option<ModelIdentity>,   // None when the DB is empty
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScopeKind { Global, Workspace }

#[derive(Debug, Serialize)]
pub struct ModelIdentity {
    pub name: String,
    pub version: String,
}
```

Strict (`#[serde(deny_unknown_fields)]` on `ScopeKind` not applicable; on the others, unnecessary because we never deserialise these — emit-only). Stable JSON shape; consumed by `tome doctor` and by users piping into `jq`.

## 5. `DoctorReport` — `tome doctor` output record

```rust
#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub tome_version: String,                            // env!("CARGO_PKG_VERSION")
    pub workspace: WorkspaceInfo,                        // §4
    pub embedder: ModelHealth,                           // Phase 2 ModelHealth, unchanged
    pub reranker: ModelHealth,
    pub index: IndexHealth,                              // Phase 2 IndexHealth, unchanged
    pub drift: DriftStatus,                              // Phase 2 DriftStatus, unchanged
    pub catalogs: Vec<CatalogCacheHealth>,               // NEW
    pub harnesses: Vec<HarnessPresence>,                 // NEW
    pub overall: DoctorClassification,                   // NEW (extends OverallHealth)
    pub suggested_fixes: Vec<SuggestedFix>,              // NEW
}

#[derive(Debug, Serialize)]
pub struct CatalogCacheHealth {
    pub name: String,
    pub url: String,                                     // scrubbed
    pub cache_path: PathBuf,
    pub state: CatalogCacheState,                        // "ok" | "missing" | "not_a_repo" | "manifest_invalid"
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogCacheState { Ok, Missing, NotARepo, ManifestInvalid }

#[derive(Debug, Serialize)]
pub struct HarnessPresence {
    pub name: String,                                    // "claude_code" | "codex" | "cursor" | "gemini" | "opencode" | "continue"
    pub path: PathBuf,                                   // absolute ~/.{name}/
    pub present: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DoctorClassification { Ok, Degraded, Unhealthy }

#[derive(Debug, Serialize)]
pub struct SuggestedFix {
    pub subsystem: String,                               // "embedder" | "catalog:foo" | "schema" | …
    pub diagnosis: String,
    pub command: String,                                 // copy-pasteable
    pub auto_fixable: bool,                              // true if `--fix` handles it
}
```

**Classification rules**:
- `Unhealthy` if any: embedder missing/corrupt; index integrity check failed; embedder drift detected; schema-too-new on any opened DB; workspace marker malformed (at the resolved scope).
- `Degraded` if any (and not Unhealthy): reranker missing/corrupt; reranker drift; catalog cache broken; orphan catalog clone (a clone on disk with no config reference, the inverse of FR-133's reference-counting).
- `Ok` otherwise.

## 6. `SuggestedFix` taxonomy

| Subsystem | Diagnosis | Auto-fixable? | Command |
|---|---|---|---|
| embedder / reranker | Missing | yes | `tome models download` |
| embedder / reranker | Checksum mismatch | yes | `tome models download --force` |
| catalog:NAME | Cache missing / not a repo | yes | `tome catalog update NAME` |
| catalog:NAME | Manifest invalid | no | `tome catalog show NAME` (manual repair) |
| schema | Older on disk | yes | (`--fix` runs `apply_pending`) |
| schema | Newer on disk | no | "Upgrade Tome to a version that supports schema vN" |
| embedder drift | Stored vectors are from a different model | no | `tome reindex --force` |
| reranker drift | Reranker differs from stored identity | no | `tome reindex --force` (optional) |
| orphan clone | Catalog cache directory not referenced by any config | no | manual rm; doctor reports the path |

`auto_fixable = false` items appear in the report's "suggested fixes" section but `--fix` skips them; they are surfaced as commands the developer must run.

## 7. MCP tool schemas

### `search_skills` input

```rust
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchSkillsInput {
    pub query: String,
    #[serde(default = "default_top_k")]
    pub top_k: u32,                       // default 10
    #[serde(default)]
    pub catalog: Option<String>,
    #[serde(default)]
    pub plugin: Option<String>,           // requires `catalog`; rejected otherwise
}

fn default_top_k() -> u32 { 10 }
```

### `search_skills` output

```rust
#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchSkillsOutput {
    pub matches: Vec<SkillMatch>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SkillMatch {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    pub description: String,
    pub plugin_version: String,
    pub path: PathBuf,                    // absolute
    pub score: f32,                       // reranker output, or embedding similarity if reranker drift forced fallback
}
```

### `get_skill` input

```rust
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetSkillInput {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
}
```

### `get_skill` output

```rust
#[derive(Debug, Serialize, JsonSchema)]
pub struct GetSkillOutput {
    pub content: String,                  // SKILL.md body with frontmatter stripped
    pub path: PathBuf,                    // absolute path to SKILL.md
    pub resources: Vec<PathBuf>,          // absolute paths of every OTHER file in the skill's directory
}
```

### `get_skill` error

The MCP-level error envelope (`rmcp`'s `Error`) carries:
- `code` — string discriminator: `"unknown_catalog"` | `"unknown_plugin"` | `"unknown_skill"` | `"skill_file_missing"` | `"frontmatter_strip_failed"`.
- `message` — human-readable diagnosis.
- `data` — optional `{ "catalog": "...", "plugin": "...", "name": "..." }`.

## 8. MCP log line shape

JSON-lines (one record per line) in `${XDG_STATE_HOME}/tome/mcp.log`:

```json
{"ts":"2026-05-14T12:34:56.789Z","level":"info","target":"tome::mcp::server","msg":"startup ok","embedder":"bge-small-en-v1.5","scope":"workspace","workspace":"/abs/path"}
{"ts":"2026-05-14T12:34:57.123Z","level":"info","target":"tome::mcp::tools::search_skills","msg":"call","query_len":42,"top_k":10,"matches":7,"elapsed_ms":214}
{"ts":"2026-05-14T12:34:58.001Z","level":"error","target":"tome::mcp::tools::get_skill","msg":"unknown_plugin","catalog":"acme","plugin":"ghost"}
```

Every line passes through `git::scrub_credentials` before write. Workspace paths in messages, query strings, and catalog URLs are scrubbed at line construction.

## 9. Migration metadata

The `meta` table (Phase 2) gains no new columns. Phase 3 keys (read via the same Phase 2 `MetaKey` enum) — none new; `schema_version` is already there. The `apply_pending` function takes `current` and `target` as parameters and updates `meta.schema_version` atomically inside each migration's transaction.

## 10. Error variant additions

Phase 3 extends the closed `TomeError` enum with eight new variants per FR-201. See [`contracts/exit-codes-p3.md`](./contracts/exit-codes-p3.md) for the canonical exit-code mapping. Variant names:

| Variant | Exit code | Trigger |
|---|---|---|
| `McpStartupFailed { reason }` | 60 | Composite pre-condition failure (DB missing, schema mismatch, etc.) where the more specific exit code would not be reachable from inside the MCP context. |
| `McpProtocolIo { source }` | 61 | I/O failure on the MCP stdio transport. |
| `WorkspaceMalformed { path, reason }` | 70 | `.tome/` exists but config or DB is unreadable. |
| `WorkspaceNotFound { path }` | 71 | `--workspace <path>` named a path with no `.tome/`. |
| `WorkspaceConflict` | 72 | `--workspace` and `--global` both passed. |
| `SchemaVersionTooNew { on_disk, expected }` | 73 | On-disk schema version > `target` known to running Tome. |
| `SchemaMigrationFailed { from, to, source }` | 74 | A registered migration returned an error. |
| `DoctorFixNotSafe { subsystem }` | 75 | `--fix` was passed for a class the doctor refuses to auto-apply. |

## 11. CLI argument additions

Two new global flags, parsed at the top level before subcommand dispatch:

```rust
#[derive(clap::Args, Debug, Clone)]
pub struct GlobalScopeArgs {
    #[arg(long, global = true, conflicts_with = "global", value_name = "PATH")]
    pub workspace: Option<PathBuf>,

    #[arg(long, global = true)]
    pub global: bool,
}
```

Three new subcommands:

```rust
pub enum Command {
    // … Phase 1/2 commands unchanged …
    Mcp(McpArgs),                  // tome mcp
    Workspace(WorkspaceCommand),   // tome workspace …
    Doctor(DoctorArgs),            // tome doctor
}

pub enum WorkspaceCommand {
    Init(WorkspaceInitArgs),       // tome workspace init [<path>] [--inherit-global] [--force]
    Info,                          // tome workspace info  (uses GlobalScopeArgs from the top level)
}

pub struct McpArgs;                // No subcommand-local args; honours GlobalScopeArgs

pub struct WorkspaceInitArgs {
    pub path: Option<PathBuf>,
    #[arg(long)]
    pub inherit_global: bool,
    #[arg(long)]
    pub force: bool,
}

pub struct DoctorArgs {
    #[arg(long)]
    pub fix: bool,
}
```

## 12. State transitions

### Workspace creation

```
[no .tome/]   ─ workspace init ─→   [.tome/ + empty config.toml + uninitialised index.db slot]
                                    │
                                    ├─ catalog add ─→  catalogs populated, index still uninit
                                    │
                                    └─ plugin enable ─→  index bootstrapped (Phase 2 path), enabled flag set
```

### Catalog clone reference count

```
[clone exists, refs = {global}]    ─ workspace add same URL ─→   [refs = {global, workspace-A}]
                                                                 │
                                                                 ├─ global removes URL ─→  [refs = {workspace-A}]
                                                                 │
                                                                 └─ workspace-A removes URL ─→  [refs = {}]  → CLONE DELETED
```

### Schema migration

```
[DB at schema vN, tome expects vM]
  ├─ N == M   →  open, no-op
  ├─ N < M    →  apply Migration(N→N+1, N+1→N+2, …, M-1→M) inside fresh transactions; on failure, stop at last-good intermediate
  └─ N > M    →  refuse with SchemaVersionTooNew { on_disk: N, expected: M }
```

## 13. Reserved / out-of-scope shapes

- No `Workspace` registry table on disk; the opt-in `workspaces.txt` (R-12) is a flat text file of absolute paths, one per line. Not a database; not a structured artifact. Doctor reports it as a hint, not a source of truth.
- No `McpSession` type. Each MCP process is self-contained; no session-persistence across restarts.
- No `HarnessConfig` type. Doctor reads directory existence only.

---

*This document is the canonical type list for Phase 3. Any new struct, enum, or on-disk artefact introduced during implementation MUST be added here in the same PR.*
