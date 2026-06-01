# Implementation Plan: Phase 7 — Beta Hardening and Public Release

**Branch**: `007-phase-7-beta-release` | **Date**: 2026-06-01 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/007-phase-7-beta-release/spec.md`
**Source**: No PRD (the roadmap ended at Phase 6). The authoritative inputs are the two beta-readiness audits — [`CODE-REVIEW.md`](../../CODE-REVIEW.md) (13 verified findings) and [`RELEASE-READINESS.md`](../../RELEASE-READINESS.md) (public-MVP audit) — and the planning-session dispositions encoded in spec.md.

## Summary

Phase 7 adds no product surface. It takes the code-complete, internally-reviewed v0.6.0 across the line to a **public beta** an external developer can discover, install, and trust. The work is two halves treated as one release.

**Hardening the existing surface.** A 31-agent code review found no blockers but eight real bugs in working features — most importantly that semantic search can silently return zero rows once any workspace/`searchable`/`--catalog`/`--plugin` filter applies (the vec0 `k` limit is bound to the *global* neighbourhood before the filters apply). Phase 7 fixes the five first-impression bugs (F-KNN, F-DOCTOR-RW, F-CACHE-KEY-DIVERGE, F-MCP-PROMPT-COLLISION, F-WS-TOML-NEWLINE), the three lower-frequency correctness bugs (F-PLUGIN-MANIFEST-DOS class, F-RULES-OPENCODE, F-REMOVE-TOCTOU), and a bundle of cheap robustness/hygiene cleanups; closes the one DoS class (unbounded third-party reads); and finishes the long-deferred mechanical symlink hardening — affordable now because `rustix` (the capability primitive) is already in the dependency graph and can be promoted transitive→direct under the existing complexity-budget rule rather than pulling a new top-level dependency (research §R-1). Tome's trust posture is made explicit: it defends the *mechanical* boundary (no OOM, no path traversal, no symlink escape, no file corruption) and tells the user plainly it cannot vet a catalog's *content*.

**The release wrapper.** The crate is unpublished and `tome` is taken on crates.io, so Tome publishes as `tome-mcp` while keeping the command `tome` (`[[bin]] name = "tome"`, research §R-12). A `cargo-dist` pipeline produces self-contained prebuilt binaries for Linux + macOS (x86_64 + aarch64), pushes a Homebrew formula to a tap via a least-privilege cross-owner PAT, publishes to crates.io, and attaches a third-party-licence bundle (§R-13/14/15). The constitution is amended first to permit release tooling (MINOR → v1.4.0, no cooling-off, §R-17). The README becomes the front door, a security-disclosure channel is opened, internal process artifacts are untracked from version control (the final step, §R-18), and the repository gains discovery metadata. The first public version stays **0.6.0**.

**Two supporting investments** make the hardening durable: the 1,737-LOC `harness/sync.rs` is decomposed into per-sink reconciler modules (`reconcile/{hooks,guardrails,agents}.rs`) behind a thin orchestrator — a behaviour-preserving *file move* (the reconcile functions are already factored), landed **first** so the harness fixes land in the clean structure (§R-10); and an in-process MCP test harness gives the MCP surface end-to-end exit-code coverage, closing the `GAP-1` backlog item (§R-11).

The technical approach **adds no new top-level dependency** (`rustix` is a transitive→direct promotion), **no SQLite schema migration**, and **no new exit code** (existing variants are reused: config parse → `ManifestInvalid::TomlParse` exit 5; non-array hooks → `HookSettingsWriteFailed` exit 44, §R-19). It introduces one new internal module cluster (`src/harness/reconcile/`, populated by moving existing code) and the release/CI tooling (cargo-dist config + workflow, not a runtime dependency).

## Technical Context

**Language/Version**: Rust stable (MSRV `rust-version = "1.93"`, pinned in `Cargo.toml`). **MSRV unchanged this phase** (NFR-009), verified green on the CI matrix. Edition 2024.

**Primary Dependencies (existing, consumed in Phase 7)**:
- `rusqlite` (`bundled`) — the F-KNN over-fetch+widen loop is a query-shape change in `src/index/query.rs`, no schema/DDL change. `sqlite-vec` virtual tables still require `DELETE`-then-`INSERT` (no `ON CONFLICT`) for the FR-013 duplicate-entry fix.
- `toml_edit` — `settings.toml` emission in `workspace/init.rs` (FR-005), replacing the bespoke `escape_toml_basic`.
- `serde_json` (`preserve_order`) — MCP prompt-collision taken-set (FR-004); the in-process MCP harness assertions.
- `tempfile` — atomic writes preserved across the decomposition and the symlink-guard consolidation.
- `regex` — credential scrubbing (unchanged; `scrub_credentials` rules stay order-dependent).
- `rmcp` + `tokio` (single-threaded, `src/mcp/` only) + `schemars` — the in-process MCP test harness drives a real server instance via the library API (FR-012).
- `tracing` — degraded-doctor diagnostics, dropped-entry warnings.

**Primary Dependencies (new direct)**: **`rustix`** — promoted transitive→direct (`{ version = "1", features = ["fs"] }`, matching `1.1.4` already in `Cargo.lock` via `tempfile`/`crossterm`). **No new package enters the graph** (NFR-004). Provides `openat`/`openat2` symlink-safe path resolution for FR-007. Licence (`Apache-2.0 OR MIT`) on the allowlist. See Complexity Tracking + research §R-1.

**Release/CI tooling (new, not runtime dependencies)**: `cargo-dist` (release pipeline), `cargo-about` (third-party-licence bundle). Both are CI tools, not `[dependencies]` — they do not affect the binary or the dependency graph. Gated on the constitution amendment (§R-17).

**Storage**: Existing central SQLite database (`<home>/.tome/index.db`, WAL + advisory lockfile). **No new columns, no new tables, no migration.** The schema version is unchanged. The F-KNN fix re-queries with a wider vec0 `k`; the F-DOCTOR-RW fix opens the same DB **read-only**.

**Testing**: `cargo test` (existing). New integration test files expected: `search_knn_recall.rs` (over-fetch+widen, ≥`top_k` nearer non-matching rows), `doctor_readonly_schema.rs` (stale/future schema, no abort, no lock), `catalog_ssh_roundtrip.rs` (scrubbed-URL cache key), `prompt_collision_global.rs` (command+skill+`foo2`), `workspace_toml_control_chars.rs`, `bounded_reads.rs` (oversized third-party files across the site list), `symlink_intermediate_guard.rs` (intermediate-component refusal across all sinks), `rules_opencode_inline.rs`, `catalog_remove_toctou.rs`, `exit_codes_e2e_mcp.rs` (NEW — GAP-1: codes 9, 26–29 + FR-004 verification), plus the one-time `search_knn_recall_realmodel` check (SC-001, real embedding models, run once — not in the fast CI suite). The decomposition's behaviour-preservation evidence is the **unchanged** `sync_idempotence.rs` / `harness_sync_p6_idempotence.rs` / `harness_sync_p6_first_error.rs` / `SyncOutcome` JSON-pin suites (NFR-005). Heavy paths use the library API + `StubEmbedder` + `HARNESS_MODULES_OVERRIDE`/`StubHarness`; light/exit-code paths use the CLI binary via `tests/exit_codes_e2e.rs`; MCP-internal codes use the new in-process harness.

**Target Platform**: macOS (`macos-latest`) and Linux (`ubuntu-latest`) — CI verified on stable + MSRV. Release binaries add aarch64 targets via cargo-dist (Linux + macOS × x86_64 + aarch64). The Linux release build targets a glibc baseline so the binary runs across mainstream distributions (spec Edge Case). Non-UTF-8 / symlink refusal tests gate on `#[cfg(target_os = "linux")]` where APFS rejects the fixture at `mkdir(2)` (Phase 4 P3 retro). **Windows remains unsupported and stated** (the symlink hardening is Unix-centric).

**Project Type**: Single Rust project (binary + library; no workspace split).

**Performance Goals**:
- F-KNN widen loop: geometric over-fetch bounded by the table row count; the common single-workspace/no-filter path resolves in one query (no extra cost); the worst case (heavily-filtered corpus) does a bounded number of widening re-queries, each a single indexed vec0 scan.
- Symlink-guard: per-component `openat` walk is linear in the write path's component count (small, bounded harness/project paths); applied at write sites only.
- Decomposition: zero runtime cost (a pure file move; no new allocations or passes).
- In-process MCP harness: test-only; no production cost.
- No new asymptotic cost classes in production; reconciler efficiency (RUST-1/2) is explicitly **out of scope** and untouched.

**Constraints**:
- **Sync only outside `src/mcp/`** (constitution §Async). All hardening/decomposition is sync; the in-process MCP harness drives the existing single-threaded `src/mcp/` island via the library API. `tests/sync_boundary.rs` continues to enforce the boundary.
- **Atomic writes + symlink refusal** at every write sink (Phase 4 discipline). FR-007 *strengthens* this to the intermediate-component walk via `rustix`, applied across **all** sinks in one pass (hooks `settings.local.json`, guardrails in-file regions + Cursor sibling, agent files, the rules-file + mcp-config writes) — never one sink at a time.
- **Closed error set** (`TomeError`). **No new variant, no new exit code** (NFR-002); FR-014/FR-015 reuse `ManifestInvalid::TomlParse` (5) and `HookSettingsWriteFailed` (44). Occupied set verified: `1–9, 13–37, 40–46, 50–54, 60–61, 70, 73–75`.
- **Credential scrubbing at every boundary** (§XIII) preserved; the cache-key fix (FR-003) keys by the *scrubbed* URL while cloning from the raw URL. `scrub_credentials` rules stay order-dependent (add a `tests/scrubbing.rs` case for any rule touch).
- **The reconcile mass-delete safeguard** (open central DB read-only and *propagate* on an existing DB, never `.ok()`-swallow) MUST survive the decomposition intact, carried per module (NFR-003, §R-10).
- **50 MB binary cap** (§Binary size). `rustix` is tiny (already compiled); projected delta ≈ 0. Current ~27 MiB macOS arm64 / ~35–37 MiB Linux x86_64 — ample headroom. CI keeps asserting `target/release/tome` size (path stays valid because `[[bin]] name = "tome"`). Record the post-rename size in `RELEASE-BINARY-SIZE.md`.
- **`--locked` builds** (NFR-008): `Cargo.lock` committed, authoritative, shipped in the tarball. The crate rename regenerates `Cargo.lock`'s `name` field — run `cargo check` after the rename before committing so the commit's own gate doesn't dirty the lock.
- **No telemetry** (NFR-006): the only network egress remains prompted, checksum-pinned model downloads + `git`/catalog fetches. cargo-dist adds no runtime egress.
- **Strictness boundary** (§IV): control-char rejection in catalog names is a *value* reject on a recognised field (FR-005); third-party manifests stay lenient on *unknown* fields.

**Scale/Scope**: 5 user stories (P1–P5), 26 functional requirements, 10 non-functional requirements, 11 success criteria. **0 new exit codes, 0 schema migrations, 0 new top-level dependencies** (one transitive→direct promotion). 1 new internal module cluster (`src/harness/reconcile/`, populated by moving existing `reconcile_*` code). 1 constitution amendment (→v1.4.0). 1 crate rename. New release/CI tooling (cargo-dist + cargo-about). Touched modules listed in Project Structure.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Each principle from `CONSTITUTION.md` (v1.3.0 at gate time; v1.4.0 after the FR-023 amendment):

- **I. Unix Philosophy**: PASS. No new top-level commands. Fixes ride existing commands (`query`, `doctor`, `catalog`, `plugin`, `harness sync`, `mcp`); `--json` preserved on every touched output path. Release tooling is external (cargo-dist), not a Tome subcommand.
- **II. Predictable Exit Codes (NON-NEGOTIABLE)**: PASS. **Zero new exit codes** (NFR-002, §R-19). FR-014 reuses `ManifestInvalid::TomlParse` (5); FR-015 reuses `HookSettingsWriteFailed` (44) and makes the meta-corruption case a *diagnostic distinction* over existing variants. No shipped code's meaning changes. *(The Phase-6 CON-1 precedent — a write-guard symlink failure on a dedicated sink returns that sink's code, e.g. 44 for settings, 46 for guardrails — is preserved by the FR-007 consolidation: the unified primitive returns each sink's existing dedicated code, never a regression to generic 7.)*
- **III. Scriptable by Default**: PASS. No new interactive prompts. The release pipeline is CI-driven and non-interactive; `cargo install`/`brew install` are standard non-interactive flows.
- **IV. Strict Schemas, Helpful Errors**: PASS. FR-005 adds a value-level reject of control chars in the recognised catalog `name` field; third-party manifests stay lenient on unknown fields. FR-006's bounded reads fail with the dedicated per-class parse/size error naming the file. FR-014 surfaces the specific TOML-parse error instead of a generic internal one.
- **V. Fail Fast, Fail Clear**: PASS. FR-002 (doctor degrades, never aborts) and FR-015 (fail closed, don't coerce) both *strengthen* fail-clear. FR-006 names the offending file. The F-KNN, F-RULES-OPENCODE, and F-MCP-PROMPT-COLLISION fixes each remove a *silent* wrong-result path.
- **VI. KISS / YAGNI**: PASS. F-KNN uses the no-schema over-fetch+widen (not a migration); FR-006 reuses existing caps (not a new one); the decomposition is a file move (not a redesign); RUST-1/2 efficiency work is **not** folded in. The release pipeline uses the de-facto tool (cargo-dist) rather than a hand-rolled matrix.
- **VII. Modular by Boundary**: PASS. The new `src/harness/reconcile/` cluster lives inside the existing `harness` capability module behind the `HarnessModule`/orchestrator surface (no new *top-level* module). The symlink-safe open primitive is a single SSOT in `src/util/` consumed by all sinks. No circular dependencies.
- **VIII. Test What Matters**: PASS. Integration tests for every fix against real fixtures + stubs; the behaviour-preservation of the decomposition is evidenced by the **unchanged** idempotence/first-error/wire-pin suites (NFR-005); the in-process MCP harness closes the GAP-1 exit-code coverage; the one-time real-model recall check proves SC-001 (the stub cannot). JSON wire-pins stay byte-stable; `ProjectBindingState` pin closed opportunistically.
- **IX. Conventional Commits**: PASS. `cog` hook enforces. The crate rename + amendment + untracking are conventional `chore`/`docs`/`refactor` commits.
- **X. CI Gates Every Merge**: PASS. The existing `{macos,ubuntu} × {stable,MSRV}` matrix + weekly `cargo-audit`/`cargo-deny` continue. FR-026 upgrades the deprecated checkout at all sites; the generated release workflow is subject to the same gates (NFR-010). `--locked` enforced (NFR-008).
- **XI. Documentation Is Part of the Change**: PASS. FR-021 (README front door), FR-022 (SECURITY.md), FR-010 (trust-model doc), FR-025 (CHANGELOG/metadata), and the cleanup-bundle doc-comment sweep + `--help` citation strip (FR-016, DOC-06) are all in-scope documentation work landing with their behaviour.
- **XII. Inherit, Don't Reimplement**: PASS. `rustix` (already present) for symlink-safe opens rather than hand-rolled `libc::openat`; `toml_edit` for settings emission rather than a bespoke escaper; cargo-dist/cargo-about for release/licence tooling rather than hand-rolled scripts; `git` shell-outs unchanged.
- **XIII. Never Log Secrets**: PASS. The cache-key fix keys by the *scrubbed* URL and clones from the raw URL without logging it; the Homebrew PAT is a never-logged least-privilege CI secret (NFR-010); `scrub_credentials` discipline carries to any new surfaced IO error.

Operational Constraints:
- **Async**: All hardening sync; the MCP harness drives the existing async island via the library API. Boundary test unchanged.
- **Dependencies**: One transitive→direct promotion (`rustix`), justified below; cargo-dist/cargo-about are CI tools, not runtime deps. `cargo-deny` stays green (advisories/bans/licences/sources).
- **Binary size**: ≈0 delta from `rustix`; record post-rename in `RELEASE-BINARY-SIZE.md`; CI assertion path stays `target/release/tome`.
- **Paths**: No new Tome-owned path classes. The symlink-guard hardens existing write sinks. The crate tarball `include` allowlist keeps `vendor/sqlite-vec`.
- **Release tooling**: **Requires the FR-023 amendment first** (the current clause defers it). The amendment is the one constitutional change; it lands before any cargo-dist work.

**Verdict**: PASS, with two Complexity-Tracking entries (the `rustix` promotion and the crate rename, each a one-paragraph justification per §Complexity budget) and one constitutional amendment (FR-023, the release-tooling clause → v1.4.0). No NON-NEGOTIABLE principle is touched; no cooling-off applies (§R-17). The amendment is *enabling* (it authorises previously-deferred tooling), not a relaxation of a quality gate.

## Project Structure

### Documentation (this feature)

```text
specs/007-phase-7-beta-release/
├── spec.md                               # /sdd:specify output (frozen)
├── plan.md                               # This file (/sdd:plan output)
├── research.md                           # Phase 0 output (24 R-decisions)
├── data-model.md                         # Phase 1 output
├── quickstart.md                         # Phase 1 output
├── contracts/                            # Phase 1 output
│   ├── exit-codes-p7.md                  # NO new codes — the reuse map (FR-014/015, NFR-002)
│   ├── correctness-beta-gate.md          # FR-001/002/003/004/005 (US1)
│   ├── symlink-guard.md                  # FR-007 — rustix spike, the SSOT primitive, all-sinks-one-pass, fallback
│   ├── robustness-trust.md               # FR-006/008/009/010 (US2)
│   ├── reconcile-decomposition.md        # FR-011 — module boundaries + behaviour-preservation evidence (US3)
│   ├── mcp-test-harness.md               # FR-012 — in-process harness + GAP-1 + FR-004 verification (US3)
│   ├── cleanup-bundle.md                 # FR-013/014/015/016 (US3)
│   ├── release-pipeline.md               # FR-017/018/019/020 + NFR-007/008/010 — rename, cargo-dist, tap, licences (US4)
│   ├── constitution-amendment.md         # FR-023 — the v1.4.0 amendment text + complexity-budget notes
│   └── repo-hygiene.md                   # FR-021/022/024/025/026 — README, SECURITY, untrack, metadata, checkout (US5)
├── checklists/
│   └── requirements.md                   # /sdd:specify output (frozen)
├── retro/                                # Created per slice by closeout PRs
├── review/                               # Created by reviewer passes
└── tasks.md                              # /sdd:tasks output — NOT created by /sdd:plan
```

### Source Code (repository root)

```text
src/
├── cli.rs                    # (existing) FR-016: strip FR-/NFR-/contracts/*.md citations from clap /// doc-comments (DOC-06)
├── error.rs                  # (existing) NO new variant; FR-014/015 reuse ManifestInvalid::TomlParse (5) + HookSettingsWriteFailed (44)
├── index/
│   ├── query.rs              # (existing) FR-001: bounded over-fetch + widen loop in knn(); no schema change
│   ├── skills.rs             # (existing) FR-013: DELETE-then-INSERT duplicate detection (no ON CONFLICT on vec table)
│   └── migrations.rs         # (existing) FR-015: distinguish meta-corruption from fresh DB (explicit match, existing variants)
├── doctor/
│   ├── checks.rs             # (existing) FR-002: open_read_only + degrade (l.58); FR-006: bounded read of tome-catalog.toml (l.174)
│   └── mod.rs                # (existing) read-only degrade pattern reused
├── commands/
│   └── catalog/
│       ├── add.rs            # (existing) FR-003: key cache_dir + refcount by scrubbed_url; clone from raw url
│       └── remove.rs         # (existing) FR-009: re-derive cascade input inside the index.lock closure
├── mcp/
│   ├── prompt_collision.rs   # (existing) FR-004: single global taken-set, suffix-until-free
│   └── prompts.rs            # (existing) FR-004 consumer; FR-016: stale prompts/get doc-comment sweep
├── workspace/
│   └── init.rs               # (existing) FR-005: emit settings.toml via toml_edit; delete escape_toml_basic
├── catalog/
│   ├── manifest.rs           # (existing) FR-005: reject control chars in catalog name; FR-006: bounded read (l.46)
│   └── store.rs              # (existing) FR-014: config.toml parse → ManifestInvalid::TomlParse (l.20); FR-016: delete dead reference_count
├── plugin/
│   ├── manifest.rs           # (existing) FR-006: bounded_read(PLUGIN_MANIFEST_MAX) (l.61)
│   ├── lifecycle.rs          # (existing) FR-006: bounded read (l.958); FR-013: detect duplicate (kind,name), truthful count
│   └── components.rs         # (existing) FR-006: bounded read (l.170)
├── harness/
│   ├── sync.rs               # (existing → thin orchestrator) sync_project + SyncDeps/SyncOutcome/SyncSubsystem + shared snapshot/group_by_path/relative_path + rules/mcp write helpers; FR-008 OpenCode LCD body in compute_rules_body
│   ├── reconcile/            # NEW cluster (populated by MOVING existing code — behaviour-preserving)
│   │   ├── mod.rs            # NEW — re-exports + shared reconcile types if any
│   │   ├── hooks.rs          # NEW — reconcile_hooks + merge/remove/compute_plugins_with_hooks_json (moved from sync.rs)
│   │   ├── guardrails.rs     # NEW — reconcile_guardrails + target/action helpers (moved)
│   │   └── agents.rs         # NEW — reconcile_agents + prepare/emit/cleanup/write_agent_file (moved)
│   ├── hooks.rs              # (existing) FR-007: route settings.local.json write through the SSOT symlink-safe open
│   ├── guardrails.rs         # (existing) FR-007: route in-file region + Cursor sibling writes through the SSOT primitive
│   ├── agents.rs             # (existing) FR-007: route agent-file writes through the SSOT primitive
│   ├── mcp_config.rs         # (existing) FR-007: refuse_symlink → SSOT primitive (one of the duplicated copies)
│   └── rules_file.rs         # (existing) FR-007: rules-file write through the SSOT primitive
└── util/
    ├── atomic_dir.rs         # (existing) FR-007: refuse_symlink → delegate to the SSOT primitive
    └── symlink_safe.rs       # NEW (or extend an existing util) — the single rustix-backed openat/O_NOFOLLOW write primitive (FR-007 SSOT)

tests/
├── search_knn_recall.rs              # NEW (US1) — over-fetch+widen, ≥top_k nearer non-matching rows (stub)
├── search_knn_recall_realmodel.rs    # NEW (US1) — one-time real-embedding-model recall check (SC-001; not in fast CI)
├── doctor_readonly_schema.rs         # NEW (US1) — stale/future schema, no abort, no lock
├── catalog_ssh_roundtrip.rs          # NEW (US1) — scrubbed-URL cache key; SSH source show/update/remove
├── prompt_collision_global.rs        # NEW (US1) — command+skill+foo2 collision
├── workspace_toml_control_chars.rs   # NEW (US1) — newline-bearing catalog name
├── bounded_reads.rs                  # NEW (US2) — oversized third-party files across the site list
├── symlink_intermediate_guard.rs     # NEW (US2) — intermediate-component refusal across ALL sinks (Linux-gated fixture)
├── rules_opencode_inline.rs          # NEW (US2) — OpenCode receives inline body when paired with Codex/Gemini
├── catalog_remove_toctou.rs          # NEW (US2) — re-derive cascade in lock
├── exit_codes_e2e_mcp.rs             # NEW (US3) — in-process MCP harness: GAP-1 codes 9, 26–29 + FR-004 verification
└── (existing suites) sync_idempotence.rs, harness_sync_p6_idempotence.rs, harness_sync_p6_first_error.rs,
                      *_json_shape.rs, exit_codes*.rs — UNCHANGED; the decomposition's behaviour-preservation evidence (NFR-005)

# Repo root / packaging / CI
Cargo.toml                    # FR-017 [package] name = "tome-mcp" + [[bin]] name = "tome"; FR-024 include/exclude allowlist;
                              #   FR-025 authors/homepage/documentation + [package.metadata.docs.rs]; rustix direct dep
Cargo.lock                    # regenerated post-rename (cargo check before commit); committed; shipped in tarball
.github/workflows/ci.yml      # FR-026 checkout@v4→@v5 (l.22)
.github/workflows/security.yml# FR-026 checkout@v4→@v5 (l.20, l.38)
.github/workflows/release.yml # NEW — cargo-dist generated; same gates as CI (NFR-010)
CONSTITUTION.md               # FR-023 amendment → v1.4.0 (STAYS tracked)
README.md                     # FR-021 rewrite (front door)
CHANGELOG.md                  # FR-025 [Unreleased] to top; version 0.6.0
SECURITY.md                   # NEW (FR-022)
THIRD-PARTY-LICENSES          # NEW (FR-019/NFR-007; cargo-about + native append)
RELEASE-BINARY-SIZE.md        # post-rename size row
.gitignore                    # FR-024 ignore untracked process dirs
```

**Structure Decision**: Single Rust project (binary + library). Phase 7 adds **no new top-level module** — `src/harness/reconcile/` is a sub-cluster inside the existing `harness` capability module, populated by *moving* the already-factored `reconcile_*` functions (research §R-10). The symlink-safe open primitive is a single SSOT helper inside the existing `src/util/` module. This is the smallest structure that satisfies the spec (§VI, §VII).

## Complexity Tracking

> Two entries — both required by the spec (FR-023) and satisfied by the §Complexity-budget one-paragraph-justification rule, not by a constitution amendment. The release-tooling **authorisation** is the separate FR-023 amendment (→v1.4.0).

| Item | Why Needed | Simpler Alternative Rejected Because |
|------|------------|--------------------------------------|
| **`rustix` transitive→direct promotion** (`features = ["fs"]`) | FR-007's intermediate-directory-component symlink hardening requires `openat`/`openat2`-based relative opens (Linux `RESOLVE_NO_SYMLINKS`; portable per-component `O_NOFOLLOW`). `rustix v1.1.4` is **already** in the graph (via `tempfile`/`crossterm`) with `fs` enabled; promoting it adds **no new package** (NFR-004), stays on the licence allowlist, and adds ≈0 binary size (already compiled). Mirrors the `filetime`/`encoding_rs` make-it-explicit precedent. | A *new* `cap-std` dependency (rejected — a new top-level package + heavier `Dir` abstraction for the same primitives, the cost that deferred this 3× under the Phase-6 no-new-dep gate); final-node-only `O_NOFOLLOW` with no `rustix` (rejected — buys zero intermediate-dir protection, the half-measure CONCERNS.md already rejected); a manual `libc::openat` FFI (rejected — `rustix` is the safe, already-present wrapper; `libc`-direct would itself be a new direct dep). |
| **Crate rename `tome` → `tome-mcp`** (with `[[bin]] name = "tome"`) | `cargo publish` to `tome` is rejected — the name is permanently owned on crates.io (all versions yanked; verified live, B1). The rename is the minimum change that unblocks the crates.io distribution channel; `[[bin]] name = "tome"` keeps the user-facing command, the binary-size assertion path (`target/release/tome`), and the exit-code e2e harness unchanged. Touches packaging metadata only — no module, no dependency. | A discretionary crates.io name-reclaim request (rejected — not timeline-safe for a beta); renaming the *command* too (rejected — gratuitous user-facing churn; the binary name is decoupled from the crate name for exactly this case); `tome-cli`/`tomekit` (rejected — `tome-mcp` best signals the dual CLI+MCP identity). |

> **Amendment vs. complexity-budget boundary**: the FR-023 constitution amendment authorises the release *action* (including crates.io publish **under** the renamed crate); the crate **rename itself** is the §Complexity-budget packaging decision justified in the table above, and `rustix` is a wholly separate §Complexity-budget item. The amendment does not "cover" the rename — they are disjoint mechanisms that happen to reference the same `tome-mcp` name. See `contracts/constitution-amendment.md` § "What this amendment does NOT cover".

## Pre-emptive slice plan

Per the Phase 4 P3 + Phase 5/6 lessons (encode the slice shape so `/sdd:tasks` and the per-slice agents inherit ≤ 8 KB briefs), the implementation is sliced **by dependency DAG, not by US priority** (the spec deliberately sequences US3's decomposition early). Each slice ≤ ~400 lines / ≤ 2 modules except the two noted cross-module exceptions. Detailed decisions in research §R-22.

- **Foundational** (before the hardening):
  - **F1 — Constitution amendment** (`CONSTITUTION.md` → v1.4.0): rewrite the Development-Workflow "Release tooling" clause to authorise the named set (cargo-dist pipeline, prebuilt-binary distribution, cross-owner-PAT Homebrew tap, crates.io publish under the renamed crate) + rationale + amendment-log entry. Docs-only; MUST land before any cargo-dist work (§R-17). No cooling-off.
  - **F2 — `rustix` promotion + symlink-primitive spike**: add `rustix = { version = "1", features = ["fs"] }` as a direct dep; spike-confirm `openat2`/`RESOLVE_NO_SYMLINKS` (Linux) + portable per-component `O_NOFOLLOW` are reachable under the enabled feature set. Output: a go/no-go recorded against FR-007 (full-path primitive vs. documented fallback). Tiny PR (dep + a feature-probe test). Gates the symlink-guard slice (§R-1).
  - **F3 — `actions/checkout@v4 → @v5`** at all three sites (`ci.yml:22`, `security.yml:20,38`): **time-sensitive** (Node-20 forced to Node-24 ~2026-06-02), pulled early as a standalone trivial PR (§R-21). The cargo-dist-generated-workflow-gating half of FR-026 lands with the release slice.
- **D — `harness/sync.rs` decomposition** (US3 work, sequenced first per the spec): behaviour-preserving move of `reconcile_{hooks,guardrails,agents}` + private helpers into `src/harness/reconcile/{hooks,guardrails,agents}.rs` behind the thin orchestrator. Sub-slices (each gate-green, idempotence/first-error/wire-pin suites **unchanged**): **D.a** scaffold `reconcile/mod.rs` + move `reconcile_agents` (largest); **D.b** move `reconcile_guardrails`; **D.c** move `reconcile_hooks` + finalise the orchestrator + module docs. The mass-delete safeguard carried per module; RUST-1/2 NOT folded in (§R-10).
- **US1 (P1) — beta-gate correctness** (small independent PRs, can land in parallel after D): **K1** F-KNN over-fetch+widen + stub regression test (`index/query.rs`, §R-2) — plus the one-time real-model recall check (SC-001); **K2** F-DOCTOR-RW read-only+degrade (`doctor/checks.rs`, §R-3); **K3** F-CACHE-KEY scrubbed-URL keying + SSH round-trip test (`catalog/add.rs`, §R-4); **K4** F-MCP-PROMPT-COLLISION global taken-set (`mcp/prompt_collision.rs`, §R-5; verified later via T1); **K5** F-WS-TOML-NEWLINE `toml_edit` emission + control-char reject (`workspace/init.rs` + `catalog/manifest.rs`, §R-6).
- **US2 (P2) — robustness & honest trust** (ride on the clean structure; FR-007/008 require D): **R1** bounded third-party reads across the site list (`plugin/*`, `catalog/manifest.rs`, `doctor/checks.rs`, §R-7 — the cross-module sweep exception); **R2** symlink-guard SSOT primitive + route **all** sinks in ONE pass (`util/` + hooks/guardrails/agents/rules/mcp writes, §R-1 — the second cross-module exception, mandated by the "fix-all-sinks-at-once" rule); **R3** F-RULES-OPENCODE inline LCD body (`harness/sync.rs::compute_rules_body`, §R-8); **R4** F-REMOVE-TOCTOU re-derive in lock (`catalog/remove.rs`, §R-9). (The trust-model security doc FR-010 lands with the README/SECURITY slice but is tracked here.)
- **US3 (P3) — test foundation + cleanup**: **T1** in-process MCP test harness + `tests/exit_codes_e2e_mcp.rs` (GAP-1 codes 9, 26–29 + end-to-end verification of the K4 prompt-collision fix, §R-11); **C1** cleanup bundle (FR-013 duplicate-entry detect+truthful-count; FR-014 config-parse→exit 5; FR-015 non-array-hooks fail-closed + meta-corruption diagnostic; FR-016 dead `reference_count` removal + stale doc-comment sweep + `--help` citation strip, §R-19/R-20) — split into ~2 small themed PRs.
- **US4/US5 (P4/P5) — release wrapper** (LAST; gated on US1–US3 merged): **REL1** crate rename `tome`→`tome-mcp` + `[[bin]] name = "tome"` + `Cargo.lock`/CI/README sweep + `--locked` re-verify (§R-12); **REL2** crate discovery metadata (authors/homepage/documentation) + docs.rs fix + CHANGELOG `[Unreleased]` to top, version 0.6.0 (§R-16, FR-025); **REL3** cargo-dist pipeline (Linux+macOS × x86_64+aarch64, per-target sidecar-absence check, checksums, cross-owner-PAT tap, crates.io publish, third-party-licence bundle, `--locked`, generated workflow under CI gates — publish/tag/tap-merge user-reserved, §R-13/14/15); **REL4** README front-door rewrite + `SECURITY.md` + private reporting + placeholder-email removal + the FR-010 trust-model doc (every getting-started command resolves; SC-008 via `file://` fixture); **REL5 — FINAL STEP** FR-024 untracking (`git rm --cached` `specs/`/`.sdd/`/`CLAUDE.md`/`review/`/`retro/`/the two `*.local.json`; `.gitignore`; Cargo `include`/`exclude` allowlist shipping `vendor/sqlite-vec`; `CONSTITUTION.md` STAYS tracked; local copies retained) — only after every hardening PR that reviews against those artifacts has merged (§R-18).
- **Polish**: phase-wide 4-reviewer pass (contract / Rust-lens / test / security) over the assembled surface — the decomposition × four-sink symlink guard × release wrapper is exactly its cross-cutting target (§R-23); findings + disposition committed before fixes; `/sdd:map incremental` after the decomposition and at phase close; retro fill + `CLAUDE.md` update (before the FR-024 untracking renders `CLAUDE.md` untracked). The v0.6.0 git tag + `cargo publish` + Homebrew-tap-PR merge + release-notes posting remain **user-reserved** (the standing project discipline).

Each slice closeout runs the review pass appropriate to its risk (the decomposition and the symlink-guard one-pass slices get the full 4-reviewer treatment); findings + disposition committed before fixes; `/sdd:map incremental` at the structural-change closeouts.
