# Phase 4 Polish — Consolidated Findings

Cross-slice reviewer pass run 2026-05-26 on branch `004-phase-4-polish-pr-a`
(post-US5 close, before any Polish PR). All five user stories shipped and
merged to main (916 tests across 125 suites). The Phase 4 Polish phase opens
per `tasks.md` Phase 8 (T403-T431).

Four reviewers ran in parallel, phase-wide (not per-US):
- **Contract**: `/tmp/tome-phase4-polish-contract.md` — 3 blockers, 12 majors, 10 minors, 7 nits.
- **Rust-lens**: `/tmp/tome-phase4-polish-rust.md` — 0 blockers, 12 majors, 8 minors, 4 nits.
- **Security**: `/tmp/tome-phase4-polish-security.md` — 0 blockers, 8 majors, 6 minors, 3 nits.
- **Test audit**: `/tmp/tome-phase4-polish-test.md` — 0 blockers, 11 majors, 8 minors, 4 nits.

Aggregate: **3 blockers, 43 majors, 32 minors, 18 nits**.

See `disposition.md` for the apply / defer routing across the Polish PRs.

## Blockers (3) — Contract drift, must fix before v0.4.0

### C-B1 — Exit code 20 → 24 drift in three contract documents

`exit-codes-p4.md` and `src/error.rs` correctly emit code 24 for
`SummariserFailure` (the closed-set discipline forbids collision with
Phase 2's `PluginNotFound` (20)). Three other contract documents still
claim code 20:

- `specs/004-phase-4-refactor-harnesses/spec.md:224` (FR-385)
- `specs/004-phase-4-refactor-harnesses/spec.md:331` (FR-601)
- `specs/004-phase-4-refactor-harnesses/contracts/catalog-and-plugin-extensions-p4.md:118, 129`
- `specs/004-phase-4-refactor-harnesses/contracts/workspace-commands.md:177`

**Action**: contract doc fix only; implementation is correct.

### C-B2 — `tome workspace init --json` envelope drift

`workspace-commands.md:41-43` pins the JSON as
`{"name": "<name>", "path": "...", "catalogs_inherited": 3, "id": 7}`.
Implementation emits
`{"name": "<name>", "workspace_dir": "...", "inherited_catalogs": 3}` —
three field renames and one missing field (`id`).

Either the contract or the impl must move. Field-byte-stable JSON is the
hardest-to-change wire contract; the renames look like impl drift during
US2.a-1 that wasn't caught at the pre-merge review.

**Action**: align impl to contract (rename + add `id`). Update
`tests/workspace_init_json_shape.rs` pin.

### C-B3 — Doctor `harnesses[].name` uses underscored names; rest of doctor uses hyphenated

`src/doctor/harness_detect.rs:22-29`'s `KNOWN_HARNESSES` const emits
`"claude_code"` in `HarnessPresence.name`; every other surface
(`HarnessModule::name()`, `SUPPORTED_HARNESSES` registry, settings
composition, `harness_rules[].harness`, `harness_mcp[].harness`,
`effective_harness_list[].name`, `detected_uninstalled_harnesses[]`) uses
`"claude-code"`. Two name styles for the same harness on the same JSON
document — a doctor `--json` consumer correlating field rows cannot
match them.

**Action**: change `KNOWN_HARNESSES` to hyphenated names (canonical form
across the rest of the surface).

## Majors — 43 findings consolidated below

(Each row links the originating reviewer + finding ID for follow-up. See
disposition.md for apply/defer routing.)

### Contract majors (12)

| # | Summary | Origin |
|---|---|---|
| C-M1 | `claude-code` `detected_path` reports `~/.claude-code/` instead of `~/.claude/` (multi-word harness bug) | contract M1 |
| C-M2 | `Paths::resolve` doesn't canonicalise; data-model invariant violated for symlinked `$HOME` | contract M2 |
| C-M3 | `SyncOutcome` field names diverge from sync-algorithm.md step 11 | contract M3 |
| C-M4 | data-model TOML example missing `mcp_servers.` prefix (data-model drift, not impl) | contract M4 |
| C-M5 | `WorkspaceNameInvalid` message awkward for `global` reserved name | contract M5 |
| C-M6 | summariser SHORT_PROMPT literal says "2400 chars" but length-window const is 2500 | contract M6 |
| C-M7 | `RulesCopyState::SourceMissing` variant ships in impl, missing from data-model §15 | contract M7 |
| C-M8 | workspace-commands.md still says exit 20 (downstream of C-B1) | contract M8 |
| C-M9 | `harness_rules` / `harness_mcp` named-struct shape vs data-model tuple | contract M9 |
| C-M10 | `tome harness use` holds lock across phase B (contrary to sync-algorithm phase A/B split); `workspace use` correct | contract M10 |
| C-M11 | `workspace use --force` bypasses dangerous-CWD pre-check; not in contract | contract M11 |
| C-M12 | doctor binding-broken SuggestedFix `command` field contains two commands in one string | contract M12 |

### Rust-lens majors (12)

| # | Summary | Origin |
|---|---|---|
| R-M1 | Three near-identical `atomic_write` helpers — promote to `util::atomic_file` | rust M1 |
| R-M2 | Three override registries with different concurrency primitives | rust M2 |
| R-M3 | `HARNESS_MODULES_OVERRIDE` `expect("poisoned")` — apply `PoisonError::into_inner` | rust M3 |
| R-M4 | Duplicate `ProjectMarkerConfig` types in `settings::` and `workspace::resolution::` | rust M4 |
| R-M5 | Three `read_project_marker` functions; promote to `settings::parser::read_project_marker` | rust M5 |
| R-M6 | Duplicate `relative_path` impls in `doctor::harness_integration` + `harness::sync` | rust M6 |
| R-M7 | SQL `SELECT id FROM workspaces WHERE name = ?1` repeated 9 times | rust M7 |
| R-M8 | `harness::lookup` is `pub` but unused; doc misleading | rust M8 |
| R-M9 | `CompositionErrorKind::BadExclusion` reused for non-exclusion errors | rust M9 |
| R-M10 | Defensive-unreachable arm in `doctor::fixes::apply_one` for `HarnessRules/Mcp` | rust M10 |
| R-M11 | `sync_one` (broadcast) vs `sync_one_project` (single) — rename to make blast radius visible | rust M11 |
| R-M12 | Rename no-op routes through `Usage` (exit 2) instead of workspace-error variant | rust M12 |

### Security majors (8)

| # | Summary | Origin |
|---|---|---|
| S-M1 | 26+ `unbounded read_to_string` sites — promote `util::bounded_read_to_string` | security M1 |
| S-M2 | `atomic_dir::land_directory*` missing symlink refusal | security M2 |
| S-M3 | Summariser prompt injection via third-party skill descriptions broadcasting to MCP | security M3 |
| S-M4 | `doctor --fix --force` rewrites MCP entries with no enumeration / backup | security M4 |
| S-M5 | `repair_summariser` runs ~400 MB download outside `index.lock` | security M5 |
| S-M6 | `orphan_cleanup::sweep_one` follows symlinks via `entry.metadata()` | security M6 |
| S-M7 | `home_root()` doesn't validate absolute / exists / canonical | security M7 |
| S-M8 | `llama-cpp-2 = "=0.1.146"` exact-pin lacks llama.cpp upstream commit traceability | security M8 |

### Test audit majors (11)

| # | Summary | Origin |
|---|---|---|
| T-M1 | `seed_bound_project` duplicated × 6 — promote to `tests/common/mod.rs` | test M1 |
| T-M2 | `open_central` duplicated × 7 — promote (two variants: registry- and stub-seeded) | test M2 |
| T-M3 | `seed_enabled_skill` duplicated × 5 — promote (minimal + full forms) | test M3 |
| T-M4 | `ws()` + `project()` duplicated × 8 settings test files | test M4 |
| T-M5 | `install_synthetic()` tuple-drop-order discipline duplicated × 4 — promote single RAII guard | test M5 |
| T-M6 | JSON wire-shape pins missing for `EffectiveEntry`, `AsWrittenOutcome`, `SyncOutcome`, `WorkspaceCatalogEntry`, `DoctorReport` Phase-4 additions | test M6 |
| T-M7 | Exit-code e2e coverage gaps: codes 14, 16, 17, 18 + reused 7, 70 | test M7 |
| T-M8 | `paths_phase2.rs` + `paths_phase3.rs` use local `EnvGuard` instead of `HomeGuard` | test M8 |
| T-M9 | `doctor_json.rs` field-presence test misses Phase 4 additions | test M9 |
| T-M10 | Scrubbing tests don't cover summariser URLs + harness MCP error chains | test M10 |
| T-M11 | JSON shape pins lack empty-collection edge cases for gated attributes | test M11 |

## Minors (32) + Nits (18) — Bulk-deferred

Tracked in individual reviewer files. Disposition is "follow-up tracking
issue post-v0.4.0" unless a specific minor surfaces in a Polish PR's
neighbourhood (in which case fold into the same PR for cohesion).

See `/tmp/tome-phase4-polish-{contract,rust,security,test}.md` for the full
prose of each finding.
