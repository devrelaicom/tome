# Tome — Claude Code Project Context

This file gives Claude Code persistent context about the Tome project. Keep it terse.
Deep per-phase / per-US detail lives in the retros (`specs/*/retro/P*.md`) and git history — link to it, don't duplicate it here.

## Project

**Tome** is a Rust CLI (and MCP server) that makes Claude Code's plugin ecosystem work across other agentic coding harnesses (Cursor, Codex, Gemini CLI, OpenCode, …).

- **Current phase:** **Phase 6 planning** on the `006-phase-6-hooks-agents` branch (spec + plan complete; no code yet). Phase 6 closes out hooks + agents. **Phase 5 shipped (v0.5.0)**; all 5 USs feature-complete, phase-wide 4-reviewer pass applied (0 blockers, 7 majors, security clean); v0.5.0 git tag reserved for user push (`cargo publish` + release-notes are user-reserved per the constitution). Phase 4 (v0.4.0) shipped 2026-05-26.
- **Phase 6 will add:** real Claude Code hooks (rewrite + structural-match merge into `.claude/settings.local.json`); a `GUARDRAILS.md` prose fallback rendered as per-plugin marker regions in each harness's rules file; native agent translation across four harnesses (claude-code/codex/cursor/opencode); optional agent-as-MCP-prompt personas (off by default); and the Phase 4 correction making Claude Code's rules sink `CLAUDE.md`, not `AGENTS.md`. Adds no new top-level dependency; 4 new exit codes (43–46); `EntryKind` gains an `Agent` variant.
- **Phase 5 added:** commands as first-class entries alongside skills (`kind` discriminator); user-invocable entries exposed as MCP prompts; hand-rolled variable substitution layer (built-ins + env passthrough + Claude Code-compatible argument substitution, single-sweep regex enforcing the NFR-007 no-rescan invariant); middle-tier `get_skill_info` MCP tool; `when_to_use` frontmatter indexed for embedding. No new top-level dependencies (`regex` promoted transitive→direct at phase start).

### Pointers

- **PRDs:** [`PRDs/phase-1.md`](./PRDs/phase-1.md) · [`phase-2.md`](./PRDs/phase-2.md) · [`phase-3.md`](./PRDs/phase-3.md) · [`phase-4.md`](./PRDs/phase-4.md) · [`phase-5.md`](./PRDs/phase-5.md) (shipped) · [`phase-6.md`](./PRDs/phase-6.md) (planning).
- **Constitution:** [`CONSTITUTION.md`](./CONSTITUTION.md) (v1.3.0; Phase 6 introduces no amendments — zero new top-level deps, zero new top-level modules).
- **Active spec/plan:** [`specs/006-phase-6-hooks-agents/`](./specs/006-phase-6-hooks-agents/) — `spec.md`, `plan.md`, `research.md` (20 R-decisions), `data-model.md`, `contracts/` (9), `quickstart.md`. `tasks.md` pending `/sdd:tasks`.
- **Frozen specs:** `specs/00{1,2,3,4,5}-*/` for Phases 1–5 (each with its own design artefacts + retros).
- **Codebase docs:** [`.sdd/codebase/`](./.sdd/codebase/) — 8 documents, refreshed at each phase boundary via `/sdd:map incremental`.
- **Retros:** each phase's `retro/P*.md` holds the deep per-US detail (decisions, workarounds, patterns, deferred items).

### Command surface (all shipped)

- `tome catalog {add,remove,list,update,show}` — `remove --force` cascades plugin disable.
- `tome plugin {enable,disable,list,show}` + bare `tome plugin` (interactive catalog→plugin→action). `show`/`list` group Skills/Commands with per-entry annotations.
- `tome query` — KNN + reranker; `--strict`.
- `tome models {download,list,remove}` — against pinned `MODEL_REGISTRY`.
- `tome reindex [<scope>] [--force]` — scope is omitted | `<catalog>` | `<catalog>/<plugin>`.
- `tome status [--verify] [--json]` — read-only pre-flight; never takes the advisory lock.
- `tome doctor [--fix] [--force] [--verify] [--json]` — reports + repairs every subsystem; FR-124 read-only by default.
- `tome workspace {use,init,list,info,rename,regen-summary,remove,sync}`.
- `tome harness {<bare>,list,use,remove,info,sync}` — layered settings + composition resolver.
- `tome mcp` — the async island (`src/mcp/`); tools `search_skills` + `get_skill` + `get_skill_info`, plus user-invocable entries as MCP prompts.

## Active Technologies

### Phase 1 (shipped, unchanged)

- **Language**: Rust stable, MSRV `rust-version = "1.93"` (verified in CI).
- **CLI**: `clap` (derive). **Config/manifest**: `serde` + `toml` (Tome-owned structs are `#[serde(deny_unknown_fields)]`).
- **Errors**: `thiserror` (closed `TomeError` enum → exit codes); `anyhow` at the app boundary.
- **Logging**: `tracing` + `tracing-subscriber` (stderr; orthogonal to `--json`).
- **Hashing**: `sha2`, `hex`. **Atomic writes**: `tempfile`. **Signals**: `ctrlc` (SIGINT → exit 8).
- **Regex**: `regex` (credential scrubbing; direct dep since Phase 1). **Time**: `time`. **Semver**: `semver`. **Colour**: `anstream`/`anstyle`.

### Phase 2+ additions

- **DB**: `rusqlite` (`bundled`). **Vector search**: `sqlite-vec` C ext vendored under `vendor/sqlite-vec/`, compiled via `build.rs`.
- **Inference**: `fastembed-rs` over `ort` (ONNX Runtime, CPU only). **Models** (runtime-downloaded, MIT): `bge-small-en-v1.5` INT8 (~45 MB), `bge-reranker-base` INT8 (~280 MB).
- **Summariser (Phase 4)**: `llama-cpp-2 = "=0.1.146"` + `encoding_rs` inside `src/summarise/` (sync); Qwen2.5-0.5B-Instruct.
- **Async (Phase 3)**: `tokio` inside `src/mcp/` only — the one async island, enforced by `tests/sync_boundary.rs`. MCP via `rmcp` + `schemars`.
- **TOML edits**: `toml_edit` (comment/order-preserving surgical edits). **JSON**: `serde_json` (`preserve_order`).
- **UX**: `indicatif`, `comfy-table`, `owo-colors`, `inquire`. **HTTP**: `reqwest` (`blocking` + `rustls-tls`).

**Strictness boundary** (FR-013a): `deny_unknown_fields` on Tome-owned inputs (config, model `manifest.json`, index `meta`). Third-party inputs (`plugin.json`, SKILL.md frontmatter) parse leniently.

## Architectural Constraints (from the constitution)

- **Sync only**, except the `src/mcp/` async island. `reqwest::blocking`, `rusqlite`, `fastembed-rs`, `llama-cpp-2` are all sync.
- **Inherit `git`** — shell out to system `git`; never vendor a Git library.
- **Closed error set** — `TomeError` has no `Other`/`Unknown` arm; every failure class has its own variant + exit code.
- **Atomic writes** — registry, cache, models dir, index DB. SQLite WAL + Tome-owned advisory lockfile (`index.lock`) for the index concurrency contract (FR-040).
- **Credential scrubbing at the boundary** — `git::scrub_credentials` extends to model/summariser download URLs and `reqwest` error chains.
- **50 MB binary cap** (revised from 10 MB 2026-05-13). CI asserts release binary size on Linux. Profile: `lto = "thin"`, `panic = "abort"`, `strip = "symbols"`. The discipline is non-waivable (NFR-001); the number is sized to current reality + headroom.
- **Licence allowlist** — enforced by `cargo-deny`. Downloaded models surfaced in `tome models list`.

## Conventions

- **Commits**: Conventional Commits, enforced by `cocogitto` (`cog verify`) in the `commit-msg` hook. Format `type(scope): subject`.
- **Branching**: trunk-based; short-lived branches off `main`. **PRs**: small batches (~400 lines / 2 modules soft cap).
- **Comments**: explain *why*, not *what*. Reader knows Rust.
- **Modules**: capability-organised — `catalog`, `config`, `paths`, `error`, `output`, `logging`, `plugin`, `index`, `embedding`, `presentation`, `workspace`, `mcp` (async), `summarise`, `harness`, `settings`, `substitution`, `util`.
- **Errors**: `thiserror` inside modules; `anyhow` at the app boundary.

### Established patterns (detail in the retros)

- **Silent compute / emit wrapper** — split a command into `pipeline(args, deps) -> Result<Outcome>` (no I/O) + a thin `run_with_deps` that emits per mode, so MCP / library callers reuse the compute path. (P3 US1.b)
- **Test injection via `#[doc(hidden)] pub static` + RAII guard** — integration tests under `tests/` can't see `#[cfg(test)]`; gate override slots with `#[doc(hidden)] pub static` and pair with a `Drop`-clears guard (`MigrationsGuard`, `HarnessModulesGuard`, `SummariserOverrideGuard`, `HomeGuard`/`HOME_MUTEX`, `EnvVarGuard`/`ENV_MUTEX`).
- **Atomic populated-directory landing** — build in a sibling `.tome.tmp.*` staging dir, `keep()` + `std::fs::rename` (POSIX-atomic same-FS). `util::atomic_dir`.
- **Reuse closed-set error variants** over promoting new ones when a failure maps semantically onto an existing variant+code.
- **Specific-over-generic exit codes** — emit the matching domain variant, not a generic residual.
- **Single-source-of-truth promotion at the second consumer** — promote a helper/accessor to `pub`/`pub(crate)` rather than duplicate (`check_model`/`check_index`/`check_drift`, `resolve_entry_body_path`, `validate_db_stored_path`, `plugin_data_root`, `MCP_SLASH_PREFIX`, `build_context_for_entry`).
- **Reference-counted shared on-disk resources** — content-addressed catalog clones deleted only at refcount 0 (`catalog::store::reference_count`); unlocked, benign TOCTOU.
- **`spawn_blocking` for sync work inside async MCP handlers** — keep the single-threaded runtime responsive.
- **Bounded `char_indices` walk** for caller-controlled string truncation (avoids full-string + double-pass DoS amplifier).
- **Canonical enum dispatch** over stringly-typed `match kind.as_str()` — surfaces schema drift as `IndexIntegrityCheckFailure`.
- **Byte-stable JSON wire-shape pin tests** for emit-only `Serialize` records (no `deny_unknown_fields` — boundary is inputs only).
- **Marker-only migration for free-text domain widening** — when a free-text TEXT column admits a new value (no DDL needed), register a `Migration` whose `apply` is a documented no-op that only advances `SCHEMA_VERSION`, so the migration registry + doctor's schema check stay monotonic and auditable (P6/F2: `kind` domain widened to `agent`, v3→v4).
- **Validate third-party strings as a safe path segment before composing them into a path** — reject empty/NUL/`/`/`\`/`.`/`..`/leading-dot and require exactly one `Component::Normal` at the *boundary*, then add a defensive `target.parent() == Some(dir)` check at the write site (`harness::agents::is_safe_agent_name`, P6/US1 — closed a path-traversal via plugin-supplied agent `name`). Applies wherever plugin/frontmatter-supplied text becomes a filename.
- **`reconcile_<sink>` sync template** (P6 `reconcile_hooks`/`reconcile_guardrails`/`reconcile_agents`) — open the central DB read-only and **propagate** the error for an existing DB (never `.ok()`-swallow → that empties the enabled set and mass-deletes), compute shared inputs once, per-harness loop, an actions map + `first_error` forward-progress (surfaced after the prior sink's error), and a `SyncSubsystem` variant + `<sink>_action` field appended **LAST** so the byte-stable JSON pin only gains a trailing field. Fixed sink order hooks → guardrails → agents. Fail closed (don't `to_string_lossy`) on a non-UTF-8 path that becomes an executed command.
- **Validate verbatim third-party content for managed-marker collisions at the boundary** — when plugin/frontmatter content is copied *verbatim* into a marker-delimited region of a file Tome re-parses, scan each line against ALL managed-marker regexes (the region's own + the `tome:begin/end` block) and **fail closed** (`harness::guardrails::body_contains_marker_line`, P6/US3 — closed region-escape/file-wedge/rules-block corruption). Escaping is wrong for verbatim content; refusal is the defence. Security companion to the path-segment-validation rule above.
- **Mirror the sibling query's full column projection when adding a parallel row path** — a new consumer reading the `skills` table (e.g. the P6/US4 persona path vs the command/skill registry query) must SELECT the same load-bearing columns (`plugin_version`, `indexed_at`, …); omitting them is silent (empty strings) and degrades downstream behaviour (substitution built-ins rendering empty, collision tie-breaks biased). Two US4 reviewer majors traced to one missing projection.
- **Doctor = read-only projection over the reconcilers; `--fix` = re-run the idempotent reconciler** — doctor check fns re-read the same on-disk/source/index state the writers produced (no writes, no dir creation, FR-124); `--fix` re-runs `sync_project` (or the relevant idempotent op) so it inherits all the writer's safety (structural-match-only removal, marker-bounded edits, symlink refusal) rather than re-implementing a destructive path. Refresh ALL affected report surfaces after the fix, gated on "the op ran", not "my branch ran it" (P6/US5 C5-1).
- **Per-US closeout discipline** — 4-reviewer parallel pass (contract / Rust-lens / test / security) dispatched in ONE message; findings + disposition committed *before* applying fixes; then `/sdd:map incremental` refresh + retro fill + this CLAUDE.md update.

## Common Commands

```sh
# Build / run
cargo build                                      # debug
cargo build --release                            # release (CI binary-size check)
cargo run -- catalog list                        # run a subcommand from source

# Quality gates (also in .githooks/pre-commit)
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
typos

# Tests (CI runs the full suite + build matrix; pre-push mirrors pre-commit)
cargo test                                       # all (stub embedder — fast, no model files)
cargo test --test catalog_add                    # one integration test file
cargo test catalog_add::                         # one test by path

# Security / dependency hygiene
cargo audit
cargo deny check

# Conventional Commits
cog verify --file <commit-msg-file>

# Git hooks (versioned under .githooks/; no external manager)
git config core.hooksPath .githooks              # one-time, per clone
.githooks/pre-commit                              # run the pre-commit chain manually

# MSRV verification (CI uses rust-version from Cargo.toml)
cargo +<MSRV> build
```

## File Structure

Capability-organised under `src/`. Top-level orientation:

```
src/
├── main.rs / lib.rs / cli.rs / error.rs / config.rs / paths.rs / output.rs / logging.rs
├── catalog/        # manifest, store (atomic registry), git (shell-outs + scrub_credentials)
├── commands/       # one module per command surface (catalog, plugin/, query, models/, reindex,
│                   #   status, doctor, workspace/, harness/, mcp)
├── plugin/         # manifest (lenient), frontmatter, components, identity, lifecycle (atomic per plugin)
├── index/          # db, schema, migrations (forward-only under lock), vec_ext, skills, query, meta,
│                   #   integrity, lock, workspaces, workspace_catalogs
├── embedding/      # Embedder + Reranker traits, fastembed impl, #[cfg(test)] stub, registry, download, runtime
├── presentation/   # comfy-table / indicatif / owo-colors / inquire wrappers
├── workspace/      # scope / resolution / inventory / init / binding / rename / sync
├── settings/       # layered settings + composition resolver + surgical toml_edit
├── harness/        # sync orchestrator, rules_file, mcp_config, per-harness modules (claude_code, codex, …)
├── summarise/      # Summariser trait, LlamaSummariser, StubSummariser (sync, src-local)
├── substitution/   # mod, context, builtins, env, arguments, data_dir, regex_sets (single-sweep)
├── mcp/            # the async island: mod, runtime, log, preflight, server, state, tools/, prompts
└── util/           # atomic_dir, bounded io

vendor/sqlite-vec/  # pinned C source + LICENSE; compiled via build.rs
tests/              # integration tests, one file per surface; tests/common/mod.rs helpers; tests/fixtures/
```

## Recent Changes

Terse changelog. Deep detail in each phase's `retro/P*.md` and in git history / PR descriptions.

- **2026-05-28 — Phase 6 planning (spec + plan).** `/sdd:specify` → `specs/006-phase-6-hooks-agents/spec.md` (5 USs, 54 FRs / 11 NFRs / 11 SCs, 0 clarification markers; Rust-lens review folded in 2 blockers + 10 majors + 6 minors). `/sdd:plan` → `plan.md`, `research.md` (20 R-decisions), `data-model.md`, 9 contracts, `quickstart.md`. Constitution gate PASS, no amendments (0 new top-level deps, 0 new top-level modules — leanest phase since Phase 1). Phase 6 = hooks (real Claude Code JSON hooks + `GUARDRAILS.md` prose fallback) + agents (native translation across claude-code/codex/cursor/opencode + optional MCP-prompt personas) + the Phase 4 correction (Claude Code rules sink → `CLAUDE.md` not `AGENTS.md`). New exit codes 43–46 (PRD-proposed 30–33 collided with the model-on-disk cluster). `EntryKind` gains `Agent`; no new schema columns/tables. `tasks.md` pending `/sdd:tasks`.
- **2026-05-27 — Phase 5 shipped (v0.5.0).** Polish phase across PR-A→E on `phase-5-polish`; phase-wide 4-reviewer pass (0 blockers, 7 majors, security clean). All 5 USs feature-complete: commands as MCP prompts (US1), substitution layer paths/env (US2), argument substitution (US3), `get_skill_info` + `when_to_use` indexing (US4), per-entry invocability flags + doctor extensions (US5). Substitution layer complete — 4 stages via single-sweep regex, NFR-007 enforced structurally. Test total 954 → 1194 across 127 → 151 suites. No new top-level deps. See `specs/005-*/retro/P{3..8}.md`.
- **2026-05-26 — Phase 4 shipped (v0.4.0).** PRs #59–#108. `Paths` collapsed under `<home>/.tome/`; central SQLite DB; `WorkspaceName` newtype + `Scope`; `--global` deleted; `workspace_catalogs`/`workspace_skills` junctions as sources of truth. Workspace bind (US1), lifecycle commands (US2), layered settings + composition resolver + `tome harness` (US3), bundled local-LLM summarisation (US4), `tome doctor` extensions (US5). New deps: `llama-cpp-2`, `encoding_rs`, `toml_edit`, `filetime` (dev). See `specs/004-*/retro/P{2..8}.md`.
- **2026-05-14 — Phase 3 shipped (v0.3.0).** MCP server + workspaces. New async island under `src/mcp/` (`rmcp`, `tokio`, `schemars`); `tome workspace {info,init}`, `tome doctor`, forward schema-migration framework, per-command scope honouring + reference-counted catalog clones. See `specs/003-*/retro/P*.md`.
- **2026-05-13 — Phase 2 shipped (v0.2.0).** Plugin lifecycle + semantic index. `rusqlite`/`sqlite-vec`/`fastembed-rs`; `tome plugin {enable,disable,list,show}`, `tome query`, `tome models`, `tome reindex`, `tome status`, catalog cascade. Binary cap revised 10 MB → 50 MB. See `specs/002-*/retro/P*.md`.
- **2026-05-11 — Phase 1 shipped (v0.1.0).** Catalog foundations + constitution v1.0.0. See `specs/001-*/`.
