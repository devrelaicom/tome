# Architecture

> **Purpose**: Document system design, patterns, component relationships, and data flow.
> **Generated**: 2026-05-29
> **Last Updated**: 2026-05-31 (Phase 6 US3; guardrails prose fallback: per-plugin marker regions in harness rules files with Claude Code suppression when hooks present; rules-file correction: CLAUDE.md → sink, AGENTS.md dropped; sync reconciliation 3a subsystem)

## Architecture Overview

Tome is a Rust CLI tool and MCP server that manages plugin ecosystems across coding harnesses (Claude Code, Cursor, Gemini CLI, Codex, OpenCode). It provides a centralized index for skill discovery and reranking, multi-workspace support with per-project bindings, harness composition management, workspace-scoped plugin enablement, comprehensive health diagnostics with auto-repair, command indexing and MCP prompts capability, variable substitution engine with four-stage single-pass rendering pipeline, three-tier MCP discovery flow with middle-tier metadata fetching, per-entry invocability flags with read-only doctor extensions, **Phase 6 US1 COMPLETE** native agent translation pipeline, **Phase 6 US2 COMPLETE** real Claude Code hooks: JSON-based hook entries from `<plugin>/hooks/hooks.json` with targeted two-variable rewrite (`${CLAUDE_PLUGIN_ROOT}` / `${CLAUDE_PLUGIN_DATA}`) and deep-equal structural merge/remove into the machine-local `.claude/settings.local.json`, and **Phase 6 US3 COMPLETE** guardrails prose fallback: verbatim `GUARDRAILS.md` body rendered into per-plugin marker regions in each harness's rules-file target with Claude Code suppression (when same plugin ships real JSON hooks) and per-file reconciliation (lex-ordered, overwrite-in-place, orphan removal).

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
- **Sync reconciliation (3b subsystem)** — `reconcile_hooks()` enumerates enabled plugins per-workspace, reads + rewrites each plugin's hooks once, dispatches merge/remove to live/non-live harnesses, records per-file changes in `SyncOutcome` with `SyncSubsystem::Hooks` discriminator; forward progress on parse failures (FR-084); only Claude Code harness participates (`RealJson` strategy); every other harness is `GuardrailsOnly` fallback
- **Atomic write discipline** — All writes to `settings.local.json` use atomic tempfile + rename (symlink-refusing, mode-preserving), target the machine-local gitignored file never the committed `settings.json` (FR-002 contract)
- **Two-variable rewrite mechanism** — Textual replace of exactly two fixed needle tokens (`${CLAUDE_PLUGIN_ROOT}`, `${CLAUDE_PLUGIN_DATA}`) in every JSON string leaf (keys/numbers/booleans/nulls untouched per FR-003), never touches unrecognized `${CLAUDE_*}` tokens (left for Claude Code runtime resolution), fails closed on non-UTF-8 rewrite targets (exit 44, R2-2)

Phase 6 **US3 COMPLETE** — guardrails prose fallback (soft enforcement degradation):
- **Guardrails parsing pipeline** — `read_guardrails_source()` reads a plugin's `<plugin>/hooks/GUARDRAILS.md`, copies body **verbatim** (never parses), validates that body contains no marker-shaped lines (START/END guardrails or `tome:begin/end` block markers, fail-closed per B-1), returns `Ok(None)` when file absent; I/O failure or marker-shaped body → exit 46
- **Marker region reconciliation** — Per-plugin region defined by `<!-- START GUARDRAILS: <catalog>:<plugin> -->` … `<!-- END GUARDRAILS: <catalog>:<plugin> -->` (FR-011/FR-011a); distinct from Phase 4 `tome:begin/end` rules block (both coexist); `<catalog>:<plugin>` is the removal key, state inferred from filesystem markers (no sidecar per FR-015/NFR-004)
- **Per-harness target placement** — `HarnessModule::guardrails_target()` returns `GuardrailsTarget` with placement (in-file region or Cursor standalone sibling) and suppression flag (FR-012); Claude Code suppresses `CLAUDE.md` region when plugin ships real JSON hooks (FR-013, hooks subsystem passes suppression set); target coordinates (in-file candidate set for Claude Code: `CLAUDE.md` > `.claude/CLAUDE.md` per FR-020/021/022 — **Phase 6 correction**, AGENTS.md dropped)
- **Sync reconciliation (3a subsystem)** — `reconcile_guardrails()` enumerates enabled plugins per-workspace, reads + validates each plugin's guardrails once, computes per-file suppression set from hooks (Claude Code only), per-harness dispatch: `compose_in_file()` lex-merges regions (overwrite-in-place, append new, remove orphans, determinism via lex order), atomically writes (symlink-refusing, mode-preserving); per-file granularity + per-harness action in `SyncOutcome` with `SyncSubsystem::Guardrails` discriminator; forward progress on read failure (FR-084)
- **Determinism + idempotence** — Within each target file: `tome:begin/end` block first (Phase 4), then guardrails regions in lexicographic `<catalog>:<plugin>` order (FR-014); existing regions overwritten in place (never duplicated, never reordered); orphaned regions removed; re-sync with no change rewrites nothing (FR-525 byte-for-byte idempotence, NFR-001)

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
| Phase 6 / US3 — Guardrails prose fallback | Guardrails parsing SSOT in `src/harness/guardrails.rs` (verbatim body read, marker-shape validation, per-plugin per-harness target placement with Claude Code hooks suppression); sync reconciliation pass (3a subsystem, runs BEFORE hooks 3b) enumerates enabled plugins once, reads guardrails once, computes suppression set, per-harness `compose_in_file()` lex-merges regions (deterministic lex order, overwrite-in-place, orphan removal); per-file granularity in outcome; atomic writes (symlink-refusing, mode-preserving); forward progress on read failure; **Phase 6 correction**: Claude Code rules-file candidate set is `CLAUDE.md` > `.claude/CLAUDE.md` (AGENTS.md dropped per FR-020/021/022 — Claude Code does not read AGENTS.md natively) |

## Core Components

### Guardrails Soft-Fallback Writer (`src/harness/guardrails.rs`)

- **Purpose**: Phase 6 / US3 — Read plugin's `GUARDRAILS.md` body verbatim, render into per-plugin marker regions in harness rules-file targets (or Cursor sibling), suppress Claude Code region when same plugin ships real JSON hooks
- **Location**: `src/harness/guardrails.rs`
- **Public functions** (the SSOT for guardrails rendering):
  - `read_guardrails_source(plugin_root) → Result<Option<String>>` — Read `<plugin>/hooks/GUARDRAILS.md` verbatim (never parsed); symlink-refused, bounded-read; returns body or `Ok(None)` when absent; body validated for marker-shaped lines (fail-closed per B-1); read failure/marker-shaped body → exit 46
  - `region_key(catalog, plugin) → String` — Render provenance key `<catalog>:<plugin>` (SSOT for the removal identifier)
  - `begin_marker(key)` / `end_marker(key)` — Render canonical START/END marker lines (compiled once via `OnceLock`)
- **Types**: None new; uses `MarkerRegion` / `MarkerSpec` from `rules_file` module
- **Marker structure** (FR-011/FR-011a):
  - START: `<!-- START GUARDRAILS: <catalog>:<plugin> -->`
  - END: `<!-- END GUARDRAILS: <catalog>:<plugin> -->`
  - Body: verbatim plugin-supplied text (never parsed, never escaped)
  - Distinct from Phase 4 `tome:begin/end` rules block (both coexist in same file)
- **Suppression model** (FR-013): Claude Code is the ONLY harness that suppresses a plugin's guardrails region when that plugin ships real JSON hooks (computed by sync orchestrator from hooks set, passed as per-file filter)

### Rules-File Integration — Generalized Marker Engine (`src/harness/rules_file.rs`)

- **Purpose**: Parameterised block-in-file + standalone-file writer, extended to handle multiple marker families (Phase 4 `tome:begin/end` + Phase 6 guardrails)
- **Location**: `src/harness/rules_file.rs`
- **New abstractions**:
  - `MarkerSpec` — Parameterised regex pair (START / END) + render functions; compiled once
  - `MarkerRegion` — Parsed marker pair with line numbers + body
  - `find_marker_regions(body, spec) → Vec<MarkerRegion>` — Scan for all marker-delimited regions
  - `compose_in_file(target_path, regions_to_write, spec, style) → Result<bool>` — Lex-merge regions (deterministic lex order within marker family), overwrite-in-place, append new, remove orphans, atomic write; returns `true` on change
- **Public (crate-level) promotions**:
  - `refuse_symlink(path)` — Security hardening: reject symlinks at target paths
  - `atomic_write(path, content) → Result<()>` — Atomic tempfile + rename (symlink-refusing, mode-preserving)
- **Determinism + Idempotence** (FR-014, FR-525, NFR-001):
  - Within a target file: `tome:begin/end` block (Phase 4) first, then guardrails regions in lex-order
  - Existing regions overwritten in place (never duplicated, never reordered)
  - Orphaned regions removed
  - Re-sync with no change rewrites nothing

### Sync Reconciliation — Guardrails Subsystem (3a) (`src/harness/sync.rs`)

- **Purpose**: Phase 6 / US3 — Orchestrate guardrails soft-fallback rendering across all harnesses; runs BEFORE hooks (3b) in sink order
- **Location**: `src/harness/sync.rs` (`reconcile_guardrails()` + helpers)
- **Algorithm**:
  1. **Fast exit**: If no harness targets a guardrails file, return (no guardrails reconciliation)
  2. **DB enumeration**: Open central DB read-only; enumerate enabled plugins for workspace
  3. **Guardrails parsing**: For each enabled plugin, resolve root dir, read + validate source once (forward progress on read failure per FR-084); plugins with no `hooks/GUARDRAILS.md` contribute no region
  4. **Hooks suppression set**: Query hooks subsystem for enabled plugins with real JSON hooks (Claude Code only); compute per-file suppression filter (Claude Code `CLAUDE.md` only)
  5. **Per-harness dispatch**: For each harness with a guardrails target:
     - If **live** (in effective list): `compose_in_file()` lex-merges collected regions (excluding suppressed) into target (deterministic lex order, overwrite-in-place, orphan removal)
     - If **non-live** (not in effective list): `compose_in_file()` removes all guardrails regions (cleanup)
  6. **Result tracking**: Record per-file granularity in `SyncOutcome` (added/updated/removed) with `SyncSubsystem::Guardrails` discriminator; aggregate per-harness action in `HarnessDecision::guardrails_action`
- **Data structures**:
  - `HarnessSnapshot::guardrails_target` — `Some(GuardrailsTarget)` with placement + suppression flag per harness
  - `GuardrailsTarget` — Placement (in-file region or Cursor standalone sibling) + `suppress_if_hooks_present` flag (Claude Code only)
  - `GuardrailsPlacement` enum — `InFileRegion { file }` or `CursorStandaloneSibling { dir }`
  - `SyncOutcome::added/updated/removed` — Records per-file changes with `SyncSubsystem::Guardrails`
  - `HarnessDecision::guardrails_action` — Aggregate action per harness (Created/Updated/Removed/LeftAlone)
- **Forward progress** (FR-084): Read failures on guardrails source recorded as first error; plugin is skipped; sibling plugins still reconcile; surface error after all harnesses processed
- **Cursor standalone handling** (R-5, R-6): Cursor's guardrails land in `.cursor/rules/TOME_GUARDRAILS.md` (standalone sibling, not block-in-file); `reconcile_guardrails()` dispatches to per-harness `compose_in_file()` which routes to appropriate writer (block vs standalone)

### Claude Code Harness Extensions (`src/harness/claude_code.rs`)

- **Purpose**: Phase 6 / US2–US3 — Implement real-hooks strategy, hook-settings path, and guardrails-target placement for Claude Code
- **Location**: `src/harness/claude_code.rs`
- **Trait method overrides**:
  - `hooks_strategy() → HooksStrategy` — Returns `HooksStrategy::RealJson` (only Claude Code)
  - `hook_settings_path(project_root) → Option<PathBuf>` — Returns `<project>/.claude/settings.local.json` (machine-local, gitignored)
  - `guardrails_target(project_root) → GuardrailsTarget` — Returns in-file placement in `CLAUDE.md` (the corrected rules-file candidate); suppresses region when plugin ships real JSON hooks (FR-013)
- **Rules-file correction** (FR-020/021/022): Candidate set is `CLAUDE.md` > `.claude/CLAUDE.md` (AGENTS.md DROPPED — Claude Code does not natively read `AGENTS.md`); first existing candidate wins; fall back to `CLAUDE.md` on first write
- **Contract**: Real hooks carry machine-specific absolute paths (from two-variable rewrite), land in `settings.local.json` only (never committed `settings.json`)

### Other Harness Modules (Codex, Cursor, Gemini, OpenCode)

- **Purpose**: Implement per-harness guardrails placement (all support; Cursor has standalone sibling)
- **Locations**: `src/harness/{codex,cursor,gemini,opencode}.rs`
- **Trait method overrides**:
  - `guardrails_target(project_root) → GuardrailsTarget` — Each harness returns its placement (in-file region + no suppression, except Claude Code)
  - Codex / Gemini / OpenCode: in-file regions in shared `AGENTS.md` (not `CLAUDE.md` — correction is Claude Code only)
  - Cursor: in-file region in `.cursor/AGENTS.md` (primary placement per R-5) OR standalone `.cursor/rules/TOME_GUARDRAILS.md` (sibling, used in US3)

## Real Claude Code Hooks (`src/harness/hooks.rs`)

- **Purpose**: Phase 6 / US2 — Read a plugin's `hooks/hooks.json`, rewrite two path variables, merge/remove entries into `.claude/settings.local.json` (only Claude Code harness)
- **Location**: `src/harness/hooks.rs`
- **Public functions** (the SSOT for hooks rewriting):
  - `read_rewritten_entries(plugin_root, plugin_data) → Result<Option<RewrittenHooks>>` — Read and rewrite a plugin's `<plugin>/hooks/hooks.json`; validates top-level object shape (event-keyed arrays); applies targeted two-variable rewrite to every JSON string leaf; returns `RewrittenHooks` or `Ok(None)` when absent; malformed/unreadable → exit 43
  - `merge_into_settings(target, hooks) → Result<bool>` — Merge rewritten hooks into `<project>/.claude/settings.local.json`, appending each entry under its event only when no deep-equal entry exists (idempotent per FR-004); creates `settings.local.json` with `{"hooks": {}}` when absent (FR-002); atomic, mode-preserving, symlink-refusing; returns `true` on change, `false` on no-op; any failure → exit 44
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

## Layer Boundaries

| Layer | Responsibility | Can Access | Cannot Access |
|-------|----------------|------------|---------------|
| CLI | Argument parsing, mode dispatch, error formatting | Commands | Database, embedder directly |
| Commands | Command logic, outcome assembly, emit wrappers | Business logic (workspace, plugin, harness, settings, summarise, doctor, substitution) | Database directly (via deps) |
| Business logic | Policy (binding, lifecycle, sync, substitution, diagnostics, harness trait dispatch, agent translation, hooks rewriting, guardrails rendering) | Index, catalog, plugin, settings, embedding, summarise, substitution, harness | CLI, presentation |
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
- **Phase 6 / US2**: Hooks rewriting core (`src/harness/hooks.rs`) is the harness-agnostic SSOT for parsing, two-variable rewrite, merge/remove semantics; only Claude Code harness participates (`RealJson` strategy); sync reconciliation (`reconcile_hooks`) enumerates enabled plugins once, rewrites once, dispatches merge/remove per harness (runs BEFORE guardrails 3a & agents 3c as 3b subsystem); ownership model is re-derivation + structural equality with no provenance marker; forward progress (FR-084) on parse failures allows remaining plugins/harnesses to reconcile; all writes atomic + symlink-refusing, target machine-local `settings.local.json` only
- **Phase 6 / US3**: Guardrails rendering core (`src/harness/guardrails.rs`) is the harness-agnostic SSOT for parsing (verbatim body, marker validation), per-harness target placement; `rules_file::compose_in_file` is the shared SSOT for region reconciliation (lex-ordered deterministic merge, overwrite-in-place, orphan removal); sync reconciliation (`reconcile_guardrails`) enumerates enabled plugins once, reads guardrails once, computes suppression set (Claude Code hooks only), dispatches per-harness region composition (runs BEFORE hooks 3b & agents 3c as 3a subsystem); forward progress (FR-084) on read failures allows remaining plugins/harnesses to reconcile; all writes atomic + symlink-refusing; Claude Code rules-file candidate set corrected to `CLAUDE.md` > `.claude/CLAUDE.md` (AGENTS.md dropped per FR-020/021/022)

---

*This document describes HOW the system is organized at Phase 6 / US3 COMPLETE (guardrails prose soft-fallback: per-plugin marker regions in rules files with Claude Code suppression when hooks present; sync reconciliation 3a subsystem before hooks 3b before agents 3c; Phase 6 correction: Claude Code rules-file sink CLAUDE.md not AGENTS.md). Test suites: Phase 5 baseline + entry_kind_agent_indexing, harness_trait_p6, schema_migration_p6, exit_codes + US1: agent_translation, agent_sync_reconciliation, agent_indexing_lifecycle, agent_e2e + US2: hooks_parsing, hooks_merge_remove, hooks_sync_reconciliation + US3: guardrails_parsing, guardrails_reconciliation, guardrails_marker_engine, phase6_correction_claude_code.*
