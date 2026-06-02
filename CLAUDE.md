# Tome — Claude Code Project Context

This file gives Claude Code persistent context about the Tome project. Keep it terse.
Deep per-phase / per-US detail lives in the retros (`specs/*/retro/P*.md`) and git history — link to it, don't duplicate it here.

## Project

**Tome** is a Rust CLI (and MCP server) that makes Claude Code's plugin ecosystem work across other agentic coding harnesses (Cursor, Codex, Gemini CLI, OpenCode, …).

- **Current phase:** **Phase 7 beta-hardening complete** — *Beta hardening + public release*. All ~24 implementation slices (#138–#165) merged to `main` (decomposition + US1 beta-gate incl. two discovered model blockers + US2 robustness + the in-process MCP harness + the release wrapper); the P9 phase-wide review closed 0 blockers / 2 majors-fixed. **Pending (user-reserved):** the final FR-024 untracking of `specs/`/`.sdd/`/`CLAUDE.md`/`review/`/`retro/` (`CONSTITUTION.md` stays tracked), and the publish hard-stops — `cargo publish` (as `tome-mcp`), the `v0.6.0` tag, the Homebrew-tap-PR merge + tap PAT, release-notes, and enabling GitHub private vuln reporting. The crate version stays **0.6.0** (the first PUBLIC release = Phase 6 features + Phase 7 hardening). Phase 5 (v0.5.0) 2026-05-27; Phase 4 (v0.4.0) 2026-05-26.
- **Phase 6 added:** real Claude Code hooks (rewrite + structural-match merge into `.claude/settings.local.json`); a `GUARDRAILS.md` prose fallback rendered as per-plugin marker regions in each harness's rules file (+ a Tome-owned Cursor sibling); native agent translation across four harnesses (claude-code/codex/cursor/opencode; Gemini has no native agents); optional agent-as-MCP-prompt personas (off by default); and the Phase 4 correction making Claude Code's rules sink `CLAUDE.md`, not `AGENTS.md`. No new top-level dependency; 4 new exit codes (43–46); `EntryKind` gained an `Agent` variant (schema v3→v4, marker-only migration).
- **Phase 5 added:** commands as first-class entries alongside skills (`kind` discriminator); user-invocable entries exposed as MCP prompts; hand-rolled variable substitution layer (built-ins + env passthrough + Claude Code-compatible argument substitution, single-sweep regex enforcing the NFR-007 no-rescan invariant); middle-tier `get_skill_info` MCP tool; `when_to_use` frontmatter indexed for embedding. No new top-level dependencies (`regex` promoted transitive→direct at phase start).

### Pointers

- **PRDs:** [`PRDs/phase-1.md`](./PRDs/phase-1.md) · [`phase-2.md`](./PRDs/phase-2.md) · [`phase-3.md`](./PRDs/phase-3.md) · [`phase-4.md`](./PRDs/phase-4.md) · [`phase-5.md`](./PRDs/phase-5.md) (shipped) · [`phase-6.md`](./PRDs/phase-6.md) (shipped).
- **Constitution:** [`CONSTITUTION.md`](./CONSTITUTION.md) (v1.3.0; Phase 6 introduced no amendments — zero new top-level deps, zero new top-level modules).
- **Phase 7 spec/plan (planning):** [`specs/007-phase-7-beta-release/`](./specs/007-phase-7-beta-release/) — `spec.md` (5 USs, 26 FRs / 10 NFRs / 11 SCs), `plan.md`, `research.md` (24 R-decisions), `data-model.md`, `contracts/` (10), `quickstart.md`, `checklists/requirements.md`; `tasks.md` pending `/sdd:tasks`. Source of WHAT = the two beta-readiness audits ([`CODE-REVIEW.md`](./CODE-REVIEW.md), [`RELEASE-READINESS.md`](./RELEASE-READINESS.md)).
- **Phase 6 spec/plan (shipped):** [`specs/006-phase-6-hooks-agents/`](./specs/006-phase-6-hooks-agents/) — `spec.md`, `plan.md`, `research.md` (20 R-decisions), `data-model.md`, `contracts/` (9), `quickstart.md`, `tasks.md` (T001–T159 complete), `review/` (per-US + phase-wide findings/disposition) + `retro/P{2..8}.md`.
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
- **Phase-wide review at Polish** — after all USs merge, a second 4-reviewer pass over the *assembled* surface catches cross-US drift the per-US passes structurally can't (proven P5/P8 + P6/P8): e.g. an exit-code policy fixed on one sink (guardrails symlink 7→46) but not its parallel sink (hooks, fixed 7→44 at P8), or a multi-sink error-precedence path no per-US test covered. Same discipline — findings + disposition committed before fixes.
- **Keep `[lib] name`/`[[bin]] name` = the original on a `[package]` rename** — when publishing under a new crate name (P7/REL1 `tome`→`tome-mcp`), pin `[lib] name = "tome"` so every `use tome::` import compiles unedited (705 sites, zero churn) and `[[bin]] name = "tome"` so the installed binary stays `tome`. Only `Cargo.toml`/`Cargo.lock` get a name-only change. The package name is a registry/discovery concern; the lib/bin names are the API/UX surface — decouple them.
- **Run the real-model / `#[ignore]`d gates before any release** — stub-only fast CI (`StubEmbedder`) is fast and deterministic but structurally cannot exercise on-disk model download + real ONNX inference. Running the SC-001 real-model recall gate for the FIRST time at P7 surfaced TWO stacked beta-blockers that had silently broken the headline `tome query` feature since Phase 2 (F-MODEL-FILES: only the primary `.onnx` downloaded, never `tokenizer.json`; F-MODEL-ONNX-CPU: a GPU/fp16 artefact failing CPU inference). The heavy `--ignored` gates (`model_download_complete`, `reranker_cpu_inference`, `search_knn_recall_realmodel`) are out of fast CI by design — they are a *release* gate, not a per-PR one. Run them with `--ignored` before publishing.

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

- **2026-06-02 — Phase 7 shipped (beta hardening + public-release wrapper).** All ~24 slices (#138–#165) merged branch-per-slice → CI-green → squash-merge. **Decomposition (lands first):** `harness/sync.rs` → `src/harness/reconcile/{hooks,guardrails,agents}.rs` behind a thin orchestrator (behaviour-preserving; sink order + first-error precedence byte-identical). **US1 beta-gate (5 K-fixes):** F-KNN over-fetch+widen (`index/query.rs`, `OVER_FETCH_MULTIPLIER=4` then geometric widen to corpus size), F-DOCTOR-RW read-only-open+degrade (`doctor/checks.rs`), F-CACHE-KEY scrubbed-URL cache+refcount keying (`catalog/add.rs`), F-MCP-PROMPT-COLLISION global taken-set (`mcp/prompt_collision.rs`), F-WS-TOML-NEWLINE `toml_edit`+control-char reject (`workspace/init.rs` + `catalog/manifest.rs`). **Two DISCOVERED model beta-blockers** (caught running the SC-001 real-model gate for the FIRST time — stub-only testing had hidden them since Phase 2): F-MODEL-FILES (`download_model` fetched only the primary `.onnx`, never `tokenizer.json` → real embedder couldn't load → `tome query` non-functional; fixed via additive `aux_urls` + a download loop) and F-MODEL-ONNX-CPU (the pinned embedder was a GPU/fp16/fused ONNX that failed CPU inference; re-pinned to the Xenova bge-small CPU INT8 artefact, name/version/`files` unchanged → no index drift). Real-model semantic search verified end-to-end. **US2 robustness (4):** bounded third-party reads (FR-006 per-class caps), SSOT symlink-safe write guard across ALL sinks + intermediate-component refusal + per-sink exit codes (FR-007; `rustix` promoted transitive→direct, NO new package), OpenCode inline-rules union (FR-008), catalog-remove TOCTOU re-derive-under-lock (FR-009). **US3:** in-process MCP test harness (`tests/common/mcp_harness.rs` + `tests/exit_codes_e2e_mcp.rs`, closes `GAP-1`, verifies the K4 fix e2e) + correctness cleanups (config→exit 5, hooks fail-closed→44, dup-`(kind,name)` warn+truthful count) + doc hygiene. **Release wrapper:** crate rename `tome`→`tome-mcp` (`[lib]`/`[[bin]]` name stay `tome` — 705 `use tome::` sites unedited), discovery metadata + docs.rs config, cargo-dist pipeline (`dist-workspace.toml` + tag-only `release.yml`, **wired NOT triggered**), README front door + `SECURITY.md`. **P9 phase-wide review:** 0 blockers, 2 majors both FIXED — MAJOR-1 agents cleanup-removal symlink refusal now returns its dedicated exit 45 not `Io` 7 (CON-1, #164); MAJOR-2 the unverified aux model-file download is now 64-MiB-capped + the `SECURITY.md` verification claim corrected (#165). **Invariants held throughout:** NO new top-level dependency (`rustix` transitive→direct), NO new top-level module, NO schema change (`SCHEMA_VERSION` stays 4), NO new exit code (every fix reused an existing `TomeError` variant). **User-reserved (NOT done):** `cargo publish`, the `v0.6.0` tag, the Homebrew-tap-PR merge + tap PAT, release-notes, enabling GitHub private vuln reporting; plus the final FR-024 untracking. The crate version stays **0.6.0** (the first PUBLIC release). **Discovered fast-follow (handoff, NOT a Phase-7 task):** the scoped catalog-discovery commands (`plugin list --catalog` / `plugin show` / `reindex <catalog>`) still read the deprecated `config.toml [catalogs]` → exit 3 on an enrolled catalog (medium severity, bare forms work). See `specs/007-*/retro/P{4,5,7,8,9}.md` + `review/`.
- **2026-06-01 — Phase 7 planning (spec + plan).** `/sdd:specify` → `specs/007-phase-7-beta-release/spec.md` (5 USs, 26 FRs / 10 NFRs / 11 SCs, 0 clarification markers; one documented implementation-aware deviation; Rust-lens reviewed). `/sdd:plan` → `plan.md`, `research.md` (24 R-decisions), `data-model.md`, 10 contracts, `quickstart.md`. **No PRD** — the two beta-readiness audits (`CODE-REVIEW.md` 13 findings, `RELEASE-READINESS.md`) are the source of WHAT. Phase 7 = beta hardening (F-KNN over-fetch+widen, read-only `doctor`, scrubbed-URL cache key, MCP prompt-collision global taken-set, ws-toml control-char reject, bounded third-party reads, **intermediate-component symlink guard via `rustix` transitive→direct [NO new package]**, OpenCode rules inline-body fix, catalog-remove TOCTOU, cleanup bundle) + a behaviour-preserving `harness/sync.rs` decomposition into `reconcile/{hooks,guardrails,agents}.rs` (lands first) + an in-process MCP test harness (closes `GAP-1`) + the release wrapper (crate rename `tome`→`tome-mcp` with `[[bin]] name = "tome"`, cargo-dist prebuilt Linux+macOS binaries, Homebrew tap via cross-owner PAT, crates.io publish, third-party-licence bundle, README/SECURITY rewrite, **FR-024 untracking of `specs/`/`.sdd/`/`CLAUDE.md`/`review/`/`retro/` as the FINAL step** — `CONSTITUTION.md` stays tracked). **No new exit codes, no schema change, no new top-level dependency.** Constitution gate PASS with one planned **MINOR amendment → v1.4.0** (release-tooling clause; no cooling-off) + two §Complexity-budget notes (rustix promotion, crate rename). `tasks.md` pending `/sdd:tasks`. See `specs/007-*/`.
- **2026-05-29 — Phase 6 shipped (v0.6.0).** Hooks + agents. Real Claude Code hooks (rewrite + structural-match merge into `.claude/settings.local.json`, never the committed `settings.json`); `GUARDRAILS.md` prose fallback as per-plugin marker regions (+ a Tome-owned Cursor sibling, deleted when empty); native agent translation across claude-code/codex/cursor/opencode (Gemini has no native agents); optional agent-as-MCP-prompt personas (off by default); the Phase 4 rules-file correction (Claude Code rules sink → `CLAUDE.md`). New exit codes 43–46; `EntryKind::Agent` (schema v3→v4 marker-only migration). Polish: phase-wide 4-reviewer pass over the assembled surface — 0 blockers, 2 majors fixed (CON-1 hooks symlink exit 7→44 reconciliation with the guardrails 7→46 precedent; TEST-1 multi-sink `first_error` precedence test), security clean; cap-std/`O_NOFOLLOW` intermediate-dir-symlink hardening DEFER per the no-new-top-level-dep constitution gate. No new top-level dependency, no new top-level module — leanest phase since Phase 1. Test suite 151 → 175 suites (+24); ≈1427 test fns. v0.6.0 git tag + `cargo publish` + release-notes user-reserved. See `specs/006-*/retro/P{2..8}.md` + `review/`.
- **2026-05-28 — Phase 6 planning (spec + plan).** `/sdd:specify` → `specs/006-phase-6-hooks-agents/spec.md` (5 USs, 54 FRs / 11 NFRs / 11 SCs, 0 clarification markers; Rust-lens review folded in 2 blockers + 10 majors + 6 minors). `/sdd:plan` → `plan.md`, `research.md` (20 R-decisions), `data-model.md`, 9 contracts, `quickstart.md`. Constitution gate PASS, no amendments (0 new top-level deps, 0 new top-level modules — leanest phase since Phase 1). Phase 6 = hooks (real Claude Code JSON hooks + `GUARDRAILS.md` prose fallback) + agents (native translation across claude-code/codex/cursor/opencode + optional MCP-prompt personas) + the Phase 4 correction (Claude Code rules sink → `CLAUDE.md` not `AGENTS.md`). New exit codes 43–46 (PRD-proposed 30–33 collided with the model-on-disk cluster). `EntryKind` gains `Agent`; no new schema columns/tables. `tasks.md` pending `/sdd:tasks`.
- **2026-05-27 — Phase 5 shipped (v0.5.0).** Polish phase across PR-A→E on `phase-5-polish`; phase-wide 4-reviewer pass (0 blockers, 7 majors, security clean). All 5 USs feature-complete: commands as MCP prompts (US1), substitution layer paths/env (US2), argument substitution (US3), `get_skill_info` + `when_to_use` indexing (US4), per-entry invocability flags + doctor extensions (US5). Substitution layer complete — 4 stages via single-sweep regex, NFR-007 enforced structurally. Test total 954 → 1194 across 127 → 151 suites. No new top-level deps. See `specs/005-*/retro/P{3..8}.md`.
- **2026-05-26 — Phase 4 shipped (v0.4.0).** PRs #59–#108. `Paths` collapsed under `<home>/.tome/`; central SQLite DB; `WorkspaceName` newtype + `Scope`; `--global` deleted; `workspace_catalogs`/`workspace_skills` junctions as sources of truth. Workspace bind (US1), lifecycle commands (US2), layered settings + composition resolver + `tome harness` (US3), bundled local-LLM summarisation (US4), `tome doctor` extensions (US5). New deps: `llama-cpp-2`, `encoding_rs`, `toml_edit`, `filetime` (dev). See `specs/004-*/retro/P{2..8}.md`.
- **2026-05-14 — Phase 3 shipped (v0.3.0).** MCP server + workspaces. New async island under `src/mcp/` (`rmcp`, `tokio`, `schemars`); `tome workspace {info,init}`, `tome doctor`, forward schema-migration framework, per-command scope honouring + reference-counted catalog clones. See `specs/003-*/retro/P*.md`.
- **2026-05-13 — Phase 2 shipped (v0.2.0).** Plugin lifecycle + semantic index. `rusqlite`/`sqlite-vec`/`fastembed-rs`; `tome plugin {enable,disable,list,show}`, `tome query`, `tome models`, `tome reindex`, `tome status`, catalog cascade. Binary cap revised 10 MB → 50 MB. See `specs/002-*/retro/P*.md`.
- **2026-05-11 — Phase 1 shipped (v0.1.0).** Catalog foundations + constitution v1.0.0. See `specs/001-*/`.
