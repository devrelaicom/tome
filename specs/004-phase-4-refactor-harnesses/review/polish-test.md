# Phase 4 Polish — Test Audit

Branch: `004-phase-4-polish-pr-a` at `/Users/aaronbassett/Projects/devrel-ai/tome`
Surface: 916 tests across 125 suites on main; Phase 4 added ~426 tests across ~61 new suites.
Reviewer scope: phase-wide consolidation only (cross-suite fixture duplication, JSON wire-shape pin completeness, exit-code e2e gaps, mutex/serialisation discipline, ignore markers).

---

## Blockers (0)

No blockers. The test surface is correct (production code is exercised); the gaps below are coverage thinness, not coverage absence, and the fixture duplication is reviewable code, not broken code.

---

## Majors (11)

### M1 — `seed_bound_project` duplicated across 6 workspace test files

Files: `tests/workspace_rename.rs:47`, `tests/workspace_sync.rs:45`, `tests/workspace_remove.rs:48`, `tests/workspace_remove_cascade.rs:34`, `tests/workspace_regen_summary.rs:73`, `tests/workspace_list.rs:194` (variant `seed_bound_project_for_test`).

All six implementations are byte-for-byte identical except `workspace_remove_cascade.rs` which extends the project marker with `harnesses = ["claude-code"]\n`. The shared core:

```rust
fn seed_bound_project(paths: &..., workspace_name: &str, project_root: &Path) {
    fs::create_dir_all(project_root.join(".tome"))...;
    fs::write(project_root.join(".tome/config.toml"), format!("workspace = \"{workspace_name}\"\n"))...;
    let conn = open_central(paths);
    let workspace_id: i64 = conn.query_row("SELECT id FROM workspaces WHERE name = ?1", ...);
    conn.execute("INSERT INTO workspace_projects (project_path, workspace_id, bound_at) VALUES (?1, ?2, ?3)", ...);
}
```

The `workspace_list.rs:191` comment ("Other suites have their own copies; we keep the helper inline here to avoid a `common/mod.rs` widening") explicitly acknowledges the duplication and rejects promotion — but that justification holds at N=2, not N=6. At six call sites the promotion cost is the new `tests/common/mod.rs` line; the maintenance cost without promotion is N updates to every helper change. Promote `seed_bound_project(paths, name, project_root, marker_extra: Option<&str>) -> ()` into `tests/common/mod.rs`. Why: the bug class this guards against (the marker config format diverging from `workspace_projects` schema) is the exact class that benefits from a single point of truth.

### M2 — `open_central` duplicated across 7 workspace/summariser test files

Files: `tests/workspace_rename.rs`, `tests/workspace_sync.rs`, `tests/workspace_list.rs`, `tests/workspace_remove.rs`, `tests/workspace_init.rs`, `tests/workspace_regen_summary.rs`, `tests/workspace_remove_cascade.rs`.

Two variants exist: the registry-seed form (5 files use `tome::commands::plugin::registry_seeds()`) and the stub-seed form (2 files use `stub_*_seed()` from `common`). Both shapes are reasonable, but the registry-seed variant duplicates the helper-promotion blocker because `registry_seeds` is already `pub(crate)`-style accessible via the lib. Promote two thin wrappers into `tests/common/mod.rs`:

```rust
pub fn open_central_registry_seeded(paths: &Paths) -> rusqlite::Connection { ... }
pub fn open_central_stub_seeded(paths: &Paths) -> rusqlite::Connection { ... }
```

Why: a `meta` open with mismatched seeds is the single most common Phase 4 test setup foot-gun (drift detection on subsequent opens). A central helper documents the discipline.

### M3 — `seed_enabled_skill` duplicated across 5+ summariser/workspace test files

Files: `tests/summariser_cache.rs:28`, `tests/summariser_forward_progress.rs:49`, `tests/plugin_summariser_forward_progress.rs:50`, `tests/workspace_regen_summary.rs:40`, `tests/workspace_list.rs:162` (variant `seed_enabled_skill_for_test`).

All five seed a `skills` row + a `workspace_skills` junction row. Signatures diverge slightly (some take `(workspace_name, skill_name)`, some take the full `(catalog, plugin, skill_name, description)` tuple). The N=5 callers cluster around one of two shapes; both should be promoted. Add to `tests/common/mod.rs`:

```rust
pub fn seed_enabled_skill_minimal(paths: &Paths, workspace_name: &str, skill_name: &str);
pub fn seed_enabled_skill_full(paths: &Paths, workspace_name: &str, catalog: &str,
                                plugin: &str, skill_name: &str, description: &str);
```

Why: same as M1/M2 — a future schema change to `workspace_skills` (foreseeable as the junction grows) requires 5 in-lockstep updates today.

### M4 — `fn ws(...) -> WorkspaceSettings` + `fn project(...) -> ProjectMarkerConfig` duplicated across 8 settings test files

Files: `tests/settings_composition.rs`, `tests/settings_composition_resolves_to_as_written.rs`, `tests/settings_unknown_workspace_resolver.rs`, `tests/settings_workspace_ref_outside_project.rs`, `tests/settings_harness_not_supported.rs`, `tests/settings_priority.rs`, `tests/settings_bad_exclusion.rs`, `tests/settings_cycle_detection.rs`.

All 8 files duplicate the same two 9-line constructors verbatim:

```rust
fn ws(name: &str, harnesses: Option<Vec<String>>) -> WorkspaceSettings {
    WorkspaceSettings { name: WorkspaceName::parse(name).expect(...), summaries: None,
                        catalogs: Vec::new(), harnesses }
}
fn project(workspace: &str, harnesses: Option<Vec<String>>) -> ProjectMarkerConfig {
    ProjectMarkerConfig { workspace: WorkspaceName::parse(workspace).expect(...), harnesses }
}
```

Eight is well past rule-of-three. Promote both into `tests/common/mod.rs` as `make_workspace_settings(name, harnesses)` and `make_project_marker(workspace, harnesses)`. Why: when `WorkspaceSettings` gains a new field (likely — `summaries` was added in US4 and the trajectory suggests more) every settings test file needs an in-lockstep update.

### M5 — `install_synthetic()` duplicated across 4 settings/composition test files with non-trivial drop-order docs

Files: `tests/settings_composition.rs:31`, `tests/settings_priority.rs:25`, `tests/settings_unknown_workspace_resolver.rs:22`, `tests/settings_workspace_ref_outside_project.rs:31`, `tests/settings_bad_exclusion.rs` (and approximately).

Each file defines:

```rust
fn install_synthetic() -> (HarnessModulesGuard, std::sync::MutexGuard<'static, ()>) {
    // Tuple drop order matters: HarnessModulesGuard MUST drop BEFORE MutexGuard ...
    let lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let guard = HarnessModulesGuard::install(NamedStubHarness::boxed_set([...]));
    (guard, lock)
}
```

The drop-order comment is paragraph-length and reproduced in each file because reversal is a real macOS-stable flake risk. This is **a load-bearing invariant duplicated without a single point of truth**. Two paths to fix:

1. Lift the helper into `tests/common/mod.rs::install_synthetic_harness_modules(names: &[&str]) -> SyntheticHarnessGuard` — a single RAII guard whose Drop is field-ordered correctly. Each file just calls `let _guard = install_synthetic_harness_modules(&["a", "b", "c"])`. Drop-order discipline lives in ONE place where the invariant is enforced.
2. Make `tests/common/mod.rs::HarnessModulesGuard::install` acquire the per-binary `OVERRIDE_MUTEX` internally — but that requires per-binary state which can't be in the shared module. So option 1 wins; the `OVERRIDE_MUTEX` static stays per-binary (correct), but the install-pattern wrapper lives once.

Why this matters: this is the biggest correctness footgun in the Phase 4 test surface. A copy-paste with the tuple reversed produces flakes that read as "production bug" but are actually test infrastructure bugs.

### M6 — JSON wire-shape pins missing for 5 outcome/report types

`tests/harness_json_shape.rs` + `tests/workspace_use_json_shape.rs` + `tests/workspace_rename_json_shape.rs` + `tests/workspace_init_json_shape.rs` + `tests/workspace_remove_json_shape.rs` + `tests/workspace_regen_summary_json_shape.rs` cover 8 of the 13 Phase 4 outcome types. The following lack a `serde_json::to_string` byte-stable pin:

1. **`WorkspaceListEntry`** (`src/commands/workspace/list.rs:29`) — `tests/workspace_list.rs::list_json_wire_shape_is_byte_stable` covers the array-of-entries shape but only for the bootstrap-not-yet single-row case. No multi-row pin, no empty-collection pin.
2. **`harness::list::EffectiveEntry` + `AsWrittenOutcome` enum** (`src/commands/harness/list.rs:35`,`:41`) — no byte-stable serde test exists at all in `tests/harness_list_effective.rs` or `tests/harness_list_as_written.rs`. The `AsWrittenOutcome` enum is an `internally_tagged` shape that's especially worth pinning.
3. **`harness::sync::SyncOutcome` + `SyncChange` + `SyncSubsystem` + `HarnessDecision`** (`src/harness/sync.rs:78–110`) — wire-stable per doc comment ("Serialised verbatim in the CLI's `--json` envelope") but no byte-stable pin in `tests/sync_algorithm.rs` or `tests/harness_sync.rs`.
4. **`WorkspaceCatalogEntry`** (`src/workspace/info.rs:42`) — Phase 3 type extended in Phase 4; no pin in `tests/workspace_info.rs` for the catalog-entry sub-shape.
5. **`DoctorReport`** (`src/doctor/...`) — `tests/doctor_json.rs::doctor_json_shape_is_pinned_on_healthy_install` is **presence-based**, not byte-stable. The 11 top-level fields asserted predate Phase 4. **The newly-added top-level fields `project_binding` / `summariser` / `harness_rules` / `harness_mcp` / `harness_modules` (whatever the actual Phase 4 additions) are not in the presence assertion.** This is the biggest pin gap in the doctor surface and the test passes today even if a Phase 4 field silently disappears from the JSON.

Why: editor integrations + `jq` consumers depend on wire stability. A field-rename or field-reorder regression escapes today because the existing tests are field-name-presence, not byte-stable.

### M7 — Exit-code e2e coverage incomplete for new codes 14, 16, 17, 18 and reused codes 7, 70

`tests/exit_codes_e2e.rs` covers codes 13 (`WorkspaceNotFound`), 15 (`WorkspaceNameInvalid`), 19 (`HarnessClash`), 24 (`SummariserFailure { ModelMissing }`). The contract `contracts/exit-codes-p4.md` enumerates 8 new variants (13–19, 24); the missing 4 are:

- **14 `WorkspaceAlreadyExists`** — library coverage in `tests/exit_codes.rs:359` + `tests/workspace_rename.rs:93` but no CLI binary surface test.
- **16 `WorkspaceHasBoundProjects`** — library coverage in `tests/exit_codes.rs:361` + `tests/workspace_remove.rs:178` but no CLI binary test.
- **17 `CompositionError`** — library coverage in `tests/exit_codes.rs:362` + multiple settings test files for each `CompositionErrorKind` sub-variant, but no CLI binary test. Cycle/UnknownWorkspace/BadExclusion/WorkspaceRefOutsideProject paths are all library-only.
- **18 `HarnessNotSupported`** — library coverage in `tests/settings_harness_not_supported.rs` but no CLI binary test.

The reused-variant table is also unexercised at the CLI binary level:

- **70 `WorkspaceMalformed`** for "project marker config malformed" — no Phase-4-shaped e2e (Phase 3 had one for workspace `config.toml`; the binding-pointer variant is new and untested at the CLI).
- **7 `Io`** for "per-user state dir unwritable" — no Phase 4 e2e.

The Phase 10 / `exit_codes_e2e.rs:39–60` block already documents which codes are deferred to library-level coverage and why (real model loads, etc.). The Phase 4 additions in the file's docblock (lines 23–37) only enumerate the three currently-covered codes. Either add CLI binary tests for 14/16/17/18/70 or extend the documentation block with the explicit "deferred to library-level" reasoning. **Preference: cover them.** None of those four require ONNX model loads — the failure modes are setup-side (e.g. `workspace add foo && workspace add foo` for 14; `settings.toml` with a cycle for 17). The cost is ~5 short CLI tests; the benefit is the CLI binary's `TomeError -> ExitCode` mapping for each variant.

### M8 — `paths_phase2.rs` and `paths_phase3.rs` use a per-file `ENV_LOCK` + `EnvGuard` instead of the centralised `HOME_MUTEX` + `HomeGuard`

`tests/paths_phase2.rs:14–53` and `tests/paths_phase3.rs:20–59` each define their own `ENV_LOCK: Mutex<()>` and `EnvGuard` struct (with `unsafe` `std::env::set_var`/`remove_var`). `tests/common/mod.rs::HOME_MUTEX` + `HomeGuard` provides the same shape with documented drop-order discipline + poison recovery.

Per-binary `Mutex` definitions are technically correct (cargo runs each `tests/*.rs` as a separate process; mutexes don't cross binaries). The problem is **pattern duplication** — three different idioms for the same job:

1. `HomeGuard::install(path)` + `HOME_MUTEX` (used by `harness_bare.rs`, `harness_info.rs`, `harness_sync.rs`, `harness_use_scope.rs`).
2. `EnvGuard::set(&[("HOME", ...)])` + local `ENV_LOCK` (used by `paths_phase2.rs`, `paths_phase3.rs`).
3. `tests/workspace_resolution.rs::ENV_LOCK` with its own `Guard` (4 keys including `TOME_WORKSPACE` and CWD).

Pattern 2 (`paths_phase2/3`) only mutates `HOME`; it could call `HomeGuard::install` instead. Pattern 3 needs additional env keys (`TOME_WORKSPACE`) and CWD — that's structurally different and the local pattern is appropriate.

Fix: collapse pattern 2 → pattern 1 by importing `common::HomeGuard` in `paths_phase2.rs` and `paths_phase3.rs`. This eliminates two `unsafe` blocks and 40 lines of duplicate code.

Why: every additional `unsafe std::env::set_var` site is review surface. One canonical entry point is auditable; three idioms are not.

### M9 — Doctor JSON pin doesn't cover Phase 4 additions

`tests/doctor_json.rs:28–42` enumerates 11 top-level fields and asserts their presence. The Phase 4 additions to `DoctorReport` are not listed: `project_binding`, `summariser`, `harness_rules`, `harness_mcp`. If any of those four field-renames or disappears in a future refactor, this test passes silently.

Cross-reference: `tests/doctor_p4.rs` asserts on `report.project_binding`, `report.summariser.state`, `report.harness_rules`, `report.harness_mcp` at the **library API level** — they exist as struct fields. But the **JSON wire-shape** (`#[serde(skip_serializing_if = "Option::is_none")]` could be silently added, `#[serde(rename = "...")]` could land, the `Subsystem` enum's `rename_all` could change) is not pinned.

Fix: extend `tests/doctor_json.rs::doctor_json_shape_is_pinned_on_healthy_install` to assert presence of the four Phase 4 fields. Better: add a sibling `doctor_json_shape_is_byte_stable_for_minimal_report` that uses a literal-constructed `DoctorReport` + `serde_json::to_string` + exact-string compare (the pattern `tests/workspace_rename_json_shape.rs` already uses).

Why: the doctor envelope is consumed by editor integrations + CI; it's the most-consumed JSON surface in Tome and the only one without a byte-stable pin.

### M10 — Credential-scrubbing coverage doesn't extend to Phase 4 surfaces

`tests/scrubbing.rs` covers Phase 1 (git URLs) + Phase 2 (model download URLs, presigned URLs, reqwest error chains). It does not exercise:

- **Summariser download URLs** — `SUMMARISER_SOURCE_URL` (HuggingFace, no credentials in practice) is not credential-bearing, so no leak risk, but the test discipline is "every download URL surface is exercised". A 2-line test asserting `scrub_to_string(SUMMARISER_SOURCE_URL)` is idempotent + preserves host/path documents the discipline.
- **Harness MCP config paths in error chains** — `harness::mcp_config::write_entry` returns `TomeError` variants that name on-disk paths (e.g. `/home/user/.claude/settings.json`). Per the closed-error-set discipline these aren't credential-bearing, but the contract's "credential scrubbing at the boundary" principle says every error-chain text should pass through `scrub_to_string`. Either pin that boundary holds in tests or document the explicit absence.

Fix: 1–2 short tests in `tests/scrubbing.rs` confirming summariser URL scrub idempotency + asserting harness MCP error chains have no scrub-eligible content. The contract `paths-and-layout-p4.md` should be cited.

Why: T412 from your task list calls out summariser + harness in `tests/scrubbing.rs`; the file doesn't show those additions.

### M11 — Empty-array / empty-string edge cases under-covered in JSON shape pins

The byte-stable pins in `workspace_remove_json_shape.rs`, `workspace_init_json_shape.rs`, `harness_json_shape.rs`, `workspace_rename_json_shape.rs`, `workspace_regen_summary_json_shape.rs` each have ONE "happy path" assertion against fully-populated structs. The good exception is `workspace_remove_json_shape::remove_outcome_empty_collections_render_as_empty_arrays` which pins the empty-vec rendering.

Phase 4 introduces `#[serde(skip_serializing_if = "Vec::is_empty")]` / `#[serde(skip_serializing_if = "Option::is_none")]` as gate-able attributes on several outcome types. Where they're applied, the wire shape changes between `"field": []` and field absent — both are valid JSON but consumers may break on either. Audit:

- `BindOutcome` (`tests/workspace_use_json_shape.rs`) — 4 tests exist but all use the same populated literal shape. No empty-`Vec<HarnessPath>` test exists.
- `RemoveOutcome` — covered (good).
- `InitOutcome` — no `Option`/`Vec` fields to gate; OK.
- `RenameOutcome` — no `Option`/`Vec` fields to gate; OK.
- `HarnessInfoOutcome` — has `Option<...>` fields. The byte-stable pin includes a populated `references` vec; what about `references: vec![]` + `mcp_tome_owned: None`?

Fix: extend each `*_json_shape.rs` with a "minimal/empty" assertion alongside the existing "populated" one. The gating attributes that decide between empty-array and field-absent need pinning per-attribute.

Why: gate attributes are easy to add and easy to change accidentally; consumers that parse with `serde_json::from_value` typically refuse field-absent where the test only proved field-present.

---

## Minors (8)

### m1 — `project_scope` helper duplicated across `tests/doctor_p4.rs:27` and `tests/doctor_fix_p4.rs:37`

Identical 6-line helper. N=2 (rule of three holds at three); promote when a third caller appears.

### m2 — `Fixture::build_*` helpers duplicated across `tests/sync_idempotence.rs`, `tests/workspace_use_claude_code_e2e.rs`, `tests/workspace_use_cross_product.rs`, `tests/workspace_use_forward_progress.rs`

Each defines a local `struct Fixture` that builds a tempdir + `paths` + seeded workspace. Shapes diverge (some carry a `home_path`, some hold `BindDeps`, some seed two harnesses). At N=4 the question is "is there a common shape worth extracting?" — probably not without invasive refactoring; the divergence is real. Acknowledged duplication.

### m3 — `OVERRIDE_MUTEX` static defined per-binary in 18 places

Documented above (M5) — the static itself stays per-binary (correct), but the install pattern doesn't.

### m4 — `mcp_config_*` tests don't byte-stable-pin `TomeEntry` serialised form

`tests/mcp_config_create.rs`, `tests/mcp_config_clash.rs`, `tests/mcp_config_preserve_order.rs`, `tests/mcp_config_remove.rs`, `tests/mcp_config_update.rs` exercise behaviour but no byte-stable assertion against the literal JSON/TOML/YAML output (the `preserve_order` test gets closest). The wire format is internal to each harness's config so JSON-pin discipline is weaker; flag as a minor.

### m5 — `tests/doctor_subsystem_serialize.rs` byte-stable-pins `Subsystem` enum but not the parent `SubsystemHealth` enum

`Subsystem::Summariser → "summariser"` etc. is pinned at `tests/doctor_subsystem_serialize.rs:25` but no companion test pins `SubsystemHealth::{Ok, Degraded, Broken, Missing}` etc. to their wire-shape forms.

### m6 — Several ignored tests document unhide targets but don't link the tracking issue

E.g. `tests/workspace_commands.rs:16` (8 tests `#[ignore = "F11: per-workspace isolation moves to workspace_catalogs/workspace_skills junction tables"]`) — F11 has shipped per CLAUDE.md. Why are they still ignored? If the production code path now exists, either unhide them or update the reason to reflect the new blocker. Audit pass through `grep -rn "^#\[ignore" tests/` against the current F11 status is overdue.

### m7 — `WorkspaceCatalogEntry` byte-stable pin would catch the Phase 3 → Phase 4 rename audit

When `Config.catalogs` was deprecated and `workspace_catalogs` became the source of truth, the `WorkspaceCatalogEntry` wire-shape was carried forward. A byte-stable pin would have surfaced the transitional state earlier. Add at low priority.

### m8 — `tests/sync_boundary.rs` exempts everything under any `mcp` path component

`tests/sync_boundary.rs:39–43` exempts paths whose components contain `mcp`. If a future module is named `mcp_config` (already exists) or `harness/mcp` (exists at `src/harness/mcp_config.rs`), the exemption applies — which is wrong, those should be sync. The current exemption matches `src/harness/mcp_config.rs` and `src/mcp/`. Tighten to `src/mcp/` only (exact component match at depth 1, not any-depth component name match).

Verified: `src/harness/mcp_config.rs` exists and would be exempted by today's rule even though it's sync.

---

## Nits (4)

### n1 — `tests/common/mod.rs::HarnessModulesGuard` docstring says "must serialise via the `OVERRIDE_MUTEX` pattern documented in `tests/harness_sync_stub.rs`" but the pattern is documented identically in 18 files

Pick one canonical reference and inline-link it. `tests/harness_sync_stub.rs:34` is as good as any.

### n2 — `tests/exit_codes_e2e.rs:22–37` documentation block uses three-row-narrow tables and re-numbers the contract codes; one consolidated table covering 8 codes + reused-variant table would read better

The format is fine; just a readability nit.

### n3 — `tests/manifest_strictness.rs` grep guard tolerates `// not-strict` comment-marker opt-out (line 35)

The single use of this marker is `src/settings/mod.rs:103` per CLAUDE.md (the `untagged` `Repr` enum). The marker is silent and one-shot. Consider promoting to `#[allow(deny_unknown_fields_required)]` or similar at the type level rather than a freeform comment marker. (Today's marker is good; just signposting future-proofing.)

### n4 — `tests/workspace_list.rs::seed_enabled_skill_for_test` and `seed_bound_project_for_test` use the `_for_test` suffix while peer files use bare names

Consistency nit only. Promotion (M1/M3) collapses the divergence.

---

## Verdict

**Approved with majors.** The Phase 4 test surface is correct and exercises the production code paths comprehensively at the library level. The phase-wide concerns are:

1. **Fixture duplication.** Six identical `seed_bound_project`, seven identical `open_central`, five identical `seed_enabled_skill`, eight identical `ws()`/`project()`, four identical `install_synthetic`. The fix is mechanical and low-risk — move helpers into `tests/common/mod.rs`. The win is that future schema changes update one place, not 5–8.

2. **Drop-order discipline duplicated 4–8 times.** The `install_synthetic → (HarnessModulesGuard, MutexGuard)` tuple-ordering invariant is reproduced verbatim in 4+ files because reversal is a real flake. Promote to a single guard type.

3. **JSON wire-shape pin coverage.** Eight of thirteen outcome types have byte-stable pins; the doctor envelope's Phase 4 additions (`project_binding`, `summariser`, `harness_rules`, `harness_mcp`) are not pinned. `EffectiveEntry`, `AsWrittenOutcome`, `SyncOutcome`, `WorkspaceListEntry` (beyond bootstrap), `WorkspaceCatalogEntry` lack pins. The doctor field-presence test passes silently if Phase 4 fields disappear.

4. **Exit-code e2e gaps.** Codes 14, 16, 17, 18 lack CLI binary surface coverage. Codes 70 (Phase 4 reuse) and 7 (Phase 4 reuse) lack any Phase-4-shaped e2e. Library coverage holds, but the CLI binary's `TomeError → ExitCode` mapping is untested for those four new + two reused variants.

5. **Pattern duplication for env-mutation.** `HomeGuard` + `HOME_MUTEX` centralisation works for 9 files; `paths_phase2.rs` + `paths_phase3.rs` use their own `EnvGuard` + `ENV_LOCK` with `unsafe std::env::set_var`. Collapse to a single canonical entry point.

Recommendation: PR-A should land the M1–M5 fixture promotions together (mechanical, ~150 LOC delta net-negative), PR-B should land M6 + M9 (JSON wire-shape pin extensions, ~100 LOC), PR-C should land M7 + M8 (exit-code e2e + env-mutation pattern collapse, ~80 LOC). M10 + M11 + minors are optional polish.

The 14 `#[ignore]` markers all document unhide targets explicitly — that discipline is in place and visible. The `HARNESS_MODULES_OVERRIDE` / `OVERRIDE_MUTEX` / `HOME_MUTEX` discipline is intact: zero files install the override without holding a serialisation mutex, zero files mutate `$HOME` without `HomeGuard` (except `paths_phase{2,3}.rs` which use their own equivalent). `SUMMARISER_OVERRIDE` is `thread_local!` and the `SummariserOverrideGuard` lives in production source.

File path of this report: `/tmp/tome-phase4-polish-test.md`
