# Implementation Plan: Phase 5 — Commands as Prompts, Unified Entries, and Variable Substitution

**Branch**: `005-phase-5-commands-prompts` | **Date**: 2026-05-26 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/005-phase-5-commands-prompts/spec.md`
**Source PRD**: [PRDs/phase-5.md](../../PRDs/phase-5.md) (authoritative for HOW; spec is authoritative for WHAT)

## Summary

Phase 5 widens what counts as a Tome entry — commands and skills become two `kind`s of one underlying row with a shared indexing pipeline — adds an MCP prompts surface for user-invocable entries, ships a three-tier discovery model (search → middle-tier info → read), and introduces a hand-rolled three-stage variable substitution layer that runs on every entry retrieval (Tome built-ins, environment passthrough, argument substitution, plus an ARGUMENTS append fallback).

The technical approach is conservative: a registered schema migration adds four columns (`kind`, `searchable`, `user_invocable`, `when_to_use`) and widens the unique constraint to `(catalog, plugin, kind, name)`; the existing reindex pipeline gains a second directory walk (`commands/*.md`) and an `embedding_text` composer that includes `when_to_use`; a new `src/substitution/` module ships the substitution engine with four ordered stages and bounded memory; the MCP server (`src/mcp/`) gains a `prompts` capability with `prompts/list` and `prompts/get` handlers backed by a new state field, plus a third tool `get_skill_info` alongside the existing `search_skills` and `get_skill`. No new top-level dependencies are required — `regex` (already a transitive dep via Phase 1's `catalog::git::scrub_credentials`) is promoted to a direct dep for the substitution engine.

Phase 5 also threads two persistent data directories (`${TOME_PLUGIN_DATA}`, `${TOME_WORKSPACE_DATA}`) into the central state tree, lazily created on first substitution, idempotent under concurrent retrievals.

## Technical Context

**Language/Version**: Rust stable (MSRV 1.93, pinned in `Cargo.toml`). No MSRV change for Phase 5.
**Primary Dependencies (existing, consumed in Phase 5)**:
- `rusqlite` 0.32 (`bundled`) — schema migration v2 → v3
- `sqlite-vec` vendored — unchanged
- `rmcp` 1.x (`transport-io`, `schemars`) — `prompts` capability + third tool
- `tokio` 1.x (single-threaded; `src/mcp/` only) — MCP async handlers
- `schemars` 1.x — argument schemas for prompts + new tool I/O
- `serde` + `serde_yaml` — frontmatter parsing (lenient; widened field set)
- `toml`, `toml_edit` — workspace settings updates if FR-025 relocates `${TOME_WORKSPACE_DATA}`
- `tempfile` 3.x — atomic directory landing for data dirs
- `tracing` + `tracing-subscriber` — diagnostic logging
- `time` 0.3 — `${TOME_TIMESTAMP}`, `${TOME_DATE}`

**Primary Dependencies (new direct)**:
- `regex` 1.x — promoted from transitive (used in `catalog::git::scrub_credentials` since Phase 1) to direct for the substitution engine. Already in the dep tree; no binary size cost.

**Storage**: Existing central SQLite database (`<home>/.tome/index.db`) with WAL + advisory lockfile. Schema v2 → v3 migration adds 4 columns + widens unique constraint on the `skills` table. No new tables. Two new on-disk directory classes (`<home>/.tome/plugin-data/<catalog>/<plugin>/` and `<home>/.tome/workspaces/<workspace>/plugin-data/<catalog>/<plugin>/`) created lazily by the substitution layer.

**Testing**: `cargo test` (existing). Unit tests live in source modules; integration tests under `tests/`. New integration test files expected: `entry_kind_indexing.rs`, `substitution_*.rs` (per-stage), `mcp_prompts_*.rs`, `mcp_get_skill_info.rs`, `frontmatter_p5_fields.rs`, `schema_migration_v3.rs`, `plugin_data_dir_*.rs`, `prompt_naming.rs`, `prompt_collision.rs`, `doctor_p5.rs`, `entry_e2e.rs`. JSON wire-shape pins for every new emit-only type (`SearchResult` post-truncation, `SkillInfo`, `PromptDescriptor`, `PromptListResponse`, `PromptGetResponse`).

**Target Platform**: macOS (`macos-latest`) and Linux (`ubuntu-latest`) — CI verified. Windows-native and WSL1 remain unsupported (per Phase 4 carry-over).

**Project Type**: Single Rust project (binary + library; no workspace split).

**Performance Goals**:
- Substitution layer: linear in entry body size; bounded constant-factor memory overhead. Negligible relative to existing embedding/reranking costs.
- `prompts/list`: linear in active workspace's user-invocable entry count; no per-request DB scan beyond a single query.
- `get_skill_info`: small constant-time payload size relative to `get_skill`; bounded by the per-directory cap of 5 children + sentinel.
- Schema migration: in-process on first open; ALTER TABLE adds with NOT NULL DEFAULT for `kind`/`searchable`/`user_invocable` should run in milliseconds on typical row counts (≤10k entries).

**Constraints**:
- Sync only outside `src/mcp/` (constitution III async). Substitution layer + reindex + schema migration all sync. NFR-010 makes this an explicit substitution-layer requirement.
- Atomic writes for all Tome-owned on-disk artefacts (constitution + Phase 4 patterns). The two persistent data directories use `create_dir_all` (idempotent + concurrent-safe per FR-021/NFR-012).
- 50 MB binary cap (constitution v1.3.0 §Binary size). Phase 5 adds no new top-level deps; size projection +0–1 MiB from substitution module + new MCP tool/prompt handlers. Final budget margin ≈ 23 MiB on macOS arm64 from Phase 4's 26.32 MiB baseline.
- Closed-error-enum discipline (`TomeError`). Phase 5 adds the new exit codes specified in PRD §Exit codes (21–25) and surfaces every distinct failure mode with a dedicated variant.
- Strictness boundary (constitution IV). Tome-owned inputs strict; third-party frontmatter lenient. The widened frontmatter field set (`disable-model-invocation`, `user-invocable`, `arguments`, `argument-hint`, `prompt_name`, `when_to_use`) parses leniently.

**Scale/Scope**: 5 user stories (P1–P5), 64 functional requirements, 12 non-functional requirements, 13 success criteria, 5 new exit codes, 4 new schema columns, 3 new MCP surfaces (the `prompts` capability + `get_skill_info` tool + updated `search_skills`/`get_skill`), one new top-level module (`src/substitution/`), updates to four existing top-level modules (`src/plugin/`, `src/index/`, `src/mcp/`, `src/commands/plugin/`).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Each principle from `CONSTITUTION.md` (v1.3.0):

- **I. Unix Philosophy**: PASS. New CLI surface is minimal (the source PRD declines to add `tome plugin new` or any plugin-authoring command); the `tome doctor`, `tome plugin show`, and MCP-server surfaces stay one-thing-at-a-time. `--json` is preserved on every new output path.
- **II. Predictable Exit Codes (NON-NEGOTIABLE)**: PASS with attention. Phase 5 introduces 5 new failure classes per PRD §Exit codes (Entry not found / Substitution failed / Invalid argument frontmatter / Prompt argument count exceeds supplied / Workspace data directory write failed). Each gets a dedicated variant + exit code; the spec's Edge Cases section defers exact numeric assignment to the contract. **Action**: pin the numeric values in `contracts/exit-codes-p5.md` (avoid reusing Phase 1–4 codes; pick a contiguous range that does not collide).
- **III. Scriptable by Default**: PASS. No new interactive prompts in Phase 5. `tome plugin show` and `tome doctor` both already non-interactive. MCP prompts are user-triggered on the harness side; Tome only enumerates them.
- **IV. Strict Schemas, Helpful Errors**: PASS. The widened frontmatter field set lands inside the existing lenient `src/plugin/frontmatter.rs` boundary; Tome-owned types (the substitution context, the new schema columns, the JSON envelopes for `get_skill_info` / `PromptDescriptor`) all `deny_unknown_fields`. New parse errors name the file + field per the existing pattern.
- **V. Fail Fast, Fail Clear**: PASS. The substitution failure surface (FR-022 on built-ins, FR-032 on env passthrough, FR-044 on argument schema mismatches) all surface with clear errors. The data-directory creation failure (FR-021) yields a dedicated error code rather than a partial substitution.
- **VI. KISS / YAGNI**: PASS. Hand-rolled substitution (no templating engine), no recursive expansion, no shell exec (Phase 6+). New `src/substitution/` module is bounded — four stages, one regex compile per stage, single-pass composition. No speculative abstractions.
- **VII. Modular by Boundary**: PASS. `src/substitution/` is a new capability module with an explicit public surface (`SubstitutionContext`, `render(body, context) -> Result<String, SubstitutionError>`). The frontmatter changes stay within `src/plugin/frontmatter.rs`. The schema migration lives in `src/index/migrations.rs`. MCP surface updates stay in `src/mcp/`. No circular dependencies.
- **VIII. Test What Matters**: PASS. Integration tests for every shipped behavior (entry indexing, substitution stages, prompts surface, middle-tier tool, doctor extensions). Library-API + StubEmbedder for heavy paths; CLI binary for exit-code coverage. JSON wire-shape pins for every new emit-only type per the Phase 4 P8 retro lesson.
- **IX. Conventional Commits**: PASS. Existing `cog` hook enforces.
- **X. CI Gates Every Merge**: PASS. Existing `fmt + clippy + build + test` matrix continues.
- **XI. Documentation Is Part of the Change**: PASS. Each user-story slice updates contracts under `specs/005-phase-5-commands-prompts/contracts/`; final Polish updates README + CHANGELOG.
- **XII. Inherit, Don't Reimplement**: PASS. `regex` (existing transitive) for pattern matching; rmcp's `#[tool_router]` for the new third tool + `prompts` capability; serde_yaml for the widened frontmatter. No new substitution-engine library — hand-rolled.
- **XIII. Never Log Secrets**: PASS. Substitution layer's environment passthrough restricts to `${TOME_ENV_*}` only (NFR-005); references outside the namespace pass through unchanged (FR-033). New diagnostic logging in the substitution layer never logs entry body content nor host-env-variable values (only the variable names that were resolved).

Operational Constraints:
- **Async**: Substitution layer is sync (NFR-010). MCP `prompts/get` handler invokes it via `spawn_blocking` per the Phase 3 US1 pattern.
- **Binary size**: No new deps; expected delta < 1 MiB.
- **Paths**: Persistent data directories anchored under `<home>/.tome/` per constitution §Paths v1.3.0 (FR-021). No project-marker pollution.
- **Licensing**: All consumed deps are MIT or Apache-2.0; no new license review needed.

**Verdict**: PASS. No deviations require Complexity Tracking.

## Project Structure

### Documentation (this feature)

```text
specs/005-phase-5-commands-prompts/
├── spec.md                           # /sdd:specify output (frozen)
├── plan.md                           # This file (/sdd:plan output)
├── research.md                       # Phase 0 output (/sdd:plan)
├── data-model.md                     # Phase 1 output (/sdd:plan)
├── quickstart.md                     # Phase 1 output (/sdd:plan)
├── contracts/                        # Phase 1 output (/sdd:plan)
│   ├── entry-schema-p5.md
│   ├── schema-migration-p5.md
│   ├── frontmatter-p5.md
│   ├── substitution-engine.md
│   ├── mcp-tools-p5.md               # search_skills updates + get_skill updates + get_skill_info NEW
│   ├── mcp-prompts.md                # prompts capability + list + get + naming
│   ├── doctor-extensions-p5.md
│   ├── catalog-and-plugin-extensions-p5.md
│   └── exit-codes-p5.md
├── checklists/
│   └── requirements.md               # /sdd:specify output (frozen)
├── retro/                            # Created per user story by closeout PRs
├── review/                           # Created by US-closeout reviewer passes
└── tasks.md                          # /sdd:tasks output — NOT created by /sdd:plan
```

### Source Code (repository root)

```text
src/
├── main.rs                          # (existing) unchanged — clap dispatch
├── cli.rs                           # (existing) extended for new `tome plugin show` annotations (FR-130)
├── error.rs                         # (existing) extended with Phase 5 TomeError variants + exit codes 21–25
├── paths.rs                         # (existing) extended with plugin_data_dir(catalog, plugin) + workspace_plugin_data_dir(workspace, catalog, plugin) accessors
├── config.rs                        # (existing) unchanged
├── logging.rs                       # (existing) unchanged
├── output.rs                        # (existing) unchanged
│
├── plugin/
│   ├── manifest.rs                  # (existing) unchanged
│   ├── frontmatter.rs               # (existing) extended for the widened lenient field set (Phase 5 §Frontmatter spec)
│   ├── components.rs                # (existing) extended to walk `commands/*.md` alongside `skills/*/SKILL.md`
│   ├── identity.rs                  # (existing) unchanged
│   └── lifecycle.rs                 # (existing) extended to plumb `kind` through enable/disable/reindex orchestrator
│
├── index/
│   ├── schema.rs                    # (existing) schema v3 DDL with widened unique constraint
│   ├── migrations.rs                # (existing) Phase 5 migration v2→v3 registered here
│   ├── skills.rs                    # (existing) extended to insert/select on `(catalog, plugin, kind, name)`
│   ├── query.rs                     # (existing) extended for searchable filter + kind in result rows + description truncation
│   └── ...                          # (existing) other index modules unchanged
│
├── substitution/                    # NEW module
│   ├── mod.rs                       # Public API: render(body, context) -> Result<String, SubstitutionError>
│   ├── context.rs                   # SubstitutionContext + builder; clock injection seam
│   ├── builtins.rs                  # Built-in variable resolution (12 builtins per FR-020 + sanitisation per FR-024)
│   ├── env.rs                       # ${TOME_ENV_*} passthrough with default-value syntax (FR-030–FR-033)
│   ├── arguments.rs                 # $ARGUMENTS family + name binding + append-fallback (FR-040–FR-046)
│   ├── regex.rs                     # Compiled regex set (one per stage, once_cell-cached)
│   └── data_dir.rs                  # Lazy create_dir_all for ${TOME_PLUGIN_DATA} and ${TOME_WORKSPACE_DATA}; FR-025 rename relocation
│
├── mcp/
│   ├── server.rs                    # (existing) extended for prompts capability + new tool registration
│   ├── state.rs                     # (existing) extended with PromptRegistry field + workspace entry cache
│   ├── prompts.rs                   # NEW — prompts/list handler + prompts/get handler + name registry
│   ├── prompt_name.rs               # NEW — name derivation (plugin__entry); sanitisation; truncation; override
│   ├── prompt_collision.rs          # NEW — collision detection + counter suffixing + diagnostic logging
│   └── tools/
│       ├── search_skills.rs         # (existing) extended for truncation parameter + kind in result + searchable filter
│       ├── get_skill.rs             # (existing) extended for kind parameter + substitution layer invocation
│       └── get_skill_info.rs        # NEW — middle-tier metadata tool
│
├── commands/
│   ├── plugin/
│   │   └── show.rs                  # (existing) extended for per-entry kind/searchable/user_invocable annotations + derived prompt name
│   └── doctor.rs                    # (existing) extended for FR-120/121/122/123/124 surfaces
│
└── presentation/                    # (existing) extended formatting helpers as needed (kind discriminator, prompt-name column)

tests/
├── entry_kind_indexing.rs           # NEW (US1) — both directory walks + kind discriminator
├── frontmatter_p5_fields.rs         # NEW — widened lenient field set
├── schema_migration_v3.rs           # NEW — v2 → v3 with backfill verification
├── substitution_builtins.rs         # NEW (US2)
├── substitution_env.rs              # NEW (US2)
├── substitution_arguments.rs        # NEW (US3)
├── substitution_pipeline.rs         # NEW — stage ordering + once-pass invariant
├── substitution_data_dir.rs         # NEW (US2) — lazy create + concurrent safety + FR-025 rename
├── mcp_prompts.rs                   # NEW (US1) — prompts capability + list + get
├── mcp_get_skill_info.rs            # NEW (US4)
├── mcp_search_skills_truncation.rs  # NEW (US4) — description_max_chars parameter + default
├── prompt_naming.rs                 # NEW — sanitisation + truncation + override
├── prompt_collision.rs              # NEW — counter suffixing + diagnostics
├── doctor_p5.rs                     # NEW (US5) — FR-120/121/122/123/124
├── plugin_show_p5.rs                # NEW — kind/searchable/user_invocable annotations
├── entry_e2e.rs                     # NEW — end-to-end through enable/index/search/info/get/prompts
└── (existing tests) extended where Phase 5 changes affect them (catalog_*, plugin_*, query, reindex, mcp_*)
```

**Structure Decision**: Single Rust project (binary + library). The new `src/substitution/` module is the only top-level addition; it sits alongside `src/embedding/` and `src/summarise/` as a deterministic content-transform module, not a service. The new MCP module-level files (`prompts.rs`, `prompt_name.rs`, `prompt_collision.rs`) cluster under `src/mcp/` per the existing convention. Schema migrations stay in `src/index/migrations.rs`. New tool handler `src/mcp/tools/get_skill_info.rs` follows the established per-tool-per-file pattern from Phase 3 / US1.

## Complexity Tracking

> No constitution violations. No entries.

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| _(none)_  | _(n/a)_    | _(n/a)_                              |
