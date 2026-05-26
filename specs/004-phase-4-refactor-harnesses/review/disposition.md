# Phase 4 Polish — Disposition

Routing for the 3 blockers + 43 majors + 50 minors/nits across the Polish
PR sequence (PR-A through PR-H per tasks.md Phase 8).

## PR-B — Blocker fixups (T406-T407)

| # | Disposition |
|---|---|
| C-B1 | Apply: sweep `spec.md:224,331`, `contracts/catalog-and-plugin-extensions-p4.md:118,129`, `contracts/workspace-commands.md:177` — change "exit 20" → "exit 24" in all four sites. Contract doc edit only. |
| C-B2 | Apply: rename `InitOutcome` fields in `src/workspace/init.rs` to match contract: `workspace_dir` → `path`, `inherited_catalogs` → `catalogs_inherited`. Add `id: i64` field populated from the central DB `workspaces.id` post-insert. Update `tests/workspace_init_json_shape.rs` pin. |
| C-B3 | Apply: change `KNOWN_HARNESSES` in `src/doctor/harness_detect.rs:22-29` from underscored to hyphenated form to match every other harness surface. Update probe path keys to retain `~/.claude/` mapping. Update affected doctor tests. |

## PR-C — Major fixups (T408-T409)

Selected for blast-radius and consolidation leverage. Cherry-picked from 43:

| # | Disposition |
|---|---|
| C-M1 | Apply: expose `HarnessModule::detect_path(&home) -> PathBuf` trait method (or `probe_dir`); `tome harness info` calls it for `detected_path` instead of `home.join(format!(".{}", m.name()))`. |
| C-M3 | Apply: align `SyncOutcome` field shape to `sync-algorithm.md` step 11, OR update the contract to match impl's `{added, updated, removed, leave_alones, decisions}` shape. Choice: align impl (smaller blast on impl-side; contract example was explicit). |
| C-M9 | Apply: align data-model §15 to impl's `HarnessSubsystemReport { harness, health }` named-struct shape (impl is the right shape for ergonomic JSON consumption; tuple shape was an earlier sketch). |
| C-M12 | Apply: split binding-broken SuggestedFix into two suggestion entries (rebind, recreate) instead of one compound `command` field. |
| R-M3 | Apply: `HARNESS_MODULES_OVERRIDE.read().unwrap_or_else(PoisonError::into_inner)` — mirror the discipline from `src/summarise/mod.rs:213`. |
| R-M4 | Apply: drop `workspace::resolution::ProjectMarkerConfig`; route all callers through `settings::ProjectMarkerConfig`. |
| R-M5 | Apply: promote `settings::parser::read_project_marker(path)` → `Result<ProjectMarkerConfig, TomeError>`; collapse three duplicate readers + two inline parses. |
| R-M7 | Apply: add `index::workspaces::resolve_id(conn, name)` and consolidate the nine `SELECT id FROM workspaces WHERE name = ?1` sites. |
| R-M8 | Apply: demote `harness::lookup` to `pub(crate) fn` (tests only). Fix `commands/harness/use_.rs:7` doc comment to reference `with_effective_modules`. |
| R-M12 | Apply: route `rename old == new` through `WorkspaceNameInvalid` with `reason: "rename old and new names are identical"`. |

## PR-D — Deferred coverage (T410-T415)

Test gaps from per-US disposition + Phase 4 Polish reviewer findings:

| # | Disposition |
|---|---|
| T-M6 | Apply: add JSON wire-shape pins for `EffectiveEntry` + `AsWrittenOutcome` (`tests/harness_list_json_shape.rs`); `SyncOutcome` (`tests/sync_outcome_json_shape.rs`); `WorkspaceCatalogEntry` (extend `tests/workspace_info.rs`). Doctor pin handled by T-M9. |
| T-M7 | Apply: extend `tests/exit_codes_e2e.rs` with CLI binary tests for codes 14, 16, 17, 18 (all four are setup-side failures — no model load required). Add reused-variant rows for 70 + 7 with Phase 4 shapes. |
| T-M9 | Apply: extend `tests/doctor_json.rs` to assert presence + add byte-stable pin for `project_binding`, `summariser`, `harness_rules`, `harness_mcp`. |
| T-M10 | Apply: add 2 short scrubbing tests for summariser URL + harness MCP error chains. |
| T410 | Apply: T410 from tasks.md — `tests/exit_codes_e2e.rs` rows for new Phase 4 codes; subsumed by T-M7. |
| T411 | Apply: T411 — coverage matrix doc update. |
| T412 | Apply: T412 — credential scrubbing for summariser URLs + harness MCP paths; subsumed by T-M10. |
| T413 | Apply: T413 — `tests/manifest_strictness.rs` extension for the four Phase 4 strict types (`WorkspaceSettings`, `ProjectMarkerConfig`, `GlobalSettings`, summariser `ModelManifest`). Per F11c-2 these already grep-clean — confirm and extend the table-driven assert list. |

## PR-E — Security hardening (T416-T419)

| # | Disposition |
|---|---|
| S-M1 | Apply: introduce `util::bounded_read_to_string(path, max) -> Result<String, TomeError>`; convert ~26 production read sites with per-class caps (1 MiB Tome-owned, 256 KiB plugin manifests, 1 MiB harness MCP, 4 MiB harness rules). New error kind via `TomeError::Io(InvalidInput, ...)` reuses code 7; closed-set unchanged. |
| S-M2 | Apply: add symlink refusal to `atomic_dir::land_directory` + `land_directory_with_replace` + `.old` aside cleanup. |
| S-M6 | Apply: `orphan_cleanup::sweep_one` adds `symlink_metadata` check before `is_dir()`; refuse symlinks per `mcp/tools/get_skill::walk_dir` precedent. |
| S-M7 | Apply: `home_root()` (both copies) validates absolute, canonicalises, surfaces `TomeError::Usage` on relative `$HOME`. |
| T-M8 | Apply: collapse `paths_phase2.rs` + `paths_phase3.rs` per-file `EnvGuard`/`ENV_LOCK` to `tests/common::HomeGuard` + `HOME_MUTEX`. Two `unsafe` blocks removed. |
| T416 | Apply: T416 — 0o600 mode audit across Tome-owned writes (already in mode-preservation discipline; assert via a grep test). |
| T417 | Apply: T417 — symlink refusal completeness; subsumed by S-M2 + S-M6. |
| T418 | Apply: T418 — credential scrubber extensions; subsumed by T-M10 in PR-D. |
| T419 | Apply: T419 — binary-size record post-Polish. |

## PR-F — Docs + release (T420-T424)

| # | Disposition |
|---|---|
| T420 | Apply: README Phase 4 section — new commands (`tome workspace`, `tome harness`), new exit codes, new dependencies (`llama-cpp-2`, `toml_edit`, `encoding_rs`). |
| T421 | Apply: CHANGELOG `[0.4.0]` entry. |
| T422 | Apply: `--help` text audit (consistency, brevity, every flag documented). |
| T423 | Apply: `Cargo.toml` 0.3.0 → 0.4.0. |
| T424 | Apply: constitution v1.3.0 §Paths amendment final ratification (drop staged-amendment language). |
| C-B1 | Sweep complete by this PR (downstream of PR-B). |

## PR-G — Final docs + retro (T425-T427)

| # | Disposition |
|---|---|
| T425 | Apply: final `/sdd:map incremental` (4 mappers parallel against Polish-complete tree). |
| T426 | Apply: fill `retro/P8.md` with Polish learnings (consolidation patterns, brief-cap discipline, mid-phase mapper rotation recommendation). |
| T427 | Apply: this CLAUDE.md update with v0.4.0 close. |

## PR-H — Ready-for-merge gate (T428-T431)

| # | Disposition |
|---|---|
| T428 | Apply: push branch + open PR. |
| T429 | Apply: green CI gate. |
| T430 | Apply: ready-status report. |
| T431 | Apply: self-merge on green per autonomous-mode handover. |

## Deferred to follow-up issue (post-v0.4.0)

Items below get tracked but do NOT block v0.4.0:

| # | Reason for deferral |
|---|---|
| C-M2 | `Paths::resolve` canonicalisation — affects symlinked `$HOME`; non-functional regression today; defer to v0.5.0 path normalisation pass. |
| C-M4 | data-model TOML example missing `mcp_servers.` prefix — doc-only drift; impl is correct. |
| C-M5 | `WorkspaceNameInvalid` for `global` — message ergonomics, not correctness. |
| C-M6 | summariser SHORT_PROMPT 2400 vs 2500 — inside the prompt body; the LLM tolerates the inconsistency. |
| C-M7 | `RulesCopyState::SourceMissing` missing from data-model §15 — sync the data-model in a separate doc-pass. |
| C-M10 | `harness use` lock semantics — refactor to phase A/B split would touch a stable command surface; defer. |
| C-M11 | `workspace use --force` bypassing dangerous-CWD — documented in CLI flag help; aligning the contract is a doc-pass. |
| R-M1 | Three atomic_write helpers — defer to v0.5.0 (interface design + extensive test churn). |
| R-M2 | Three override-registry primitives — defer (same scope as R-M1). |
| R-M6 | Duplicate `relative_path` impls — defer (the slight behavioural divergence is benign). |
| R-M9 | `CompositionErrorKind::BadExclusion` reuse — defer (semantic clean-up). |
| R-M10 | Defensive-unreachable arm in `apply_one` — defer (compiler can't help; the arm is reachable in theory). |
| R-M11 | `sync_one` rename — defer (breaking-name change). |
| S-M3 | Summariser prompt-injection — needs design pass (truncation + docs + warn); defer to v0.5.0 trust-boundary work. |
| S-M4 | `doctor --fix --force` enumeration / dry-run — UX work; defer. |
| S-M5 | `repair_summariser` lock acquisition — race window is benign (atomic-rename); defer. |
| S-M8 | `llama-cpp-2` upstream-commit traceability — defer to v0.5.0 supply-chain pass. |
| T-M1, T-M2, T-M3, T-M4, T-M5 | Fixture promotion — defer to a test-hardening pass (mechanical but extensive churn). |
| T-M11 | JSON shape pin empty-collection edge cases — defer. |
| All minors + nits | Tracking issue post-v0.4.0. |

## Net effect of Polish phase

- PR-B: ~3 commits (contract docs + one impl rename + one doctor const sweep)
- PR-C: ~10 production-source touches + test updates
- PR-D: ~6 new test files + 2 file extensions
- PR-E: ~1 new util module (`bounded_read_to_string`) + 26 callsite touches + 3 symlink-refusal additions
- PR-F: docs only
- PR-G: closeout
- PR-H: ready-gate

Estimated total Polish surface: ~60 file touches, ~80 test additions,
~150 LOC removed (consolidation). 8 PRs in line with Phase 3 polish
pattern (PR-A through PR-H).
