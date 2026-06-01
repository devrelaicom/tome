---
description: "Phase 7 task list — Beta Hardening and Public Release"
---

# Tasks: Phase 7 — Beta Hardening and Public Release

**Input**: Design documents from `/specs/007-phase-7-beta-release/`
**Prerequisites**: plan.md, spec.md, research.md (24 R-decisions), data-model.md, contracts/ (10) — all present
**Source**: No PRD. WHAT comes from the two beta-readiness audits (`CODE-REVIEW.md`, `RELEASE-READINESS.md`) encoded in spec.md; HOW comes from plan.md § Pre-emptive slice plan + research §R-22.

**Tests**: INCLUDED. The constitution §VIII mandates integration tests against real fixtures, and every contract names its test obligations. Heavy paths use the library API + `StubEmbedder` + `HARNESS_MODULES_OVERRIDE`/`StubHarness`; light/exit-code paths use the CLI binary via `tests/exit_codes_e2e.rs`; MCP-internal codes use the new in-process harness. The F-KNN fix gets a one-time real-embedding-model recall check (SC-001) the stub cannot provide.

**Organization**: Tasks are grouped into phases that follow the **dependency DAG**, not strict US priority — the spec deliberately sequences the `harness/sync.rs` decomposition (US3 work) **first** so the harness fixes land in the clean structure. Each task carries its true `[US#]` label. The slice shape mirrors plan.md § Pre-emptive slice plan (F1–F3 → D → US1 → US2 → US3 → REL1–5 → Polish).

## Format: `[ID] [P?] [Story?] Description (use <agent>)`

- **[P]**: parallelizable (different files, no incomplete-task dependency).
- **[Story]**: US1–US5; Setup / Foundational / Polish / `[GIT]` tasks carry **no** story label.
- All Rust source/test work uses the **devs:rust-dev** agent; markdown / TOML / CI / git tasks need no agent.

## PR & git discipline (project conventions — override the generic template)

- **Trunk-based, small batches** (constitution §X, §Branching/§PRs): each **slice** below (F1, F2, …, REL5) is its own short-lived branch off latest `main` and its own **small PR** (~≤400 lines / ≤2 modules). The planning branch `007-phase-7-beta-release` already exists and carries the planning-artifacts PR (Setup).
- **Conventional Commits** enforced by `cog` in the `commit-msg` hook (`type(scope): subject`).
- **Hooks must pass**: pre-commit (`fmt`/`clippy -D warnings`/`typos`) and pre-push (`cargo test`); **never** `--no-verify`. A failed pre-commit aborts the commit (nothing lands) → fix or confirm-transient, make a NEW commit (never `--amend`).
- **CI green before LGTM**: each slice's PR closeout pushes, opens/updates the PR, waits for all CI checks green (fix + re-push until green), then reports `**PR #<n> READY FOR MERGE. AWAITING LGTM**` + the URL and **STOPS** for human review.
- **User-reserved** (the implementer NEVER runs these): `cargo publish` (final), the `v0.6.0` git tag, the Homebrew-tap-PR merge, release-notes posting. **Operator-only**: enable GitHub private vulnerability reporting; set repo description + topics; provide the least-privilege Homebrew-tap PAT secret. See the final "User-reserved & operator-only actions" section.

## Path conventions

Single Rust project: `src/`, `tests/` at repo root. No workspace split.

---

## Phase 1: Setup

**Purpose**: Confirm the planning branch and land the planning artifacts. (`007-phase-7-beta-release` was created during `/sdd:specify`; the artifacts from `/sdd:specify` + `/sdd:plan` are currently uncommitted, alongside the two audit docs at repo root.)

- [ ] T001 [GIT] Verify current branch is `007-phase-7-beta-release` (`git branch --show-current`); review the working tree (`git status --short`: `specs/007-phase-7-beta-release/`, modified `CLAUDE.md`, untracked `CODE-REVIEW.md` + `RELEASE-READINESS.md`, and the stray `.claude/agent-foundry/...local.json`).
- [ ] T002 [GIT] Commit planning artifacts: `git add specs/007-phase-7-beta-release/ CLAUDE.md CODE-REVIEW.md RELEASE-READINESS.md && git commit` — `docs(phase-7): spec, plan, research, data-model, 10 contracts, quickstart, tasks + beta-readiness audits`. (Do NOT stage the `.local.json`.)
- [ ] T003 [GIT] Push `007-phase-7-beta-release`; open the planning PR; on CI green report `PR READY FOR MERGE. AWAITING LGTM` and STOP. (Once merged, implementation slices branch off the updated `main`.)

**Checkpoint**: Planning artifacts on `main`; tooling already exists (clippy/fmt/typos/cog/cargo-deny/cargo-audit + full suite) — nothing to scaffold.

---

## Phase 2: Foundational (blocking prerequisites)

**Purpose**: Land the constitution amendment (gates all release tooling), promote `rustix` + run the symlink-primitive spike (gates FR-007), and bump the time-sensitive deprecated checkout. No release or symlink work can begin until F1/F2 land.

- [ ] T004 Create `specs/007-phase-7-beta-release/retro/P2.md` from the retro template.
- [ ] T005 [GIT] Commit: `docs(phase-7): init P2 retro`.

### F1 — Constitution amendment → v1.4.0 (`contracts/constitution-amendment.md`, FR-023)

- [ ] T006 [GIT] Branch `007-p7-f1-constitution-amendment` off latest `main`.
- [ ] T007 Rewrite the Development-Workflow **§Release tooling** clause in `CONSTITUTION.md` to authorise the named set (cargo-dist pipeline, prebuilt-binary distribution, cross-owner-PAT Homebrew tap, crates.io publish under `tome-mcp`); add the **v1.4.0** amendment-log entry; bump `**Version**` → 1.4.0 and `**Last Amended**`. No cooling-off (not a NON-NEGOTIABLE principle).
- [ ] T008 [GIT] Commit: `docs(phase-7): amend constitution → v1.4.0 (authorise release tooling)`.
- [ ] T009 [GIT] Push `007-p7-f1-constitution-amendment` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP. (MUST merge before any cargo-dist work — REL3.)

### F2 — `rustix` promotion + symlink-primitive spike (`contracts/symlink-guard.md`, §R-1, FR-007 gate)

- [ ] T010 [GIT] Branch `007-p7-f2-rustix-spike` off latest `main`.
- [ ] T011 Confirm `rustix` is already transitive: `cargo tree -i rustix -e features` (expect `v1.1.4` via `tempfile` (default) + `crossterm`, `fs` enabled). Record the finding (no new package expected).
- [ ] T012 Promote `rustix` to a direct dependency in `Cargo.toml`: `rustix = { version = "1", features = ["fs"] }` (matching the locked version); add the transitive→direct + `filetime`/`encoding_rs`-precedent rationale comment (use devs:rust-dev agent).
- [ ] T013 Run `cargo check` then `cargo deny check licenses` — confirm **no new package** in `Cargo.lock` and `rustix` (Apache-2.0 OR MIT) stays on the allowlist.
- [ ] T014 [P] Spike test in `tests/symlink_intermediate_guard.rs` (initial scaffold): probe that `rustix::fs::openat2(RESOLVE_NO_SYMLINKS)` (Linux, `#[cfg(target_os="linux")]`) and a per-component `openat` + `OFlags::NOFOLLOW` walk (portable) compile and refuse a symlinked component (use devs:rust-dev agent).
- [ ] T015 Record the spike outcome against FR-007 in `research.md` §R-1 / the symlink-guard contract: **pass** → full intermediate-walk primitive (R2); **fail** → final-node `O_NOFOLLOW` + documented trust-model fallback (no new package either way).
- [ ] T016 [GIT] Commit: `chore(phase-7): promote rustix transitive→direct + confirm symlink-safe primitive (spike)`.
- [ ] T017 [GIT] Push `007-p7-f2-rustix-spike` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP.

### F3 — `actions/checkout@v4 → @v5` (time-sensitive, `contracts/repo-hygiene.md`, FR-026)

- [ ] T018 [GIT] Branch `007-p7-f3-checkout-v5` off latest `main`.
- [ ] T019 Bump `actions/checkout@v4 → @v5` at all three sites: `.github/workflows/ci.yml:22`, `.github/workflows/security.yml:20`, `.github/workflows/security.yml:38`.
- [ ] T020 [GIT] Commit: `ci(phase-7): upgrade actions/checkout v4→v5 at all sites (Node-24 deadline)`.
- [ ] T021 [GIT] Push `007-p7-f3-checkout-v5` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP. (Time-sensitive — land first among foundational PRs.)

### Foundational closeout

- [ ] T022 Run codebase mapping for Phase 2 changes (`/sdd:map incremental`).
- [ ] T023 Review `retro/P2.md`; extract critical learnings to `CLAUDE.md` (conservative).
- [ ] T024 [GIT] Commit on a `007-p7-p2-closeout` branch: `docs(phase-7): P2 closeout — mapping + retro`; push + PR + CI green + LGTM stop.

**Checkpoint**: Constitution authorises release tooling; `rustix` direct (no new package) with the FR-007 approach decided; CI checkout current.

---

## Phase 3: Harness decomposition [US3 — sequenced first] (`contracts/reconcile-decomposition.md`, FR-011)

**Goal**: Decompose `src/harness/sync.rs` (1,737 LOC) into a thin orchestrator + `reconcile/{hooks,guardrails,agents}.rs`. **Strictly behaviour-preserving** — a file move of already-factored functions. Lands before FR-007/FR-008.

**Independent Test**: the pre-existing `sync_idempotence.rs` / `harness_sync_p6_idempotence.rs` / `harness_sync_p6_first_error.rs` / `SyncOutcome` JSON-pin suites stay **green and unchanged** (NFR-005, SC-011).

- [ ] T025 Create `specs/007-phase-7-beta-release/retro/P3.md` from the retro template.
- [ ] T026 [GIT] Commit: `docs(phase-7): init P3 retro`.

### D.a — scaffold `reconcile/` + move `reconcile_agents`

- [ ] T027 [GIT] Branch `007-p7-d-a-reconcile-agents` off latest `main`.
- [ ] T028 [US3] Create `src/harness/reconcile/mod.rs` (re-exports + any shared reconcile type) and wire it into `src/harness/mod.rs` (use devs:rust-dev agent).
- [ ] T029 [US3] Move `reconcile_agents` + `AgentReconciliation`, `PreparedAgent`, `prepare_agent`, `emit_agents_for_harness`, `cleanup_all_owned_agents`, `removed_disabled_owned`, `all_owned_in_dir`, `AgentWrite`, `write_agent_file`, `record_action` verbatim from `src/harness/sync.rs` into `src/harness/reconcile/agents.rs`; adjust visibility to `pub(crate)` as needed; carry the **mass-delete safeguard** (read-only DB open that *propagates* on an existing DB) intact (use devs:rust-dev agent).
- [ ] T030 [US3] Verify the agents reconcile-fn tests + `harness_sync_p6_idempotence.rs` + `harness_sync_p6_first_error.rs` + the `SyncOutcome` pin are unchanged and green (no behavioural diff) (use devs:rust-dev agent).
- [ ] T031 [GIT] Commit: `refactor(phase-7): move reconcile_agents into harness/reconcile/agents.rs (no behaviour change)`.
- [ ] T032 [GIT] Push `007-p7-d-a-reconcile-agents` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP.

### D.b — move `reconcile_guardrails`

- [ ] T033 [GIT] Branch `007-p7-d-b-reconcile-guardrails` off latest `main`.
- [ ] T034 [US3] Move `reconcile_guardrails` + `GuardrailsReconciliation`, `PreparedGuardrails`, `guardrails_target_path`, `guardrails_action_to_action` into `src/harness/reconcile/guardrails.rs`; carry the mass-delete safeguard intact (use devs:rust-dev agent).
- [ ] T035 [US3] Verify idempotence + first-error + wire-pin suites unchanged and green (use devs:rust-dev agent).
- [ ] T036 [GIT] Commit: `refactor(phase-7): move reconcile_guardrails into harness/reconcile/guardrails.rs (no behaviour change)`.
- [ ] T037 [GIT] Push `007-p7-d-b-reconcile-guardrails` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP.

### D.c — move `reconcile_hooks` + finalise the thin orchestrator

- [ ] T038 [GIT] Branch `007-p7-d-c-reconcile-hooks` off latest `main`.
- [ ] T039 [US3] Move `reconcile_hooks` + `HooksReconciliation`, `compute_plugins_with_hooks_json`, `merge_hooks_for_harness`, `remove_hooks_for_harness` into `src/harness/reconcile/hooks.rs`; carry the mass-delete safeguard intact (use devs:rust-dev agent).
- [ ] T040 [US3] Confirm `src/harness/sync.rs` is now the thin orchestrator (`sync_project` + `SyncDeps`/`SyncOutcome`/`SyncSubsystem`/`HarnessDecision`/`Action` + shared `HarnessSnapshot`/`collect_harness_snapshots`/`group_by_path`/`read_*_settings`/`relative_path` + the rules/mcp write helpers); add module-level docs to `reconcile/mod.rs` describing the fixed sink order + first-error precedence. **Do NOT fold in RUST-1/RUST-2 efficiency cleanups** (out of scope) (use devs:rust-dev agent).
- [ ] T041 [US3] Add (or carry) an explicit regression test asserting a DB-open error on an existing DB **aborts** the sync rather than producing an empty enabled set (the mass-delete safeguard) (use devs:rust-dev agent).
- [ ] T042 [US3] Final behaviour-preservation gate: `cargo test --test sync_idempotence --test harness_sync_p6_idempotence --test harness_sync_p6_first_error` + the `SyncOutcome` pin — all green, unchanged (use devs:rust-dev agent).
- [ ] T043 [GIT] Commit: `refactor(phase-7): move reconcile_hooks; sync.rs is now a thin orchestrator (no behaviour change)`.
- [ ] T044 [GIT] Push `007-p7-d-c-reconcile-hooks` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP.

### Decomposition closeout

- [ ] T045 Run codebase mapping for Phase 3 changes (`/sdd:map incremental`) — structural change diffs ARCHITECTURE.md + STRUCTURE.md (+ CONCERNS.md RUST-1/2 status).
- [ ] T046 Review `retro/P3.md`; extract critical learnings to `CLAUDE.md` (conservative).
- [ ] T047 [GIT] Commit on `007-p7-p3-closeout`: `docs(phase-7): P3 closeout — mapping + retro`; push + PR + CI green + LGTM stop.

**Checkpoint**: Reconcilers live in per-sink modules behind a thin orchestrator; zero behavioural diff. FR-007/FR-008 can now land in the clean structure.

---

## Phase 4: Beta-gate correctness [US1] (`contracts/correctness-beta-gate.md`)

**Goal**: Fix the five first-impression bugs (F-KNN, F-DOCTOR-RW, F-CACHE-KEY, F-MCP-PROMPT-COLLISION, F-WS-TOML-NEWLINE). No schema change, no new exit code.

**Independent Test**: per-fix integration tests (below) + the one-time real-model recall check (SC-001). Each `K*` is an independent small PR.

- [ ] T048 Create `specs/007-phase-7-beta-release/retro/P4.md` from the retro template.
- [ ] T049 [GIT] Commit: `docs(phase-7): init P4 retro`.

### K1 — F-KNN over-fetch + widen (FR-001, §R-2)

- [ ] T050 [GIT] Branch `007-p7-k1-knn-widen` off latest `main`.
- [ ] T051 [P] [US1] Write failing regression test `tests/search_knn_recall.rs` (stub embedder): place **≥`top_k` nearer non-matching rows** ahead of the match, on a corpus large enough that a fixed-multiplier over-fetch would still miss it; assert `min(top_k, total matches)` returned and the match present (use devs:rust-dev agent).
- [ ] T052 [US1] Implement the bounded over-fetch + widen loop in `src/index/query.rs::knn`: geometric `effective_k` growth, post-filter, terminate at `top_k` matches or candidate-set exhaustion; no schema change. Confirm `commands/query.rs` (`--strict`) + `mcp/tools/search_skills.rs` consume it unchanged (use devs:rust-dev agent).
- [ ] T053 [US1] Add the widen-ceiling test (filtered corpus genuinely < `top_k` → true smaller set, no error, no global leakage) (use devs:rust-dev agent).
- [ ] T054 [P] [US1] Write `tests/search_knn_recall_realmodel.rs` (`#[ignore]`d — real BGE models, not in fast CI) for the **one-time SC-001 recall check** on a realistically-populated multi-workspace index (use devs:rust-dev agent).
- [ ] T055 [GIT] Commit: `fix(query): over-fetch+widen so filtered KNN returns min(top_k, matches) (F-KNN)`.
- [ ] T056 [US1] Run the one-time real-model recall check **locally, before the K1 PR merges** (it is `#[ignore]`d out of the fast CI suite and downloads ~325 MB of BGE models on first run): `cargo test --test search_knn_recall_realmodel -- --ignored --nocapture`; record the result (recall figure + verdict) in `retro/P4.md`. This is SC-001's one-off gate, not a recurring CI check.
- [ ] T057 [GIT] Push `007-p7-k1-knn-widen` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP.

### K2 — F-DOCTOR-RW read-only + degrade (FR-002, §R-3)

- [ ] T058 [GIT] Branch `007-p7-k2-doctor-readonly` off latest `main`.
- [ ] T059 [P] [US1] Write `tests/doctor_readonly_schema.rs`: (a) stale-schema → doctor completes, no migration, no lock; (b) future-schema → degraded report, no exit-73 abort; (c) `--fix` on stale → migration runs (use devs:rust-dev agent).
- [ ] T060 [US1] Change `src/doctor/checks.rs` (~l.58–68) to `index::open_read_only` + swallow the schema error to a degraded report (mirror `check_index` in `doctor/mod.rs`); keep `--fix`'s lock-held `repair_schema` (use devs:rust-dev agent).
- [ ] T061 [GIT] Commit: `fix(doctor): open index read-only, degrade not abort on schema mismatch (F-DOCTOR-RW)`.
- [ ] T062 [GIT] Push `007-p7-k2-doctor-readonly` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP.

### K3 — F-CACHE-KEY scrubbed-URL keying (FR-003, §R-4)

- [ ] T063 [GIT] Branch `007-p7-k3-cache-key` off latest `main`.
- [ ] T064 [P] [US1] Write `tests/catalog_ssh_roundtrip.rs`: add a catalog from a plain-SSH source; assert `show`/`update`/`remove` resolve the cached clone and **zero orphaned clones**; keep a plain-`https` (raw==scrubbed) regression case (use devs:rust-dev agent).
- [ ] T065 [US1] In `src/commands/catalog/add.rs`, key `cache_dir_for` + `refcount_by_url` by the **scrubbed** URL; keep `git.clone_shallow` on the **raw** URL for auth (use devs:rust-dev agent).
- [ ] T066 [GIT] Commit: `fix(catalog): key cache dir + refcount by scrubbed URL so SSH sources round-trip (F-CACHE-KEY)`.
- [ ] T067 [GIT] Push `007-p7-k3-cache-key` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP.

### K4 — F-MCP-PROMPT-COLLISION global taken-set (FR-004, §R-5)

- [ ] T068 [GIT] Branch `007-p7-k4-prompt-collision` off latest `main`.
- [ ] T069 [P] [US1] Write `tests/prompt_collision_global.rs`: Command `foo` + user-invocable Skill `foo` + Command `foo2` → all three present in `prompts/list`, all resolvable on `prompts/get`, `doctor` reports the true resolution (use devs:rust-dev agent).
- [ ] T070 [US1] In `src/mcp/prompt_collision.rs` (+ consumer `prompts.rs`), assign final names against one **global taken-set**, suffixing `{base}{n}` until free and re-checking each suffix against the same set before the terminal insert (use devs:rust-dev agent).
- [ ] T071 [GIT] Commit: `fix(mcp): assign prompt names against a single global taken-set (F-MCP-PROMPT-COLLISION)`.
- [ ] T072 [GIT] Push `007-p7-k4-prompt-collision` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP. (End-to-end verification lands in Phase 6 / T1.)

### K5 — F-WS-TOML-NEWLINE toml_edit emission + control-char reject (FR-005, §R-6)

- [ ] T073 [GIT] Branch `007-p7-k5-ws-toml` off latest `main`.
- [ ] T074 [P] [US1] Write `tests/workspace_toml_control_chars.rs`: a newline-bearing catalog name is (a) rejected at the manifest boundary; (b) if already present (via `workspace init --inherit-global`), the emitted `settings.toml` stays parseable + every harness op succeeds (no exit-70 brick) (use devs:rust-dev agent).
- [ ] T075 [US1] Emit `settings.toml` via `toml_edit` in `src/workspace/init.rs` (delete `escape_toml_basic`; match the sibling `rename`/`regen_summary` paths) (use devs:rust-dev agent).
- [ ] T076 [US1] Reject control chars in the recognised catalog `name` field at the manifest boundary in `src/catalog/manifest.rs` (value reject; unknown fields stay lenient) (use devs:rust-dev agent).
- [ ] T077 [GIT] Commit: `fix(workspace): emit settings.toml via toml_edit + reject control chars in catalog names (F-WS-TOML-NEWLINE)`.
- [ ] T078 [GIT] Push `007-p7-k5-ws-toml` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP.

### US1 closeout

- [ ] T079 Run codebase mapping for Phase 4 changes (`/sdd:map incremental`).
- [ ] T080 Review `retro/P4.md` (incl. the recorded real-model recall result); extract critical learnings to `CLAUDE.md` (conservative).
- [ ] T081 [GIT] Commit on `007-p7-p4-closeout`: `docs(phase-7): P4 closeout — mapping + retro`; push + PR + CI green + LGTM stop.

**Checkpoint**: The five first-impression bugs fixed; semantic search proven against the real models (SC-001).

---

## Phase 5: Robustness & honest trust posture [US2] (`contracts/robustness-trust.md`, `contracts/symlink-guard.md`)

**Goal**: Bound third-party reads (FR-006), harden the symlink guard across **all** sinks in one pass (FR-007), fix the OpenCode rules body (FR-008), and close the catalog-remove TOCTOU (FR-009). (The FR-010 trust-model doc lands with REL4 but is tracked here.)

**Independent Test**: per-fix integration tests (below); SC-005 (bounded named error, no OOM), SC-006 (intermediate-component refusal). FR-007/FR-008 depend on the Phase 3 decomposition + the F2 spike outcome.

- [ ] T082 Create `specs/007-phase-7-beta-release/retro/P5.md` from the retro template.
- [ ] T083 [GIT] Commit: `docs(phase-7): init P5 retro`.

### R1 — bounded third-party reads (FR-006, §R-7)

- [ ] T084 [GIT] Branch `007-p7-r1-bounded-reads` off latest `main`.
- [ ] T085 [P] [US2] Write `tests/bounded_reads.rs`: feed an oversized file at each site through `enable`/`show`/`list`/`doctor`; assert a bounded named per-class error (never OOM) (use devs:rust-dev agent).
- [ ] T086 [US2] Route every unbounded third-party read through `crate::util::bounded_read(path, <per-class cap>)` at the site list — `plugin/manifest.rs:61`, `catalog/manifest.rs:46`, `plugin/lifecycle.rs:958`, `plugin/components.rs:170`, `doctor/checks.rs:174`; grep `fs::read`/`read_to_string` to confirm no sibling site is missed (fix the class) (use devs:rust-dev agent). *(Cross-module sweep — the noted exception to the ≤2-module cap; one theme.)*
- [ ] T087 [GIT] Commit: `fix(plugin,catalog): bound every third-party read by its per-class cap (F-PLUGIN-MANIFEST-DOS class)`.
- [ ] T088 [GIT] Push `007-p7-r1-bounded-reads` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP.

### R2 — symlink-guard SSOT primitive, ALL sinks in one pass (FR-007, §R-1)

- [ ] T089 [GIT] Branch `007-p7-r2-symlink-guard` off latest `main`.
- [ ] T090 [US2] Implement the single rustix-backed symlink-safe write primitive in `src/util/symlink_safe.rs` per the F2 spike outcome (primary: `openat2(RESOLVE_NO_SYMLINKS)` / per-component `openat`+`O_NOFOLLOW`; fallback: final-node `O_NOFOLLOW`) (use devs:rust-dev agent).
- [ ] T091 [US2] Route **all** write sinks through the primitive in ONE pass — `harness/hooks.rs`, `harness/guardrails.rs`, `harness/agents.rs`, `harness/rules_file.rs`, `harness/mcp_config.rs`, `util/atomic_dir.rs` — and make the existing duplicated `refuse_symlink` copies delegate to it. Each sink's refusal returns **its dedicated** write-guard code (settings 44, guardrails 46, …), never generic `Io` (7) (the CON-1 precedent) (use devs:rust-dev agent).
- [ ] T092 [US2] Complete `tests/symlink_intermediate_guard.rs`: an **intermediate** directory-component symlink on each sink's path is refused with that sink's dedicated code; the final-node refusal still holds; fixture gated `#[cfg(target_os="linux")]`, macOS exercises the portable walk (SC-006) (use devs:rust-dev agent).
- [ ] T093 [GIT] Commit: `feat(harness): symlink-safe write primitive across all sinks (FR-007; intermediate-component guard)`.
- [ ] T094 [GIT] Push `007-p7-r2-symlink-guard` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP.

### R3 — OpenCode inline rules body (FR-008, §R-8)

- [ ] T095 [GIT] Branch `007-p7-r3-opencode-rules` off latest `main`.
- [ ] T096 [P] [US2] Write `tests/rules_opencode_inline.rs`: OpenCode + Codex (and OpenCode + Gemini) → OpenCode's shared file contains the **inline body**, not `@.tome/RULES.md` as literal prose; the include-capable harness still resolves it (use devs:rust-dev agent).
- [ ] T097 [US2] In `src/harness/sync.rs::compute_rules_body`, pick the lowest-common-denominator body style: if any live sharer requires `Inline`, write `Inline` (mirrors the guardrails reconciler's union) (use devs:rust-dev agent).
- [ ] T098 [GIT] Commit: `fix(harness): write inline rules body when any sharer needs it so OpenCode receives Tome's rules (F-RULES-OPENCODE)`.
- [ ] T099 [GIT] Push `007-p7-r3-opencode-rules` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP.

### R4 — catalog remove TOCTOU (FR-009, §R-9)

- [ ] T100 [GIT] Branch `007-p7-r4-remove-toctou` off latest `main`.
- [ ] T101 [P] [US2] Write `tests/catalog_remove_toctou.rs`: a `catalog remove --force` racing a concurrent `plugin enable` (serialised on `index.lock`) leaves no ghost-enabled plugin; single-process case is the regression guard (use devs:rust-dev agent).
- [ ] T102 [US2] In `src/commands/catalog/remove.rs`, re-derive the enabled-plugins cascade input **inside** the lock-held closure (don't reuse the pre-lock `Vec`) (use devs:rust-dev agent).
- [ ] T103 [GIT] Commit: `fix(catalog): re-derive remove --force cascade inside the lock (F-REMOVE-TOCTOU)`.
- [ ] T104 [GIT] Push `007-p7-r4-remove-toctou` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP.

### US2 closeout

- [ ] T105 Run codebase mapping for Phase 5 changes (`/sdd:map incremental`) — FR-007 closes CONCERNS.md TD-062 / SEC-019 / C-1.
- [ ] T106 Review `retro/P5.md`; extract critical learnings to `CLAUDE.md` (conservative).
- [ ] T107 [GIT] Commit on `007-p7-p5-closeout`: `docs(phase-7): P5 closeout — mapping + retro`; push + PR + CI green + LGTM stop.

**Checkpoint**: Read boundary bounded; symlink guard hardened across every sink; OpenCode receives rules; remove race closed.

---

## Phase 6: Test foundation & cleanup [US3] (`contracts/mcp-test-harness.md`, `contracts/cleanup-bundle.md`)

**Goal**: Build the in-process MCP test harness (closes GAP-1; verifies the K4 fix end-to-end) and clear the cleanup bundle (FR-013/014/015/016). No schema change, no new exit code.

**Independent Test**: `tests/exit_codes_e2e_mcp.rs` exercises codes 9, 26–29 against a real server instance (SC-010) + verifies the command+skill+`foo2` collision end-to-end; each cleanup defect has a test or truthful message.

- [ ] T108 Create `specs/007-phase-7-beta-release/retro/P6.md` from the retro template.
- [ ] T109 [GIT] Commit: `docs(phase-7): init P6 retro`.

### T1 — in-process MCP test harness + GAP-1 + FR-004 verification (FR-012, §R-11)

- [ ] T110 [GIT] Branch `007-p7-t1-mcp-harness` off latest `main`.
- [ ] T111 [US3] Build the in-process MCP server driver in `tests/common/` (construct + drive a real server via the library API with `StubEmbedder`; issue `initialize`/`prompts/list`/`prompts/get`/tool calls; observe exit codes); reuse the `#[doc(hidden)] pub static` + RAII-guard seam (promote `EnvVarGuard`/`ENV_MUTEX` to `tests/common/mod.rs` if this is the 5th consumer) (use devs:rust-dev agent).
- [ ] T112 [US3] Write `tests/exit_codes_e2e_mcp.rs` exercising GAP-1 codes **9** (`PluginDataDirWriteFailed`), **26** (`PromptArgumentMismatch`), **27** (`EntryNotFound`), **28** (`SubstitutionFailed`), **29** (`InvalidArgumentFrontmatter`) end-to-end (use devs:rust-dev agent).
- [ ] T113 [US3] Add the FR-004 end-to-end verification to `exit_codes_e2e_mcp.rs`: command+skill+`foo2` → all present + resolvable through `prompts/list` + `prompts/get` (use devs:rust-dev agent).
- [ ] T114 [US3] Confirm `tests/sync_boundary.rs` stays green (the harness drives the existing `src/mcp/` async island; no new runtime) (use devs:rust-dev agent).
- [ ] T115 [GIT] Commit: `test(mcp): in-process MCP harness covering exit codes 9,26–29 + prompt-collision e2e (GAP-1, FR-012)`.
- [ ] T116 [GIT] Push `007-p7-t1-mcp-harness` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP.

### C1a — correctness cleanups (FR-013, FR-014, FR-015)

- [ ] T117 [GIT] Branch `007-p7-c1a-correctness-cleanups` off latest `main`.
- [ ] T118 [P] [US3] FR-013: detect intra-plugin duplicate `(kind, name)`, `tracing`-warn it, count rows actually written (`plugin/lifecycle.rs` + `index/skills.rs`); respect `sqlite-vec`'s `DELETE`-then-`INSERT` (no `ON CONFLICT`); add a test (use devs:rust-dev agent).
- [ ] T119 [P] [US3] FR-014: map malformed `~/.tome/config.toml` to `ManifestInvalid::TomlParse` (exit 5) in `src/catalog/store.rs:20–35`; add an exit-5 test (use devs:rust-dev agent).
- [ ] T120 [P] [US3] FR-015: fail closed on a non-array `hooks` event value → `HookSettingsWriteFailed` (exit 44) in `harness/hooks.rs` (don't coerce to `[]`); distinguish a meta-corruption row from a fresh DB in `index/migrations.rs` (explicit match, existing variants, no new code); add tests (use devs:rust-dev agent).
- [ ] T121 [GIT] Commit: `fix(phase-7): off-spec inputs fail closed; config parse → exit 5; duplicate (kind,name) warned + truthfully counted (FR-013/014/015)`.
- [ ] T122 [GIT] Push `007-p7-c1a-correctness-cleanups` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP.

### C1b — dead-code removal + doc-comment sweep + `--help` citation strip (FR-016)

- [ ] T123 [GIT] Branch `007-p7-c1b-doc-hygiene` off latest `main`.
- [ ] T124 [P] [US3] Delete the dead `store::reference_count` accessor + fix its misdirecting doc pointer in `paths.rs` (use devs:rust-dev agent).
- [ ] T125 [P] [US3] Sweep stale doc-comments describing shipped features as stubs: `mcp/prompts.rs` (the `METHOD_NOT_FOUND` claim), `commands/workspace/use_.rs` (the "US1.a stub" comment), `mcp/tools/search_skills.rs` (the "F2a single global config" comment), `substitution/mod.rs` (the Stage-3 no-args comment), `index/meta.rs` (dead `MetaKey::LastWriterPid`), the embedding registry size-vs-hash comment (use devs:rust-dev agent).
- [ ] T126 [P] [US3] Strip internal `FR-`/`NFR-`/`contracts/*.md` citations from user-facing clap `///` doc-comments in `src/cli.rs` (DOC-06) (use devs:rust-dev agent).
- [ ] T127 [US3] Verify: `cargo clippy --all-targets --all-features -- -D warnings` clean (doc-lints); no `FR-`/`NFR-`/`contracts/` token in `--help` output (use devs:rust-dev agent).
- [ ] T128 [GIT] Commit: `refactor(phase-7): remove dead reference_count; sweep stale doc-comments; strip internal citations from --help (FR-016)`.
- [ ] T129 [GIT] Push `007-p7-c1b-doc-hygiene` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP.

### US3 closeout

- [ ] T130 Run codebase mapping for Phase 6 changes (`/sdd:map incremental`) — closes CONCERNS.md GAP-1; close the `ProjectBindingState` wire-pin opportunistically if touched.
- [ ] T131 Review `retro/P6.md`; extract critical learnings to `CLAUDE.md` (conservative).
- [ ] T132 [GIT] Commit on `007-p7-p6-closeout`: `docs(phase-7): P6 closeout — mapping + retro`; push + PR + CI green + LGTM stop.

**Checkpoint**: MCP surface has end-to-end exit-code coverage; the repo no longer reads as half-finished. Hardening (US1–US3) complete — the release wrapper can begin.

---

## Phase 7: Release wrapper — install path [US4 + release-prerequisite US5 items] (`contracts/release-pipeline.md`, `contracts/repo-hygiene.md`)

**Goal**: Rename the crate, add discovery metadata, and stand up the cargo-dist pipeline so `cargo install tome-mcp` / `brew install …/tome` / prebuilt binaries work. **Gated on F1 (amendment) merged.** Publish/tag remain user-reserved.

**Independent Test**: `cargo install tome-mcp` (or path/dry-run) yields a runnable `tome`; per-target `ldd`/`otool` shows no `libonnxruntime` (SC-007); `cargo package --list` ships `vendor/sqlite-vec`; `cargo publish --dry-run --locked` exits 0.

- [ ] T133 Create `specs/007-phase-7-beta-release/retro/P7.md` from the retro template.
- [ ] T134 [GIT] Commit: `docs(phase-7): init P7 retro`.

### REL1 — crate rename `tome` → `tome-mcp` (FR-017, §R-12)

- [ ] T135 [GIT] Branch `007-p7-rel1-crate-rename` off latest `main`.
- [ ] T136 [US4] In `Cargo.toml`, set `[package] name = "tome-mcp"` and add `[[bin]] name = "tome"`; add the complexity-budget rationale comment (use devs:rust-dev agent).
- [ ] T137 [US4] `cargo check` to regenerate `Cargo.lock`'s `name` field **before** committing (avoid dirtying `--locked`); sweep `tome`-as-package references — CI workflows, the binary-size assertion (`target/release/tome` stays valid), `tests/exit_codes_e2e.rs` (CLI binary by name), the `--version`/`-V` pre-parse (use devs:rust-dev agent).
- [ ] T138 [US4] Verify `cargo build --release --locked` → `target/release/tome` exists; `./target/release/tome --version` + `-V` correct post-rename.
- [ ] T139 [GIT] Commit: `chore(phase-7)!: rename crate tome→tome-mcp, keep [[bin]] name = tome (FR-017)`.
- [ ] T140 [GIT] Push `007-p7-rel1-crate-rename` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP.

### REL2 — discovery metadata + docs.rs + CHANGELOG (FR-025, §R-16) `[US5]` (publish prerequisite)

- [ ] T141 [GIT] Branch `007-p7-rel2-metadata` off latest `main`.
- [ ] T142 [US5] Add `authors`, `homepage`, `documentation` to `Cargo.toml`; add `[package.metadata.docs.rs]` (feature-gate the embedder off for docs) or set `documentation` → repo as fallback (use devs:rust-dev agent).
- [ ] T143 [US5] Move the `CHANGELOG.md` `[Unreleased]` section to the top; keep the first public version `0.6.0`; add a v0.6.0 row to `RELEASE-BINARY-SIZE.md` (record the post-rename size).
- [ ] T144 [GIT] Commit: `docs(phase-7): crate discovery metadata + docs.rs config + CHANGELOG/[Unreleased] reorder (FR-025)`.
- [ ] T145 [GIT] Push `007-p7-rel2-metadata` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP.

### REL3 — cargo-dist release pipeline + licence bundle (FR-018/019/020, NFR-007/008/010, §R-13/14/15)

- [ ] T146 [GIT] Branch `007-p7-rel3-cargo-dist` off latest `main`. **(Requires F1 merged.)**
- [ ] T147 [US4] Configure `cargo-dist` for Linux + macOS × x86_64 + aarch64 (archives + checksums); generate `.github/workflows/release.yml` subject to the same gates as CI (fmt/clippy/`cargo-deny`, `actions/checkout@v5`) and `--locked` (NFR-008/010).
- [ ] T148 [US4] Add a per-target self-contained check to the release workflow: `ldd`/`otool -L` asserts **no `libonnxruntime`** (the Linux build targets a glibc baseline) (SC-007, FR-018).
- [ ] T149 [US4] Configure the Homebrew-tap push to `aaronbassett/homebrew-tap` via the operator-provided least-privilege cross-owner PAT secret (never logged); generate the `tome` formula (FR-019, NFR-010).
- [ ] T150 [US4] Generate `THIRD-PARTY-LICENSES` with `cargo-about` over the cargo graph + append the native-component notices (ONNX Runtime, llama.cpp, vendored sqlite-vec); attach to releases (FR-019, NFR-007).
- [ ] T151 [US4] Add the Cargo `include`/`exclude` allowlist scaffolding note (the actual untracking + tarball trim is REL5 in Polish); verify `cargo publish --dry-run --locked` exits 0 and `cargo package --list` ships `vendor/sqlite-vec` + `build.rs`. **Explicitly assert the stripped release binary stays under the 50 MB cap** in the release pipeline (`stat` `target/release/tome`; FR-020/SC-009) — complements the existing CI size gate and the figure recorded in `RELEASE-BINARY-SIZE.md` (T143).
- [ ] T152 [GIT] Commit: `ci(phase-7): cargo-dist pipeline (Linux+macOS prebuilt binaries, tap, licence bundle) (FR-018/019/020)`.
- [ ] T153 [GIT] Push `007-p7-rel3-cargo-dist` + open PR; CI green (incl. a dry-run release on a test tag if cargo-dist supports it); report `PR READY FOR MERGE. AWAITING LGTM` and STOP. (Actual publish/tag are user-reserved.)

### Release-install closeout

- [ ] T154 Run codebase mapping for Phase 7 changes (`/sdd:map incremental`) — STACK.md (rustix direct; cargo-dist/cargo-about tooling), INTEGRATIONS.md (crates.io/Homebrew tap).
- [ ] T155 Review `retro/P7.md`; extract critical learnings to `CLAUDE.md` (conservative).
- [ ] T156 [GIT] Commit on `007-p7-p7-closeout`: `docs(phase-7): P7 closeout — mapping + retro`; push + PR + CI green + LGTM stop.

**Checkpoint**: The crate is `tome-mcp` (command `tome`); the pipeline produces self-contained prebuilt binaries + a tap formula + a licence bundle, wired but not triggered.

---

## Phase 8: Release wrapper — public front door [US5] (`contracts/repo-hygiene.md`, `contracts/robustness-trust.md` FR-010)

**Goal**: Make the README the front door and open a security channel. (FR-024 untracking is deferred to Polish so the closeout review still has its working context.)

**Independent Test**: read the README top-to-bottom and run every getting-started command end-to-end (automated check uses a `file://` fixture, SC-008); a private-reporting channel exists; the security page draws the mechanical-vs-semantic line.

- [ ] T157 Create `specs/007-phase-7-beta-release/retro/P8.md` from the retro template.
- [ ] T158 [GIT] Commit: `docs(phase-7): init P8 retro`.

### REL4 — README + SECURITY + trust-model doc (FR-021, FR-022, FR-010)

- [ ] T159 [GIT] Branch `007-p7-rel4-readme-security` off latest `main`.
- [ ] T160 [US5] Rewrite `README.md` as the front door: lead with "a Rust CLI **and** MCP server"; state build prerequisites (C/C++ toolchain + CMake + the build-time ORT download) + supported platforms (Linux + macOS; Windows untested); no-telemetry guarantee; **Qwen2.5-0.5B = Apache-2.0** (fix LIC-002); real install commands (`cargo install tome-mcp`; `brew install …/tome`; `--path .` fallback); **absolute** repo links; a worked example pointing at a real public catalog (or a `file://` fixture).
- [ ] T161 [US5] Add `SECURITY.md` (incl. the FR-010 mechanical-vs-semantic trust framing: enumerate the mechanical defences — bounded reads, path-segment validation, the FR-007 symlink guard, atomic writes — and state plainly that Tome cannot vet catalog *content* and "only add catalogs you trust"); remove the `security@example.invalid` placeholder in `CONTRIBUTING.md:66`.
- [ ] T162 [US5] Add a `tests/`/CI smoke check that every README getting-started command resolves against a `file://` local-catalog fixture (SC-008, decoupled from the public catalog fork); rebuild the binary first (HYG-7).
- [ ] T163 [GIT] Commit: `docs(phase-7): README front door + SECURITY.md + trust-model doc (FR-021/022/010)`.
- [ ] T164 [GIT] Push `007-p7-rel4-readme-security` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP.

### Front-door closeout

- [ ] T165 Run codebase mapping for Phase 8 changes (`/sdd:map incremental`).
- [ ] T166 Review `retro/P8.md`; extract critical learnings to `CLAUDE.md` (conservative).
- [ ] T167 [GIT] Commit on `007-p7-p8-closeout`: `docs(phase-7): P8 closeout — mapping + retro`; push + PR + CI green + LGTM stop.

**Checkpoint**: README + security channel ready; every getting-started command resolves. All feature/doc surface complete — Polish reviews the assembled whole.

---

## Phase 9: Polish & Cross-Cutting Concerns

**Purpose**: Phase-wide review over the assembled surface (decomposition × four-sink symlink guard × release wrapper), final docs, then the FR-024 untracking as the **absolute final step**, then the user-reserved release handoff.

- [ ] T168 Create `specs/007-phase-7-beta-release/retro/P9.md` from the retro template.
- [ ] T169 [GIT] Commit: `docs(phase-7): init P9 retro`.
- [ ] T170 Dispatch the phase-wide 4-reviewer parallel pass (contract / Rust-lens / test / security) over the whole Phase 7 surface in ONE message; reviewers read CONCERNS.md + the per-slice notes first. Write findings to `specs/007-phase-7-beta-release/review/findings.md`.
- [ ] T171 Triage into `specs/007-phase-7-beta-release/review/disposition.md` (applied vs deferred, with severity → PR shape).
- [ ] T172 [GIT] Commit (on `007-p7-p9-review`): `docs(review): phase-7 phase-wide findings + disposition`; push + PR + CI green + LGTM stop. (Findings committed BEFORE fixes.)
- [ ] T173 Apply blockers/majors from the disposition as small themed PRs (one branch + PR each; CI green + LGTM stop per PR; use devs:rust-dev agent for Rust fixes). Confirm: no new exit code, no schema change, no new top-level dep introduced by any fix.
- [ ] T174 [US5] Finalise `CHANGELOG.md` for v0.6.0 (the public-beta entry: bug-fix bundle, harness decomposition, in-process MCP harness, release wrapper, the `rustix` promotion + crate rename headline, "no new exit codes/schema"); verify the version is `0.6.0`.
- [ ] T175 Run the final codebase mapping (`/sdd:map incremental`); review `retro/P9.md`; extract critical learnings to `CLAUDE.md` (conservative). **Do this BEFORE the untracking step (T177) renders `CLAUDE.md` untracked.**
- [ ] T176 [GIT] Commit on `007-p7-p9-docs`: `docs(phase-7): P9 closeout — final mapping + retro + CHANGELOG`; push + PR + CI green + LGTM stop.

### REL5 — FR-024 untracking (THE FINAL STEP, §R-18)

- [ ] T177 [GIT] Branch `007-p7-rel5-untrack` off latest `main` — **only after every prior PR (T001–T176) has merged.**
- [ ] T178 [US5] `git rm --cached` (keep local copies) `specs/`, `review/`, `retro/`, `.sdd/`, `CLAUDE.md`, and the two `*.local.json`; add them to `.gitignore`. **Keep `CONSTITUTION.md` tracked.**
- [ ] T179 [US5] Add the `Cargo.toml include`/`exclude` allowlist (`src/**`, `build.rs`, `vendor/**`, `Cargo.toml`, `Cargo.lock`, `README.md`, `CHANGELOG.md`, `LICENSE-*`); re-run `cargo package --list` to confirm `vendor/sqlite-vec` + `build.rs` ship and the internal dirs are excluded; `git rm --cached` the two `.local.json`.
- [ ] T180 [GIT] Commit: `chore(phase-7): untrack internal process artifacts + add crate include allowlist (FR-024)`; push `007-p7-rel5-untrack` + open PR; CI green; report `PR READY FOR MERGE. AWAITING LGTM` and STOP.

### User-reserved release handoff (NOT executed by the implementer)

- [ ] T181 ⚠️ **USER-RESERVED** — after all PRs merge + `cargo publish --dry-run --locked` is green: rebuild the binary (HYG-7), then **the user** tags `v0.6.0`, runs `cargo publish` (as `tome-mcp`), merges the Homebrew-tap PR, and posts release notes. **Operator-only**: enable GitHub private vulnerability reporting; `gh repo edit --description … --add-topic rust,cli,mcp,claude-code,ai,plugins`; provide the Homebrew-tap PAT secret.

**Checkpoint**: Phase 7 complete — Tome is a publicly installable, hardened beta, pending the user-reserved publish.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (P1)**: planning artifacts land first.
- **Foundational (P2)**: **F1 (amendment) BLOCKS REL3 (cargo-dist)**; **F2 (rustix spike) BLOCKS R2 (symlink guard)**; F3 (checkout) is independent + time-sensitive (land first).
- **Decomposition (P3)**: **BLOCKS R2 (FR-007) + R3 (FR-008)** — they land in the clean structure.
- **US1 (P4)**: independent of US2/US3; the **K4 fix** is verified end-to-end later by **T1 (FR-012)**.
- **US2 (P5)**: R2/R3 depend on P3 + F2; R1/R4 are independent.
- **US3 (P6)**: T1 depends on K4 (for the verification regression); C1a/C1b are independent.
- **Release (P7/P8)**: gated on **US1–US3 merged**; REL1 (rename) before REL2/REL3; REL3 needs F1.
- **Polish (P9)**: after all USs; **FR-024 untracking (REL5) is the absolute last code step**; the v0.6.0 tag + publish are **user-reserved** beyond that.

### Within each slice

- Tests written first where practical (K1's regression must FAIL before the fix); for behaviour-preserving moves (P3), the evidence is the **unchanged** pre-existing suites.
- Each slice = one small PR (~≤400 lines / ≤2 modules) → hooks pass → CI green → LGTM stop.

### Parallel Opportunities

- F1 / F2 / F3 are independent (3 parallel PRs); F3 should merge first (deadline).
- After P3 merges, US1 (K1–K5) and the P3-independent US2 fixes (R1, R4) can proceed in parallel.
- Within a slice, `[P]` test-authoring tasks run alongside reading the target code.
- The phase-wide review (T170) fans out 4 reviewers in one dispatch.

---

## Parallel Example: Foundational (Phase 2)

```text
# Three independent foundational PRs (F3 merges first — deadline):
Branch 007-p7-f3-checkout-v5:        bump actions/checkout v4→v5 (ci.yml, security.yml ×2)
Branch 007-p7-f1-constitution-amendment: rewrite §Release tooling clause → v1.4.0
Branch 007-p7-f2-rustix-spike:       promote rustix direct + confirm openat2/O_NOFOLLOW primitive
```

## Parallel Example: US1 (Phase 4, after P3 merges)

```text
Branch 007-p7-k1-knn-widen:      over-fetch+widen in src/index/query.rs + recall tests
Branch 007-p7-k2-doctor-readonly: open_read_only + degrade in src/doctor/checks.rs
Branch 007-p7-k3-cache-key:      scrubbed-URL keying in src/commands/catalog/add.rs
Branch 007-p7-k4-prompt-collision: global taken-set in src/mcp/prompt_collision.rs
Branch 007-p7-k5-ws-toml:        toml_edit emission + control-char reject
```

---

## Implementation Strategy

### Hardening first (the beta gate)

1. P1 Setup → P2 Foundational (amendment + rustix spike + checkout).
2. **P3 decomposition (behaviour-preserving) — lands first.**
3. P4 US1 beta-gate fixes → **STOP and VALIDATE** (incl. the one-time real-model recall check, SC-001). This is the minimum "first run no longer looks broken" increment.
4. P5 US2 robustness (symlink guard one-pass, bounded reads, OpenCode, TOCTOU).
5. P6 US3 test foundation (in-process MCP harness) + cleanup bundle.

### Release wrapper last

6. P7 install path (rename → metadata → cargo-dist pipeline). 7. P8 README + SECURITY.
8. P9 Polish: phase-wide review → fixes → final docs → **FR-024 untracking (last)** → user-reserved publish handoff.

### Incremental delivery

Each slice is an independently reviewable PR that keeps `main` green. The hardening PRs (P3–P6) deliver a trustworthy artifact before the release wrapper makes it public.

---

## User-reserved & operator-only actions (NEVER run by the implementer)

- **User-reserved**: `cargo publish` (final, as `tome-mcp`); the `v0.6.0` git tag; the Homebrew-tap PR merge; release-notes posting. (Per the constitution + standing project discipline — the pipeline is wired, the trigger is the user's.)
- **Operator-only**: enable GitHub private vulnerability reporting (Settings → Code security); `gh repo edit --description … --add-topic …`; provide the least-privilege Homebrew-tap PAT secret.

---

## Notes

- `[P]` = different files, no incomplete-task dependency. `[Story]` maps a task to its spec US (some US5 metadata tasks sit in the earlier release-install phase as publish prerequisites — phase = dependency order, label = US).
- **No new exit code, no schema change, no new top-level dependency** anywhere in Phase 7 (`rustix` is transitive→direct; cargo-dist/cargo-about are CI tools). Re-confirm at each slice.
- **Never** `--no-verify`; **never** `--amend` a failed pre-commit (it aborted — make a new commit). `cargo check` after the crate rename **before** committing (keep `--locked` clean).
- `-D warnings` promotes clippy doc-lints `cargo test` ignores — run clippy on touched files before committing (Phase 6 P8 lesson).
- ubuntu/MSRV CI flake (bus-error/no-space on heavy C/C++ builds) is infra — `gh run rerun --failed`; the cargo-dist matrix lands on the same runners (budget retry tax).
- The behaviour-preservation gate for P3 is the **unchanged** idempotence/first-error/wire-pin suites — do not edit them to make the refactor pass.
- Stop at any phase checkpoint to validate the increment independently.
