# Tome — Claude Code Project Context

This file gives Claude Code persistent context about the Tome project. Keep it terse.

## Project

**Tome** is a Rust CLI (and eventually MCP server) that makes Claude Code's plugin ecosystem work across other agentic coding harnesses (Cursor, Codex, Gemini CLI, OpenCode, …).

- **Current phase:** Phase 2, User Story 7 — closing. `tome catalog remove` refuse-or-cascade shipped (PR #32). Closeout in PR #33. **All Phase 2 feature work complete — Phase 10 (Polish & Cross-Cutting Concerns) is next and out of scope for this push.**
- **Phase 1 PRD (shipped):** [`PRDs/phase-1.md`](./PRDs/phase-1.md)
- **Phase 2 PRD (in progress):** [`PRDs/phase-2.md`](./PRDs/phase-2.md)
- **Constitution:** [`CONSTITUTION.md`](./CONSTITUTION.md) (v1.2.0 — binary size cap revised 10 MB → 50 MB on 2026-05-13)
- **Active spec:** [`specs/002-phase-2-plugins-index/spec.md`](./specs/002-phase-2-plugins-index/spec.md)
- **Active plan:** [`specs/002-phase-2-plugins-index/plan.md`](./specs/002-phase-2-plugins-index/plan.md)
- **Codebase docs:** [`.sdd/codebase/`](./.sdd/codebase/) — all 8 documents refreshed 2026-05-13 against US7-complete source via `/sdd:map incremental`.
- **US1 status:** `tome plugin enable | list | show` + `tome query` ship end-to-end. Manual SC-001 / SC-002 against real BGE models still pending — see `retro/P3.md` § "T088 manual verification".
- **US2 status:** Bare `tome plugin` drops into an interactive catalog → plugin → action flow (`run_interactive`) with Back/Quit at every level; non-TTY exits 54 (`NotATerminal`) per FR-051. Tested via `rexpect` pty harness.
- **US3 status:** `tome plugin disable <id>` ships with `--force` short-circuit; non-TTY without `--force` exits 54 (`NotATerminal`) with a documented pointer message. Cheap re-enable (FR-006) verified — `enable_plugin_atomic` skips the embedder when `content_hash` matches.
- **US4 status:** `tome models download | list | remove` ships against the existing `embedding::download` library + pinned `MODEL_REGISTRY`. `--force` on download + remove, `--verify` on list (rehashes via `embedding::download::sha256_file`). Non-TTY without `--force` on remove → exit 54 with pointer message. CLI download path NOT exercised in CI (real network fetch) — coverage is library-level via `tests/model_download.rs` + CLI skip-path via sparse-file fabricators.
- **US5 status:** `tome catalog update` now reindexes every enabled plugin in each refreshed catalog (cheap-skip unchanged, re-embed modified, drop removed). Plugins gone upstream auto-disable per FR-033. `tome reindex [<scope>] [--force]` ships as the explicit recovery path — scope grammar is omitted | `<catalog>` | `<catalog>/<plugin>`. Per-plugin atomicity (each `lifecycle::reindex_plugin` acquires its own advisory lock).
- **US6 status:** `tome status [--verify] [--json]` ships as the read-only doctor / pre-flight. Reports embedder + reranker on-disk state, index integrity, plugin/skill counts, drift detection. Exit 0 / 1 / 1 for Ok / Degraded / Unhealthy. Never takes the advisory lock — works even when a writer is running. Extended `tome --version` includes embedder + reranker identities; `--version --json` emits the structured form. Pre-parse hook in `main.rs` (clap's `disable_version_flag = true`).
- **US7 status:** `tome catalog remove <name>` refuses with exit 53 (`CatalogHasEnabledPlugins`) when the catalog has enabled plugins and `--force` is not passed. `--force` cascades disable + row drop for each enabled plugin inside ONE advisory-lock window (single-lock-per-batch, distinct from the per-plugin pattern in catalog update / reindex), then proceeds with Phase 1 catalog removal. New library helper `lifecycle::cascade_disable_for_catalog`. JSON `removed` envelope gains an optional `cascade` array. 240 tests pass across 36 suites.

## Active Technologies

### Phase 1 (shipped, unchanged)

- **Language**: Rust stable, MSRV pinned at `rust-version = "1.93"` (verified in CI).
- **CLI**: `clap` (derive feature) — `--help` / `--version` / global flags.
- **Config / manifest**: `serde` + `serde_derive`, `toml` — Tome-owned structs use `#[serde(deny_unknown_fields)]`.
- **Errors**: `thiserror` for the closed `TomeError` enum (drives exit codes); `anyhow` for application-level context chaining.
- **Logging**: `tracing` + `tracing-subscriber` (stderr only; orthogonal to `--json`).
- **Paths**: `directories` (XDG-aware).
- **Hashing**: `sha2`, `hex` (cache directory naming, model checksums, content hashes).
- **Atomic writes**: `tempfile` (registry, catalog cache, models dir atomicity).
- **Signal handling**: `ctrlc` (SIGINT cancellation; exits with code 8).
- **Colour / NO_COLOR**: `anstream` + `anstyle` (transitive via clap 4).
- **Regex**: `regex` (credential scrubbing in `src/catalog/git.rs`).
- **Time**: `time` (RFC 3339 timestamps).
- **Semver**: `semver`.

### Phase 2 additions

- **Embedded database**: `rusqlite` with the `bundled` feature — statically linked SQLite, no system dep.
- **Vector search**: `sqlite-vec` C extension vendored under `vendor/sqlite-vec/`, compiled in via `build.rs`.
- **Inference**: `fastembed-rs` wrapping `ort` (ONNX Runtime). CPU execution provider only; CUDA / CoreML / DirectML disabled.
- **Models** (downloaded at runtime; not bundled): `bge-small-en-v1.5` INT8 (~45 MB, MIT), `bge-reranker-base` INT8 (~280 MB, MIT). Stored under `${XDG_DATA_HOME}/tome/models/`.
- **Progress / spinners**: `indicatif`.
- **Tables**: `comfy-table`.
- **Colours**: `owo-colors` (native `NO_COLOR`).
- **Prompts**: `inquire` (Select / MultiSelect / Confirm; refuses on non-TTY).
- **HTTP**: `reqwest` with `blocking` + `rustls-tls`, `default-features = false`.

**Strictness boundary** (FR-013a): `#[serde(deny_unknown_fields)]` applies to Tome-owned inputs (config, model `manifest.json`, index `meta` rows). Third-party inputs (`plugin.json`, `SKILL.md` YAML frontmatter) parse leniently — forward-compat with upstream additions.

**Not used**: `tokio`, `libgit2`/`git2`, `atty`, `colored`, `lazy_static`, `once_cell` (std `OnceLock` covers it). Phase 2 stays synchronous.

## Architectural Constraints (from the constitution)

- **Sync only.** No async runtime. `reqwest::blocking`, `rusqlite`, and `fastembed-rs` are all sync. The MCP server is the future forcing function for async.
- **Inherit `git`.** Shell out to system `git`. Never vendor a Git library.
- **Closed error set.** `TomeError` has no `Other`/`Unknown` arm. Every Phase 2 failure class has its own enumerated variant and exit code (see `specs/002-phase-2-plugins-index/contracts/exit-codes.md`).
- **Strictness boundary.** Tome-owned declarative inputs are strict (`#[serde(deny_unknown_fields)]`); third-party inputs (`plugin.json`, SKILL.md frontmatter) are lenient — see spec FR-013a. The strict-on-Tome-owned principle is enforced by `tests/manifest_strictness.rs`.
- **Atomic writes.** Registry, cache, models directory, and index DB mutations are atomic. SQLite WAL + a Tome-owned advisory lockfile (`index.lock`) provide the index concurrency contract (FR-040).
- **Credential scrubbing at the boundary.** `git::scrub_credentials` extends to model download URLs and `reqwest` error chains.
- **50 MB binary cap** (revised from 10 MB on 2026-05-13 — see `retro/P3.md` §"Binary-size cap revision"). CI asserts `stat -c%s target/release/tome` on Linux. `ort` (CPU-only static) is the load-bearing dep; profile is `lto = "thin"`, `panic = "abort"`, `strip = "symbols"`. If breached, the plan revises — the discipline is non-waivable (NFR-001); the specific number is sized to current reality plus headroom.
- **Licence allowlist.** Unchanged. Every Phase 2 dep verified inside the allowlist. `cargo-deny` enforces. Downloaded models (BGE family, MIT) are surfaced in `tome models list`.

## Conventions

- **Commits**: Conventional Commits. Enforced locally by `cocogitto` (`cog verify`) in the `commit-msg` hook (versioned under `.githooks/`). Format: `type(scope): subject`.
- **Branching**: trunk-based; short-lived branches off `main`.
- **PRs**: small batches — ~400 lines or 2 modules max as a soft cap.
- **Comments**: explain *why*, not *what*. Reader knows Rust.
- **Modules**: capability-organised. Phase 1: `catalog`, `config`, `paths`, `error`, `output`, `logging`. Phase 2 adds: `plugin` (manifest/frontmatter/lifecycle), `index` (db/schema/migrations/vec-ext/skills/query/meta/integrity/lock), `embedding` (fastembed wrapper + stub + registry + download + runtime), `presentation` (wraps comfy-table / indicatif / owo-colors / inquire).
- **Errors**: `thiserror` inside modules; `anyhow` at the application boundary.

## Common Commands

```sh
# Build / run
cargo build                                      # debug build
cargo build --release                            # release build (used by CI binary-size check)
cargo run -- catalog list                        # run a subcommand from source

# Quality gates (also enforced by the .githooks/pre-commit hook)
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
typos

# Tests (.githooks/pre-push runs the full suite)
cargo test                                       # all tests (uses stub embedder — fast, no model files)
cargo test --test catalog_add                    # one integration test file
cargo test catalog_add::                         # one test by path
cargo test --test query                          # Phase 2 query tests
cargo test --test concurrency                    # two-process index contention
cargo test --test atomicity                      # interrupt-injection tests

# Security and dependency hygiene
cargo audit
cargo deny check

# Conventional Commits
cog verify --file <commit-msg-file>

# Git hooks (versioned under .githooks/; no external manager)
git config core.hooksPath .githooks              # one-time, per clone
.githooks/pre-commit                              # run the pre-commit chain manually
.githooks/pre-push < /dev/null                    # run pre-push manually (drain empty stdin)

# MSRV verification (CI uses dtolnay/rust-toolchain @ rust-version from Cargo.toml)
cargo +<MSRV> build
```

## File Structure

```
src/
├── main.rs                   # entry: parse → dispatch → map errors → exit
├── lib.rs                    # re-exports
├── cli.rs                    # clap derive defs + global flags
├── error.rs                  # closed TomeError enum + ExitCode mapping
├── config.rs                 # config.toml (strict)
├── paths.rs                  # XDG paths (Phase 1) + index_db, models_dir, index_lock (Phase 2)
├── output.rs                 # human/--json formatter, NO_COLOR, TTY detection
├── logging.rs                # tracing-subscriber wiring
├── catalog/                  # Phase 1
│   ├── manifest.rs           # tome-catalog.toml (strict)
│   ├── store.rs              # registry persistence (atomic) — Phase 2 hooks cascade
│   └── git.rs                # git shell-outs + scrub_credentials
├── commands/
│   ├── catalog.rs            # tome catalog {add,remove,list,update,show}
│   ├── plugin/               # NEW — enable/disable/list/show; bare `tome plugin` → interactive
│   │   ├── mod.rs            # dispatch + shared helpers (aggregate_for_plugin, open_index_for_read, …)
│   │   ├── enable.rs         # `tome plugin enable`
│   │   ├── disable.rs        # `tome plugin disable` — confirm prompt + `--force`; non-TTY → exit 54
│   │   ├── list.rs           # `tome plugin list`
│   │   ├── show.rs           # `tome plugin show`
│   │   └── interactive.rs    # bare `tome plugin` — three-level loop + LoopExit
│   ├── query.rs              # NEW
│   ├── models/               # NEW — download/list/remove
│   │   ├── mod.rs            # dispatch + shared helpers (cheap_state, etc.)
│   │   ├── download.rs       # `tome models download` — indeterminate spinner; idempotent skip
│   │   ├── list.rs           # `tome models list [--verify]` — cheap probe / rehash via sha256_file
│   │   └── remove.rs         # `tome models remove <name> [--force]` — confirm + non-TTY exit 54
│   ├── reindex.rs            # NEW — `tome reindex [<scope>] [--force]` (Scope enum + run_with_deps lib entry)
│   └── status.rs             # NEW — `tome status [--verify]` + `print_version(json)` for the --version pre-parse hook
├── plugin/                   # NEW
│   ├── manifest.rs           # plugin.json (lenient)
│   ├── frontmatter.rs        # SKILL.md frontmatter (lenient + fallbacks)
│   ├── components.rs         # skills/agents/commands/hooks/.mcp.json walks
│   ├── identity.rs           # <catalog>/<plugin> address parsing
│   └── lifecycle.rs          # enable/disable orchestrator (atomic per plugin)
├── index/                    # NEW
│   ├── db.rs                 # rusqlite open, WAL, busy_timeout
│   ├── schema.rs             # CREATE TABLE statements (mirror of contracts/index-schema.sql)
│   ├── migrations.rs         # forward-only migrations under advisory lock
│   ├── vec_ext.rs            # sqlite-vec extension load (build.rs compiled)
│   ├── skills.rs             # CRUD on skills table; content-hash diff
│   ├── query.rs              # KNN search + reranker invocation
│   ├── meta.rs               # drift detection
│   ├── integrity.rs          # PRAGMA integrity_check
│   └── lock.rs               # advisory lockfile
├── embedding/                # NEW
│   ├── mod.rs                # Embedder + Reranker traits
│   ├── fastembed.rs          # fastembed-rs impl
│   ├── stub.rs               # #[cfg(test)] deterministic stub
│   ├── registry.rs           # MODEL_REGISTRY (pinned URLs + checksums)
│   ├── download.rs           # reqwest::blocking + SHA-256 + atomic persist
│   └── runtime.rs            # ort Environment, CPU EP only
└── presentation/             # NEW
    ├── tables.rs             # comfy-table helpers
    ├── progress.rs           # indicatif wrappers (TTY-aware)
    ├── colour.rs             # owo-colors + NO_COLOR
    └── prompt.rs             # inquire wrappers (refuse on non-TTY)

vendor/
└── sqlite-vec/               # NEW — pinned C source + LICENSE; compiled via build.rs

tests/
├── catalog_*.rs              # Phase 1 (catalog_remove extended for cascade)
├── manifest_strictness.rs    # Phase 1 (extended: model manifest is strict)
├── path_validation.rs        # Phase 1
├── exit_codes.rs             # extended for the 18 Phase 2 codes
├── scrubbing.rs              # extended for model URL scrubbing
├── atomicity.rs              # extended for enable / model download interrupts
├── plugin_{enable,disable,list,show,interactive,repeated}.rs   # NEW (interactive uses `rexpect`; disable + repeated drive CLI binary)
├── query.rs                  # NEW
├── models_{download,list,remove}.rs                   # NEW
├── reindex.rs                # NEW
├── status.rs                 # NEW
├── catalog_update_reindex.rs # NEW
├── catalog_remove_cascade.rs # NEW
├── concurrency.rs            # NEW — two-process index contention
├── schema_migrations.rs      # NEW
├── version_output.rs         # NEW
├── frontmatter.rs            # NEW — table-driven parser cases
└── fixtures/
    ├── sample-catalog/       # Phase 1
    └── sample-plugin/        # NEW
```

## Recent Changes

- 2026-05-13: Closed Phase 9 / User Story 7 across PRs #32–#33 (final feature phase of Phase 2). Slice 1+2 combined (PR #32) shipped `tome catalog remove` Phase 2 extensions per `contracts/catalog-extensions.md`. Refuse path: enabled plugins + no `--force` → exit 53 (`CatalogHasEnabledPlugins`, pre-existing variant from Phase 2 foundational, used in production for the first time). Cascade path: `--force` + enabled plugins → drop each plugin's `skills` + `skill_embeddings` rows inside ONE advisory-lock window (single-lock-per-batch — different from Phase 7's per-plugin reindex pattern, because catalog removal is semantically one user action). New library helper `lifecycle::cascade_disable_for_catalog(paths, catalog, plugins, embedder_seed, reranker_seed) -> u32` deliberately takes a narrower signature than `auto_disable_orphan` (no `LifecycleDeps`, no `Embedder`) because the cascade is pure deletion. Pre-check `enabled_plugins_for_catalog` runs WITHOUT the lock (readers don't block writers; the worst TOCTOU outcome is dropping an extra plugin's rows, still correct). JSON `removed` envelope extended with optional `cascade` array (`skip_serializing_if = "Vec::is_empty"`). 3 new tests in `tests/catalog_remove_cascade.rs` via the CLI binary (cascade is pure deletion — no `FastembedEmbedder` needed); enable in test setup uses library API + `StubEmbedder`. Closeout (PR #33) refreshes all 8 codebase docs and the P9 retro. Headline learnings: (a) single-lock-per-batch vs per-plugin atomicity now coexist as established patterns — pick based on whether the operation is semantically one action or N independent operations; (b) pre-allocated error variants in the closed enum (variant existed since Phase 2, used in Phase 9 — zero refactor cost); (c) helper signature minimisation (don't pass `LifecycleDeps` when only `(paths, seeds)` are used); (d) combine feat+tests in one PR when total < ~250 lines AND nothing in feat needs a deeper review than the tests provide. Test total 237 → 240 across 35 → 36 suites. No new dependencies. No new error variants.
- 2026-05-13: Closed Phase 8 / User Story 6 across PRs #29–#31. Slice 1 (PR #29) shipped `tome status [--verify] [--json]` as a read-only doctor / pre-flight per `contracts/status.md`. New `src/commands/status.rs` (~330 lines): `StatusReport { tome, embedder, reranker, index, drift, overall }` with `ModelHealth`, `IndexHealth`, `OverallHealth::{Ok, Degraded, Unhealthy}`. `pub fn run` emits + `std::process::exit(1)` for non-Ok; `pub fn assemble_report` is the library-API entry point that bypasses the exit for tests (same library-bypass pattern as `reindex::run_with_deps`). Classification policy: embedder missing/corrupt OR index integrity failure OR embedder drift → Unhealthy; reranker missing/corrupt OR reranker drift → Degraded; otherwise Ok. Reads model state via the now-pub `commands::models::cheap_state`, opens index read-only (never takes the advisory lock — by design FR-056), runs `PRAGMA integrity_check`, detects drift via `index::meta::detect_drift`. `--verify` rehashes each model's primary artefact via `embedding::download::sha256_file`. Extended `tome --version` per `contracts/version-output.md`: clap's auto handler disabled (`disable_version_flag = true` on Cli derive), pre-parse hook in `main.rs` scans `std::env::args` BEFORE clap dispatch — emits three-line plain text (`tome <v>` / `embedder: <name> <v>` / `reranker: <name> <v>`) by default, or the structured JSON record when `--json` is also passed. Identities are compile-time constants from `MODEL_REGISTRY`; a model bump auto-bumps the output. Helper promotions slice 1: `ModelState`, `cheap_state`, `read_manifest`, `primary_file_path`, `human_mb` in `commands/models/mod.rs` from `pub(crate)` to `pub`. Slice 2 (PR #30) shipped 14 integration tests across `tests/status.rs` (10 tests: 7 library-API via `assemble_report` + 3 CLI binary for exit-code semantics) and `tests/version_output.rs` (4 tests: plain text + JSON + flag-order-irrelevant + `-V` short flag). Slice 2 also promoted `commands::plugin::registry_seeds` from `pub(crate)` to `pub` so drift tests bootstrap meta with BGE seeds (matching what status reads on the configured side); the StubEmbedder stays decoupled — seed records identity, embedder produces vectors. Test total 223 → 237 across 33 → 35 suites. No new dependencies. No new error variants.
- 2026-05-13: Closed Phase 7 / User Story 5 across PRs #25–#28. Slice 1 (PR #25) added `index::skills::reindex_plugin_atomic` (mirrors `enable_plugin_atomic`; classifies each skill as Added/Modified/Removed/Unchanged inside one SQLite transaction; honours `force` to re-embed unchanged rows) + `ReindexSummary` struct. Added `plugin::lifecycle::reindex_plugin(id, deps, force) -> ReindexOutcome` and `plugin::lifecycle::auto_disable_orphan(id, deps) -> u32` (drops all rows for a plugin whose upstream is gone — FR-033). Fixed a latent bug: `skill_embeddings` is a `sqlite-vec` virtual table that doesn't support `INSERT OR REPLACE` / `ON CONFLICT`, so `upsert_skill` now does `DELETE`-then-`INSERT`. The bug was unreachable from the existing enable path; reindex's Modified branch caught it. 9 new unit tests. Slice 2 (PR #26) added `enabled_plugins_for_catalog` query helper + `pub fn reindex_catalog_plugins(catalog, enabled, deps) -> CatalogReindexOutcome` in `commands/catalog/update.rs`, wired into `run()`. Lazy `FastembedEmbedder` loading via a `GetOrInsertWithResult` helper trait — a zero-enabled-plugin install never touches ONNX models on `tome catalog update`. SHA-pinned catalogs skip the reindex pass (intentionally frozen). Auto-disable cascades on `PluginNotFound` / `PluginManifestParseError`; other errors propagate. `commands/catalog` module promoted to `pub mod` so the test crate can reach `update::reindex_catalog_plugins`. 3 new integration tests via library API + `StubEmbedder`. Slice 3 (PR #27) shipped `tome reindex [<scope>] [--force]`: new `src/commands/reindex.rs` with `Scope` enum (All / Catalog / Plugin), `ReindexAggregate` outcome, `execute(...)` + `pub fn run_with_deps(...)` library entry point for tests. Same lazy-embedder pattern. Per-plugin atomicity (each `lifecycle::reindex_plugin` acquires its own advisory lock — a Ctrl-C between plugins leaves earlier ones committed, by design). 7 new tests (4 library + 3 CLI binary). Library/CLI test boundary now applies a third time: heavy-state paths → library + `StubEmbedder`; light/error paths → CLI binary. The pattern is firm. Test total 204 → 213 (slice 1) → 216 (slice 2) → 223 (slice 3) across 33 suites. No new dependencies. No new error variants. No new top-level error codes (the contract's exit codes 30/31/34/35/36/50/51/52 are already enumerated for the embedder/index domain).
- 2026-05-13: Closed Phase 6 / User Story 4 across PRs #22–#24. Slice 1 (PR #22) added the `tome models download | list | remove` CLI surface as a new `src/commands/models/` module mirroring the `plugin/` layout (`mod.rs` dispatch + per-subcommand files); wires the already-shipped `embedding::download::download_model` library, the pinned `MODEL_REGISTRY`, and `presentation::{tables, progress, prompt}`. Promoted `embedding::download::sha256_file(path) -> Result<String, TomeError>` to `pub` (streaming chunked SHA-256, used by `models list --verify` and likely Phase 7 reindex). Relaxed `output::write_json` to `T: Serialize + ?Sized` so slice serialisation works without an intermediate `Vec` allocation. Indeterminate `indicatif` spinner ships on both `plugin enable` and `models download` — byte-progress callback refactor of `download_model` is now strictly past rule-of-three; tracked as TD-010 in CONCERNS.md. Slice 2 (PR #23) added 9 integration tests across `tests/models_{download,list,remove}.rs` plus `fabricate_installed_model` / `fabricate_all_installed_models` helpers in `tests/common/mod.rs`. The fabricators use `std::fs::File::set_len` to create zero-filled sparse files — a 280 MB reranker fixture takes ~no disk space, and the all-zero contents intentionally disagree with the registry-pinned SHA-256, which the `--verify` test relies on to flip `checksum_mismatched`. The pattern is reusable for any future test needing realistic-size on-disk artefacts without IO cost. The `tome models download` CLI binary path is intentionally NOT tested end-to-end in CI (real `MODEL_REGISTRY` URLs = 66 MB embedder + 280 MB reranker downloads) — same boundary as `tome plugin enable`. CLI-level coverage is restricted to the skip path (all-installed → all-`skipped` records), exit-2 usage error, exit-30 missing-model, and exit-54 non-TTY refusal. Test total 195 → 204 across 28 → 31 suites. No new dependencies. No new error variants.
- 2026-05-13: Closed Phase 5 / User Story 3 across PRs #19–#21. Slice 1 (PR #19) added `src/commands/plugin/disable.rs` as a thin wrapper over the already-shipped `plugin::lifecycle::disable` orchestrator (PR #11) — `PluginCommand::Disable(PluginDisableArgs { id, force })` variant, `--force` short-circuits the confirm prompt, non-TTY without `--force` exits 54 (`NotATerminal`) with the documented pointer message ("Re-run with `--force` to skip the prompt"). Banner skipped in JSON mode (precedent: `enable::run`). Cheap re-enable (FR-006) is already in `index::skills::enable_plugin_atomic` — when `stored_hash == hash` the embedder closure is NOT invoked and the row is updated with `UPDATE skills SET enabled = 1`; T114 was therefore a verification task. Slice 2 (PR #20) added 3 integration tests in `tests/plugin_disable.rs` (CLI binary; disable doesn't construct `FastembedEmbedder`), 2 tests in `tests/plugin_repeated.rs` consolidating FR-008 (enable-of-enabled via library API per handover #10; disable-of-disabled via CLI binary for real `Some(21)` process exit), and `cheap_reenable_after_disable_invokes_embedder_zero_times` in `plugin_enable.rs` using `StubEmbedder::call_count()`. Promoted `paths_for(&ToolEnv) -> Paths` to `tests/common/mod.rs` at the 4th caller — resolves the P4 triage item. Closeout in PR #21 (`/sdd:map incremental` refresh). Test total 189 → 195 across 28 suites. No new dependencies. No new error variants.
- 2026-05-13: Closed Phase 4 / User Story 2 across PRs #16–#18. Slice 1 (PR #16) added `src/commands/plugin/interactive.rs` (`run_interactive` + three-level `catalog_loop` → `plugin_loop` → `view_loop` with a `LoopExit` enum encoding Back/Quit unwind vs propagated error), wired bare `tome plugin` via `Command::Plugin(PluginArgs { command: Option<PluginCommand> })`, and reuses `presentation::prompt::{select, confirm, require_terminal}` for TTY-gated interaction. Enable delegates to `enable::run`; Disable inlines `lifecycle::disable` + confirm. Slice 2 (PR #17) added `tests/plugin_interactive.rs` driven by a `rexpect = "0.7"` (dev-dep, Unix-only) pty harness — pre-enable via library API + StubEmbedder, then drive the read/write CLI through the pty; the CLI enable path is intentionally not exercised in CI because it loads `FastembedEmbedder` (~345 MB ONNX). Non-TTY case asserts exit 54 (`NotATerminal`) plus the documented pointer message. Closeout in PR #18 (`/sdd:map incremental` refresh + retro extraction). Test total 187 → 189 across 26 suites. New `paths_for(&ToolEnv) -> Paths` helper duplicated across `plugin_list.rs`, `plugin_show.rs`, `plugin_interactive.rs` — promote to `tests/common/mod.rs` at the 4th caller (likely Phase 5 `plugin_disable.rs`).
- 2026-05-13: Closed Phase 3 / User Story 1 across PRs #11–#15. Slice 1a (`plugin::lifecycle::enable` / `disable` orchestrator + pinned `MODEL_REGISTRY` SHA-256s; reranker URL moved upstream from `BAAI/bge-reranker-base` to `onnx-community/bge-reranker-base-ONNX`), slice 1b (`tome plugin enable | list | show` CLI + T074 prompt UI), slice 2 (`tome query` with reranker + `--strict`), slice 3 (5 new integration-test files + `tests/fixtures/sample-plugin-catalog/` + `StubEmbedder::with_force_fail_after`), and the resolver-bug fix folded into PR #14 (`lifecycle::resolve_plugin_dir` is now manifest-first via `tome-catalog.toml`; falls back to flat join only when the manifest is absent/unparsable). Constitution v1.2.0 — binary size cap revised 10 MB → 50 MB after slice 1b measured 29.56 MB on Linux (research §Binary size budget's ~9.2 MB worst-case projection underestimated `ort`). Test total 156 → 187 across 25 suites. T088 manual SC-001 / SC-002 verification against real BGE models is the only outstanding US1 task and lives in `retro/P3.md` for a developer pass.
- 2026-05-12: Closed Phase 2 foundational — landed slices 1–7 across PRs #2–#10. T057 (model-download integration test with hand-rolled `TcpListener` HTTP fixture) is in slice 7 rather than slice 5 where it was originally scheduled. The cleanup bug it caught (partial-dir leaking on checksum mismatch because cleanup only ran on `stream_to_partial` errors, not later pipeline errors) is fixed by wrapping the full post-stream pipeline in a closure. Codebase docs (`.sdd/codebase/STACK.md`, `STRUCTURE.md`) refreshed; retro at `specs/002-phase-2-plugins-index/retro/P2.md` extended with workarounds, package gotchas, patterns, and "for next time" entries.
- 2026-05-12: Generated Phase 2 `/sdd:plan` artefacts on `002-phase-2-plugins-index` — plan.md, research.md (15 R-decisions including binary-size strategy, SQLite concurrency model, schema migration, frontmatter strictness boundary), data-model.md, contracts/* (plugin-commands, query, models-commands, reindex, status, catalog-extensions, version-output, exit-codes, index-schema.sql), quickstart.md. Constitution gates: PASS with one justified deviation (`#[cfg(test)]` stub for the embedder/reranker traits — keeps CI fast and bounded; principle VIII boundary case).
- 2026-05-11: Generated Phase 2 `/sdd:specify` artefacts — spec (7 user stories, 60 FRs, 5 NFRs, 15 SCs) and refreshed `.sdd/codebase/*` against the Phase 1 source. Rust-lens review folded in 3 blockers + 12 majors before validation passed.
- 2026-05-11: Ratified CONSTITUTION.md v1.0.0; later patched to v1.0.1.
- 2026-05-11: Wrote Phase 1 PRD amendments resolving the constitution-review report.
- 2026-05-11: Generated `/sdd:specify` and `/sdd:plan` artefacts on `001-phase-1-foundations`. Constitution gates: PASS, zero violations.
- 2026-05-11: Added exit code 8 (SIGINT interrupted) after Rust-lens review of the Phase 1 spec.

<!-- MANUAL ADDITIONS START -->
<!-- Notes that should not be touched by automation go here. -->
<!-- MANUAL ADDITIONS END -->
