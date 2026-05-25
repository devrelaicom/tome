# US3 — Pre-Closeout Review Findings

Four reviewers dispatched in parallel against the merged US3 surface (PRs #88–#91).

Per-reviewer source files in `/tmp/tome-review-us3-{contract,rust,test,security}.md`.

Counts: **6 blockers, 29 majors, ~40 minors+nits.**

Triage in `us3-disposition.md`.

---

## Blockers (6)

| # | Source | One-line | File / cite |
|---|---|---|---|
| **C-B1** | contract | Production `harness::sync::sync_project` uses `StubScope::new()` — every `[workspaces.<name>]` composition ref in production returns `UnknownWorkspace` → exit 13 even when the workspace exists. Asymmetric with `tome harness list` which correctly uses `PathsScopeProvider`. | `src/harness/sync.rs:173` |
| **C-B2** | contract | `PathsScopeProvider` consults the on-disk settings.toml instead of the central `workspaces` table. A real workspace with no settings.toml (legal — all fields default) is reported as `UnknownWorkspace`. | `src/commands/harness/mod.rs:101-124` |
| **C-B3** | contract | `tome harness info` drops the "via composition" half of the contract requirement. Users can't tell that disabling `[global]` would remove the harness from the effective list. | `src/commands/harness/info.rs:139-167` |
| **T-B1** | test | Process-global `std::env::set_var("HOME", ...)` race across 12+ call sites in `harness_bare.rs`, `harness_info.rs`, `harness_use_scope.rs` with no mutex serialising parallel tests. | multi-site |
| **T-B2** | test | `harness_bare.rs`, `harness_list_effective.rs`, 2/3 of `harness_list_as_written.rs`, 2/3 of `harness_info.rs` assert only `result.is_ok()` — `commands::harness::*::run` writes directly to stdout, so the output content is never verified. A broken impl that returned `Ok(())` without producing output would pass. | multi-site |
| **T-B3** | test | `harness_skeleton.rs::effective_modules_falls_back_to_static_when_no_override` silently no-ops if the override slot is dirty under parallel scheduling. No cross-test mutex on `HARNESS_MODULES_OVERRIDE` in this file. | `tests/harness_skeleton.rs:283` |

## Majors (29)

### Contract audit (8)

| # | One-line | File ref |
|---|---|---|
| C-M1 | `EffectiveHarness.source_chain: Vec<ScopeKind>` cannot emit the contract's mixed-notation chain (`["project", "[workspaces.shared]"]`). | `src/settings/resolver.rs:54-58` |
| C-M2 | `tome harness list <workspace>` silently returns empty for missing workspace's settings.toml — no DB membership check, no exit 13. | `src/commands/harness/list.rs:63-85` |
| C-M3 | `tome harness` bare uses `starts_with($HOME)` heuristic to filter junk paths — misclassifies any project root under `~`. | `src/commands/harness/bare.rs:80-100` |
| C-M4 | Unsupported-harness validation runs end-of-resolution; `["bad", "!bad"]` silently passes (contract says per-entry). | `src/settings/resolver.rs:192-201` vs `contracts/settings-composition.md:36-39` |
| C-M5 | `tome harness use|remove` lacks advisory lockfile around settings.toml read-modify-write — concurrent edits lose each other. | `src/commands/harness/{use_,remove}.rs` |
| C-M6 | `tome harness sync` doesn't fully honour idempotence when prior file has multiple Tome blocks (rare; hand-edited). | `src/harness/sync.rs:441-457` |
| C-M7 | `tome harness use` always emits same informational notice; doesn't distinguish "no project resolved" from "effective list unchanged". | `src/commands/harness/use_.rs:140-172` |
| C-M8 | Cycle path renders without `workspace `/`global ` prefix (contract: `composition cycle: workspace \`a\` → workspace \`b\` → workspace \`a\``). | `src/error.rs:432-434` |

### Rust-lens (8)

| # | One-line | File ref |
|---|---|---|
| R-M1 | `PathsScopeProvider` silently rewrites permission errors AND TOML parse failures to `UnknownWorkspace` → exit 13. | `src/commands/harness/mod.rs:106-122` |
| R-M2 | No concurrency control on settings-file edit read-modify-write. | `src/commands/harness/{use_,remove}.rs:63-67,52-56` |
| R-M3 | `WorkspaceRefOutsideProject` reports `found_in: Workspace` when `[workspace]` is actually in the project marker. | `src/settings/resolver.rs:347-354` |
| R-M4 | Post-edit re-snapshot's error path masks settings drift. | `src/commands/harness/use_.rs` |
| R-M5 | `home_root()` redundantly called per row in `bare.rs`. | `src/commands/harness/bare.rs` |
| R-M6 | `direct_scopes: Vec<String>` should be a typed enum. | `src/commands/harness/info.rs` |
| R-M7 | Dead `_for_use` aliases. | multi-site |
| R-M8 | `CompositionRef::parse` falls through to `Include` for malformed bracketed refs like `"[workspaces.a"`. | `src/settings/composition.rs:79-98` |

### Test audit (9)

| # | One-line | File ref |
|---|---|---|
| T-M1 | FR-447 (`[workspaces.<name>]` to non-existent workspace → exit 13) — only `From` boundary mapping tested, never the resolver path producing `UnknownWorkspace`. | "no test" |
| T-M2 | Zero JSON wire-shape pins for `HarnessBareEntry`, `HarnessInfoOutcome`, `HarnessUseOutcome`, `HarnessRemoveOutcome`. | `tests/harness_*` |
| T-M3 | `tome harness use --force` never tested with `force: true` (all 6 tests pass false). | `tests/harness_use_scope.rs` |
| T-M4 | Informational-notice path (FR-522/FR-523 last clause) untested. | same |
| T-M5 | `tome harness sync` CLI thin-wrapper untested (no exit-2 negative path for missing project marker). | "no test" |
| T-M6 | `remove_last_entry_leaves_empty_array` doesn't actually verify `harnesses = []` — only that substring `harnesses` survives. | `tests/harness_remove_scope.rs` |
| T-M7 | Cycle-through-`[global]` untested. | `tests/settings_cycle_detection.rs` |
| T-M8 | FR-460 cancellation invariant (`["fake", "!fake"]` resolves cleanly) untested — pairs with C-M4. | `tests/settings_harness_not_supported.rs` |
| T-M9 | `use`'s idempotence test only checks settings-file mtime; not rules-file or MCP-config target mtimes. | `tests/harness_use_scope.rs` |

### Security audit (4)

| # | One-line | File ref |
|---|---|---|
| S-M1 | Same root as C-B1 (production sync StubScope). Composition refs silently fail. | `src/harness/sync.rs:173` |
| S-M2 | Same root as C-M5 / R-M2 (read-modify-write race on settings.toml). | `src/commands/harness/{use_,remove}.rs` |
| S-M3 | Zero US3 coverage in `tests/security_hardening.rs` for `settings::edit::save_settings` (mode preservation + symlink refusal aren't pinned for the new write path). | `tests/security_hardening.rs` |
| S-M4 | Same as R-M1 (PathsScopeProvider masks EACCES/EIO as UnknownWorkspace). | same |

---

## Verdict

US3 surface is comprehensive (109 new tests across 14 files) but ships with a production-vs-test divergence: the resolver works correctly in tests (US3.a's 19 tests pass), but the production sync caller (US1's `tome workspace use`-triggered sync, US2's `tome workspace sync`-triggered, US3's `tome harness use|remove`-triggered, US3's `tome harness sync`) uses `StubScope::new()` instead of the central-DB-backed provider. The fix is bounded (one provider implementation + a few call sites + the harness::sync seam). Plan: US3.d-1 ships all 6 blockers + ~12 selected majors; US3.d-2 closeout (sdd:map + retro + CLAUDE.md).
