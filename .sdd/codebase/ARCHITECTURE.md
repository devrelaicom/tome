# Architecture

> **Purpose**: Document system design, patterns, component relationships, and data flow.
> **Generated**: 2026-05-29
> **Last Updated**: 2026-06-04 (Phase 6 US4; agent personas via MCP prompts: persona-role path in prompts/get wrapping agent body in role-assumption template; scalar flag resolver for expose_agents_as_personas across project/workspace/global scopes; startup-scope flag resolution in MCP server preflight; personas appended to Phase 5 prompt registry in collision namespace; settings scopes loader SSOT)

## Architecture Overview

Tome is a Rust CLI tool and MCP server that manages plugin ecosystems across coding harnesses (Claude Code, Cursor, Gemini CLI, Codex, OpenCode). It provides a centralized index for skill discovery and reranking, multi-workspace support with per-project bindings, harness composition management, workspace-scoped plugin enablement, comprehensive health diagnostics with auto-repair, command indexing and MCP prompts capability, variable substitution engine with four-stage single-pass rendering pipeline, three-tier MCP discovery flow with middle-tier metadata fetching, per-entry invocability flags with read-only doctor extensions, **Phase 6 US1 COMPLETE** native agent translation pipeline, **Phase 6 US2 COMPLETE** real Claude Code hooks: JSON-based hook entries from `<plugin>/hooks/hooks.json` with targeted two-variable rewrite (`${CLAUDE_PLUGIN_ROOT}` / `${CLAUDE_PLUGIN_DATA}`) and deep-equal structural merge/remove into the machine-local `.claude/settings.local.json`, **Phase 6 US3 COMPLETE** guardrails prose fallback: verbatim `GUARDRAILS.md` body rendered into per-plugin marker regions in each harness's rules-file target with Claude Code suppression (when same plugin ships real JSON hooks) and per-file reconciliation (lex-ordered, overwrite-in-place, orphan removal), and **Phase 6 US4 COMPLETE** agent personas via MCP prompts: parallel persona-role path in `prompts/get` wrapping agent body in role-assumption template, scalar flag resolver for `expose_agents_as_personas` across project/workspace/global scopes with startup-scope resolution, one `<name>-persona` prompt per enabled agent plus reserved global `drop-persona` in unified Phase 5 collision namespace, project_version + indexed_at caching for persona agents.

The architecture is **monolithic with layered structure** split across two execution contexts:
- **CLI layer** — sync command dispatcher + lifecycle orchestrator
- **MCP layer** — async stdio server (Phase 3+)

The central nervous system is a **single SQLite database** (`<home>/.tome/index.db`) that centralizes all state: plugin metadata, embeddings, workspace bindings, project bindings, enabled entries (skills/commands/agents), and diagnostic metadata. Per-workspace composition settings and summaries live in separate TOML files (`<root>/workspaces/<name>/settings.toml`) and central RULES.md. Project markers (`<project>/.tome/config.toml`) are thin binding pointers, not databases.

Phase 6 **US1 COMPLETE** — native agent translation pipeline:
- **Agent parsing pipeline** — `CanonicalAgent::parse` consumes `<plugin>/agents/<name>.md` (YAML frontmatter + Markdown body), validates agent name (`is_safe_agent_name` single-segment gate), deserializes CloudCode canonical vocabulary (name, description, model, tools/disallowed_tools, privileged hooks/mcp_servers/permission_mode)
- **Per-harness translation** — `HarnessModule::translate_agent()` overrides consumed by five harness impls; `map_model()` enforces same-vendor-only model alias table (FR-034/037); `infer_read_only()` detects read-only intent from tool posture (FR-036); `displayed_name()` handles clash-prefixed naming (FR-041); `render_markdown_yaml()` / `render_codex_toml()` emit harness-native file formats
- **Sync reconciliation (3c subsystem)** — `reconcile_agents()` enumerates enabled agents per-workspace, computes clash set once (FR-072), parses canonicals, dispatches per-harness translation, atomically writes `<plugin>__<name>.<ext>` (symlink-refusing, mode-preserving), removes orphaned `<plugin>__*` for non-live/non-supporting harnesses; forward progress on failure (FR-084); new `SyncSubsystem::Agents` discriminator in outcome
- **Agent indexing** — `list_agent_files()` walks `agents/*.md`; `collect_agent_entries()` parses frontmatter + validates names; agent rows inserted with `kind='agent'`, `searchable=false`, `user_invocable=false` per FR-070a; `agent_name_clash_set()` / `enabled_agents_for_workspace()` queries support sync reconciliation; agents project `plugin_version` + `indexed_at` in enabled-agent enumeration
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

Phase 6 **US4 COMPLETE** — agent personas via MCP prompts (advisory conversational context):
- **Persona exposure flag** — `expose_agents_as_personas: Option<bool>` on three settings layers (project marker / workspace settings / global settings) per FR-060/FR-067; first-declarer-wins scalar resolver (`resolve_scalar()` / `resolve_scalar_with()`) walks project → workspace → global, stops at first scope that declares the field (FR-053, R-12)
- **Startup-scope resolution** — `resolve_expose_personas()` called once at MCP server startup after scope resolution, reads (project, workspace, global) settings via canonical scope loaders (`scopes::{load_project_marker, load_workspace_settings, load_global_settings}`), applies scalar resolver to determine flag for the MCP session; promotes R4-2 scope-loader copies to `src/settings/scopes.rs` SSOT to eliminate drift hazard (each loader was verbatim-duplicated in three sites)
- **Persona-role path in prompts/get** — New `PersonaRole` enum on `PromptEntry` discriminates Phase 5 command/skill (`None`) from Phase 6 persona variants (`Agent` / `Drop`); persona path bypasses Phase 5 body-path rendering, instead frontmatter-strips agent body and wraps it in role-assumption template (fixed verbatim per contract § Persona prompt body), reusing Phase 5 substitution pipeline (NFR-007, no parallel substitution path); `drop-persona` fixed body (no on-disk file)
- **Persona registry appending** — `build_for_workspace()` first builds Phase 5 skills/commands identities, then calls `collect_persona_identities()` to append one `EntryIdentity` per enabled agent plus `drop-persona`, folding all into SINGLE Phase 5 collision namespace (FR-066); clash prefix applied before collision pass (`<plugin>-<name>-persona` for clashing agents, `<name>-persona` otherwise, per FR-061 + derived via `derive_suffixed_name()` to preserve `-persona` suffix under length caps); `drop-persona` reserved (empty catalog/plugin + empty indexed_at seed guarantees it sorts first in any collision bucket, FR-063)
- **Persona entry caching** — Agent personas carry `display_name` (frontmatter `name`, else filename stem, resolved before stripping) and `plugin_version` (from DB column for `${TOME_PLUGIN_VERSION}` substitution in persona body, C4-1); `indexed_at` carries agent's real timestamp for tie-breaking alongside command/skill entries (R4-1, FR-062); persona-drop reserves empty indexed_at only
- **Persona invocability** — Agents remain `user_invocable=0` in index; personas are surfaced via `PersonaRole` discriminator at `prompts/get` dispatch time, not index query time (FR-064); persona arguments always Case B (catch-all `args`, optional, as persona template always carries `$ARGUMENTS`)

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
| Phase 6 / US4 — Agent personas | Scalar flag resolver `resolve_scalar()` applies first-declarer-wins walk over (project, workspace, global) scope layers (FR-053, R-12); startup-scope resolution via canonical scope-loaders SSOT (`scopes` module); persona identities appended to Phase 5 registry before collision pass, folding into unified namespace (FR-066); persona-role path in `prompts/get` wraps agent body (frontmatter-stripped) in role-assumption template (fixed), applying Phase 5 substitution (NFR-007); `drop-persona` reserved via empty-indexed-at seed; agent personas always Case B arguments (catch-all `args`, optional) |

## Core Components

### Settings Scopes Loaders (`src/settings/scopes.rs`)

- **Purpose**: Phase 6 / US4 (R4-2) — Canonical project-marker / workspace-settings / global-settings loaders (promoted SSOT), consumed by MCP startup-scope resolver, harness list command, and harness sync
- **Location**: `src/settings/scopes.rs` (NEW module inside existing `src/settings/`)
- **Public functions**:
  - `load_project_marker(project_root: Option<&Path>) → Result<Option<ProjectMarkerConfig>>` — Read `<project>/.tome/config.toml`; `Ok(None)` when marker absent; parse failure → exit 70; IO failure → exit 7
  - `load_workspace_settings(paths, workspace_name) → Result<Option<WorkspaceSettings>>` — Read `<root>/workspaces/<name>/settings.toml`; `Ok(None)` when file absent; parse failure → exit 70
  - `load_global_settings(paths) → Result<GlobalSettings>` — Read `<root>/settings.toml`; absent file → `GlobalSettings::default()`; parse failure → exit 70
- **Contract**: Three copies of these loaders existed verbatim in `commands/harness/list`, `harness/sync`, and `mcp::resolve_expose_personas`, a textbook drift hazard; consolidation eliminates drift via single SSOT

### Scalar Settings Resolver (`src/settings/mod.rs`)

- **Purpose**: Phase 6 (FR-053, R-12) — First-declarer-wins priority walk for boolean settings (e.g., `expose_agents_as_personas`)
- **Public functions**:
  - `resolve_scalar(project: Option<bool>, workspace: Option<bool>, global: Option<bool>) → bool` — Walk project → workspace → global, return first `Some(v)`; default `false` when all `None`
  - `resolve_scalar_with<FP, FW, FG>(project, workspace, global, project_field, workspace_field, global_field) → bool` — Closure-based form; extractors parameterise the field being resolved (reusable for `expose_agents_as_personas` and US5's `strip_plugin_agent_privileges`)
- **Contract**: Deliberately NOT the `harnesses` composition grammar; a project `false` simply overrides global `true`; no list union/subtraction or composition references
- **Settings structures updated**:
  - `ProjectMarkerConfig::expose_agents_as_personas: Option<bool>` — Phase 6 / US4
  - `WorkspaceSettings::expose_agents_as_personas: Option<bool>` — Phase 6 / US4
  - `GlobalSettings::expose_agents_as_personas: Option<bool>` — Phase 6 / US4

### Agent Personas in MCP Prompts (`src/mcp/prompts.rs`)

- **Purpose**: Phase 6 / US4 — Expose enabled agents as advisory-context persona prompts in unified Phase 5 collision namespace
- **Location**: `src/mcp/prompts.rs`
- **Types**:
  - `PersonaRole` enum — `None` (Phase 5 command/skill), `Agent` (persona from enabled agent), `Drop` (reserved drop-persona)
- **PromptEntry fields (Phase 6 additions)**:
  - `persona: PersonaRole` — Discriminator for routing in `prompts/get` dispatch
  - `display_name: String` — Agent's display name (frontmatter `name`, else filename stem); used in persona template; empty for non-persona entries
- **Registry building**:
  - `build_for_workspace()` first queries enabled user-invocable entries (skills/commands only per FR-070a)
  - When `expose_personas=true`, `collect_persona_identities()` appends one `EntryIdentity` per enabled agent + `drop-persona` to identities/hydrated maps
  - All identities pass through SINGLE Phase 5 collision pass (no separate persona collision namespace)
  - Name derivation: `<name>-persona` normally, `<plugin>-<name>-persona` for clashing agents (clash set from DB), `drop-persona` reserved
  - `drop-persona` seeded with empty catalog/plugin/indexed_at → sorts first in collision buckets, guaranteeing reservation
- **Prompts/get dispatch**:
  - `PersonaRole::None` → existing Phase 5 path (read entry body file)
  - `PersonaRole::Agent` → new persona path: parse agent frontmatter, strip YAML, wrap body in role-assumption template (fixed, per contract), apply Phase 5 substitution (NFR-007, no parallel path)
  - `PersonaRole::Drop` → fixed body (no file read)
- **Agent entry caching** (US1 queries extended in US4):
  - `enabled_agents_for_workspace()` returns `Vec<EnabledAgent>` now projecting `plugin_version` + `indexed_at` (used in persona template substitution + collision tie-break)

### MCP Server Startup Scope Resolution (`src/mcp/mod.rs`)

- **Purpose**: Phase 6 / US4 (FR-067) — Resolve `expose_agents_as_personas` once at server startup against the MCP server's single scope
- **Location**: `src/mcp/mod.rs`
- **Public function**:
  - `resolve_expose_personas(scope: &ResolvedScope, paths: &Paths) → Result<bool>` — Load (project, workspace, global) settings via canonical scope loaders, apply scalar resolver; called once per server session after scope resolution; result passed to `PromptRegistry::build_for_workspace()`
- **Contract**: Scope is fixed for the server's lifetime (no per-request scope switching); flag controls whether persona identities are appended to registry

## Persona Template & Formatting

- **Role-assumption template** (fixed, per contract):
  ```
  Assume the following {display_name} persona until instructed otherwise.
  
  <{persona_name}>
  {body}
  </{persona_name}>
  
  While acting as the {display_name} persona, you must: $ARGUMENTS
  ```
  Where `display_name` is the agent's frontmatter `name` (or filename stem), `persona_name` is the derived slug (`<name>-persona` or `<plugin>-<name>-persona`), and `body` is the agent's frontmatter-stripped Markdown body.

- **Persona description** (auto-generated): `"Assume the \`{display_name}\` agent persona (advisory conversational context, not enforced configuration — the agent may drift or ignore it; not the isolation a native subagent provides)."`

- **Drop-persona body** (fixed): `"Stop acting as any assumed persona and return to your default behaviour and personality."`

- **Drop-persona description** (fixed): `"Stop acting as any assumed agent persona and return to default behaviour."`

## Layer Boundaries

| Layer | Responsibility | Can Access | Cannot Access |
|-------|----------------|------------|---------------|
| CLI | Argument parsing, mode dispatch, error formatting | Commands | Database, embedder directly |
| Commands | Command logic, outcome assembly, emit wrappers | Business logic (workspace, plugin, harness, settings, summarise, doctor, substitution) | Database directly (via deps) |
| Business logic | Policy (binding, lifecycle, sync, substitution, diagnostics, harness trait dispatch, agent translation, hooks rewriting, guardrails rendering, persona exposure) | Index, catalog, plugin, settings, embedding, summarise, substitution, harness | CLI, presentation |
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
- **Phase 6 / US4**: Scalar settings resolver (`resolve_scalar()` / `resolve_scalar_with()`) is the SSOT for first-declarer-wins boolean settings walks; scope loaders (`scopes` module) are the SSOT for reading (project, workspace, global) settings files with unified error classification; MCP startup calls `resolve_expose_personas()` once to determine persona registry path for the session; persona identities appended in `build_for_workspace()` after Phase 5 command/skill collection, folding into unified collision namespace; persona-role dispatch in `prompts/get` wraps agent body in fixed template, applying Phase 5 substitution only (no parallel substitution path per NFR-007)

---

*This document describes HOW the system is organized at Phase 6 / US4 COMPLETE (agent personas via MCP prompts: persona-role path in prompts/get wrapping agent body in role-assumption template; scalar flag resolver for expose_agents_as_personas across project/workspace/global scopes; startup-scope flag resolution in MCP server preflight; personas appended to Phase 5 prompt registry in collision namespace; settings scopes loader SSOT). Test suites: Phase 5 baseline + entry_kind_agent_indexing, harness_trait_p6, schema_migration_p6, exit_codes + US1: agent_translation, agent_sync_reconciliation, agent_indexing_lifecycle, agent_e2e + US2: hooks_parsing, hooks_merge_remove, hooks_sync_reconciliation + US3: guardrails_parsing, guardrails_reconciliation, guardrails_marker_engine, phase6_correction_claude_code.* + US4: personas_startup_scope, personas_registry_building, personas_prompts_get, personas_scalar_resolver*
