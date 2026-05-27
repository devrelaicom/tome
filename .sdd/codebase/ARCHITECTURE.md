# Architecture

> **Purpose**: Document system design, patterns, component relationships, and data flow.
> **Generated**: 2026-05-27
> **Last Updated**: 2026-05-27 (Phase 5 Polish-complete; v0.5.0 shipped; per-entry invocability + doctor read-only extensions + single-source-of-truth promotion; 1193 tests)

## Architecture Overview

Tome is a Rust CLI tool and MCP server that manages plugin ecosystems across coding harnesses (Claude Code, Cursor, Gemini CLI, Codex, OpenCode). It provides a centralized index for skill discovery and reranking, multi-workspace support with per-project bindings, harness composition management, workspace-scoped plugin enablement, comprehensive health diagnostics with auto-repair, command indexing and MCP prompts capability, variable substitution engine with four-stage single-pass rendering pipeline, three-tier MCP discovery flow with middle-tier metadata fetching, and **Phase 5 SHIPPED** per-entry invocability flags with read-only doctor extensions (prompts report, entry-kind counts, orphan data-dir detection).

The architecture is **monolithic with layered structure** split across two execution contexts:
- **CLI layer** — sync command dispatcher
- **MCP layer** — async stdio server (Phase 3+)

The central nervous system is a **single SQLite database** (`<home>/.tome/index.db`) that centralizes all state: plugin metadata, embeddings, workspace bindings, project bindings, enabled entries (skills/commands), and diagnostic metadata. Per-workspace composition settings and summaries live in separate TOML files (`<root>/workspaces/<name>/settings.toml`) and central RULES.md. Project markers (`<project>/.tome/config.toml`) are thin binding pointers, not databases.

Phase 5 **SHIPPED** — all five user stories complete:
- **US1** — Commands as first-class database entries + MCP prompts capability
- **US2** — Single-pass substitution (builtins, env) + workspace-data relocation
- **US3** — Argument substitution (shell_split + coerce_arguments) + argument validation
- **US4** — Three-tier MCP discovery with bounded-memory description truncation + when_to_use indexing
- **US5** — Per-entry invocability enforcement + doctor read-only extensions (prompts report, entry-kind counts, orphan data-dir detection)

**Polish phase complete** — Single-source-of-truth promotion now firmly established across four instances: `plugin_data_root()`, `workspace_data_dir_for()`, `MCP_SLASH_PREFIX`, and `validate_db_stored_path` + `build_context_for_entry`. Bounded-walk discipline for caller-controlled string truncation is now uniform across both truncate sites. Stringly-typed enum dispatch rejected at six sites via canonical `EntryKind::from_str`.

## Architecture Pattern

| Pattern | Description |
|---------|-------------|
| Layered (capability-based) | Commands → Business Logic (Lifecycle, Embedding, Workspace, Harness, Summarise, Doctor, Substitution) → Data Access (Index, Catalog, Config) → Persistence (SQLite, Filesystem, Git) |
| Hexagonal (ports & adapters) | Trait boundaries for `Embedder`/`Reranker`/`Summariser`/`HarnessModule`/`ScopeProvider` allow swappable implementations (production vs stub for tests) |
| Trait-driven | Core abstractions decouple policy from mechanism; composition via struct fields rather than factory functions |
| Phase 5 / US1 — Unified entry dispatch | `EntryKind` enum (`Skill` \| `Command`) with kind-discriminated `skills` table rows; MCP prompts derived from user-invocable entries |
| Phase 5 / US2–US3 — Single-pass substitution | COMBINED_RE union regex processes all stages (builtins, env, arguments, ARGUMENTS tail) in one loop with per-match dispatch |
| Phase 5 / US3 — Argument substitution | Claude Code `$ARGUMENTS` / `$N` / `$name` with shell_split + coerce_arguments + apply_arguments_match pipeline; ARGUMENTS footer appended in render tail |
| Phase 5 / US4 — Three-tier MCP discovery | `search_skills` (small ranked list, truncated via char_indices walk) → `get_skill_info` (full description + when_to_use + 5-cap resource enumeration) → `get_skill` (full body); when_to_use indexed for semantic search |
| Phase 5 / US5 — Per-entry invocability + Doctor read-only | `user_invocable` frontmatter field controls MCP prompts visibility; Doctor read-only extensions report prompts registry status, entry-kind counts, orphan data directories (FR-124 read-only enforcement structural) |
| Phase 5 Polish — Single-source-of-truth accessors | `plugin_data_root()` for process-wide root; `workspace_data_dir_for()` for workspace-scoped paths; `validate_db_stored_path()` as canonical boundary-check helper; `build_context_for_entry()` as shared MCP context builder (eliminates ~50 LOC duplication across `prompts::build_get_context` + `tools::get_skill::build_substitution_context`) |
| Phase 5 Polish — Stringly-typed dispatch rejection | Six sites use canonical `EntryKind::from_str()` + exhaustive match instead of substring patterns; defence-in-depth for schema drift |

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

### Substitution Engine (`src/substitution/`)

- **Purpose**: Phase 5 / US1–US3 — Render entry bodies through a single-pass four-stage variable pipeline
- **Location**: `src/substitution/{mod,context,builtins,env,arguments,data_dir,regex_sets}.rs`
- **Phase 5 COMPLETE pipeline**:
  - **Stage 1: Built-ins** (`{{TOME_PLUGIN_DATA}}`, `{{TOME_WORKSPACE_DATA}}`, `{{TOME_WORKSPACE_NAME}}`, `{{TOME_CATALOG_NAME}}`, `{{TOME_PLUGIN_NAME}}`)
  - **Stage 2: Environment** (`{{$VAR}}` passthrough with TOME_ENV_ prefix)
  - **Stage 3: Arguments** (`$ARGUMENTS`, `$N` / `$1`–`$N`, `$name` with shell-style quoting; ARGUMENTS footer appended in render tail)
  - **Stage 4: ARGUMENTS Tail** (Optional `-- ${remaining_args}` footer for catch-all arguments)
- **Single-pass loop** in `render(body, context) -> Result<String, SubstitutionError>`: One regex scan (COMBINED_RE) over entire body with per-match dispatch
- **Module layout**:
  - `mod.rs` — `render(body, context)` entry point (single-pass loop); `body_has_bare_arguments(body) -> bool` helper (replaces substring check per US3.d R-M1 fix); `SubstitutionError` enum (6 variants)
  - `context.rs` — `SubstitutionContext` + `SubstitutionContextBuilder`; `ArgumentValues` enum (named/positional pairs)
  - `builtins.rs` — Stage 1 handler (5 placeholder patterns); lazy data-dir creation on first match
  - `env.rs` — Stage 2 handler (env var passthrough); TOME_ENV_ prefix support
  - `arguments.rs` — **Phase 5 / US3 COMPLETE** Stage 3 handler with three sub-pipelines:
    - `shell_split(input) -> Vec<String>` — POSIX shell quoting parser (handles single/double quotes, backslash escape)
    - `coerce_arguments(supplied: Vec<String>, declared: &[PromptArgument]) -> Result<ArgumentValues, SubstitutionError>` — match supplied args to declared schema (positional + named + validation)
    - `apply_arguments_match(pattern, values) -> String` — resolve `$ARGUMENTS`, `$N`, `$name` placeholders to their values
  - `data_dir.rs` — Lazy creation helpers: `ensure_plugin_data()` / `ensure_workspace_data()`
  - `regex_sets.rs` — `OnceLock<Regex>` COMBINED_RE (compiled once at startup or on first use)
- **Test injection seam**: `SUBSTITUTION_OVERRIDE` thread_local (mirrors `MIGRATIONS_OVERRIDE` / `SUMMARISER_OVERRIDE` pattern)

### Argument Substitution Details (`src/substitution/arguments.rs`)

- **Purpose**: Phase 5 / US3 — Match Claude Code argument syntax and render to entry contexts
- **Location**: `src/substitution/arguments.rs`
- **Key functions**:
  - **`shell_split(input: &str) -> Vec<String>`**: Parse POSIX shell quoting (respects single/double quotes, backslash escape sequences) to split arguments on unquoted whitespace. Returns all tokens including empty strings from consecutive separators (preserves intent).
  - **`coerce_arguments(supplied: Vec<String>, declared: &[PromptArgument]) -> Result<ArgumentValues, SubstitutionError>`**: Match supplied argument list to declared `PromptArgument` schema. Validates: positional count matches, named arguments exist in schema, no duplicates. Returns `ArgumentValues { positional: Vec<String>, named: HashMap<String, String> }`.
  - **`apply_arguments_match(pattern: &str, values: &ArgumentValues) -> String`**: Resolve a single matched pattern (e.g., `$1`, `$filename`, `$ARGUMENTS`) to its value from the validated `ArgumentValues`. Returns empty string for missing optional arguments (per Claude Code spec).
  - **Integration in `render()`**: For each regex match of type `$ARGUMENTS` / `$N` / `$name`, invoke `apply_arguments_match` to substitute the value in-place.
  - **ARGUMENTS footer**: In `render()`'s tail, after all inline substitutions complete, if `body_has_bare_arguments(body)` is true, append ` -- ${remaining_args}` using unconsumed positional arguments (catch-all collect behavior per contract).
- **Error handling**: `SubstitutionError::PromptArgumentMismatch { expected, supplied }` when count mismatch; `InvalidArgumentFrontmatter { reason, file }` on schema parse error.

### Entry Kind Discriminator (`src/plugin/identity.rs::EntryKind`)

- **Purpose**: Phase 5 / US1 — Distinguish skills from commands in the unified `skills` table
- **Location**: `src/plugin/identity.rs`
- **Type**: `#[serde(rename_all = "lowercase")] pub enum EntryKind { Skill, Command }`
- **Usage**:
  - Written to `skills.kind` column (v3 schema migration backfills from directory source)
  - Serialized as `"skill"` / `"command"` in JSON (wire shape matches v3 migration SQL constants)
  - Read by `plugin::components::list_command_files` (enumerates `<plugin>/commands/*.md`)
  - Plumbed through `PendingSkill` struct in `index::skills` (F3 skeleton)
  - Propagated through MCP prompts registry to surface command entries as invocable prompts
  - **Polish phase**: Canonical `EntryKind::from_str()` consumed at six sites instead of stringly-typed dispatch

### Plugin Components & Commands (`src/plugin/components.rs`)

- **Purpose**: Phase 5 / US1 — Walk plugin directories and enumerate commands
- **Location**: `src/plugin/components.rs`
- **Key additions**:
  - `list_command_files(plugin_dir) -> Vec<CommandFile>` — enumerate `<plugin>/commands/*.md` flat (non-recursive)
  - `CommandFile { path: PathBuf, name: String }` — one discovered command entry
  - Naming: `name` is filename stem (fallback when frontmatter omits); on-disk snapshot stays deterministic
- **Integration**: Called by `plugin::lifecycle::collect_pending_commands` to expand the enable pipeline to both skills and commands

### Paths & Data Directories (`src/paths.rs`)

- **Purpose**: Phase 4 consolidated root; Phase 5 / US1–US2 — Central data-directory accessors; Phase 5 Polish — **single source of truth** for `plugin_data_root()`
- **Location**: `src/paths.rs`
- **New methods**:
  - `plugin_data_root() -> PathBuf` — **Polish phase**: New accessor introduced as SSOT; replaces two prior inline `<root>/plugin-data/` paths
  - `plugin_data_dir_for(catalog, plugin) -> PathBuf` — `<root>/plugin-data/<catalog>/<plugin>/` (process-wide)
  - `workspace_data_dir_for(workspace, catalog, plugin) -> PathBuf` — `<root>/workspaces/<name>/plugin-data/<catalog>/<plugin>/` (workspace-scoped)
- **Semantics**:
  - Process-wide vs workspace-scoped scratch space (mirrors substitution engine's dual reference)
  - Paths computed in F3 (Phase 5 skeleton); creation wired in US2 (lazy, within substitution render)
  - Matching `{{TOME_PLUGIN_DATA}}` / `{{TOME_WORKSPACE_DATA}}` built-in variables
  - **Phase 5 / US2**: `ensure_plugin_data()` / `ensure_workspace_data()` called by `substitution::render()` on first `{{TOME_*}}` reference (lazy, idempotent)

### Plugin Lifecycle & Command Indexing (`src/plugin/lifecycle.rs`)

- **Purpose**: Phase 5 / US1 — Extended enable/disable/reindex to handle commands alongside skills
- **Location**: `src/plugin/lifecycle.rs`
- **Changes**:
  - `enable_plugin` now calls `plugin::components::list_command_files` to enumerate commands
  - `collect_pending_commands(plugin_dir, catalog, plugin, plugin_version) -> Vec<PendingCommand>`
  - Both skills and commands are inserted via a unified `index::skills::enable_plugin_atomic` call
  - `PendingSkill` struct extended with `kind: EntryKind`, `when_to_use: Option<String>`, `searchable: bool`, `user_invocable: bool`

### Workspace Rename & Plugin-Data Relocation (`src/workspace/rename.rs`)

- **Purpose**: Phase 5 / US2 — Relocate plugin-data directories during workspace rename
- **Location**: `src/workspace/rename.rs`
- **Algorithm additions**:
  1. Existing steps 1–5 (rename markers, update DB, rename workspace dir) — **unchanged**
  2. **Step 6: Plugin-data relocation** (inside the workspace directory rename at step 5):
     - Before `std::fs::rename(<root>/workspaces/<old>/, ...)`, enumerate `<root>/workspaces/<old>/plugin-data/` for any existing plugin-specific data
     - Move each `<catalog>/<plugin>/` subdirectory to the new workspace location
     - Pattern: `std::fs::rename(<old>/plugin-data/<cat>/<plug>/, <new>/plugin-data/<cat>/<plug>/)`
     - Failures are logged; doctor `--fix` can recover via simple re-copy if needed
- **Integration**: Part of the single `fs::rename` operation that relocates the workspace directory tree (same atomic boundary)

### Index Schema / Entry Records (`src/index/skills.rs`)

- **Purpose**: Phase 5 / US1 — CRUD over the unified `skills` table with kind discriminator; Polish — canonical **`validate_db_stored_path` SSOT** for path-traversal boundary checks
- **Location**: `src/index/skills.rs`
- **Changes**:
  - `SkillRecord` struct gains `kind: EntryKind` field (reads from `skills.kind` column)
  - `SkillRecord` gains `when_to_use: Option<String>`, `searchable: bool`, `user_invocable: bool` (new v3 columns)
  - `PendingSkill` struct extended with matching fields
  - `resolve_entry_body_path(catalog, plugin, name, kind) -> PathBuf` — Helper that routes via kind
    - Returns `<plugin>/skills/<name>/SKILL.md` or `<plugin>/commands/<name>.md` based on kind
    - **Polish phase**: Promoted to `pub(crate)` as canonical SSOT (consumed by `resolve_entry_body_path` + `commands/plugin/show.rs::list_entries`)
  - **`validate_db_stored_path(path: &Path) -> Result<(), TomeError>`** — **Polish phase SSOT**
    - Rejects `..` components and absolute paths (defence-in-depth for path-traversal per S-H1)
    - Called by both `get_skill::resolve_entry_body_path` + new `commands/plugin/show.rs::validate_displayed_path`
  - Schema v2→v3 migration (in `index::migrations.rs`, F3 skeleton):
    - Adds `kind` column (backfilled via directory walk: `skill` if exists in `skills/`, else `command`)
    - Adds `when_to_use`, `searchable`, `user_invocable` columns (backfilled with defaults per contract)

### MCP Prompts Registry (`src/mcp/{prompts,prompt_name,prompt_collision}.rs`)

- **Purpose**: Phase 5 / US1 — Expose commands/skills as invocable MCP prompts
- **Location**: `src/mcp/{prompts,prompt_name,prompt_collision}.rs`
- **Components**:
  - **`prompts.rs`** — `PromptRegistry` and `PromptEntry` (one resolved entry ready for registration):
    - Driven by workspace's enabled + user-invocable entries at MCP startup
    - Hand-rolled `PromptRouter` via `rmcp::handler::server::router::prompt::PromptRoute::new_dyn` (NOT macro)
    - `PromptsCapability` declared in `Server::get_info` alongside tools
  - **`prompt_name.rs`** — Derivation algorithm: `<plugin>__<entry>` with sanitisation/truncation
    - `sanitise(input) -> String` — ASCII-lowercase, `[a-z0-9_-]` charset, collapse `_` runs, strip boundaries
    - `sanitise_trunc(input, max) -> String` — sanitise + truncate at char boundary
    - `derive_name(plugin, entry, name_override) -> String` — apply override or format `<plugin>__<entry>`
    - Caps: PLUGIN_PORTION_MAX=16, ENTRY_PORTION_MAX=32, OVERRIDE_MAX=48
  - **`prompt_collision.rs`** — Collision detection when multiple entries map to the same prompt name:
    - `CollisionRecord { prompt_name, entries: Vec<EntryIdentity> }`
    - `resolve_collisions(registry) -> Vec<CollisionRecord>` — identifies conflicts for user visibility
  - **`tool_description.rs`** (Phase 4 US4.b, preserved): Compose runtime tool description from scaffold + cached summary

### MCP Discovery Flow — Phase 5 / US4 (Three-Tier) & Polish (`src/mcp/tools/`)

- **Purpose**: Three-tier discovery pattern optimized for semantic search agent workflows; Polish phase — **single-source-of-truth context builder**
- **Location**: `src/mcp/tools/{search_skills,get_skill_info,get_skill}.rs` + **Polish: `src/mcp/substitution_helpers.rs`**
- **Tier 1: `search_skills` tool**:
  - KNN + optional reranking against `when_to_use` + `description` embeddings
  - Returns **5–10 top results** (configurable), each with:
    - Catalog, plugin, entry name, kind, plugin_version
    - **Truncated description** (512 chars by default) — see `truncate_description` hardening below
    - First 100 chars of `when_to_use` guidance (if present)
    - Example command-line invocation (for skills only)
  - **Phase 5 / US4 C-1**: Description truncation via **bounded-memory char_indices walk** (O(n) worst-case, O(1) fast-path when input fits)
- **Tier 2: `get_skill_info` tool (NEW, Phase 5 / US4)**:
  - Middle-tier metadata fetch: allows agent to decide whether to fetch full body
  - Input: catalog, plugin, name, kind (defaults to Skill)
  - Output:
    - Full frontmatter description (NO truncation — search_skills handles that)
    - Full `when_to_use` guidance text (not truncated)
    - Plugin version, user_invocable flag
    - **Resource enumeration** (skill-only; commands return None per FR-083):
      - `files`: top-level sibling files in the entry's parent dir (excl. entry itself), alphabetized, capped at 5 entries + sentinel `"and N more"` if overflow
      - `directories`: BTreeMap of immediate subdirectories (keyed by name, alphabetized) with their immediate children (capped per-subdir, same sentinel rule)
      - Symlinks skipped at every level (hostile-catalog defence)
  - **Polish phase**: Builds context via **`build_context_for_entry()` SSOT** (shared with `prompts/get` handler; eliminates ~50 LOC duplication that existed across `mcp::prompts::build_get_context` + `mcp::tools::get_skill::build_substitution_context`)
  - Latency: O(n) walk of parent directory + subdirs; all paths returned as absolute strings
- **Tier 3: `get_skill` tool (existing)**:
  - Full entry body fetch: SKILL.md or command markdown
  - Preceded by Tier 2 (agent now knows whether this is worth fetching)
  - Returns complete body, component list, all sibling resources

### Description Truncation Hardening (`src/mcp/tools/search_skills.rs`)

- **Purpose**: Phase 5 / US4 C-1 — Efficient bounded-memory truncation in `search_skills` results; Polish — **mirrored at `get_skill_info` for consistency**
- **Location**: `src/mcp/tools/search_skills.rs::truncate_description(s: &str, max: usize) -> String`
- **Algorithm**:
  1. If `max == 0`, return empty string
  2. Iterate via `s.char_indices()` and count characters (NOT bytes)
  3. **Fast-path**: If input fits within `max` chars, return unchanged (no allocation, O(1) when no truncation needed)
  4. **Truncation path**: At first character beyond `max`, capture its byte offset via `char_indices`
  5. Slice at that boundary, append UTF-8 ellipsis `'…'` (U+2026), return
- **Correctness**: Guaranteed UTF-8 safe — slices always happen at char boundaries (never mid-multibyte)
- **Performance**: O(n) in worst case (must scan full input if no truncation), but O(k) when truncation happens at position k << n; no intermediate allocations in fast-path
- **Polish phase**: Now mirrors identical pattern at both `search_skills.rs` + `get_skill_info.rs`; single-source-of-truth helper pattern established

### Substitution Context Builder (`src/mcp/substitution_helpers.rs`)

- **Purpose**: Phase 5 Polish — Shared SSOT for building `SubstitutionContext` across both `prompts/get` + `get_skill_info` MCP handlers
- **Location**: `src/mcp/substitution_helpers.rs` (NEW, Polish phase)
- **Exports**:
  - `pub fn build_context_for_entry(catalog: &str, plugin: &str, entry_name: &str, ...) -> Result<SubstitutionContext, TomeError>`
  - Centralizes: plugin version lookup, entry body reading, frontmatter parsing, argument extraction, when_to_use access
  - Eliminates ~50 LOC duplication that existed across two MCP handler implementations
- **Pattern**: First instance of shared handler helper across multiple MCP tools; reusable for future tiered-discovery tools

### Prompt Arguments & Frontmatter (`src/plugin/frontmatter.rs`)

- **Purpose**: Phase 5 / US1 — Parse `arguments` frontmatter for entry invocation
- **Location**: `src/plugin/frontmatter.rs`
- **Changes**:
  - `SkillFrontmatter` extended with:
    - `arguments: Option<Vec<PromptArgument>>` — ordered list of expected arguments (name, type hint, optional description)
    - `argument_hint: Option<String>` — hint for catch-all `args` argument description (Case B fallback)
    - `prompt_name: Option<String>` — override for derived `<plugin>__<entry>` format
    - `when_to_use: Option<String>` — guidance indexed for search (Phase 5 / US4: now indexed for semantic search)
    - `searchable: Option<bool>` (default `true`) — controls `search_skills` visibility
    - `user_invocable: Option<bool>` (default `false` for skills; **Phase 5 / US5: now enforced by Doctor read-only checks**) — controls `prompts/list` visibility + reported in `plugin show`

### Doctor Read-Only Extensions (`src/doctor/`)

- **Purpose**: Phase 5 / US5 — Structural enforcement of FR-124 read-only invariant + per-entry invocability reporting
- **Location**: `src/doctor/{checks.rs, report.rs}`
- **Key additions**:
  - **`build_prompts_report(workspace, paths) -> PromptsReport`** (NEW):
    - Reuses `mcp::prompts::PromptRegistry::build_for_workspace` (no re-implementation per DRY)
    - Gathers enabled + user-invocable entries ready for MCP exposure
    - Reports count by kind (Skills / Commands)
    - Read-only — never modifies DB or filesystem
  - **`count_entries_by_kind(workspace, paths) -> EntryCountsByKind`** (NEW):
    - `{ skills: u32, commands: u32, other: u32 }` (other reserved for future kinds)
    - Queries enabled entries grouped by `skills.kind` column
    - Read-only
  - **`detect_orphan_data_dirs(workspace, paths) -> Vec<OrphanDataDirReport>`** (NEW):
    - Walks `<root>/plugin-data/` and `<root>/workspaces/<name>/plugin-data/` for directories whose entry is not in the index
    - Returns per-orphan report with path + size + last-modified
    - Informational only — no repair offered in Phase 5 (potential Phase 6 cleanup action)
    - Read-only
  - **Structural enforcement**: All three helpers `open_read_only`, never open transaction, never modify state
- **Integration**: Rendered in `tome doctor` output + extended `plugin show` + extended `plugin list` annotations
- **Test injection seam**: None required (reads from production DB, no overrideable state)

### Plugin Show Enhancement (`src/commands/plugin/show.rs`)

- **Purpose**: Phase 5 / US5 — Enhanced display with per-entry searchable/invocable annotations
- **Location**: `src/commands/plugin/show.rs`
- **Changes**:
  - Skills + Commands grouped in output (Kind header per section)
  - Per-entry annotations: `[searchable=true/false]`, `[user_invocable=true/false]`, `[dormant]` when disabled
  - New `EntryView` struct (derived from query result + frontmatter for human + JSON consistency)
  - Rendered in both plain-text (with visual grouping) and JSON (separate `skills` / `commands` arrays)
  - Lines extended ~228: metadata parsing, grouping logic, annotation rendering
  - **Polish phase**: Added `validate_db_stored_path()` call for displayed paths (defence-in-depth S-H1)

### Plugin List Enhancement (`src/commands/plugin/list.rs`)

- **Purpose**: Phase 5 / US5 — Per-kind entry counts in plugin summary
- **Location**: `src/commands/plugin/list.rs`
- **Changes**:
  - Format: `plugin: <plugin-name> (N skills, M commands)` instead of `plugin: <plugin-name> (N entries)`
  - Queries from `count_entries_by_kind` helper (shared with doctor)
  - Lines extended ~53: count queries + format

### Data Flow — Phase 5 Complete & Polish

#### Enable + Index Pipeline (US1–US3 unchanged, US4 adds search indexing, US5 adds invocability tracking, Polish: command normalization)

```
CLI: tome plugin enable <catalog>/<plugin>
     ↓
Load workspace scope + central index
     ↓
plugin::components::list_command_files(plugin_dir) + collect_pending_commands(...)
     ↓
For each command + skill, read frontmatter (widened with when_to_use, searchable, user_invocable, arguments)
     ↓
index::skills::enable_plugin_atomic(pending_commands, pending_skills)
  ↓
  Insert/update skills table rows with kind=command/skill
  ↓
  Insert/update when_to_use column (Phase 5 / US4: now indexed for search)
  ↓
  Insert user_invocable flag (Phase 5 / US5: enforced in Doctor read-only checks)
  ↓
  Insert workspace_skills junction rows (existing)
     ↓
Release advisory lock
     ↓
regenerate_for_trigger(workspace_name, paths)  (Phase 4; include when_to_use in embeddings per US4)
```

#### Three-Tier Discovery Flow (Phase 5 / US4, Polish: shared context builder)

```
CLI/MCP Agent: "How do I do X?"
     ↓
MCP: call search_skills(query="X", rerank=true)  [Tier 1 — fast semantic search]
     ↓
tome: Embed query → KNN search + optional rerank → Top 5–10 results
  ↓
  For each result:
    - Full description truncated to 512 chars via truncate_description (char_indices fast-path)
    - when_to_use guidance clipped to first 100 chars (search-preview)
    - Example invocation (skill-only)
  ↓
Return ranked list with truncated metadata
     ↓
Agent reviews summaries; picks candidate
     ↓
MCP: call get_skill_info(catalog, plugin, name, kind)  [Tier 2 — detailed metadata + resource preview]
     ↓
tome: Lookup entry in index → build context via build_context_for_entry() SSOT → read frontmatter → walk parent dir for resources (5-cap per dir)
  ↓
  Return SkillInfo {
    - Full description (no truncation)
    - Full when_to_use (for agent decision logic)
    - Plugin version, user_invocable
    - Resource enumeration { files: [...], directories: { "name": [...], ... } }
      ↓
      Each level cap-5 + "and N more" sentinel
  ↓
Agent scans resources; decides whether to fetch full body
     ↓
If yes:
  MCP: call get_skill(catalog, plugin, name, kind)  [Tier 3 — complete body]
  ↓
  tome: Resolve body path → read full markdown → return with all components
  ↓
  Agent renders/executes full entry
```

#### Doctor Read-Only Extensions (Phase 5 / US5)

```
CLI: tome doctor [--fix] [--verify]
     ↓
assemble_report(scope, paths, home, verify) — all checks run in read-only mode
  ↓
  Existing checks: embedder, reranker, index, drift, catalogs, harness-detection
  ↓
  + Phase 5 / US5 NEW:
    - build_prompts_report() — what entries are exposed as MCP prompts
    - count_entries_by_kind() — breakdown by skill/command
    - detect_orphan_data_dirs() — plugin-data dirs with missing entries
  ↓
  All use open_read_only; never take advisory lock; never modify state
  ↓
Emit human or JSON report
     ↓
If --fix:
  Only repair classes from Phase 4 US5 (embedder, reranker, schema, catalogs, harness-binding, orphan-staging)
  Phase 5 additions are informational-only (no auto-repair offered)
```

## Layer Boundaries

| Layer | Responsibility | Can Access | Cannot Access |
|-------|----------------|------------|---------------|
| CLI | Argument parsing, mode dispatch, error formatting | Commands | Database, embedder directly |
| Commands | Command logic, outcome assembly, emit wrappers | Business logic (workspace, plugin, harness, settings, summarise, doctor, substitution) | Database directly (via deps) |
| Business logic | Policy (binding, lifecycle, sync, substitution, summarisation, diagnostics) | Index, catalog, plugin, settings, embedding, summarise, substitution | CLI, presentation |
| Data access | Queries, writes, transactions | Index, config, catalog on-disk | Commands, business logic |
| Persistence | SQLite, filesystem, git | Raw operations | Higher layers |

## Dependency Rules

- Higher layers can depend on lower layers, not vice versa
- Trait boundaries (`Embedder`, `Reranker`, `Summariser`, `HarnessModule`, `ScopeProvider`) decouple policy from mechanism
- `src/mcp/` is the only module allowed async (`tokio`); enforced by `tests/sync_boundary.rs`
- `src/substitution/` is sync-only; variable rendering is pure compute (lazy data-dir creation is the only I/O side effect)
- Workspace-specific code never reads/writes global index directly; uses scope-parameterized helpers
- Substitution engine allows test injection via `SUBSTITUTION_OVERRIDE` thread_local (mirrors `MIGRATIONS_OVERRIDE` / `SUMMARISER_OVERRIDE` pattern)
- Entry kind dispatch via `EntryKind` enum is exhaustive; matches are type-safe; canonical `EntryKind::from_str()` consumed at six sites (Polish defense-in-depth)
- **Phase 5 / US3**: Single-pass rendering pipeline with per-match dispatch ensures each stage pattern is matched exactly once per body; argument coercion is validated before render
- **Phase 5 / US4**: Three-tier MCP discovery separates concerns: `search_skills` optimizes for ranking + truncation (char_indices fast-path), `get_skill_info` separates metadata from body, `get_skill` remains unchanged; resource enumeration walks (non-recursive, 5-cap per dir, alphabetical via BTreeMap for JSON stability)
- **Phase 5 / US5**: Doctor read-only extensions use only query-level operations; structural enforcement via `open_read_only` with no transaction acquisition
- **Phase 5 Polish**: Single-source-of-truth accessors established: `plugin_data_root()` for process-wide data root; `workspace_data_dir_for()` for workspace-scoped paths; `validate_db_stored_path()` for boundary checks; `build_context_for_entry()` for shared MCP context (eliminates ~50 LOC cross-handler duplication)

---

*This document describes HOW the system is organized at Phase 5 COMPLETE + Polish shipped (per-entry invocability + doctor read-only extensions + single-source-of-truth promotion). 1193 tests across 151 suites.*
