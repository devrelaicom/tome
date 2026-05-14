# Implementation Plan: Phase 4 — Central Architecture Refactor and Cross-Harness Integration

**Branch**: `004-phase-4-refactor-harnesses` | **Date**: 2026-05-14 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `specs/004-phase-4-refactor-harnesses/spec.md`
**Source PRD** (HOW reference): [PRDs/phase-4.md](../../PRDs/phase-4.md)
**Constitution**: [CONSTITUTION.md](../../CONSTITUTION.md) — v1.2.0 today; v1.3.0 amendment lands in Foundational (F1).

## Summary

Phase 4 does two interlocked things:

1. **Refactor every storage path and database in Tome** to a single root at `<home>/.tome/` with one central SQLite database. The `directories` crate is dropped; the XDG-style separation of config / data / cache / state collapses into one tree. Per-workspace SQLite databases (Phase 3) are replaced by a workspace-skills + workspace-catalogs + workspace-projects junction model against the central DB. Workspaces become named first-class objects stored centrally; the project's `.tome/` directory shrinks to a binding pointer (`workspace = "<name>"`) plus a copied rules file. The Phase 3 `--global` flag is removed — global is just a workspace named `global` that other workspaces are equal to. The first registered forward migration debuts: schema v1 → v2, structural-only (no Phase 3 user data is migrated; pre-release wipe is the contract).
2. **Add cross-harness MCP + rules-file integration** for five harnesses (Claude Code, Codex CLI, Gemini CLI, Cursor, OpenCode). One command — `tome workspace use <name>` from inside a project — binds the project to a workspace, writes a rules-file block (or standalone file for Cursor) into each configured harness's rules-file target, and writes the Tome MCP entry into each harness's MCP config. A bundled local summariser (Qwen2.5-0.5B-Instruct via `llama-cpp-2`) produces two cached natural-language summaries per workspace; the short summary is interpolated into the MCP search tool's description, the long summary becomes the rules-file body. Layered settings (project → workspace → global) with composition syntax (`[workspace]`, `[workspaces.<name>]`, `[global]`, `<name>`, `!<name>`) control which harnesses each scope configures. `tome doctor` is extended to report bound-project consistency, per-harness integration state, and the summariser subsystem.

The most consequential design decisions for this plan are:

1. **One root, one database.** Phase 3's per-workspace databases didn't hold up under scrutiny — workspaces behave better as named, centrally-stored objects that projects bind to. The cost is a one-time refactor that touches every existing path-builder, every catalog/plugin/query/reindex/status call site, and the workspace-resolution algorithm. The benefit is a simpler mental model, atomic backup/wipe of all Tome state, and the binding-pointer model that makes Phase 4's cross-harness integration possible. The constitution's §Paths Operational Constraint requires a v1.3.0 amendment to ship this; the amendment is the Foundational PR's first commit.
2. **The summariser is the third bundled inference runtime, and it's sync.** `llama-cpp-2`'s API is synchronous; the runtime singleton is a `std::sync::OnceLock<LlamaBackend>` that lives for the lifetime of the process; the model and context are loaded lazily and dropped after use. The structural `sync_boundary` test does not need extension — the new `src/summarise/` module contains no `tokio::` / `async fn` / `.await` and is naturally on the sync side. The MCP server does not invoke the summariser in-process; it reads the cached short summary from the workspace's settings file at startup.
3. **Atomic populated-directory landings promote to a helper.** Phase 3 introduced the pattern (`tempfile::Builder::tempdir_in` → populate → `TempDir::keep` → `std::fs::rename`) inside `workspace::init`. Phase 4 hits the rule of three: `workspace init`, `workspace rename`, `workspace use` (project marker creation). Promotion to `src/util/atomic_dir.rs` happens in Foundational F4.
4. **Composition resolves to as-written lists, not effective lists.** The reviewer raised this as B7 on the spec; FR-449 pins it. A project's `[workspace]` reference resolves to the workspace's directly-declared `harnesses` array — not its computed effective list. Without this rule, every composition reference would re-trigger the full priority walk and the layered model becomes unintelligible.
5. **The forward-progress rule for summariser failures.** A plugin enable / disable / reindex commits the skill-state mutation before invoking the summariser. If the summariser fails (model missing, output empty, backend init failure), the developer's intent is honoured (skill state committed); the summariser failure is surfaced as exit 20 with a stale-cache warning in doctor. This is FR-385; it prevents "I can't enable a plugin because I haven't downloaded the summariser model yet" from blocking all forward progress.

## Technical Context

**Language/Version**: Rust stable. MSRV unchanged at `rust-version = "1.93"`. `llama-cpp-2`'s MSRV verified ≤ 1.93 during F6 (scratch-build before Foundational closes); `toml_edit`'s MSRV is unconstrained for our use.

**Primary Dependencies** (additions on top of Phase 1–3):

- **`llama-cpp-2 = "0.x"`** — Qwen2.5-0.5B-Instruct inference runtime. Sync API; statically links `llama.cpp` (CPU-only). ~6 MB binary impact on Linux x86_64.
- **`toml_edit = "0.x"`** — comment- and order-preserving TOML editor for harness MCP config files (specifically Codex CLI's `~/.codex/config.toml`). ~250 KB binary impact.
- **`serde_json` feature `preserve_order`** — order-preserving JSON map for harness MCP config files (Claude Code / Gemini / Cursor / OpenCode). Already a direct dep; this is a feature-flag addition only.

**Removed dependencies**:

- **None.** A reviewer-surfaced framing correction (see [research.md R-1](./research.md)): Tome does not currently depend on the `directories` crate, despite the constitution v1.2.0 §Paths constraint's wording. Phase 3's `src/paths.rs:14-21` is explicit about the deviation. F2's mechanical work is a `Paths`-struct reshape (drop the XDG-separated fields and introduce `<home>/.tome/` accessors) + call-site sweep, NOT a dep removal. The v1.3.0 §Paths amendment (R-12) closes the documentation/code mismatch alongside changing the on-disk layout. The Phase 4 plan's `tests/no_directories_imports.rs` is therefore a forward-looking grep that prevents accidental future reintroduction of the crate, not a regression net for a Phase-4-introduced removal.

**Storage** (refactored):

- All Tome-owned state under `<home>/.tome/`. Single central SQLite database at `<home>/.tome/index.db`. Per-workspace databases (Phase 3) are deleted from the architecture entirely.
- See [contracts/paths-and-layout-p4.md](./contracts/paths-and-layout-p4.md) for the full layout.
- Schema v2 (Phase 4) replaces v1 (Phase 3) via the first registered forward migration. See [contracts/schema-migration-p4.md](./contracts/schema-migration-p4.md).

**Testing**: `cargo test`, extending Phase 1–3 discipline:

- **New integration suites**:
  - `tests/workspace_commands_p4.rs` — extends Phase 3's workspace tests with the named-workspace + binding-pointer model. Init / list / info / use / rename / remove / sync / regen-summary cross-product.
  - `tests/harness_commands.rs` — bare / list / use / remove / info / sync; tabular output assertions; scope-flag semantics.
  - `tests/harness_modules.rs` — per-harness path resolution, strategy, block_body_style. Mock `<home>` with `TempDir`.
  - `tests/rules_file_block_in_existing.rs` — block insertion / update / removal; AtInclude + Inline; surrounding content preservation; symlink refusal.
  - `tests/rules_file_standalone.rs` — Cursor's standalone file.
  - `tests/mcp_config_create.rs`, `tests/mcp_config_update.rs`, `tests/mcp_config_clash.rs`, `tests/mcp_config_remove.rs`, `tests/mcp_config_preserve_order.rs` — read-modify-write discipline; preservation of non-Tome entries, comments, and key order.
  - `tests/settings_composition.rs`, `tests/settings_priority.rs`, `tests/settings_composition_resolves_to_as_written.rs` — every composition form; cycle detection; FR-449 invariant.
  - `tests/sync_algorithm.rs`, `tests/sync_idempotence.rs` — full reconciler; byte-for-byte idempotence verified by `rename()` syscall count and `mtime` comparison.
  - `tests/summariser_stub.rs`, `tests/summariser_triggers.rs`, `tests/summariser_forward_progress.rs`, `tests/summariser_cache.rs` — stub-based; trigger correctness; FR-385.
  - `tests/summariser_real.rs` — CI-skipped real-model round-trip (gated by `TOME_TEST_REAL_MODELS=1`).
  - `tests/migration_v1_to_v2.rs` — real production migration against synthetic v1 fixture.
  - `tests/doctor_p4.rs` — extends Phase 3's doctor tests with binding / rules-copy / per-harness rules / per-harness MCP / summariser subsystems; subsystem enum promotion; `--fix` repairs the supported classes.
  - `tests/catalog_workspace_refcount.rs`, `tests/plugin_workspace_skills.rs`, `tests/plugin_cheap_reenable.rs`, `tests/catalog_update_cross_workspace_reindex.rs` — refactored catalog and plugin semantics against the central DB.
  - `tests/atomic_dir.rs` — multi-file directory landing helper; SIGINT mid-populate (cleaned); SIGINT post-keep (orphan; doctor `--fix` cleans).
  - `tests/no_directories_imports.rs` — structural test asserting no source file imports the `directories` crate.
- **Extended Phase 1–3 suites**:
  - `tests/exit_codes.rs` — 8 new variants (codes 13–20) + reused variants (FR-602).
  - `tests/sync_boundary.rs` — unchanged (no new sync-boundary exemption needed; `src/summarise/` and friends are sync).
  - `tests/manifest_strictness.rs` — extends to cover the new strict types (workspace settings.toml, project marker config.toml, global settings.toml, summariser manifest).
  - `tests/scrubbing.rs` — extends to summariser model URLs and harness MCP config paths in error chains.

**Target Platform**: macOS arm64, Linux x86_64, and WSL2 on the WSL filesystem. Native Windows, WSL1, and WSL2-on-Windows-FS remain out of scope.

**Project Type**: Single binary crate `tome`. No workspace split (still ~12 kLOC after Phase 4; the threshold for splitting is "code size justifies the friction," not reached).

**Performance Goals**:

- `tome workspace use` happy path (no summariser invocation; idempotent re-run): well under 1 s on a recent laptop (NFR-106).
- `tome harness sync` idempotent re-run: well under 1 s (NFR-107).
- Summariser regeneration cycle (Qwen2.5-0.5B INT4 CPU-only, ~600 tokens output total across both prompts): under 30 s on a 2024-era Apple Silicon laptop; under 60 s on a 2020-era x86_64 Linux laptop. A documented timeout enforces the upper bound — exceeding it emits the dedicated summariser-failure code 20 rather than hanging the CLI (NFR-106 timeout clause).
- MCP server startup unchanged from Phase 3 (< 1 s to first message ready; `search_skills` p50 < 300 ms, p99 < 600 ms).

**Constraints**:

- Release binary stripped: ≤ 50 MB (NFR-101). Phase 4 projection: ~28.4 MiB on macOS arm64, ~34 MB on Linux x86_64. Headroom ~16 MB. Final measurement at the end of Foundational; revise component choices rather than waive the cap.
- Synchronous-only outside `src/mcp/` (NFR-103). `llama-cpp-2` is sync; the structural sync-boundary test does not need extension.
- Closed-error-set principle holds (FR-600). 8 new variants + reuse table per FR-602; no generic `Other`.
- Atomic state mutations extend to: workspace directory landings (new helper), workspace settings file writes, project marker landings, harness MCP config read-modify-write, rules-file block writes.
- Credential scrubbing extends to: summariser model download URLs, harness MCP config paths in error chains.
- All Phase 1–3 quality gates apply (NFR-108).

**Scale/Scope**:

- Named workspaces per user: typically 1–10; pathological 50.
- Projects bound to one workspace: typically 1–10; pathological 100.
- Bound projects per workspace × workspaces = total bindings in the central DB: ≤ ~500 in pathological cases.
- Phase 4 adds an estimated 3–4 kLOC of Rust on top of the Phase 3 ~9 kLOC; total ~12–13 kLOC after Phase 4. No workspace split yet.
- Summariser model on disk: ~400 MB (Qwen2.5-0.5B GGUF Q4_K_M).

## Constitution Check

*GATE: must pass before Phase 0. Re-checked after Phase 1.*

| # | Principle | Status | How this plan satisfies it |
|---|---|---|---|
| I | Unix Philosophy | PASS | Every new CLI command keeps the convention: human form on stdout, errors on stderr, global `--json`. Composition syntax is declarative; no command interprets composition inline — resolution happens in `src/settings/`. The MCP server's stdout-is-protocol invariant (Phase 3 FR-221) carries forward unchanged. |
| II | Predictable Exit Codes (NON-NEGOTIABLE) | PASS | 8 new variants (codes 13–20) per FR-601; reused variants per FR-602; closed-error-set principle preserved per FR-600 / FR-603. See [contracts/exit-codes-p4.md](./contracts/exit-codes-p4.md). |
| III | Scriptable by Default | PASS | Every new command is non-interactive. Destructive operations (`workspace remove`, `harness sync` against a clashing MCP entry) require `--force` flags. No silent auto-confirmation. |
| IV | Strict Schemas, Helpful Errors | PASS | Tome-owned settings/config files strict (`#[serde(deny_unknown_fields)]`); third-party harness config files lenient + order-preserving on write (FR-349). Composition errors name the chain of references that triggered them. Workspace-not-found names the workspace and points at `tome workspace init`. |
| V | Fail Fast, Fail Clear | PASS | Project marker malformed → dedicated error pointing at `tome doctor`. Composition cycle → dedicated error naming the loop. Harness clash → dedicated error quoting the existing entry. Summariser failure → named sub-class. |
| VI | KISS / YAGNI | PASS | One root directory. One central DB. One summariser model, no config knobs (FR-427). One sync algorithm. Five harness modules; new harnesses are additive in Phase 5+. No workspace migration tooling between Phase 3 and Phase 4 (pre-release wipe). |
| VII | Modular by Boundary | PASS | New capability modules: `src/summarise/` (inference), `src/harness/` (per-harness module impls + rules-file + MCP config logic), `src/settings/` (layered + composition resolver), `src/util/atomic_dir.rs` (multi-file directory landing). Existing modules (`src/catalog/`, `src/plugin/`, `src/index/`, `src/doctor/`) are extended without restructuring. The Phase 3 `src/workspace/inventory.rs` (opt-in registry) is deleted. |
| VIII | Test What Matters | PASS | Integration tests per new command; stub-based tests for the summariser (real model load only in CI-skipped tests); structural test asserting no `directories` imports; byte-for-byte idempotence asserted via `rename()` syscall count + `mtime` comparison. |
| IX | Conventional Commits | PASS | Unchanged. |
| X | CI Gates Every Merge | PASS | `ci.yml` extends to install nothing new for the binary build (CPU-only `llama-cpp-2` builds the C++ source statically with no external SDK). Binary-size step continues to assert ≤ 50 MB. WSL2-on-WSL-FS CI matrix entry to be added (out of scope of normal CI cost; can be a separate weekly workflow if the per-PR latency cost is unacceptable — decided during F1). |
| XI | Documentation Is Part of the Change | PASS | `quickstart.md` covers the Phase 4 flow end-to-end. README updated for Phase 4. CHANGELOG entries per slice. Command help text for every new subcommand. |
| XII | Inherit, Don't Reimplement | PASS — see Complexity Tracking | We use `llama-cpp-2` (mature, maintained) for inference rather than rolling a custom decoder. `toml_edit` for comment preservation. `serde_json` `preserve_order` for JSON key order. System Git unchanged. No new external-tool dependencies. |
| XIII | Never Log Secrets | PASS | Phase 1 credential scrubber extends to: summariser model URLs (signed if behind a CDN), harness MCP config paths (may contain `$HOME` segments that are themselves sensitive), `reqwest` error chains during summariser download. Phase 3 P8 PR-F's symlink refusal and registry validation discipline extends to project markers and harness rules-file targets. |

**Operational Constraints check**:

- **Lints**: unchanged. New code passes `clippy -D warnings`.
- **Dependencies**: two new direct, one feature-flag addition, one removal — see Technical Context. Every licence verified within the allowlist (`llama-cpp-2` MIT; `toml_edit` MIT/Apache-2.0; `llama.cpp` MIT). `cargo-deny check` enforces.
- **Async**: unchanged. `llama-cpp-2` is sync; the structural sync-boundary test does not need extension. The MCP server's async island (Phase 3) is unaffected.
- **Binary size**: load-bearing concern again. Projected ~28.4 MiB on macOS arm64, ~34 MB on Linux x86_64; headroom ~16 MB. CI binary-size step continues to assert ≤ 50 MB. Revise component choices if breached.
- **Paths**: **Operational Constraint changes via v1.3.0 amendment** (lands in Foundational F1). The new constraint reads: "Tome-owned paths resolve under `<home>/.tome/`. The home directory is resolved via a portable mechanism (the standard library where available). All Tome state lives under this root; the XDG-style separation is deliberately collapsed into a single tree." See [research.md R-12](./research.md) for the amendment's exact text.
- **Licensing**: MIT OR Apache-2.0 unchanged.

**Result: PASS** with one Operational Constraint amendment (v1.3.0 §Paths) landing in the first Foundational PR. Two deviations need justification in Complexity Tracking: the bundled summariser (third inference runtime), and the path-principle amendment itself.

## Project Structure

### Documentation (this feature)

```text
specs/004-phase-4-refactor-harnesses/
├── plan.md                          # This file
├── spec.md                          # Feature specification (/sdd:specify output)
├── research.md                      # Phase 0 output — 19 R-decisions
├── data-model.md                    # Phase 1 output — types + DDL + error variants
├── quickstart.md                    # Phase 1 output — end-to-end smoke test
├── contracts/                       # Phase 1 output (13 contract files)
│   ├── catalog-and-plugin-extensions-p4.md
│   ├── doctor-extensions-p4.md
│   ├── exit-codes-p4.md
│   ├── harness-commands.md
│   ├── harness-modules.md
│   ├── mcp-config-integration.md
│   ├── paths-and-layout-p4.md
│   ├── rules-file-integration.md
│   ├── schema-migration-p4.md
│   ├── settings-composition.md
│   ├── summariser.md
│   ├── sync-algorithm.md
│   └── workspace-commands.md
├── checklists/
│   └── requirements.md              # Spec quality checklist (PASS)
└── tasks.md                         # Phase 2 output of /sdd:tasks (NOT created here)
```

### Source code (repository root)

New modules in **bold**; refactored-in-place modules marked as `refactored`; Phase 1–3 modules untouched left unmarked.

```text
tome/
├── Cargo.toml                       # extended: llama-cpp-2, toml_edit, serde_json/preserve_order
├── deny.toml                        # extended: new dep licence rows
├── CONSTITUTION.md                  # v1.3.0 — §Paths rewritten
├── src/
│   ├── main.rs                      # refactored: --global flag removed; --workspace <name> parsing
│   ├── lib.rs                       # refactored: re-exports for new modules
│   ├── cli.rs                       # refactored: new subcommand surface (workspace lifecycle, harness commands); --global removed
│   ├── config.rs                    # refactored: Tome-owned global config carried forward; workspace settings + project marker configs in new modules
│   ├── paths.rs                     # refactored: collapses XDG-separated fields into typed accessors under <home>/.tome/
│   ├── output.rs                    # unchanged
│   ├── logging.rs                   # unchanged (MCP file appender path updated to <root>/logs/mcp.log)
│   ├── error.rs                     # refactored: 8 new variants (codes 13–20) + the reused-variant table (FR-602)
│   ├── catalog/
│   │   ├── manifest.rs              # unchanged
│   │   ├── store.rs                 # refactored: workspace_catalogs is the source of truth; refcount-under-lock per FR-366
│   │   └── git.rs                   # unchanged
│   ├── commands/
│   │   ├── catalog/                 # refactored: routes through workspace_catalogs for the resolved workspace
│   │   ├── plugin/                  # refactored: routes through workspace_skills
│   │   ├── models/                  # refactored: extended to include summariser as third model
│   │   ├── query.rs                 # refactored: joins workspace_skills
│   │   ├── reindex.rs               # refactored: operates on the resolved workspace's workspace_skills
│   │   ├── status.rs                # refactored: reads from central DB scoped to resolved workspace; new summariser subsystem
│   │   ├── doctor.rs                # refactored: extended subsystems per FR-560 through FR-564
│   │   ├── mcp.rs                   # refactored: resolves workspace from project marker; no --global flag
│   │   ├── workspace/               # refactored & extended (8 commands)
│   │   │   ├── mod.rs               # dispatcher
│   │   │   ├── init.rs              # `workspace init <name> [--inherit-global]`
│   │   │   ├── list.rs              # NEW
│   │   │   ├── info.rs              # refactored — now takes optional <name> arg
│   │   │   ├── use_.rs              # NEW — `workspace use <name>` (file named `use_.rs` because `use` is reserved)
│   │   │   ├── rename.rs            # NEW
│   │   │   ├── remove.rs            # NEW
│   │   │   ├── sync.rs              # NEW
│   │   │   └── regen_summary.rs     # NEW
│   │   └── **harness/**             # NEW capability module (6 commands)
│   │       ├── mod.rs               # dispatcher; `tome harness` (bare) list
│   │       ├── list.rs              # `tome harness list [<workspace>]`
│   │       ├── use_.rs              # `tome harness use <name> [--scope]`
│   │       ├── remove.rs            # `tome harness remove <name> [--scope]`
│   │       ├── info.rs              # `tome harness info <name>`
│   │       └── sync.rs              # `tome harness sync`
│   ├── workspace/                   # refactored
│   │   ├── mod.rs                   # public surface
│   │   ├── scope.rs                 # Scope { WorkspaceName } reshape; ResolvedScope { scope, source, project_root }
│   │   ├── name.rs                  # NEW — WorkspaceName newtype with parse() validation
│   │   ├── resolution.rs            # refactored: priority order flag > env > marker walk > global fallback
│   │   ├── lifecycle.rs             # NEW — init / rename / remove / sync internals
│   │   └── binding.rs               # NEW — `workspace use` binding flow + project marker writes
│   ├── plugin/                      # unchanged structurally
│   ├── index/
│   │   ├── mod.rs                   # unchanged public surface
│   │   ├── db.rs                    # refactored: opens central DB at <root>/index.db
│   │   ├── schema.rs                # refactored: v2 schema; bootstrap emits v2 directly
│   │   ├── migrations.rs            # refactored: MIGRATIONS = &[phase_4_v1_to_v2]; one registered production migration
│   │   ├── vec_ext.rs               # unchanged
│   │   ├── skills.rs                # refactored: operates on workspace_skills junction
│   │   ├── query.rs                 # refactored: joins workspace_skills
│   │   ├── meta.rs                  # refactored: meta carries summariser_name / summariser_version
│   │   ├── integrity.rs             # unchanged
│   │   └── lock.rs                  # refactored: single lockfile at <root>/index.lock
│   ├── embedding/                   # unchanged
│   ├── presentation/                # extended: tables for workspace list / harness list
│   ├── doctor/                      # refactored
│   │   ├── mod.rs                   # public surface
│   │   ├── report.rs                # refactored: Subsystem enum promotion; new variants
│   │   ├── checks.rs                # refactored: new check fns per FR-560
│   │   ├── harness_detect.rs        # unchanged
│   │   ├── binding.rs               # NEW — bound-project consistency check
│   │   ├── harness_integration.rs   # NEW — per-harness rules + MCP integration check
│   │   └── fixes.rs                 # refactored: new fix dispatch arms
│   ├── **summarise/**               # NEW capability module
│   │   ├── mod.rs                   # Summariser trait + backend() singleton
│   │   ├── llama.rs                 # LlamaSummariser production impl
│   │   ├── stub.rs                  # #[cfg(test)] StubSummariser
│   │   ├── prompts.rs               # SHORT_PROMPT / LONG_PROMPT constants + length-window constants
│   │   ├── registry.rs              # MODEL_REGISTRY extension for Qwen2.5-0.5B
│   │   └── download.rs              # reuses embedding::download with summariser model entry
│   ├── **harness/**                 # NEW capability module
│   │   ├── mod.rs                   # HarnessModule trait + SUPPORTED_HARNESSES static + lookup()
│   │   ├── claude_code.rs           # impl
│   │   ├── codex.rs                 # impl
│   │   ├── gemini.rs                # impl
│   │   ├── cursor.rs                # impl
│   │   ├── opencode.rs              # impl
│   │   ├── rules_file.rs            # block markers + AtInclude/Inline + StandaloneFile read/write/remove
│   │   └── mcp_config.rs            # read-modify-write of JSON + TOML harness MCP configs
│   ├── **settings/**                # NEW capability module
│   │   ├── mod.rs                   # public surface
│   │   ├── parser.rs                # parses harnesses arrays from project marker / workspace settings / global settings
│   │   ├── composition.rs           # CompositionRef parser + EffectiveHarnessList type
│   │   └── resolver.rs              # resolve_effective_list() — DFS, cycle detection, FR-449 enforcement
│   ├── **util/**                    # NEW helpers
│   │   └── atomic_dir.rs            # land_directory + land_directory_with_replace (R-10)
│   └── mcp/                         # unchanged in shape; tool description now reads cached short summary at startup
│       ├── mod.rs                   # unchanged signature
│       ├── runtime.rs               # unchanged
│       ├── server.rs                # refactored: tool description interpolates workspace short summary
│       ├── tools/                   # unchanged
│       ├── preflight.rs             # refactored: also verifies summariser presence (informational only — MCP doesn't invoke summariser)
│       └── log.rs                   # unchanged (path moves to <root>/logs/mcp.log via Paths)
└── tests/
    ├── (Phase 1–3 suites carry forward; many extended to honour the central DB and new error variants)
    ├── exit_codes.rs                # extended: 8 new variants + reused-variant assertions
    ├── manifest_strictness.rs       # extended: new strict types
    ├── scrubbing.rs                 # extended: summariser URLs, harness MCP paths
    ├── sync_boundary.rs             # unchanged (no new exemption)
    ├── workspace_commands_p4.rs     # NEW
    ├── harness_commands.rs          # NEW
    ├── harness_modules.rs           # NEW
    ├── rules_file_block_in_existing.rs  # NEW
    ├── rules_file_standalone.rs     # NEW
    ├── mcp_config_create.rs         # NEW
    ├── mcp_config_update.rs         # NEW
    ├── mcp_config_clash.rs          # NEW
    ├── mcp_config_remove.rs         # NEW
    ├── mcp_config_preserve_order.rs # NEW
    ├── settings_composition.rs      # NEW
    ├── settings_priority.rs         # NEW
    ├── settings_composition_resolves_to_as_written.rs  # NEW (FR-449 invariant)
    ├── sync_algorithm.rs            # NEW
    ├── sync_idempotence.rs          # NEW
    ├── summariser_stub.rs           # NEW
    ├── summariser_triggers.rs       # NEW
    ├── summariser_forward_progress.rs  # NEW
    ├── summariser_cache.rs          # NEW
    ├── summariser_real.rs           # NEW — CI-skipped (TOME_TEST_REAL_MODELS=1)
    ├── migration_v1_to_v2.rs        # NEW
    ├── doctor_p4.rs                 # NEW
    ├── catalog_workspace_refcount.rs  # NEW
    ├── plugin_workspace_skills.rs   # NEW
    ├── plugin_cheap_reenable.rs     # NEW
    ├── catalog_update_cross_workspace_reindex.rs  # NEW
    ├── atomic_dir.rs                # NEW
    ├── no_directories_imports.rs    # NEW — structural test
    └── fixtures/
        ├── (Phase 1–3 fixtures carry forward)
        └── (no new committed binary fixtures — every v1 DB / harness dir layout / settings file is bootstrapped in-line per the generate-at-setup discipline)
```

**Structure Decision**: Same single binary crate. Phase 4 adds four new capability modules (`summarise`, `harness`, `settings`, `util`), refactors `paths`, `error`, `workspace`, `index`, `commands/workspace`, `commands/catalog`, `commands/plugin`, `commands/query`, `commands/reindex`, `commands/status`, `commands/doctor`, `commands/mcp`, and `doctor`, and adds the new `commands/harness/` directory. The Phase 3 `src/workspace/inventory.rs` is deleted. Async lives strictly inside `src/mcp/`; no new exemption to the sync-boundary test. No workspace split — still one crate, ~12–13 kLOC total after Phase 4, below the split threshold.

## Pre-emptive slice plans

Per the P10 retro "encode pre-emptive slice splits in the plan" recommendation, Phase 2 (`/sdd:tasks`) of this SDD pipeline will use the slice shapes below as the baseline. `/sdd:tasks` is free to refine; this is the planning intent.

- **Foundational F1–F10** (one PR per slice; no user-story label):
  - **F1**: Constitution v1.3.0 amendment.
  - **F2**: Reshape `Paths` (drop the XDG-separated fields; introduce `paths::home_root()` + accessors under `<home>/.tome/`). Mechanical sweep of every call site. Add the forward-looking structural test `tests/no_directories_imports.rs`. (Per R-1 framing correction, no `directories` dep is removed because none was ever added; F2 is a reshape + sweep, not a dep removal.) Reviewer recommendation: split into F2a (`Paths` reshape; field renames; XDG accessor retirements; keep `Scope` shape) and F2b (`Scope` reshape from `Global | Workspace(PathBuf)` to `Scope(WorkspaceName)`, paired with F10's `WorkspaceName` introduction) for compile-incrementality. Decided during F1's scaffolding pass.
  - **F3**: Pre-allocate all 8 new `TomeError` variants (codes 13–20) + the reused-variant `match` arms (FR-602).
  - **F4**: Promote the atomic-populated-directory helper to `src/util/atomic_dir.rs` (R-10).
  - **F5**: Add `toml_edit` + enable `serde_json/preserve_order`; sweep audit.
  - **F6**: Bootstrap `src/summarise/` skeleton (trait, `LlamaBackend` singleton, `StubSummariser`, model registry extension with byte-progress download), no production wiring yet.
  - **F7**: Add `src/harness/` skeleton (`HarnessModule` trait, no impls), `src/harness/rules_file.rs`, `src/harness/mcp_config.rs`.
  - **F8**: Add `src/settings/` (composition parser + cycle detection + StubScope fixture).
  - **F9**: Register `phase_4_v1_to_v2` in `MIGRATIONS`; bootstrap path emits v2 directly; delete Phase 3 synthetic `SuggestedFix` injection from `tests/doctor.rs`; add `tests/migration_v1_to_v2.rs`.
  - **F10**: `WorkspaceName` newtype + reserved-word check; `workspace_projects` PK on `project_path` alone; sweep audit.

- **US1 — Bind a project to a workspace** (4 slices):
  - **US1.a**: `tome workspace use <name>` command + atomic project marker landing + workspace-projects UPSERT + advisory-lock contract.
  - **US1.b**: Harness sync inside the bind command; placeholder harness module trait usage; tests via `StubHarness`.
  - **US1.c**: First production harness module (`claude_code`); bind now writes real rules-file block + MCP config entry for Claude Code only.
  - **US1.d**: Cross-product tests (pre-state combinations); error envelope for harness-clash; closeout + retro.

- **US2 — Manage workspace lifecycle** (3 slices):
  - **US2.a**: `init` + `list` + `info` + `rename` + `regen-summary` (uses `StubSummariser` from F6).
  - **US2.b**: `remove` with cascade ordering + reserved-name check + bound-project rejection + override-flag cascade.
  - **US2.c**: `sync` (per-workspace + all-workspaces); closeout + retro.

- **US3 — Layered settings + composition** (3 slices):
  - **US3.a**: Settings parser + composition resolver + cycle detection (pure compute, all library API).
  - **US3.b**: `[workspace]` valid-only-in-project enforcement + `!`-prefix validation + harness-not-supported.
  - **US3.c**: `tome harness list` + `tome harness use` + `tome harness remove` + scope annotation. Closeout + retro.

- **US4 — Summarisation + RULES.md** (3 slices):
  - **US4.a**: Production `LlamaSummariser` + prompts module + length-window enforcement (warnings) + one CI-skipped real-model integration test.
  - **US4.b**: Trigger wiring (enable/disable/reindex/catalog-update + FR-385 forward-progress) + MCP server cached-short-summary readout.
  - **US4.c**: `regen-summary` command + closeout + retro.

- **US5 — Doctor extensions** (2 slices):
  - **US5.a**: Doctor reports binding + project-rules-copy + per-harness rules + per-harness MCP + summariser. `Subsystem` enum promotion.
  - **US5.b**: `--fix` handlers for supported repair classes; user-owned MCP-conflict is the explicit-override case; closeout + retro.

- **Polish phase** (P9): four-reviewer parallel pass (contract audit, Rust-lens, test audit, security audit). Apply blockers + majors before declaring v0.4.0.

Each slice: ≤ ~400 lines, single theme, single PR. Foundational pre-allocates everything later slices depend on (per P10 retro). User-story slices each ship end-to-end value (US1 lands incrementally with one harness, then fans out).

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|---|---|---|
| Bundle a third inference runtime (`llama-cpp-2` + Qwen2.5-0.5B-Instruct, ~6 MB binary + ~400 MB on-disk model) | Phase 4 generates two cached natural-language summaries per workspace from current plugin state. The short summary is interpolated into the MCP search tool's description so the host LLM knows when to call `search_skills`; the long summary becomes the workspace's rules-file body so the agent gets a brief workspace overview at session start. Both must be deterministic-enough-across-machines, offline-friendly, and current to the actual enabled-plugin state. A bundled local model is the only design that meets all three. The complexity-budget rule (Governance §Complexity budget) is met: the bundled summariser earns its keep by enabling the headline cross-harness integration story. | External API (OpenAI / Anthropic / Gemini): rejected — offline-first principle (FR-427); API-key burden for every user including CI; latency variance across network conditions; non-deterministic output across runs. Larger bundled model (Qwen2.5-1.5B, Phi-3-mini): rejected — triples to quintuples the on-disk footprint; quality gain marginal for short structured summarisation. Pure-Rust inference (`candle`, `mistral.rs`): rejected — significantly slower per token at INT4 quantisation; project narrower (e.g. no Qwen2.5 family support in `mistral.rs`); larger binary surface. |
| Constitution v1.2.0 §Paths Operational Constraint amended via v1.3.0 | Phase 4's central architecture (one root, one DB) is the architectural correction the spec explicitly carries from the PRD. The constitution's §Paths Operational Constraint ("XDG-aware via `directories`. Never hardcode `~/.tome`") was written under the Phase 1 assumption that XDG separation matched user expectations AND that `directories` would be the resolution dep. Phase 3's `src/paths.rs:14-21` documents the actual deviation: Tome never adopted `directories` and resolved paths through raw env vars throughout Phases 1–3. Phase 4 ratifies this by amending the constraint to: `<home>/.tome/` root, raw-env-var home resolution, no XDG separation. Constitution v1.3.0 lands the rewrite. | Keeping the v1.2.0 §Paths constraint and finding a way around it: rejected — that's the "constitution-as-decoration" failure mode the §Compliance rule exists to prevent. The constitution is the source of truth; when it's wrong, amend it (per §Amendments) — don't violate it and hope no one notices. The reviewer's surfacing of the documentation/code mismatch in Phase 3 (R-1 framing correction) made this amendment necessary regardless of Phase 4's architectural shift; the shift makes it timely. |

## Phase 2: Dev environment status

Tome's local development tooling is mature and already in place:

- **Linting / formatting**: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `typos` — enforced in `.githooks/pre-commit`.
- **Type checking**: `cargo check` (sufficient for Rust; no separate `tsc` analogue).
- **Testing**: `cargo test`, plus integration suites in `tests/`. Pre-push hook runs the full suite.
- **Conventional Commits**: `cocogitto`'s `cog verify --file <commit-msg-file>` in the `commit-msg` hook.
- **Security**: `cargo-audit` and `cargo-deny check` in weekly + on-PR CI.
- **CI**: `ci.yml` matrix `{macos-latest, ubuntu-latest} × {stable, MSRV}`.

**Phase 4 additions**:

- F1's CI workflow check ensures the new dep matrix (`llama-cpp-2`, `toml_edit`) compiles on both platforms.
- F2 adds `tests/no_directories_imports.rs` — a structural grep test (same shape as Phase 3's `sync_boundary.rs`) asserting no production source imports the `directories` crate.
- US4.a adds an env-gated integration test `tests/summariser_real.rs` for end-to-end real-model verification on developer machines; CI-skipped by default.

No new tooling required. Phase 2 (dev environment) is **PASS without changes** for Phase 4 — the existing pipeline absorbs the new deps and tests cleanly.

## Plan history

| Date | Event |
|---|---|
| 2026-05-14 | Initial plan written (this commit). Phase 0 research complete (19 R-decisions). Phase 1 artefacts generated (data-model.md, 13 contracts, quickstart.md). Constitution gate: PASS with two deviations (bundled summariser, v1.3.0 §Paths amendment); both documented in Complexity Tracking. |
