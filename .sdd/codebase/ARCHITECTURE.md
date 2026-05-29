# Architecture

> **Purpose**: Document system design, patterns, component relationships, and data flow.
> **Generated**: 2026-05-29
> **Last Updated**: 2026-05-29 (Phase 6 Foundational; harness trait extended with hooks + guardrails + agent support; EntryKind widened with Agent variant; 4 new exit codes 43–46)

## Architecture Overview

Tome is a Rust CLI tool and MCP server that manages plugin ecosystems across coding harnesses (Claude Code, Cursor, Gemini CLI, Codex, OpenCode). It provides a centralized index for skill discovery and reranking, multi-workspace support with per-project bindings, harness composition management, workspace-scoped plugin enablement, comprehensive health diagnostics with auto-repair, command indexing and MCP prompts capability, variable substitution engine with four-stage single-pass rendering pipeline, three-tier MCP discovery flow with middle-tier metadata fetching, per-entry invocability flags with read-only doctor extensions, and **Phase 6 Foundational WIRED** harness trait extensions for hooks reconciliation strategies, guardrails prose fallbacks, and native agent translation across harnesses.

The architecture is **monolithic with layered structure** split across two execution contexts:
- **CLI layer** — sync command dispatcher
- **MCP layer** — async stdio server (Phase 3+)

The central nervous system is a **single SQLite database** (`<home>/.tome/index.db`) that centralizes all state: plugin metadata, embeddings, workspace bindings, project bindings, enabled entries (skills/commands/agents), and diagnostic metadata. Per-workspace composition settings and summaries live in separate TOML files (`<root>/workspaces/<name>/settings.toml`) and central RULES.md. Project markers (`<project>/.tome/config.toml`) are thin binding pointers, not databases.

Phase 6 **Foundational WIRED** — infrastructure for hooks + agents complete:
- **New shape enums** — `HooksStrategy` (RealJson vs GuardrailsOnly), `GuardrailsPlacement` (InFileRegion vs StandaloneSibling), `GuardrailsTarget`, `AgentFormat` (MarkdownYaml vs Toml)
- **Harness trait methods (7 new, all safe-by-default)** — `hooks_strategy()`, `hook_settings_path()`, `guardrails_target()`, `supports_native_agents()`, `agent_dir()`, `agent_format()`, `translate_agent()`
- **Agent types** — `CanonicalAgent` (source form from `<plugin>/agents/<name>.md`) + `TranslatedAgent` (per-harness emission; skeleton for US1 T034 parsing + translation rules)
- **EntryKind widened** — Phase 5's Skill | Command now plus Agent (always non-searchable, never user-invocable per FR-070a)
- **4 new exit codes** — 43–46 (HookSpecParseError, HookSettingsWriteFailed, AgentTranslationFailed, GuardrailsWriteFailed) in the first free contiguous run after Phase 5's 25–29 cluster
- **Schema v3→v4 marker** — forward-only migration in `index::migrations.rs` that advances SCHEMA_VERSION (kind column admits 'agent' without DDL; backfill deferred to US1)

**Foundational discipline**: Every exhaustive match over `EntryKind` was widened to handle Agent; no catch-all re-hides the schema drift that the canonical-enum-dispatch defence-in-depth guards against. Harness trait methods use all-safe defaults; production overrides land in US1–US3.

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
| Phase 6 Foundational — Harness trait extension | Seven new trait methods (safe-by-default impls) standardize hooks reconciliation, guardrails prose placement, and agent translation across five harnesses; trait methods never called until US1 behaviour lands |

## Core Components

### Harness Trait Extension (`src/harness/mod.rs`, `src/harness/agents.rs`)

- **Purpose**: Phase 6 Foundational — Standardize hooks, guardrails, and native agent support across harnesses
- **Location**: `src/harness/mod.rs` (trait + shape enums), `src/harness/agents.rs` (agent types, skeleton)
- **New shape enums**:
  - `HooksStrategy` — RealJson (merge to `.claude/settings.local.json`) vs GuardrailsOnly (prose fallback)
  - `GuardrailsPlacement` — InFileRegion (marker-delimited block in existing file) vs StandaloneSibling (standalone file)
  - `GuardrailsTarget` — placement + suppress_if_hooks_present flag
  - `AgentFormat` — MarkdownYaml vs Toml (per-harness serialisation)
- **New trait methods** (all with safe-by-default impls; no production override yet):
  - `hooks_strategy() -> HooksStrategy` (default: GuardrailsOnly)
  - `hook_settings_path(project_root: &Path) -> Option<PathBuf>` (default: None)
  - `guardrails_target(project_root: &Path) -> GuardrailsTarget` (default: InFileRegion at rules_file_target)
  - `supports_native_agents() -> bool` (default: false)
  - `agent_dir(project_root: &Path) -> Option<PathBuf>` (default: None)
  - `agent_format() -> Option<AgentFormat>` (default: None)
  - `translate_agent(canonical: &CanonicalAgent) -> TranslatedAgent` (default: unreachable panic if called without override)
- **Agent types** (`src/harness/agents.rs`, skeleton):
  - `CanonicalAgent` — Plugin source agent from `<plugin>/agents/<name>.md`; carries name, description, body (system prompt), model, tools/disallowed_tools, privileged fields (hooks, mcp_servers, permission_mode; opaque serde_json::Value per FR-050)
  - `TranslatedAgent` — Per-harness emission result; dir, filename, displayed_name, format, rendered content, dropped_fields list (FR-032/FR-034/FR-036 diagnostics)
- **Parsing + translation behaviour**: Skeleton only; parsing `agents/*.md` into CanonicalAgent, per-harness translation rules, model alias table, and clash-set machinery are US1 (T034) work

### Entry Kind Discriminator Extended (`src/plugin/identity.rs`)

- **Purpose**: Phase 6 Foundational — Widen unified entry kind to include agents
- **Location**: `src/plugin/identity.rs`
- **Type**: `#[serde(rename_all = "lowercase")] pub enum EntryKind { Skill, Command, Agent }`
- **New variant**: `Agent` (parsed from `agents/*.md` during US1 indexing)
- **Invariants per FR-070a**:
  - Agent rows are always `searchable: false` (never indexed for semantic search)
  - Agent rows are always `user_invocable: false` (never exposed as MCP prompts)
  - Every exhaustive match over EntryKind was widened in lockstep across `commands/plugin/{mod,show,list}.rs`, `doctor/{checks,report}.rs`, `plugin/frontmatter.rs` — no catch-all re-hides schema drift
- **Canonical `from_str()` dispatch**: Consumed at six sites across Phase 5–6 (defence-in-depth pattern)

### Error Variants & Exit Codes (`src/error.rs`)

- **Purpose**: Phase 6 Foundational — Introduce 4 new exit codes (43–46) for hooks + agents
- **Location**: `src/error.rs`
- **New variants**:
  - `HookSpecParseError { path: PathBuf }` — exit 43; malformed or unparsable `hooks.json`
  - `HookSettingsWriteFailed { path: PathBuf, source: io::Error }` — exit 44; write failure to `.claude/settings.local.json` during hook merge
  - `AgentTranslationFailed { agent: String }` — exit 45; agent translation failed (T034 behaviour, currently unreachable in Foundational)
  - `GuardrailsWriteFailed { path: PathBuf }` — exit 46; guardrails prose write failure
- **Exit code cluster**: 43–46 chosen as the first free contiguous run (per constitution precedent: Phase 4 summariser moved from proposed 20 → shipped 24; Phase 5 cluster proposed 21–23 → shipped 25–29)

### Schema Migration v3→v4 Marker (`src/index/migrations.rs`, `src/index/schema.rs`)

- **Purpose**: Phase 6 Foundational — Advance schema version without DDL changes (backwards-compatible, content-only)
- **Location**: `src/index/migrations.rs`, `src/index/schema.rs`
- **Migration strategy**:
  - No new columns (kind column admits 'agent' without DDL alteration)
  - No backfill (agent rows inserted by US1 indexing, not migration)
  - `apply()` method only advances `SCHEMA_VERSION` constant (v3→v4)
  - Forward-only per constitution discipline
- **Rationale**: Agent discovery and indexing deferred to US1 (T034); Foundational wires type definitions + trait extension only

### Plugin Frontmatter Extension (`src/plugin/frontmatter.rs`)

- **Purpose**: Phase 6 Foundational — Reserve agent frontmatter fields (parsing deferred to US1)
- **Location**: `src/plugin/frontmatter.rs`
- **New fields reserved** (parsing wired in US1):
  - `name: Option<String>` (agent-specific; overrides filename stem)
  - `description: Option<String>` (agent-specific)
  - `model: Option<String>` (agent-specific; canonical form; per-harness translation via alias table in US1)
  - `tools: Option<Vec<String>>` (agent-specific; allowed tools list)
  - `disallowed_tools: Option<Vec<String>>` (agent-specific; disallowed tools list)
  - `hooks: Option<serde_json::Value>` (privileged, Claude Code only, opaque per FR-050)
  - `mcp_servers: Option<serde_json::Value>` (privileged, Claude Code only, opaque per FR-050)
  - `permission_mode: Option<String>` (privileged, Claude Code only, opaque per FR-050)
- **Parsing behaviour**: All lenient (per Phase 2+ convention for third-party inputs); invalid agent frontmatter is logged but does not fail the enable pipeline until US1 validation

### Doctor Report Extension (`src/doctor/checks.rs`, `src/doctor/report.rs`)

- **Purpose**: Phase 6 Foundational — Reserve agent diagnostic reporting (checks wired in US5)
- **Location**: `src/doctor/{checks,report}.rs`
- **New report fields reserved** (checks wired in US5 for agent/hook/guardrails diagnostics):
  - Entry-kind breakdown extended to include agent count (alongside skill/command counts)
  - Hook spec validation hints (US1 parse errors)
  - Guardrails prose placement diagnostics (US2 write failures)
  - Agent translation diagnostics (US3 T034 failures)
- **Foundational discipline**: No agent-specific checks wired yet; the docstrings + struct shapes reserve room for US5

### Plugin List & Show Enhancement (`src/commands/plugin/{list,show}.rs`)

- **Purpose**: Phase 6 Foundational — Display agent entries alongside skills/commands
- **Location**: `src/commands/plugin/{list,show}.rs`
- **Changes**:
  - `list.rs` — Extended output format to include agent count: `plugin: <name> (N skills, M commands, P agents)`
  - `show.rs` — New Agents section (grouped by kind, same pattern as Skills and Commands); per-agent annotations (`[searchable=false]`, `[user_invocable=false]`, `[dormant]` when disabled)
  - Queries expanded to fetch all three kinds (exhaustive per canonical-enum dispatch)

## Layer Boundaries

| Layer | Responsibility | Can Access | Cannot Access |
|-------|----------------|------------|---------------|
| CLI | Argument parsing, mode dispatch, error formatting | Commands | Database, embedder directly |
| Commands | Command logic, outcome assembly, emit wrappers | Business logic (workspace, plugin, harness, settings, summarise, doctor, substitution) | Database directly (via deps) |
| Business logic | Policy (binding, lifecycle, sync, substitution, substitution, diagnostics, harness trait dispatch) | Index, catalog, plugin, settings, embedding, summarise, substitution, harness | CLI, presentation |
| Data access | Queries, writes, transactions | Index, config, catalog on-disk | Commands, business logic |
| Persistence | SQLite, filesystem, git | Raw operations | Higher layers |

## Dependency Rules

- Higher layers can depend on lower layers, not vice versa
- Trait boundaries (`Embedder`, `Reranker`, `Summariser`, `HarnessModule`, `ScopeProvider`) decouple policy from mechanism
- `src/mcp/` is the only module allowed async (`tokio`); enforced by `tests/sync_boundary.rs`
- `src/substitution/` is sync-only; variable rendering is pure compute (lazy data-dir creation is the only I/O side effect)
- Workspace-specific code never reads/writes global index directly; uses scope-parameterized helpers
- Substitution engine allows test injection via `SUBSTITUTION_OVERRIDE` thread_local (mirrors `MIGRATIONS_OVERRIDE` / `SUMMARISER_OVERRIDE` pattern)
- Entry kind dispatch via `EntryKind` enum is exhaustive; matches are type-safe; canonical `EntryKind::from_str()` consumed at six+ sites (Polish + Foundational defence-in-depth)
- **Phase 5 / US3**: Single-pass rendering pipeline with per-match dispatch ensures each stage pattern is matched exactly once per body; argument coercion is validated before render
- **Phase 5 / US4**: Three-tier MCP discovery separates concerns: `search_skills` optimizes for ranking + truncation (char_indices fast-path), `get_skill_info` separates metadata from body, `get_skill` remains unchanged; resource enumeration walks (non-recursive, 5-cap per dir, alphabetical via BTreeMap for JSON stability)
- **Phase 5 / US5**: Doctor read-only extensions use only query-level operations; structural enforcement via `open_read_only` with no transaction acquisition
- **Phase 5 Polish**: Single-source-of-truth accessors established: `plugin_data_root()` for process-wide data root; `workspace_data_dir_for()` for workspace-scoped paths; `validate_db_stored_path()` for boundary checks; `build_context_for_entry()` for shared MCP context (eliminates ~50 LOC cross-handler duplication)
- **Phase 6 Foundational**: Harness trait extension follows safe-by-default pattern; all seven new methods have impls that do not alter harness discovery or sync until US1–US3 override them; trait methods never called except during sync + settings composition (wired post-Foundational)

---

*This document describes HOW the system is organized at Phase 6 Foundational WIRED (harness trait extended with hooks/guardrails/agents infrastructure; EntryKind::Agent variant; schema v3→v4 marker; 4 new exit codes 43–46). 1193 tests (Phase 5) + new test coverage for entry-kind agent indexing, harness trait P6, schema migration P6, exit codes.*
