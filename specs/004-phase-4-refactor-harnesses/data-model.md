# Phase 4 Data Model

**Branch**: `004-phase-4-refactor-harnesses` | **Date**: 2026-05-14 | **Plan**: [plan.md](./plan.md)

Documents the new types, on-disk shapes, and structured outputs introduced in Phase 4. Phase 1–3 types continue unchanged unless explicitly noted; the most consequential changes are the `Paths` reshape (drops `directories`), the `Scope` reshape (`String` workspace name instead of `PathBuf`), and the central single-DB schema replacing per-workspace databases.

## 1. `paths::home_root` + `Paths` reshape

Replaces the Phase 3 `Paths` struct entirely. No XDG-style subdirectories — every path lives under one root.

```rust
/// Resolves <home>/.tome/ via std::env::home_dir() (un-deprecated as of Rust 1.85).
pub fn home_root() -> Result<PathBuf, TomeError>;

pub struct Paths {
    pub root: PathBuf,               // <home>/.tome/
    pub global_config_file: PathBuf, // <root>/config.toml — Tome's own global config (strict)
    pub global_settings_file: PathBuf, // <root>/settings.toml — global harness settings (strict)
    pub index_db: PathBuf,           // <root>/index.db — SINGLE central database
    pub index_lock: PathBuf,         // <root>/index.lock — single advisory lockfile
    pub catalogs_dir: PathBuf,       // <root>/catalogs/ — shared catalog clones, refcounted via workspace_catalogs
    pub models_dir: PathBuf,         // <root>/models/ — embedder + reranker + summariser
    pub logs_dir: PathBuf,           // <root>/logs/
    pub mcp_log: PathBuf,            // <root>/logs/mcp.log
    pub mcp_log_prev: PathBuf,       // <root>/logs/mcp.log.1
    pub workspaces_dir: PathBuf,     // <root>/workspaces/
}

impl Paths {
    pub fn workspace_dir(&self, name: &WorkspaceName) -> PathBuf {
        self.workspaces_dir.join(name.as_str())
    }
    pub fn workspace_settings_file(&self, name: &WorkspaceName) -> PathBuf {
        self.workspace_dir(name).join("settings.toml")
    }
    pub fn workspace_rules_file(&self, name: &WorkspaceName) -> PathBuf {
        self.workspace_dir(name).join("RULES.md")
    }
    pub fn project_marker_dir(project_root: &Path) -> PathBuf {
        project_root.join(".tome")
    }
    pub fn project_marker_config(project_root: &Path) -> PathBuf {
        Self::project_marker_dir(project_root).join("config.toml")
    }
    pub fn project_marker_rules(project_root: &Path) -> PathBuf {
        Self::project_marker_dir(project_root).join("RULES.md")
    }
}
```

**Invariants**:

- `root` is an absolute, canonicalised path. Constructor `Paths::resolve()` calls `home_root()?.canonicalize()?`.
- Every accessor (`workspace_dir`, `workspace_settings_file`, etc.) returns a path strictly inside `root`. The `Paths` struct is the only place path joins happen; no other module constructs Tome-owned paths from string literals.
- `Paths` no longer carries a `state_dir` or `config_dir` distinction (the Phase 3 `directories`-backed split is gone).

**Removed in Phase 4**:

- `Paths::config_file(&Scope)` — the Phase 3 per-scope accessor. Replaced by `global_config_file` (constant) and `workspace_settings_file(&WorkspaceName)`.
- `Paths::index_db(&Scope)` and `Paths::index_lock(&Scope)` — there is now exactly one of each.
- `Paths::workspace_marker_dir(&Path)` and friends — moved to associated functions on `Paths` since they no longer depend on `self`.
- `Paths::workspace_registry` — the Phase 3 `workspaces.txt` opt-in file is deleted.

## 2. `WorkspaceName` newtype + name rules

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceName(String);

impl WorkspaceName {
    /// Validates against the FR-347 rule:
    /// - 1–64 chars from [a-zA-Z0-9_-]
    /// - MUST NOT begin or end with `-` or `_`
    /// - MUST NOT be `.`, `..`, or empty
    pub fn parse(s: &str) -> Result<Self, TomeError>;
    pub fn as_str(&self) -> &str;
    pub fn is_reserved(&self) -> bool { self.0 == "global" }
    /// The privileged default workspace, created on first bootstrap.
    pub const GLOBAL: &'static str = "global";
}
```

**Invariants**:

- A `WorkspaceName` value is always valid by construction. Parsing happens at exactly three boundaries: CLI flag input, env var read, TOML deserialise (custom `Deserialize` impl calls `parse`).
- `WorkspaceName::is_reserved()` is consulted by `workspace remove` (FR-405); a reserved name refuses removal.

## 3. `Scope` reshape — workspace names, not paths

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope(pub WorkspaceName);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeSource {
    Flag,            // --workspace <name>
    Env,             // TOME_WORKSPACE env var
    ProjectMarker,   // .tome/config.toml workspace = "<name>"
    GlobalFallback,  // no marker found, default to "global"
}

#[derive(Debug, Clone)]
pub struct ResolvedScope {
    pub scope: Scope,
    pub source: ScopeSource,
    pub project_root: Option<PathBuf>,  // Some when source == ProjectMarker; None otherwise
}
```

**Reshape rationale**: Phase 3's `Scope::Global | Scope::Workspace(PathBuf)` mixed two different things — the resolved identity (workspace vs global) and the on-disk path. In Phase 4 the identity is just a name; the path (if any) is the project root that triggered the binding lookup. Splitting them removes a class of "is this `Workspace(/foo)` the same as `Workspace(/foo/bar)` if I'm in a subdirectory" bugs.

**Resolution algorithm** (FR-344, implemented in `src/workspace/resolution.rs`):

```text
resolve(cli_flag, env_var, cwd, central_db) -> ResolvedScope:
  if cli_flag is set:
    name = parse(cli_flag)?
    if not central_db.has_workspace(&name): error 13
    return ResolvedScope { Scope(name), Flag, None }
  if env_var is set:
    name = parse(env_var)?
    if not central_db.has_workspace(&name): error 13
    return ResolvedScope { Scope(name), Env, None }
  for dir in cwd.ancestors():
    marker = dir.join(".tome/config.toml")
    if marker.exists():
      cfg = parse_marker(&marker)?  // strict; fails with code 70 on malformed
      if not central_db.has_workspace(&cfg.workspace): error 13
      return ResolvedScope { Scope(cfg.workspace), ProjectMarker, Some(dir) }
  return ResolvedScope { Scope(WorkspaceName::parse("global")?), GlobalFallback, None }
```

## 4. Central database schema (v2)

Single SQLite database at `<root>/index.db`. WAL mode + standard PRAGMA hardening (Phase 2 baseline). Phase 4 schema:

```sql
-- Schema metadata (carries forward from Phase 2/3 with new identity rows)
CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
-- Required keys after bootstrap:
--   schema_version         = "2"
--   embedder_name          = "bge-small-en-v1.5"
--   embedder_version       = "<pinned-version>"
--   reranker_name          = "bge-reranker-base"
--   reranker_version       = "<pinned-version>"
--   summariser_name        = "qwen2.5-0.5b-instruct"
--   summariser_version     = "<pinned-version>"

-- Named workspaces (always at least one: "global", seeded on bootstrap)
CREATE TABLE workspaces (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  name          TEXT UNIQUE NOT NULL,
  created_at    INTEGER NOT NULL,    -- unix seconds
  last_used_at  INTEGER NOT NULL     -- unix seconds; updated only on write-path commands (FR-411)
);

-- Skills are workspace-agnostic; enablement is in workspace_skills
CREATE TABLE skills (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  catalog         TEXT NOT NULL,
  plugin          TEXT NOT NULL,
  name            TEXT NOT NULL,
  description     TEXT NOT NULL,
  plugin_version  TEXT NOT NULL,
  path            TEXT NOT NULL,      -- absolute on-disk path to SKILL.md
  content_hash    TEXT NOT NULL,      -- SHA-256 of canonicalised SKILL.md body
  indexed_at      INTEGER NOT NULL,
  UNIQUE (catalog, plugin, name)
);

-- sqlite-vec virtual table for embeddings (unchanged from Phase 2)
CREATE VIRTUAL TABLE skill_embeddings USING vec0(
  skill_id  INTEGER PRIMARY KEY,
  embedding FLOAT[384]
);

-- Per-workspace enablement junction
CREATE TABLE workspace_skills (
  workspace_id  INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  skill_id      INTEGER NOT NULL REFERENCES skills(id)     ON DELETE CASCADE,
  enabled_at    INTEGER NOT NULL,
  PRIMARY KEY (workspace_id, skill_id)
);

-- Per-workspace catalog enrolment (sole source of truth for catalog refcount)
CREATE TABLE workspace_catalogs (
  workspace_id  INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  catalog_name  TEXT NOT NULL,
  url           TEXT NOT NULL,
  pinned_ref    TEXT NOT NULL,
  PRIMARY KEY (workspace_id, catalog_name)
);

-- Project-to-workspace bindings — PK on project_path alone enforces 1:1 (FR-322 / FR-342)
CREATE TABLE workspace_projects (
  project_path  TEXT PRIMARY KEY NOT NULL,
  workspace_id  INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  bound_at      INTEGER NOT NULL
);

CREATE INDEX idx_workspace_projects_workspace ON workspace_projects(workspace_id);
CREATE INDEX idx_workspace_skills_skill ON workspace_skills(skill_id);
CREATE INDEX idx_workspace_catalogs_url ON workspace_catalogs(url);
```

**Bootstrap behaviour**: On first open of a non-existent database, `index::schema::bootstrap` creates the v2 schema directly and inserts the seeded `global` workspace row. The schema-1 → schema-2 migration is registered for the synthetic-fixture e2e tests; in normal operation, no migration runs because no v1 database is ever opened.

## 5. Workspace on-disk shape

```text
<root>/
├── config.toml                    # Tome's global config (strict; carries forward from Phase 1)
├── settings.toml                  # global harness settings (strict; NEW in Phase 4)
├── index.db                       # central SQLite DB (schema v2)
├── index.lock                     # advisory lockfile
├── catalogs/<url-hash>/           # shared catalog clones (unchanged from Phase 1)
├── models/
│   ├── bge-small-en-v1.5/
│   ├── bge-reranker-base/
│   └── qwen2.5-0.5b-instruct/    # NEW
├── logs/mcp.log                   # MCP server log (unchanged)
└── workspaces/
    ├── global/
    │   ├── settings.toml          # workspace-scoped settings (strict)
    │   └── RULES.md               # generated rules content; body = cached long summary
    └── <user-workspace>/
        ├── settings.toml
        └── RULES.md
```

## 6. Workspace settings file (`<root>/workspaces/<name>/settings.toml`)

```toml
name = "<workspace-name>"           # required; MUST match the directory name

# Cached summaries — regenerated on enable/disable/reindex/regen-summary triggers
[summaries]
short        = "..."                # ~400-800 char target
long         = "..."                # ~1500-2500 char target
generated_at = 2026-05-14T15:00:00Z # RFC 3339 timestamp

# Workspace's enrolled catalogs (mirrored from workspace_catalogs table; this file is the human-editable view)
[[catalogs]]
name = "midnight-expert"
url  = "https://github.com/devrelaicom/midnight-expert"
ref  = "main"

# Workspace's harness declaration (optional; absent means "fall through to global")
harnesses = ["claude-code", "[global]", "!cursor"]
```

Rust deserialisation:

```rust
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSettings {
    pub name: WorkspaceName,
    #[serde(default)]
    pub summaries: Option<CachedSummaries>,
    #[serde(default)]
    pub catalogs: Vec<CatalogEntry>,
    #[serde(default)]
    pub harnesses: Option<Vec<String>>,   // None = no declaration; Some(vec) = declared (vec may be empty)
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CachedSummaries {
    pub short: String,
    pub long: String,
    pub generated_at: time::OffsetDateTime,
}
```

## 7. Project marker config file (`<project>/.tome/config.toml`)

```toml
workspace = "my-project"            # required; the workspace this project is bound to

# Optional project-scope harness declaration; composition references allowed
harnesses = ["[workspace]", "!cursor", "claude-code"]
```

```rust
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectMarkerConfig {
    pub workspace: WorkspaceName,
    #[serde(default)]
    pub harnesses: Option<Vec<String>>,
}
```

## 8. Global settings file (`<root>/settings.toml`)

```toml
# Global harness defaults — applied when no project marker and no workspace declaration narrows scope
harnesses = ["claude-code", "codex"]
```

```rust
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct GlobalSettings {
    #[serde(default)]
    pub harnesses: Option<Vec<String>>,
}
```

## 9. Layered settings + composition types

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompositionRef {
    Include(String),                    // plain harness name
    Exclude(String),                    // !name
    CurrentWorkspace,                   // [workspace] — valid only in project scope
    NamedWorkspace(WorkspaceName),      // [workspaces.<name>]
    Global,                             // [global]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompositionError {
    Cycle { path: Vec<String> },                       // exit code 17
    WorkspaceRefOutsideProject { found_in: ScopeKind }, // exit code 17
    UnknownWorkspace(WorkspaceName),                   // exit code 13 (workspace not found)
    BadExclusion(String),                              // exit code 17 (e.g. "![global]")
}

#[derive(Debug, Clone)]
pub struct EffectiveHarnessList {
    pub harnesses: Vec<EffectiveHarness>,              // ordered by first-included-from
    pub excluded: Vec<String>,                         // names subtracted by `!`-prefixes (for `tome harness list` reporting)
}

#[derive(Debug, Clone)]
pub struct EffectiveHarness {
    pub name: String,
    pub source_chain: Vec<ScopeKind>,                  // where the name was contributed from
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    Project,
    Workspace,
    Global,
}

pub fn resolve_effective_list(
    project_marker: Option<&ProjectMarkerConfig>,
    bound_workspace: Option<&WorkspaceSettings>,
    global_settings: &GlobalSettings,
    central_db: &Connection,        // for `[workspaces.<name>]` resolution
) -> Result<EffectiveHarnessList, CompositionError>;
```

Resolution is **DFS** tracking visited (scope, name) pairs to detect cycles. Composition references resolve to the referenced scope's **directly-declared** list (R-9 / FR-449), NOT its computed effective list.

## 10. Harness module trait

```rust
pub trait HarnessModule: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn detect(&self, home: &Path) -> bool;                          // per-user dir exists?

    fn rules_file_target(&self, project_root: &Path) -> PathBuf;    // path to write rules into
    fn rules_file_strategy(&self) -> RulesFileStrategy;
    fn block_body_style(&self) -> BlockBodyStyle;                   // only consulted for BlockInExistingFile

    fn mcp_config_path(&self, project_root: &Path, home: &Path) -> PathBuf;
    fn mcp_config_format(&self) -> McpConfigFormat;
    fn mcp_parent_key(&self) -> &'static str;                       // "mcpServers" (JSON) or "mcp_servers" (TOML / Codex CLI)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RulesFileStrategy {
    BlockInExistingFile,                                // claude-code, codex, gemini, opencode
    StandaloneFile,                                     // cursor
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockBodyStyle {
    AtInclude,                                          // claude-code, codex, gemini
    Inline,                                             // opencode (no documented @-include)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpConfigFormat {
    Json,                                               // serde_json with preserve_order
    Toml,                                               // toml_edit
}

pub static MCP_CONFIG_KEY: &str = "tome";              // standardised across all harnesses
```

The five concrete impls live at `src/harness/{claude_code,codex,gemini,cursor,opencode}.rs`, each ~50 lines.

## 11. Rules-file block markers

Pinned per FR-480:

```text
<!-- tome:begin -->
<body>
<!-- tome:end -->
```

`<body>` is either `@.tome/RULES.md` (for `AtInclude` style) or the full rules content verbatim (for `Inline` style).

Match regex (with trailing-whitespace tolerance): `^<!-- tome:(begin|end) -->\s*$`.
Emit format: `<!-- tome:begin -->\n<body>\n<!-- tome:end -->\n`.

## 12. MCP entry shape (Tome-owned)

```json
{
  "command": "tome",
  "args": ["mcp", "--workspace", "<workspace-name>"]
}
```

```toml
[tome]
command = "tome"
args = ["mcp", "--workspace", "<workspace-name>"]
```

`env` field, if present (developer-added), MUST be preserved on rewrite (FR-503).

Ownership marker (FR-501): `command == "tome" && args.first() == Some("mcp")`. Anything else under the key `"tome"` is user-owned and refuses rewrite without override.

## 13. Summariser types

```rust
pub trait Summariser: Send + Sync {
    fn summarise(&self, plugin_data: &PluginSummariesInput) -> Result<SummariserOutput, TomeError>;
}

pub struct PluginSummariesInput {
    pub plugins: Vec<PluginSummaryItem>,
}

pub struct PluginSummaryItem {
    pub catalog: String,
    pub plugin: String,
    pub description: String,
    pub skills: Vec<SkillSummaryItem>,
}

pub struct SkillSummaryItem {
    pub name: String,
    pub description: String,
}

pub struct SummariserOutput {
    pub short: String,
    pub long: String,
}

pub struct LlamaSummariser {
    backend: &'static LlamaBackend,     // static borrow of the OnceLock-held singleton
    model_path: PathBuf,
}

impl LlamaSummariser {
    pub fn new(paths: &Paths) -> Result<Self, TomeError>;
}

impl Summariser for LlamaSummariser { /* ... */ }

#[cfg(test)]
pub struct StubSummariser {
    call_count: AtomicU64,
}

#[cfg(test)]
impl StubSummariser {
    pub fn new() -> Self;
    pub fn call_count(&self) -> u64;
}

#[cfg(test)]
impl Summariser for StubSummariser {
    fn summarise(&self, input: &PluginSummariesInput) -> Result<SummariserOutput, TomeError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        // deterministic content-addressed output:
        let topics: Vec<String> = input.plugins.iter()
            .flat_map(|p| p.skills.iter().map(|s| s.name.clone()))
            .collect();
        Ok(SummariserOutput {
            short: topics.join(", "),
            long: format!("This workspace covers: {}. Call search_skills when working on these topics.", topics.join(", ")),
        })
    }
}
```

**LlamaBackend singleton**:

```rust
use std::sync::OnceLock;
static BACKEND: OnceLock<LlamaBackend> = OnceLock::new();
pub fn backend() -> Result<&'static LlamaBackend, TomeError> {
    BACKEND.get_or_try_init(|| LlamaBackend::init().map_err(TomeError::from))
}
```

## 14. `TomeError` extensions (new variants)

```rust
#[derive(Debug, Error)]
pub enum TomeError {
    // ... Phase 1/2/3 variants unchanged ...

    // === Phase 4 new variants ===
    #[error("workspace `{name}` not found in the central registry")]
    WorkspaceNotFound { name: String },                          // exit 13

    #[error("workspace `{name}` already exists")]
    WorkspaceAlreadyExists { name: String },                     // exit 14

    #[error("workspace name `{name}` is invalid: {reason}")]
    WorkspaceNameInvalid { name: String, reason: String },       // exit 15

    #[error("workspace `{name}` has {count} bound project(s); refusing without --force")]
    WorkspaceHasBoundProjects {
        name: String,
        count: usize,
        projects: Vec<String>,
    },                                                            // exit 16

    #[error("harness composition error: {kind}")]
    CompositionError { kind: CompositionErrorKind },             // exit 17

    #[error("harness `{name}` is not supported")]
    HarnessNotSupported { name: String },                        // exit 18

    #[error("harness MCP config clash in {path}: existing entry named `tome` does not match Tome's expected shape (command=`{command}`, first_arg=`{first_arg}`)")]
    HarnessClash {
        path: PathBuf,
        command: String,
        first_arg: String,
    },                                                            // exit 19

    #[error("summariser failure: {kind}")]
    SummariserFailure { kind: SummariserFailureKind },           // exit 20
}

#[derive(Debug)]
pub enum CompositionErrorKind {
    Cycle { path: Vec<String> },
    WorkspaceRefOutsideProject { found_in: ScopeKind },
    UnknownWorkspace(String),
    BadExclusion(String),
}

#[derive(Debug)]
pub enum SummariserFailureKind {
    ModelMissing,
    ModelChecksumMismatch { expected: String, observed: String },
    BackendInitFailed { source: String },
    OutputUnparsable { which: ShortOrLong },
    OutputEmpty { which: ShortOrLong },
}

#[derive(Debug, Clone, Copy)]
pub enum ShortOrLong { Short, Long }

impl ExitCode for TomeError {
    fn exit_code(&self) -> i32 {
        match self {
            // ... Phase 1/2/3 mappings unchanged ...
            TomeError::WorkspaceNotFound { .. }      => 13,
            TomeError::WorkspaceAlreadyExists { .. } => 14,
            TomeError::WorkspaceNameInvalid { .. }   => 15,
            TomeError::WorkspaceHasBoundProjects { .. } => 16,
            TomeError::CompositionError { .. }       => 17,
            TomeError::HarnessNotSupported { .. }    => 18,
            TomeError::HarnessClash { .. }           => 19,
            TomeError::SummariserFailure { .. }      => 20,
        }
    }
}
```

**Reused variants** (FR-602): no new variant introduced for: project marker malformed (reuse `WorkspaceMalformed` code 70); rename precondition missing project dir (reuse 70); per-user state dir unwritable (reuse `Io` code 7).

## 15. `DoctorReport` extensions

Phase 3's `DoctorReport` (in `src/doctor/report.rs`) gets new subsystem variants and a typed `Subsystem` enum (R-17 + P6 retro recommendation — promotion from `String` to enum at >6 arms).

```rust
/// Wire-shape preserving: serialises as a colon-separated string, matching the
/// Phase 3 `String`-typed `subsystem` field (`"embedder"`, `"catalog:<name>"`, …)
/// so external consumers of the doctor `--json` output don't observe a breaking
/// change. Internally this is a typed enum — see the custom `Serialize`/
/// `Deserialize` impls in `src/doctor/report.rs` for the string ↔ enum mapping.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Subsystem {
    // Phase 3 subsystems (typed)
    Embedder,                         // "embedder"
    Reranker,                         // "reranker"
    Index,                            // "index"
    Drift,                            // "drift"
    Catalog(String),                  // "catalog:<name>"
    Schema,                           // "schema"

    // Phase 4 new subsystems
    Summariser,                       // "summariser"
    Binding,                          // "binding"
    BindingRulesCopy,                 // "binding-rules-copy"
    HarnessRules(String),             // "harness-rules:<harness-name>"
    HarnessMcp(String),               // "harness-mcp:<harness-name>"
}

// Custom Serialize/Deserialize emit/parse the strings above. The on-the-wire
// shape matches Phase 3's String byte-for-byte for every Phase 3 variant
// (`"embedder"`, `"reranker"`, `"index"`, `"drift"`, `"catalog:<name>"`,
// `"schema"`); Phase 4 adds the new keys without changing any existing one.
// A unit test in src/doctor/report.rs locks the round-trip.

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub scope: ResolvedScope,
    pub project_binding: Option<ProjectBindingState>,    // NEW; None when outside any project
    pub embedder: SubsystemHealth,
    pub reranker: SubsystemHealth,
    pub summariser: SubsystemHealth,                     // NEW
    pub index: SubsystemHealth,
    pub drift: SubsystemHealth,
    pub catalogs: Vec<(String, SubsystemHealth)>,
    pub effective_harness_list: Option<EffectiveHarnessList>,  // NEW; None when outside any project
    pub harness_rules: Vec<(String, SubsystemHealth)>,         // NEW; per harness in effective list
    pub harness_mcp: Vec<(String, SubsystemHealth)>,           // NEW
    pub detected_uninstalled_harnesses: Vec<String>,           // NEW; supported harnesses present on machine but not in effective list
    pub suggested_fixes: Vec<SuggestedFix>,
    pub overall: DoctorClassification,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectBindingState {
    pub project_root: PathBuf,
    pub bound_workspace: WorkspaceName,
    pub config_well_formed: bool,
    pub rules_file_drift: RulesCopyState,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RulesCopyState { Match, Missing, Drift }

#[derive(Debug, Clone, Serialize)]
pub struct SuggestedFix {
    pub subsystem: Subsystem,
    pub auto_fixable: bool,
    pub message: String,
    pub command: Option<String>,            // exact shell command, where applicable
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DoctorClassification { Ok, Degraded, Unhealthy }
```

Auto-fixable subsystems in Phase 4: `Summariser` (model re-download); `BindingRulesCopy` (re-copy from workspace's RULES.md); `HarnessRules(<name>)` (re-run sync for that harness); `HarnessMcp(<name>)` (re-run sync for that harness — unless user-owned conflict, which requires explicit override).

NOT auto-fixable (per FR-562):

- `Binding` when the marker names a missing workspace (developer choice: rebind or recreate)
- `HarnessMcp` user-owned conflict (requires `--force`)

## 16. Atomic populated-directory helper

Lives at `src/util/atomic_dir.rs` (R-10):

```rust
pub fn land_directory<F>(target: &Path, mode_unix: u32, populate: F) -> Result<PathBuf, TomeError>
where F: FnOnce(&Path) -> Result<(), TomeError>;

pub fn land_directory_with_replace<F>(target: &Path, mode_unix: u32, populate: F) -> Result<PathBuf, TomeError>
where F: FnOnce(&Path) -> Result<(), TomeError>;
```

**Algorithm**:

1. Compute `parent = target.parent()`.
2. `let staging = tempfile::Builder::new().prefix(".tome.tmp.").tempdir_in(parent)?;`
3. `populate(staging.path())?;`
4. On Unix: `std::os::unix::fs::PermissionsExt::set_mode(staging.path(), mode_unix)`.
5. fsync the staging directory.
6. `let staged: PathBuf = staging.keep();` (infallible; consumes the auto-cleanup guard and returns the staging path)
7. For `_with_replace`: rename existing `target` to `target.with_extension("old")` if it exists; on rename failure, leave staged as-is and bubble.
8. `std::fs::rename(staged, target)?;`
9. For `_with_replace`: on success, `std::fs::remove_dir_all(target.with_extension("old"))` (best-effort; missing is fine).
10. On failure of step 8 in `_with_replace`: rename `target.with_extension("old")` back to `target`; bubble the error.
11. Return `target.canonicalize()?`.

**Test surface**: `tests/atomic_dir.rs` — happy path, SIGINT mid-populate (staged temp dir cleaned by `TempDir::drop`), SIGINT after `keep()` and before rename (orphaned staged dir, picked up by doctor `--fix` cleanup), replace rollback on rename failure.

## 17. CLI surface — new + modified commands

**New** (per FR-400 through FR-407, FR-520 through FR-525):

```text
tome workspace init <name> [--inherit-global] [--json]
tome workspace list [--json]
tome workspace info [<name>] [--json]
tome workspace use <name> [--force] [--json]
tome workspace rename <old> <new> [--json]
tome workspace remove <name> [--force] [--json]
tome workspace sync [<name>] [--json]
tome workspace regen-summary <name> [--json]

tome harness [--json]                                          # bare; lists supported harnesses
tome harness list [<workspace>] [--json]                       # effective list (no arg) or as-written (with arg)
tome harness use <name> [--scope project|workspace|global] [--force] [--json]
tome harness remove <name> [--scope project|workspace|global] [--json]
tome harness info <name> [--json]
tome harness sync [--json]
```

**Modified** (per FR-345; behavioural shifts only — surface unchanged):

```text
tome catalog add | remove | update | list | show              # operates on workspace_catalogs for the resolved workspace
tome plugin enable | disable                                  # operates on workspace_skills
tome query                                                    # joins through workspace_skills
tome reindex                                                  # scoped to the resolved workspace
tome status                                                   # reads from central DB, scoped to resolved workspace
tome doctor [--fix] [--verify] [--json]                       # extended subsystems per R-15
tome mcp                                                      # resolves workspace from project marker or env
tome models                                                   # extended to manage the summariser as a third model
```

**Removed**:

- `--global` top-level flag (replaced by `--workspace global` per FR-345)

## 18. Quickstart command sequence

The smoke-test sequence in `quickstart.md` validates the central refactor + binding flow + one harness end-to-end:

```bash
tome workspace init my-project --inherit-global
tome catalog add --workspace my-project github.com/example/skills
tome plugin enable --workspace my-project example-plugin
cd /path/to/project
tome workspace use my-project
# verify: .tome/config.toml exists, .tome/RULES.md exists, ~/.claude/settings.json has tome entry
tome doctor
# verify: every subsystem healthy
tome harness sync
# verify: byte-for-byte idempotent (no file change)
```

Specific success-criteria coverage: SC-101 (paths under <root>), SC-102 (single DB), SC-103 (workspace init), SC-104 (workspace use), SC-110 (auto-integration), SC-113 (idempotent sync), SC-114 (doctor healthy).
