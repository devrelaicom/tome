# Architecture

> **Purpose**: Document system design, patterns, component relationships, and data flow.
> **Generated**: 2026-05-26
> **Last Updated**: 2026-05-26 (Phase 4 v0.4.0 complete; 954 tests across 127 suites; Polish phase closed)

## Architecture Overview

Tome is a Rust CLI tool and MCP server that manages plugin ecosystems across coding harnesses (Claude Code, Cursor, Gemini CLI, Codex, OpenCode). It provides a centralized index for skill discovery and reranking, multi-workspace support with per-project bindings, harness composition management, workspace-scoped plugin enablement, and comprehensive health diagnostics with auto-repair.

The architecture is **monolithic with layered structure** split across two execution contexts:
- **CLI layer** — sync command dispatcher
- **MCP layer** — async stdio server (Phase 3+)

The central nervous system is a **single SQLite database** (`<home>/.tome/index.db`) that centralizes all state: plugin metadata, embeddings, workspace bindings, project bindings, enabled skills, and diagnostic metadata. Per-workspace composition settings and summaries live in separate TOML files (`<root>/workspaces/<name>/settings.toml`) and central RULES.md. Project markers (`<project>/.tome/config.toml`) are thin binding pointers, not databases.

Phase 4 / US1–US5 completes **harness synchronization, workspace lifecycle, composition management, workspace summarisation, and comprehensive health diagnostics**. Phase 4 / Polish (PR-A–PR-F) closes reviewer findings and hardens all subsystems. Phase 4 ships as v0.4.0 with 954 tests across 127 suites.

## Architecture Pattern

| Pattern | Description |
|---------|-------------|
| Layered (capability-based) | Commands → Business Logic (Lifecycle, Embedding, Workspace, Harness, Summarise, Doctor) → Data Access (Index, Catalog, Config) → Persistence (SQLite, Filesystem, Git) |
| Hexagonal (ports & adapters) | Trait boundaries for `Embedder`/`Reranker`/`Summariser`/`HarnessModule`/`ScopeProvider` allow swappable implementations (production vs stub for tests) |
| Trait-driven | Core abstractions decouple policy from mechanism; composition via struct fields rather than factory functions |
| Phase 4 / US5 — Doctor subsystem dispatch | `Subsystem` enum (11 typed variants) with byte-stable custom Serialize/Deserialize; exhaustive pattern matching replacing Phase 3 string routing |

## Core Components

### CLI Entry Point (`src/main.rs`)

- **Purpose**: Parse arguments, resolve workspace context, dispatch to subcommands
- **Location**: `src/main.rs`
- **Key flow**:
  1. Pre-parse `--version` flag (before clap) to include embedder/reranker/summariser identities
  2. Resolve `Paths` once from `$HOME/.tome/` (Phase 4 single root per constitution v1.3.0)
  3. Resolve workspace via `workspace::resolution::resolve()` (consults central DB)
  4. Route command dispatch; translate TomeError to exit codes
  5. Special-case MCP: skip stderr logging init + ctrlc handler (uses tokio signal)

### Path Resolution (`src/paths.rs`)

- **Purpose**: Resolve all Tome-owned paths from `$HOME/.tome/` root (Phase 4 consolidated)
- **Location**: `src/paths.rs`
- **Phase 4 changes**: Dropped XDG split (constitution v1.3.0 §Paths amendment); everything under single `<home>/.tome/` root
- **Public fields**:
  - `root` — `<home>/.tome/`
  - `index_db`, `index_lock` — central database
  - `catalogs_dir`, `models_dir` — on-disk resources
  - `workspaces_dir` — per-workspace settings
  - `logs_dir`, `mcp_log`, `mcp_log_prev` — diagnostics
- **Invariant**: All path joins happen here; no string literals elsewhere (enforced by test guards)

### Workspace Scope Resolution (`src/workspace/`)

- **Purpose**: Determine active workspace from CLI flag, env var, project marker, or default
- **Location**: `src/workspace/{name,scope,resolution}.rs`
- **Phase 4 changes**:
  - `Scope` → tuple struct `Scope(pub WorkspaceName)` (was: enum `Scope::Global | Scope::Workspace(PathBuf)`)
  - `ResolvedScope` gains `project_root: Option<PathBuf>` field
  - `--workspace <NAME>` flag (was: `--workspace <PATH>`); no more `--global` flag
  - Privileged `"global"` workspace is silent default
- **Resolution algorithm**:
  1. Check `--workspace <NAME>` CLI flag (validate against central `workspaces` table)
  2. Check `TOME_WORKSPACE` env var
  3. Walk project hierarchy for `.tome/config.toml` marker (read `workspace` field)
  4. Fall back to `"global"` workspace (always exists)
  5. Emit `WorkspaceConflict` (72) if multiple markers found; `WorkspaceNotFound` (13) if name not in registry

### Project-to-Workspace Binding (`src/workspace/binding.rs`)

- **Purpose**: Phase 4 / US1.a — Bind a project to a workspace; land atomic project marker
- **Location**: `src/workspace/binding.rs`
- **Key entry point**: `bind_project(project_root, workspace_name, force, deps) -> Result<BindOutcome, TomeError>`
- **Algorithm**:
  1. Dangerous CWD check (refuse `$HOME`, `/` unless `--force`)
  2. Acquire central DB advisory lock
  3. UPSERT into `workspace_projects` table (project_path PK, workspace_id FK, bound_at timestamp)
  4. Bump workspace `last_used_at` timestamp
  5. Land `<project>/.tome/config.toml` with `[workspace] = <name>` atomically via tempfile + rename
  6. Release lock; return `BindOutcome` with project_root, workspace name, and sync-outcome placeholder
- **Atomicity**: If DB commits but marker landing fails, doctor's Binding subsystem detects orphan; re-running recovers
- **Phase B** (harness sync): Runs outside this module, outside the lockfile (see `harness::sync`)

### Workspace Lifecycle (`src/workspace/{init,rename,remove,sync,regen_summary}.rs`)

- **Purpose**: Phase 4 / US2 — Complete workspace management surface
- **Location**: `src/workspace/{init,rename,remove,sync,regen_summary}.rs`
- **`init(target_root, workspace_name, inherit_global, force)` entry point**:
  - Atomic directory landing for `<root>/workspaces/<name>/` (settings.toml + RULES.md skeleton)
  - Creates row in central `workspaces` table
  - Optional catalog inheritance from global workspace (enrolment only; enablement not copied per FR-415)
  - Atomicity via `tempfile::Builder::tempdir_in` + `TempDir::keep()` + `std::fs::rename`
- **`rename(old_name, new_name, paths, workspace_name)`**:
  - Validates neither side is reserved `global` (exit 15)
  - Per-project marker rewrite (loop project_path/workspace_projects, read + replace workspace name, persist atomically per-project)
  - Filesystem rename of `<root>/workspaces/<old>/` → `<new>/`
  - Central DB UPDATE to `workspaces.name` (single transaction)
  - Drift detection post-rename; emits `RenameOutcome` with project_count, manifest_hash, summary cache state
- **`remove(workspace_name, force, paths)`**:
  - Refuses reserved `global` (exit 15)
  - Refuses non-empty bind list unless `--force` (exit 16 `WorkspaceHasBoundProjects`); returns list of bound project paths
  - 5-step cascade per FR-405:
    1. Per-project teardown: for each bound project, read marker, resolve effective harness list, per-harness cleanup (respect shared paths)
    2. Per-project marker removal: delete `<project>/.tome/config.toml`
    3. Single DB transaction: delete `workspace_skills`, `workspace_catalogs`, `workspace_projects`, `workspaces` rows
    4. Delete central `<root>/workspaces/<name>/` directory
    5. Refcount cleanup: for each catalog URL once-referenced only by removed workspace, `remove_dir_all` cache clone
- **`regen(workspace_name, paths)` (Phase 4 / US4.b)**:
  - Call summariser to generate short + long summaries from enabled plugins
  - Write to workspace settings `[summaries]` section atomically
  - Rewrite central `<root>/workspaces/<name>/RULES.md`
  - Per-project marker RULES.md copy (idempotent, skip if bytes match)
- **`sync_one(workspace_name, paths)` + `list_workspace_names(paths)`**:
  - `sync_one`: Copy central RULES.md to every bound project's marker copy (idempotent, skip if already match)
  - `list_workspace_names`: Enumerate `<root>/workspaces/` and return Vec<WorkspaceName>

### Harness Abstraction (`src/harness/`)

- **Purpose**: Trait-driven dispatch to five supported harnesses (Claude Code, Codex, Cursor, Gemini, OpenCode)
- **Location**: `src/harness/{mod,claude_code,codex,cursor,gemini,opencode,rules_file,mcp_config,sync}.rs`
- **Phase 4 NEW**: Complete harness abstraction layer with per-harness `HarnessModule` impls + sync orchestrator
- **`HarnessModule` trait methods**:
  - Identity — `name()`, `description()`
  - Detection — `detect(home) -> bool` (existence-only per FR-167)
  - Rules integration — `rules_file_target()`, `rules_file_strategy()`, `block_body_style()`
  - MCP config — `mcp_config_path()`, `mcp_config_format()`, `mcp_parent_key()`
- **Key decisions** (per research §R-8):
  - Each harness owns a file under `src/harness/`; no per-harness subdirs in commands/
  - Rules strategies: block-in-file (Claude, Codex, Gemini, OpenCode) vs standalone (Cursor)
  - MCP config: JSON for most, TOML for Codex; stored per-project (Claude, Cursor, OpenCode) or global (Codex, Gemini)
- **Registry**: `SUPPORTED_HARNESSES` static + test override hook (`HARNESS_MODULES_OVERRIDE`)

### Harness Synchronization Orchestrator (`src/harness/sync.rs`)

- **Purpose**: Phase 4 / US1.b-c — Compute effective harness list, dispatch per-harness writes, run cleanup
- **Location**: `src/harness/sync.rs`
- **Key entry point**: `sync_project(project_root, sync_deps) -> Result<SyncOutcome, TomeError>`
- **Algorithm** (mirrors `contracts/sync-algorithm.md`):
  1. **Phase B0** (locked read, caller's responsibility): Project marker landed, DB UPSERT committed, lock released
  2. **Phase B1** (unlocked filesystem reads): Compose effective harness list from project marker + workspace settings + global settings (via `settings::resolve_effective_list`)
  3. **Phase B2** (unlocked filesystem writes): Dispatch per-harness rules-file and MCP-config writes with dedup on target path
  4. **Phase B3** (unlocked cleanup): For harnesses no longer in effective list, remove their on-disk config/entries (respecting shared-path dedup)
- **Multi-harness sharing** (FR-482/483): When two harnesses target same rules-file path or MCP config path, dedup the write (first touch records the harness name); cleanup pass respects shared paths
- **Forward progress on clash** (FR-403): If user-owned `tome` entry blocks an MCP write without `--force`, record the error but keep processing; first clash wins for overall `Result::Err` (exit 19), but rules-file writes for unaffected harnesses still happen
- **Dedup logic**: `BTreeMap<PathBuf, effective_harness_name>` for rules files; same for MCP configs; per-path FIFO on first writer

### Settings & Composition (`src/settings/`)

- **Purpose**: Parse and resolve layered harness selections across project/workspace/global scopes
- **Location**: `src/settings/{mod,composition,parser,resolver,edit}.rs`
- **Phase 4 US3 changes**: Complete settings composition layer with production wiring via `CentralDbScopeProvider`
- **Layers** (priority order; first match wins):
  1. Project marker — `<project>/.tome/config.toml` (`ProjectMarkerConfig`)
  2. Workspace settings — `<root>/workspaces/<name>/settings.toml` (`WorkspaceSettings`)
  3. Global settings — `<root>/settings.toml` (`GlobalSettings`)
- **Composition references** (per `contracts/settings-composition.md`):
  - `[workspace]` — pull in active bound workspace's declared list
  - `[global]` — pull in global workspace's declared list
  - `[workspaces.<name>]` — pull in specific named workspace's declared list (one level deep, not recursive)
- **Resolver algorithm** (mirrors §Algorithm in `src/settings/resolver.rs`):
  1. Priority walk: first scope with non-None `harnesses` key is the primary declarer; others consulted only via composition refs
  2. Recursive descent: each entry parses to `CompositionRef`; bare names → inclusion set; bracketed refs → recurse into target scope's **directly-declared** list (FR-449)
  3. Cycle detection: DFS visited set of `(ScopeKind, key)` tuples; re-visit returns `CompositionErrorKind::Cycle` with path
  4. Final subtraction: exclusions (names prefixed with `!`) removed from inclusion set; result ordered by first-included-from chain
- **ScopeProvider trait** (F8 skeleton, US3.a production wiring):
  - Abstraction over workspace registry; allows resolver to be exercised against in-memory `StubScope` fixtures (tests) or production `CentralDbScopeProvider` (PR #92)
  - `directly_declared_harnesses(&WorkspaceName) -> Result<Option<Vec<String>>, CompositionErrorKind>`
  - Production impl (`CentralDbScopeProvider` in `commands/harness/mod.rs`) consults central SQLite registry to confirm membership, reads workspace's on-disk `settings.toml` for directly-declared list
- **`settings::edit` module** (US3.b): Abstraction for surgical TOML edits (project marker + workspace + global settings)
  - `open_settings(path)` → `DocumentMut`
  - `add_harness(doc, name, scope)` / `remove_harness(doc, name, scope)`
  - `save_settings(doc, path)` → atomic persist
  - Used by `harness use_ / remove` commands to append/delete harness entries
- **All types**: `#[serde(deny_unknown_fields)]` — Tome-owned inputs are strict per FR-013a boundary

### Summariser (`src/summarise/`)

- **Purpose**: Phase 4 / US4 — Generate short/long workspace summaries from enabled plugins via Qwen2.5-0.5B-Instruct GGUF
- **Location**: `src/summarise/{mod,llama,stub,trigger,registry,download,prompts}.rs`
- **Architecture**:
  - `Summariser` trait — `summarise(PluginSummariesInput) -> Result<SummariserOutput, TomeError>` (identity + trait boundary)
  - **Production**: `LlamaSummariser` via `llama-cpp-2` + process-wide `LlamaBackend` singleton (OnceLock + mutex)
  - **Test**: `StubSummariser` — deterministic, no model load
- **Model caching** (US4.d-1, S-M4):
  - SHA-256 verification + `LlamaModel::load_from_file` runs once in `LlamaSummariser::new`
  - Per-`summarise()` calls reuse the cached model
  - Per-prompt `LlamaContext` instances constructed fresh inside `summarise` and dropped before return
  - `LlamaModel` is `Send + Sync`; no `Mutex` wrapper needed
- **Singleton pattern**: First `backend()` call initializes via mutex-gated OnceLock; subsequent calls hit lock-free path
- **Triggered regeneration** (US4.b, FR-380/381/382/365/385):
  - `regenerate_for_trigger(workspace, paths)` invoked AFTER enable/disable/reindex/catalog-update commits their `workspace_skills` mutation
  - `SUMMARISER_OVERRIDE` thread_local (test injection via `SummariserOverrideGuard` RAII) bypasses production `LlamaSummariser` construct
  - Forward-progress invariant (FR-385): skill-state mutation commits BEFORE summariser invoked; on summariser failure (exit 24), skill state retained and cached summary not overwritten
  - `ModelMissing` is silent no-op in trigger callers (FR-423, documented in `contracts/summariser.md`); explicit `tome workspace regen-summary` still hard-fails
- **MCP integration** (US4.b, FR-425):
  - `mcp/tool_description.rs::compose(scaffold, cached_short)` reads workspace's `settings.toml` `[summaries].short` at startup
  - Composed description (scaffold + short summary) is applied to `search_skills` tool via runtime router mutation
  - `warn_if_too_long(desc)` emits warning if `len > 1500 chars` but applies anyway
  - No rerunning of summariser on MCP; subsequent CLI regenerations write to the same file, but MCP keeps in-memory description until restart
- **Model**: Qwen2.5-0.5B-Instruct GGUF (~400 MB, SHA-256 pinned: `74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db`, 491,400,032 bytes per US4.d-1 C-B1 fix)
- **Prompts**: Fixed SHORT_PROMPT (~400 tokens, 800 char max) + LONG_PROMPT (~1000 tokens, 2500 char max) (US4.d-1 consolidation)

### Doctor Diagnostics (`src/doctor/`)

- **Purpose**: Broad health check + auto-repair for embedder/reranker/catalogs/schema/drift/binding/harness-integration
- **Location**: `src/doctor/{mod,checks,fixes,binding,harness_integration,orphan_cleanup}.rs`
- **Key entry point**: `assemble_report(scope, paths, home, verify) -> DoctorReport`
- **Phase 4 / US5 additions** (refined Polish PR-A–PR-F):
  - **`Subsystem` enum** (11 variants, typed dispatch): Embedder, Reranker, Index, Drift, Catalog, Schema, Summariser, Binding, BindingRulesCopy, HarnessRules, HarnessMcp
  - **Custom Serialize/Deserialize**: Wire strings match Phase 3 vocabulary (e.g. `"catalog:name"`, `"harness-mcp:claude-code"`) with new Phase 4 variants slotting alongside
  - **`SubsystemHealth` enum** (5 variants): Ok, Drift, Broken, UserOwned, NotApplicable — single source of truth for per-subsystem classification (PR-A fix C-M1)
  - **`ProjectBindingState`** (T366): well-formedness check (marker parses + workspace exists), RULES.md drift classification
  - **`RulesCopyState` enum** (4 variants): Match, Missing, Drift, SourceMissing — distinguishes "source canonical RULES.md missing" from "project copy missing" for different fix paths
  - **`HarnessSubsystemReport`** (T367): per-harness rules-file + MCP-config health; test-respects overrides via `with_effective_modules`
- **Report fields**: Embedder health, reranker health, index integrity, drift, catalog cache state, harness presence, workspace registry status, summariser health, project binding state, effective harness list, harness rules/mcp integration, suggested fixes, overall classification
- **Classification**:
  - Unhealthy — embedder missing/corrupt, integrity fail, embedder drift, binding broken
  - Degraded — reranker missing/corrupt, reranker drift, any catalog cache != Ok, binding drift (RULES.md), harness integration issues
  - Ok — everything passes
- **Auto-fixes** (routed by `Subsystem` enum):
  - `Embedder` / `Reranker` / `Summariser` — `embedding::download::download_model` / `summarise::download::download_summariser_model`
  - `Catalog(name)` — `Git::clone_shallow`
  - `Schema` — `index::migrations::apply_pending` under advisory lock
  - `BindingRulesCopy` — `workspace::sync::sync_one_project` for single-project copy (C-M3)
  - `HarnessRules` / `HarnessMcp` — `harness::sync::sync_project` (coalesced single dispatch per project; R-M2)
- **Harness MCP override** (S-M2): User-owned `tome` entries are `auto_fixable: false` under plain `--fix`; `--fix --force` (US5.b) overrides and rewrites them. Scope gate: only harnesses with outstanding `UserOwned` fix participate in force dispatch
- **Orphan cleanup** (FR-410): Stale `.tome.tmp.*` staging directories older than 1 hour are swept from `<root>/workspaces/` and every bound project's parent directory (PR-E S-M1/M2 hardening)
- **No side effects** on `assemble`; `fixes::apply` mutates in place; `re_assemble` rebuilds derived state
- **Polish hardening** (PR-A–PR-F): SubsystemHealth per-harness emission (PR-A C-M1), graceful Broken collapse on SchemaTooNew (PR-B C-M2), project-local sync helper (PR-B C-M3), --force without --fix rejection (PR-B R-M1), coalesced harness sync (PR-B R-M2), per-entry validation (PR-C), SourceMissing distinction (PR-D R-M5), orphan 1-hour mtime gate (PR-E S-M1), precise --force scoping (PR-E S-M2), debug_assert safety invariants (PR-E S-M4)

### MCP Server (`src/mcp/`)

- **Purpose**: Async stdio MCP server advertising two tools: `search_skills`, `get_skill`
- **Location**: `src/mcp/{mod,runtime,log,preflight,server,state,tools}.rs`
- **Async boundary**: Only module allowed to use `tokio` (enforced by `tests/sync_boundary.rs`)
- **Concurrency model**: Single-threaded tokio runtime per research §R-2
- **Key components**:
  - `runtime.rs` — entry point; builds `tokio::runtime::Runtime`, installs file log, runs preflight, blocks on `rmcp::serve_server`
  - `preflight.rs` — FR-110 pipeline: schema-version gate → drift detection → embedder SHA-256 verify → eager-load FastembedEmbedder
  - `log.rs` — 10 MiB atomic-rotate file log (JSON lines); stderr reserved for fatal startup errors only (FR-222)
  - `state.rs` — `McpState { embedder, reranker (OnceLock), scope, paths, ... }`
  - `tools/search_skills.rs`, `tools/get_skill.rs` — handlers with spawn_blocking for sync work; Phase 4 US5.a: `search_skills` enforces 4096-char input length cap (code: `query_too_long`)
  - `tool_description.rs` (US4.b) — Compose runtime tool description from scaffold + cached workspace short summary
- **Tool handlers**: Validate input, lazy-load reranker via `OnceLock::get_or_try_init`, dispatch work inside `spawn_blocking`
- **Signal handling**: `tokio::signal::ctrl_c()` triggers graceful shutdown; 5 s timeout before hard shutdown

### Catalog Management (`src/catalog/`)

- **Purpose**: Register/list/update/remove external plugin catalogs from git repos
- **Location**: `src/catalog/{manifest,store,git}.rs`
- **Key invariants**:
  - On-disk clone cache at `<root>/catalogs/<sha256>/` (content-addressed)
  - Reference counting: `catalog::store::reference_count(url, paths) -> Vec<Scope>` determines cleanup eligibility
  - Credential scrubbing in git errors + model URLs (regex `[A-Za-z][A-Za-z0-9+.-]*://.*@`)
- **Manifest parsing**: `tome-catalog.toml` (strict, deny unknown fields)

### Configuration (`src/config.rs`)

- **Purpose**: Parse global `<root>/config.toml` — backward-compat layer for Phase 3 catalog enrolments (now moved to central DB junction)
- **Location**: `src/config.rs`
- **Type**: `Config` struct with `[catalogs]` table (read on commands that need catalog list)
- **Strictness**: `#[serde(deny_unknown_fields)]`

## Data Flow

### Primary User Flow: Bind a Project (Phase 4 / US1)

```
CLI: tome workspace use <workspace-name>
     ↓
Paths::resolve() — read $HOME, construct <home>/.tome/ paths
     ↓
Dangerous CWD check (refuse $HOME / / unless --force)
     ↓
index::open() with lock — acquire advisory lock
     ↓
workspace::binding::bind_project() — UPSERT into workspace_projects table
     ↓
Land <project>/.tome/config.toml with [workspace] = <name> atomically
     ↓
Release advisory lock
     ↓
commands::harness::sync_for_project_root() — PHASE B (unlocked)
  ↓
settings::resolve_effective_list(project, workspace, paths, home)
  ↓
harness::sync::sync_project() — per-harness rules-file + MCP-config writes
  ↓
Dedup on target paths; respect shared-path cleanup; forward-progress on clash
     ↓
CLI: print BindOutcome + SyncOutcome (added/updated/removed counts)
```

### Harness Composition Resolution (Phase 4 / US3)

```
CLI: tome harness list [workspace]
     ↓
resolve_effective_list(project_marker, workspace_settings, global_settings, ScopeProvider)
     ↓
Priority walk: find first scope with harnesses: key (others consulted only via refs)
     ↓
For each declared entry, parse CompositionRef:
  - Bare name → add to inclusion set
  - [workspace] / [global] / [workspaces.<name>] → recurse into target's **directly-declared** list
     ↓
DFS cycle detection via visited (ScopeKind, key) set; returns Cycle on re-visit
     ↓
Subtract exclusions (! prefixed) from inclusions
     ↓
Order by first-included-from chain; emit EffectiveHarnessList with source-chain per entry
     ↓
Production: ScopeProvider = CentralDbScopeProvider (consults workspaces table + reads .toml files)
Tests: ScopeProvider = StubScope (hand-rolled in-memory fixture)
     ↓
CLI: emit effective list + source chains (or error Cycle / UnknownWorkspace / SettingsReadFailure)
```

### Workspace Lifecycle Flow (Phase 4 / US2)

```
CLI: tome workspace init <name> | list | rename <old> <new> | remove <name> | regen-summary <name> | sync [<name>]
     ↓
Load central index (read-only for info/list/sync; write for init/rename/remove/regen-summary)
     ↓
Acquire lock if mutation (init/rename/remove create/update/delete in workspaces table)
     ↓
PHASE A (locked for mutations):
  - init: create workspace dir + settings skeleton + insert workspaces row
  - rename: per-project marker rewrites (unlocked after marker-time, before DB rename) + DB row update
  - remove: per-project teardown (unlocked per-project) → marker removal → DB cascade delete
  - regen-summary: invoke summariser → update workspace settings + central RULES.md + per-project copies
     ↓
PHASE B (unlocked):
  - sync: copy central RULES.md to every bound project marker (idempotent byte-match skip)
     ↓
Release lock; emit outcome (counts, project paths, summary cache state)
```

### Summarisation Flow (Phase 4 / US4.b — Triggered Regeneration)

```
CLI: tome plugin enable <catalog>/<plugin> (or disable/reindex/catalog update)
     ↓
Load workspace scope + central index
     ↓
Execute plugin enable/disable/reindex
     ↓
Commit workspace_skills mutation (INSERT/DELETE/UPDATE rows) in own transaction
     ↓
Release advisory lock
     ↓
regenerate_for_trigger(workspace_name, paths)
  ↓
Consult SUMMARISER_OVERRIDE thread_local (test injection point)
  ↓
If absent (production path):
  - Load LlamaSummariser::new(paths) — SHA-256 verify + LlamaModel::load_from_file (cached on self)
  - Return SummariserFailure::ModelMissing if GGUF absent (silent no-op per FR-423)
  ↓
Load enabled plugins for workspace (workspace_skills × skills join)
  ↓
Construct PluginSummariesInput with enabled plugin summaries
  ↓
Invoke summariser::summarise(input) → SummariserOutput { short, long }
  ↓
If SummariserFailure (other than ModelMissing): bubble to main, exit 24 (skill state retained, cache not overwritten)
  ↓
Warn if short_chars > 800 or long_chars > 2500 (FR-425)
  ↓
Update workspace settings.toml [summaries] section atomically (via toml_edit)
  ↓
Rewrite <root>/workspaces/<name>/RULES.md from long summary
  ↓
Sync new RULES.md to every bound project's marker copy (idempotent)
  ↓
CLI: return success (embedded in enable/disable/reindex outcome)
```

### MCP Tool Description Composition (Phase 4 / US4.b — Startup)

```
MCP: tome mcp starts
     ↓
Load workspace scope + central index (preflight)
     ↓
Read workspace's <root>/workspaces/<name>/settings.toml
  ↓
Extract [summaries].short field (if present and non-empty)
  ↓
Compose description = SCAFFOLD + "\n\n" + cached_short
  ↓
Warn if len > 1500 chars (FR-425)
  ↓
Apply composed description to search_skills tool via runtime router mutation
  ↓
MCP server advertises "search_skills: {description: '...scaffold...short summary...'}"
```

### Doctor Diagnosis Flow (Phase 4 / US5, hardened Polish PR-A–PR-F)

```
CLI: tome doctor [--fix] [--verify] [--force]
     ↓
Load workspace scope + central index (read-only; no lock)
     ↓
assemble_report(scope, paths, home, verify)
  ↓
Check embedder / reranker / summariser (model identity + on-disk state)
  ↓
Check index (PRAGMA integrity_check, schema version, drift)
  ↓
Check catalogs (on-disk cache state: Ok/Missing/NotARepo/ManifestInvalid/Orphan)
  ↓
Probe harnesses (five well-known dirs: ~/.claude, ~/.codex, ~/.cursor, ~/.gemini, ~/.opencode)
  ↓
Check workspace registry (workspaces.txt presence + entry count)
  ↓
If scope is ProjectMarker (T366):
  - check_binding: marker well-formedness + workspace registry membership + RULES.md drift
  ↓
If effective harness list is available (T367):
  - check_harness_integration: per-harness rules-file + MCP-config health (read-only)
  ↓
build_suggested_fixes: emit repair suggestions grouped by Subsystem, classify auto_fixable
  ↓
Overall classification: Unhealthy / Degraded / Ok (first match wins)
  ↓
--fix path (if requested):
  - Acquire lock only for schema/catalog repairs
  - Collect harness fixes (HarnessRules + HarnessMcp); coalesce to single sync_project dispatch (R-M2)
  - For each auto_fixable (or user-owned harness-mcp with --force):
    - Invoke repair handler
    - Re-run affected check
    - Update report in place
  - Orphan cleanup: sweep stale .tome.tmp.* dirs (1-hour mtime gate per PR-E S-M1)
  - re_assemble: rebuild suggested_fixes + overall classification
  ↓
CLI: emit report (human/JSON), exit 0 (healthy) / 1 (degraded) / 75 (unfixable)
```

### Primary User Flow: Enable a Skill

```
CLI: tome plugin enable <catalog>/<plugin>
     ↓
Paths::resolve() — read $HOME, construct <home>/.tome/ paths
     ↓
workspace::resolution::resolve() — consult CLI flag / env / project marker / default
     ↓
index::open() — acquire advisory lock, check schema, load embedder/reranker identities from meta
     ↓
plugin::lifecycle::enable() — read plugin.json + SKILL.md frontmatter, compute embeddings
     ↓
index::skills::enable_plugin_atomic() — INSERT/UPDATE skills, skill_embeddings, workspace_skills junction rows
     ↓
Release advisory lock
     ↓
summarise::regenerate_for_trigger(workspace_name, paths) — (US4.b)
     ↓
CLI: print summary (added/modified/unchanged skill counts)
```

### Search Flow: Query Skills

```
CLI: tome query "find a plugin that does X"
     ↓
workspace::resolution::resolve() → Scope(WorkspaceName)
     ↓
index::open_read_only() — open DB, don't take lock (readers ≠ writers)
     ↓
embedding::Embedder::embed(query) → Vec<f32> (384-dim)
     ↓
index::knn(embedding, filters) → Top-K candidates from workspace_skills
     ↓
embedding::Reranker::rerank(query, candidates) → Scored results
     ↓
CLI: print results (name, skill path, score)
```

## Layer Boundaries

| Layer | Responsibility | Can Access | Cannot Access |
|-------|----------------|------------|---------------|
| CLI | Argument parsing, mode dispatch, error formatting | Commands | Database, embedder directly |
| Commands | Command logic, outcome assembly, emit wrappers | Business logic (workspace, plugin, harness, settings, summarise, doctor) | Database directly (via deps) |
| Business logic | Policy (binding, lifecycle, sync, summarisation, diagnostics) | Index, catalog, plugin, settings, embedding, summarise | CLI, presentation |
| Data access | Queries, writes, transactions | Index, config, catalog on-disk | Commands, business logic |
| Persistence | SQLite, filesystem, git | Raw operations | Higher layers |

## Dependency Rules

- Higher layers can depend on lower layers, not vice versa
- Trait boundaries (`Embedder`, `Reranker`, `Summariser`, `HarnessModule`, `ScopeProvider`) decouple policy from mechanism
- `src/mcp/` is the only module allowed async (`tokio`); enforced by `tests/sync_boundary.rs`
- Workspace-specific code never reads/writes global index directly; uses scope-parameterized helpers
- Summariser trait allows test injection via `SUMMARISER_OVERRIDE` thread_local (mirrors `MIGRATIONS_OVERRIDE` pattern)
- Doctor's `Subsystem` enum dispatch is type-safe; matches are exhaustive

---

*This document describes HOW the system is organized at Phase 4 v0.4.0 (Polish complete). Keep focus on patterns and relationships. Phase 4 feature work (US1–US5) shipped across 20 commits (PRs #82–#101); Polish (PR-A–PR-F) hardened all subsystems with 35+ reviewer-flagged fixes applied. 954 tests across 127 suites. Binary size stable at ~29 MB macOS arm64 / ~34 MB Linux x86_64, well under the 50 MB cap.*
