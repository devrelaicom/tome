# Architecture

> **Purpose**: Document system design, patterns, component relationships, and data flow.
> **Generated**: 2026-05-29
> **Last Updated**: 2026-05-31 (Phase 6 US2; real Claude Code hooks: JSON merge/remove into settings.local.json with targeted variable rewrite; sync reconciliation 3b subsystem)

## Architecture Overview

Tome is a Rust CLI tool and MCP server that manages plugin ecosystems across coding harnesses (Claude Code, Cursor, Gemini CLI, Codex, OpenCode). It provides a centralized index for skill discovery and reranking, multi-workspace support with per-project bindings, harness composition management, workspace-scoped plugin enablement, comprehensive health diagnostics with auto-repair, command indexing and MCP prompts capability, variable substitution engine with four-stage single-pass rendering pipeline, three-tier MCP discovery flow with middle-tier metadata fetching, per-entry invocability flags with read-only doctor extensions, **Phase 6 US1 COMPLETE** native agent translation pipeline, and **Phase 6 US2 COMPLETE** real Claude Code hooks: JSON-based hook entries from `<plugin>/hooks/hooks.json` with targeted two-variable rewrite (`${CLAUDE_PLUGIN_ROOT}` / `${CLAUDE_PLUGIN_DATA}`) and deep-equal structural merge/remove into the machine-local `.claude/settings.local.json`.

The architecture is **monolithic with layered structure** split across two execution contexts:
- **CLI layer** — sync command dispatcher + lifecycle orchestrator
- **MCP layer** — async stdio server (Phase 3+)

The central nervous system is a **single SQLite database** (`<home>/.tome/index.db`) that centralizes all state: plugin metadata, embeddings, workspace bindings, project bindings, enabled entries (skills/commands/agents), and diagnostic metadata. Per-workspace composition settings and summaries live in separate TOML files (`<root>/workspaces/<name>/settings.toml`) and central RULES.md. Project markers (`<project>/.tome/config.toml`) are thin binding pointers, not databases.

Phase 6 **US1 COMPLETE** — native agent translation pipeline:
- **Agent parsing pipeline** — `CanonicalAgent::parse` consumes `<plugin>/agents/<name>.md` (YAML frontmatter + Markdown body), validates agent name (`is_safe_agent_name` single-segment gate), deserializes CloudCode canonical vocabulary (name, description, model, tools/disallowed_tools, privileged hooks/mcp_servers/permission_mode)
- **Per-harness translation** — `HarnessModule::translate_agent()` overrides consumed by five harness impls; `map_model()` enforces same-vendor-only model alias table (FR-034/037); `infer_read_only()` detects read-only intent from tool posture (FR-036); `displayed_name()` handles clash-prefixed naming (FR-041); `render_markdown_yaml()` / `render_codex_toml()` emit harness-native file formats
- **Sync reconciliation (3c subsystem)** — `reconcile_agents()` enumerates enabled agents per-workspace, computes clash set once (FR-072), parses canonicals, dispatches per-harness translation, atomically writes `<plugin>__<name>.<ext>` (symlink-refusing, mode-preserving), removes orphaned `<plugin>__*` for non-live/non-supporting harnesses; forward progress on failure (FR-084); new `SyncSubsystem::Agents` discriminator in outcome
- **Agent indexing** — `list_agent_files()` walks `agents/*.md`; `collect_agent_entries()` parses frontmatter + validates names; agent rows inserted with `kind='agent'`, `searchable=false`, `user_invocable=false` per FR-070a; `agent_name_clash_set()` / `enabled_agents_for_workspace()` queries support sync reconciliation
- **Agent lifecycle** — Phase 5's `enable` / `disable` / `reindex` pipelines extended: agent collection happens after command collection; agent entry-kind is exhaustively matched alongside skill/command in doctor + plugin show/list; agent presence tracked in component counts

Phase 6 **US2 COMPLETE** — real Claude Code hooks (JSON merge into settings.local.json):
- **Hooks parsing pipeline** — `read_rewritten_entries()` reads a plugin's `<plugin>/hooks/hooks.json`, validates top-level object shape, applies **targeted two-variable rewrite** only (`${CLAUDE_PLUGIN_ROOT}` → absolute plugin root, `${CLAUDE_PLUGIN_DATA}` → plugin data dir; all other `${CLAUDE_*}` tokens left verbatim per NFR-007), returns `RewrittenHooks` event-keyed entries; malformed/unreadable files → exit 43
- **Hooks merge/remove semantics** — `merge_into_settings()` idempotently appends rewritten entries under event keys via deep `serde_json::Value` equality (never duplicates user-authored identical entries per FR-004); creates `settings.local.json` with `{"hooks": {}}` when absent (FR-002); `remove_from_settings()` removes matching entries only, prunes empty event arrays (FR-005/FR-006), leaves non-matching/user-edited entries in place (ownership = re-derivation + structural match per NFR-003)
- **Sync reconciliation (3b subsystem)** — `reconcile_hooks()` enumerates enabled plugins per-workspace, reads + rewrites each plugin's hooks once, dispatches merge/remove to live/non-live harnesses, records per-file changes in `SyncOutcome` with `SyncSubsystem::Hooks` discriminator; forward progress on parse failures (FR-084); only Claude Code harness participates (`RealJson` strategy); every other harness is `GuardrailsOnly` fallback (US3)
- **Atomic write discipline** — All writes to `settings.local.json` use atomic tempfile + rename (symlink-refusing, mode-preserving), target the machine-local gitignored file never the committed `settings.json` (FR-002 contract)
- **Two-variable rewrite mechanism** — Textual replace of exactly two fixed needle tokens (`${CLAUDE_PLUGIN_ROOT}`, `${CLAUDE_PLUGIN_DATA}`) in every JSON string leaf (keys/numbers/booleans/nulls untouched per FR-003), never touches unrecognized `${CLAUDE_*}` tokens (left for Claude Code runtime resolution), fails closed on non-UTF-8 rewrite targets (exit 44, R2-2)

## Architecture Pattern

| Pattern | Description |
|---------|-------------|
| Layered (capability-based) | Commands → Business Logic (Lifecycle, Embedding, Workspace, Harness, Summarise, Doctor, Substitution) → Data Access (Index, Catalog, Config) → Persistence (SQLite, Filesystem, Git) |
| Hexagonal (ports & adapters) | Trait boundaries for `Embedder`/`Reranker`/`Summariser`/`HarnessModule`/`ScopeProvider` allow swappable implementations (production vs stub for tests) |
| Trait-driven | Core abstractions decouple policy from mechanism; composition via struct fields rather than factory functions |
| Phase 5 / US1 — Unified entry dispatch | `EntryKind` enum (`Skill` \| `Command` \| `Agent`) with kind-discriminated `skills` table rows; MCP prompts derived from user-invocable entries (skills & commands only) |
| Phase 5 / US2–US3 — Single-pass substitution | COMBINED_RE union regex processes all stages (builtins, env, arguments, ARGUMENTS tail) in one loop with per-match dispatch |
| Phase 5 / US3 — Argument substitution | Claude Code `$ARGUMENTS` / `$N` / `$name` with shell_split + coerce_arguments + apply_arguments_match pipeline; ARGUMENTS footer appended in render tail |
| Phase 5 / US4 — Three-tier MCP discovery | `search_skills` (small ranked list, truncated via char_indices walk) → `get_skill_info` (full description + when_to_use + 5-cap resource enumeration) → `get_skill` (full body); when_to_use indexed for semantic search |
| Phase 5 / US5 — Per-entry invocability + Doctor read-only | `user_invocable` frontmatter field controls MCP prompts visibility; Doctor read-only extensions report prompts registry status, entry-kind counts, orphan data directories (FR-124 read-only enforcement structural) |
| Phase 5 Polish — Single-source-of-truth accessors | `plugin_data_root()` for process-wide root; `workspace_data_dir_for()` for workspace-scoped paths; `validate_db_stored_path()` as canonical boundary-check helper; `build_context_for_entry()` as shared MCP context builder |
| Phase 5 Polish — Stringly-typed dispatch rejection | Six sites use canonical `EntryKind::from_str()` + exhaustive match instead of substring patterns; defence-in-depth for schema drift |
| Phase 6 / US1 — Native agent translation | Agent parsing SSOT in `src/harness/agents.rs` (`CanonicalAgent::parse`, `agent_filename`, `plugin_of_owned_file` provenance split, `map_model` alias table, `infer_read_only`, `displayed_name`, render primitives); per-harness `translate_agent()` overrides in five harness impls; sync reconciliation pass (3c) computes clash set once, parses enabled agents, dispatches translation, writes atomically, removes orphaned files; forward progress on agent translation failures |
| Phase 6 / US2 — Real Claude Code hooks | Hooks parsing SSOT in `src/harness/hooks.rs` (`read_rewritten_entries`, targeted two-variable rewrite, merge/remove semantics); only Claude Code harness participates (`RealJson` strategy); sync reconciliation pass (3b subsystem) enumerates enabled plugins once, rewrites hooks once, dispatches merge/remove per harness (live vs non-live), records per-file granularity in outcome; atomic writes to machine-local `settings.local.json` (never committed `settings.json`); forward progress on parse failures |

## Core Components

### Real Claude Code Hooks (`src/harness/hooks.rs`)

- **Purpose**: Phase 6 / US2 — Read a plugin's `hooks/hooks.json`, rewrite two path variables, merge/remove entries into `.claude/settings.local.json` (only Claude Code harness)
- **Location**: `src/harness/hooks.rs`
- **Public functions** (the SSOT for hooks rewriting):
  - `read_rewritten_entries(plugin_root, plugin_data) → Result<Option<RewrittenHooks>>` — Read and rewrite a plugin's `<plugin>/hooks/hooks.json`; validates top-level object shape (event-keyed arrays); applies targeted two-variable rewrite to every JSON string leaf; returns `RewrittenHooks` or `Ok(None)` when absent; malformed/unreadable → exit 43
  - `merge_into_settings(target, hooks) → Result<bool>` — Merge rewritten hooks into `<project>/.claude/settings.local.json`, appending each entry under its event only when no deep-equal entry exists (idempotent per FR-004); creates file + parent dir (0700) when absent (FR-002); atomic, mode-preserving, symlink-refusing; returns `true` on change, `false` on no-op; any failure → exit 44
  - `remove_from_settings(target, hooks) → Result<bool>` — Remove matching hooks from `settings.local.json` by deep structural equality, prune empty event arrays (FR-005/FR-006); non-matching/user-edited entries left in place (ownership = re-derivation + structural match per NFR-003); missing file is no-op; returns `true` on change; any failure → exit 44
- **Types**:
  - `RewrittenHooks` — Event-keyed entries post-rewrite: `Vec<(String, Vec<JsonValue>)>` where each `JsonValue` is a fully-rewritten hook object
- **Two-variable rewrite** (FR-003, R-4):
  - `${CLAUDE_PLUGIN_ROOT}` → absolute installed-plugin root (no relative path)
  - `${CLAUDE_PLUGIN_DATA}` → `~/.tome/plugin-data/<catalog>/<plugin>/`
  - Every other `${CLAUDE_*}` token (e.g., `${CLAUDE_PROJECT_DIR}`, `${CLAUDE_SESSION_ID}`) left verbatim — Claude Code resolves at runtime
  - Rewrite applied to JSON string leaves only; keys, numbers, booleans, nulls untouched
  - Non-UTF-8 rewrite targets fail closed (exit 44, R2-2)
- **Ownership model** (NFR-003): No provenance marker or sidecar — ownership established solely by re-derivation + deep `serde_json::Value` equality. A hook the user hand-edited after Tome wrote it no longer matches and is left in place; Tome never deletes a hook it cannot prove it owns.

### Sync Reconciliation — Hooks Subsystem (3b) (`src/harness/sync.rs`)

- **Purpose**: Phase 6 / US2 — Orchestrate real-hooks merge/remove across all harnesses; runs BEFORE agents reconciliation (sink order: hooks → agents)
- **Location**: `src/harness/sync.rs` (`reconcile_hooks()` + helpers)
- **Algorithm**:
  1. **Fast exit**: If no harness has a settings path (all `GuardrailsOnly`), return (no hooks participation)
  2. **DB enumeration**: Open central DB read-only; enumerate enabled plugins for workspace (shared across all `RealJson` harnesses)
  3. **Hooks parsing**: For each enabled plugin, resolve root dir, read + rewrite hooks once (forward progress on parse failure per FR-084); plugins with no `hooks/hooks.json` contribute nothing
  4. **Per-harness dispatch**: For each harness with a settings path:
     - If **live** (in effective list): `merge_into_settings()` appends rewritten entries
     - If **non-live** (not in effective list): `remove_from_settings()` removes matching entries
  5. **Result tracking**: Record per-file granularity in `SyncOutcome` (added/updated/removed) with `SyncSubsystem::Hooks` discriminator; aggregate per-harness action in `HarnessDecision::hooks_action`
- **Data structures**:
  - `HarnessSnapshot::hook_settings_path` — `Some(path)` for `RealJson` harnesses, `None` for `GuardrailsOnly`
  - `SyncOutcome::added/updated/removed` — Records per-settings-file changes with `SyncSubsystem::Hooks`
  - `HarnessDecision::hooks_action` — Aggregate action per harness (Created/Updated/Removed/LeftAlone)
- **Forward progress** (FR-084): Parse failures recorded as first error; plugin is skipped; sibling plugins still reconcile; surface error after all harnesses processed

### Claude Code Harness Extensions (`src/harness/claude_code.rs`)

- **Purpose**: Phase 6 / US2 — Implement real-hooks strategy for Claude Code (only harness with `RealJson` support)
- **Location**: `src/harness/claude_code.rs`
- **Trait method overrides**:
  - `hooks_strategy() → HooksStrategy` — Returns `HooksStrategy::RealJson` (only Claude Code)
  - `hook_settings_path(project_root) → Option<PathBuf>` — Returns `<project>/.claude/settings.local.json` (machine-local, gitignored)
- **Contract**: Rewritten hooks carry machine-specific absolute paths (from the two-variable rewrite), so they land only in `settings.local.json`, never the committed `settings.json` (FR-002)

### Agent Translation Core (`src/harness/agents.rs`)

- **Purpose**: Phase 6 / US1 — Harness-agnostic agent parsing, validation, translation, and filename provenance
- **Location**: `src/harness/agents.rs`
- **Public functions** (the SSOT for agent translation):
  - `CanonicalAgent::parse(catalog, plugin, filename_stem, contents)` — Parse `<plugin>/agents/<name>.md` (YAML frontmatter + Markdown body) into a `CanonicalAgent`; malformed frontmatter maps to `TomeError::AgentTranslationFailed` (exit 45)
  - `agent_filename(plugin, name, ext)` — Build the Tome-owned provenance filename `<plugin>__<name>.<ext>` (FR-040, R-19); the sole SSOT for agent file naming
  - `plugin_of_owned_file(filename)` — Recover the `<plugin>` prefix from `<plugin>__<name>.<ext>`; the inverse of `agent_filename` and SSOT for the sync reconciliation ownership split (FR-043)
  - `is_safe_agent_name(name)` — Validate single-safe-path-segment gate (S-1): rejects `/`, `\`, NUL, leading `.`, `..`, `.`, empty, or multi-component paths; consumed at index time before name storage
  - `map_model(harness, source)` — Same-vendor-only model alias table (FR-034/037, R-8, SC-002): maps canonical sources (opus/sonnet/haiku/inherit) to harness-native ids; returns `None` to drop the field; cross-vendor mapping forbidden
  - `infer_read_only(tools, disallowed)` — Infer read-only intent from tool posture (FR-036): returns `Some(true)` when provably read-only, `Some(false)` when write/edit/execute tool granted, `None` for indeterminate → caller drops field
  - `displayed_name(plugin, name, clashes)` — Resolve display name: clean `<name>` or clash-prefixed `<plugin>-<name>` (FR-041); OpenCode override is per-harness concern
  - `render_markdown_yaml(frontmatter, body)` — Render Markdown-with-YAML-frontmatter agent file; preserves frontmatter key order
  - `render_codex_toml(scalars, body)` — Render Codex TOML agent with body in triple-quoted `developer_instructions` string (FR-033, R-14)
- **Types**:
  - `CanonicalAgent` — Plugin source form: catalog, plugin, name, description, body (Markdown), model, tools/disallowed_tools, privileged hooks/mcp_servers/permission_mode (opaque `serde_json::Value` per FR-050)
  - `TranslatedAgent` — Per-harness emission result: dir, filename (`<plugin>__<name>.<ext>`), displayed_name, format (MarkdownYaml or Toml), rendered content, dropped_fields list (diagnostics per FR-032/034/036)
- **Validation**: Agent names validated at index time via `is_safe_agent_name`; parse failures surface as exit 45; same-vendor-only model enforcement via `map_model`; read-only inference from tool posture via `infer_read_only`

### Harness Agent Translation Dispatch (`src/harness/{claude_code,codex,cursor,gemini,opencode}.rs`)

- **Purpose**: Phase 6 / US1 — Per-harness `translate_agent()` override implementations
- **Locations**: Five harness modules
- **Overrides** (all four methods required for native-agent support):
  - `supports_native_agents() → bool` — Returns `true` for claude-code/codex/cursor/opencode; `false` for gemini
  - `agent_dir(project_root) → Option<PathBuf>` — Returns the harness-specific agent target directory (e.g., `<project>/.claude/agents` for Claude Code)
  - `agent_format() → Option<AgentFormat>` — Returns MarkdownYaml (claude-code/cursor) or Toml (codex/opencode); gemini returns `None`
  - `translate_agent(canonical) → TranslatedAgent` — Calls harness-specific translation logic: applies field map (model alias, read-only inference, privilege drop), calls render primitive (`render_markdown_yaml` or `render_codex_toml`), builds `TranslatedAgent` result; field-drop diagnostics recorded in `dropped_fields`
- **Field mapping** (per FR-032/034/036):
  - `model` field maps via `map_model(harness_name, canonical.model)` (same-vendor-only); `None` drops it
  - `read_only` field inferred via `infer_read_only(tools, disallowed_tools)` when not explicitly present; indeterminate → drop
  - Privileged fields (hooks/mcp_servers/permission_mode) passed through to Claude Code; dropped for other harnesses per FR-050
  - `tools` / `disallowed_tools` fields passed through unchanged for Cursor; dropped for Codex/OpenCode per FR-033

### Sync Reconciliation — Agents Subsystem (3c) (`src/harness/sync.rs`)

- **Purpose**: Phase 6 / US1 — Orchestrate native agent file emission, translation, and cleanup across all harnesses
- **Location**: `src/harness/sync.rs` (`reconcile_agents()` + helpers)
- **Algorithm**:
  1. **Fast exit**: If no harness supports native agents, return (no emission, no cleanup needed)
  2. **DB enumeration**: Open central DB read-only; enumerate enabled agents for workspace + compute clash set once (FR-072)
  3. **Canonical parsing**: For each enabled agent, resolve source path, read body (bounded I/O), parse `CanonicalAgent`; parse failures recorded as first error but do not abort
  4. **Per-harness dispatch**: For each live, native-supporting harness:
     - Translate each `CanonicalAgent` via `harness.translate_agent()` (produces `TranslatedAgent`)
     - Atomically write `<plugin>__<name>.<ext>` to `harness.agent_dir()` (symlink-refusing, mode-preserving)
     - Remove orphaned `<plugin>__*` files (plugins no longer enabled)
  5. **Cleanup**: For each non-live or non-supporting harness, remove all Tome-owned `<plugin>__*` files
  6. **Forward progress** (FR-084): Record first agent translation failure but continue processing remaining agents/harnesses; surface error at end
- **Data structures**:
  - `SyncOutcome::added/updated/removed` — Records per-agent file writes with `SyncSubsystem::Agents` discriminator
  - `HarnessDecision::agents_action` — Aggregate action per harness (Created/Updated/Removed/LeftAlone)
- **Clash handling** (FR-041): `agent_name_clash_set()` query identifies workspace agents with duplicate names; `displayed_name()` applies clash-prefixed form; on-disk filename stays `<plugin>__<name>` regardless

### Agent Indexing & Lifecycle (`src/plugin/components.rs`, `src/plugin/lifecycle.rs`, `src/index/skills.rs`)

- **Purpose**: Phase 6 / US1 — Enumerate, index, and manage agent entries alongside skills/commands
- **Locations**: `src/plugin/components.rs` (enumeration), `src/plugin/lifecycle.rs` (lifecycle), `src/index/skills.rs` (CRUD)
- **Enumeration** (`src/plugin/components.rs`):
  - `list_agent_files(plugin_dir)` — Walk `agents/*.md` directory; returns `Vec<EntryFile>` (path + name stem); reuses the same flat-walk pattern as commands
  - `count_components()` — Includes `agents` count in returned `ComponentCounts`
- **Lifecycle** (`src/plugin/lifecycle.rs`):
  - `collect_agent_entries(plugin, workspace)` — Part of the enable pipeline after `collect_command_entries`; parses agent frontmatter, validates names via `is_safe_agent_name`, builds `PendingAgent` list
  - Reuses `enable_plugin_atomic()` atomic transaction: agent rows inserted with `kind='agent'`, `searchable=false`, `user_invocable=false` per FR-070a
- **Index queries** (`src/index/skills.rs`):
  - `agent_name_clash_set(conn, workspace) → BTreeSet<String>` — Returns agent names appearing in 2+ enabled plugins (used by sync reconciliation for display-name decisions)
  - `enabled_agents_for_workspace(conn, workspace) → Vec<EnabledAgent>` — Returns enabled agent rows for a workspace (catalog, plugin, name); used to enumerate agents for translation
  - Agent rows always queryable via `resolve_entry_body_path()` (resolve source `.md` for reading); always non-searchable (FR-070a filter at `search_skills` query time); always non-invocable (excluded from MCP prompts)

### Plugin Show & List Extensions (`src/commands/plugin/{list,show}.rs`)

- **Purpose**: Phase 6 / US1 — Display agent entries alongside skills/commands
- **Changes**:
  - `list.rs` — Output format widened to "N skills, M commands, P agents"
  - `show.rs` — New Agents section (kind-grouped, ~228 lines extended); per-agent annotations (searchable=false per invariant, user_invocable=false per invariant, dormant when disabled in workspace)

## Layer Boundaries

| Layer | Responsibility | Can Access | Cannot Access |
|-------|----------------|------------|---------------|
| CLI | Argument parsing, mode dispatch, error formatting | Commands | Database, embedder directly |
| Commands | Command logic, outcome assembly, emit wrappers | Business logic (workspace, plugin, harness, settings, summarise, doctor, substitution) | Database directly (via deps) |
| Business logic | Policy (binding, lifecycle, sync, substitution, diagnostics, harness trait dispatch, agent translation, hooks rewriting) | Index, catalog, plugin, settings, embedding, summarise, substitution, harness | CLI, presentation |
| Data access | Queries, writes, transactions | Index, config, catalog on-disk | Commands, business logic |
| Persistence | SQLite, filesystem, git | Raw operations | Higher layers |

## Dependency Rules

- Higher layers can depend on lower layers, not vice versa
- Trait boundaries (`Embedder`, `Reranker`, `Summariser`, `HarnessModule`, `ScopeProvider`) decouple policy from mechanism
- `src/mcp/` is the only module allowed async (`tokio`); enforced by `tests/sync_boundary.rs`
- `src/substitution/` is sync-only; variable rendering is pure compute (lazy data-dir creation is the only I/O side effect)
- Workspace-specific code never reads/writes global index directly; uses scope-parameterized helpers
- Substitution engine allows test injection via `SUBSTITUTION_OVERRIDE` thread_local (mirrors `MIGRATIONS_OVERRIDE` / `SUMMARISER_OVERRIDE` pattern)
- Entry kind dispatch via `EntryKind` enum is exhaustive; matches are type-safe; canonical `EntryKind::from_str()` consumed at six+ sites (Polish + Foundational + US1 defence-in-depth)
- **Phase 5 / US3**: Single-pass rendering pipeline with per-match dispatch ensures each stage pattern is matched exactly once per body; argument coercion is validated before render
- **Phase 5 / US4**: Three-tier MCP discovery separates concerns: `search_skills` optimizes for ranking + truncation (char_indices fast-path), `get_skill_info` separates metadata from body, `get_skill` remains unchanged; resource enumeration walks (non-recursive, 5-cap per dir, alphabetical via BTreeMap for JSON stability)
- **Phase 5 / US5**: Doctor read-only extensions use only query-level operations; structural enforcement via `open_read_only` with no transaction acquisition
- **Phase 5 Polish**: Single-source-of-truth accessors established: `plugin_data_root()` for process-wide data root; `workspace_data_dir_for()` for workspace-scoped paths; `validate_db_stored_path()` for boundary checks; `build_context_for_entry()` for shared MCP context (eliminates ~50 LOC cross-handler duplication)
- **Phase 6 Foundational**: Harness trait extension follows safe-by-default pattern; all seven new methods have impls that do not alter harness discovery or sync until US1–US3 override them; trait methods never called except during sync + settings composition (wired post-Foundational)
- **Phase 6 / US1**: Agent translation core (`src/harness/agents.rs`) is the harness-agnostic SSOT for parsing, validation, filename provenance, model alias table, read-only inference, and render primitives; per-harness `translate_agent()` overrides call these helpers; sync reconciliation (`reconcile_agents`) computes clash set once per pass and delegates translation dispatch; agent indexing enforces invariants (searchable=false, user_invocable=false, non-searchable queries) at MCP discovery time; forward progress (FR-084) on agent translation failures allows rest of sync to complete
- **Phase 6 / US2**: Hooks rewriting core (`src/harness/hooks.rs`) is the harness-agnostic SSOT for parsing, two-variable rewrite, merge/remove semantics; only Claude Code harness participates (`RealJson` strategy); sync reconciliation (`reconcile_hooks`) enumerates enabled plugins once, rewrites once, dispatches merge/remove per harness (runs BEFORE agents as 3b subsystem); ownership model is re-derivation + structural equality with no provenance marker; forward progress (FR-084) on parse failures allows remaining plugins/harnesses to reconcile; all writes atomic + symlink-refusing, target machine-local `settings.local.json` only

---

*This document describes HOW the system is organized at Phase 6 / US2 COMPLETE (real Claude Code hooks: parsing, two-variable rewrite, merge/remove semantics, sync reconciliation 3b subsystem integrated before agents 3c). Test suites: Phase 5 baseline + entry_kind_agent_indexing, harness_trait_p6, schema_migration_p6, exit_codes + US1: agent_translation, agent_sync_reconciliation, agent_indexing_lifecycle, agent_e2e + US2: hooks_parsing, hooks_merge_remove, hooks_sync_reconciliation.*
