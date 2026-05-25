# US3 — Disposition

Maps findings in `us3-findings.md` to actions.

## Blockers — all applied in US3.d-1

| # | Disposition |
|---|---|
| C-B1 + S-M1 | Apply: replace `StubScope::new()` in `src/harness/sync.rs::sync_project` with a `CentralDbScopeProvider`. Lives in `src/settings/resolver.rs` (or `src/commands/harness/mod.rs` — wherever PathsScopeProvider is). Opens central DB read-only; checks `workspaces.name` for membership; reads `<root>/workspaces/<name>/settings.toml` for the directly-declared harnesses; returns `Ok(None)` if file is absent (legal — workspace exists but has no harnesses declared); returns `Err(UnknownWorkspace)` only when the workspace name isn't in the central DB. |
| C-B2 + S-M4 + R-M1 | Apply (same fix as C-B1): `PathsScopeProvider` is replaced by `CentralDbScopeProvider` that does the central-DB membership check. The new provider distinguishes "workspace exists but no settings.toml" (returns `Ok(None)`) from "workspace doesn't exist" (returns `Err(UnknownWorkspace)`) from "IO/parse error" (returns `Err(TomeError::Io)` instead of masking as UnknownWorkspace). All US3 callers swap. |
| C-B3 | Apply: in `src/commands/harness/info.rs`, compute the effective list via `resolve_effective_list`, find the named harness in the result, report its `source_chain` (after C-M1 fix below makes it useful). Restructure the wire shape from `direct_scopes: Vec<String>` to `references: Vec<HarnessReference>` where `HarnessReference { scope: ScopeKind, via: Option<String> }`. |
| T-B1 | Apply: introduce `tests/common/mod.rs::HOME_MUTEX` (process-global `Mutex<()>`) and a `HomeGuard` RAII helper that sets `HOME` to a tempdir + restores on drop. Audit `harness_bare.rs`, `harness_info.rs`, `harness_use_scope.rs` and any other test using `std::env::set_var("HOME", ...)` to acquire the mutex before mutation. Use the established tuple-drop discipline `(HomeGuard, MutexGuard)` so HOME restores BEFORE the mutex releases. |
| T-B2 | Apply: refactor `commands::harness::*::run` to take an `out: &mut dyn Write` sink instead of writing directly to stdout. Library-API tests pass a `Vec<u8>` buffer and assert on its contents. CLI dispatcher wraps `std::io::stdout().lock()`. Pattern: the existing `output::write_json` already takes a sink; extend the human-form emit functions similarly. Update affected tests to capture + assert on output content. |
| T-B3 | Apply: add per-test-file `OVERRIDE_MUTEX` to `tests/harness_skeleton.rs` matching the discipline from US3.a settings tests. |

## Majors — applied in US3.d-1 (12)

| # | Disposition |
|---|---|
| C-M1 | Apply: extend `EffectiveHarness.source_chain` to carry both `ScopeKind` AND the reference string (`[workspaces.shared]`). Use `enum ChainStep { Scope(ScopeKind), Reference(String) }`. The resolver populates the chain during DFS. Wire shape becomes `Vec<String>` (each step rendered as `"project"` or `"[workspaces.shared]"`). |
| C-M2 | Apply: `harness list <workspace>` now validates membership via the central DB; exits 13 if workspace doesn't exist. |
| C-M4 + T-M8 | Apply: validate unsupported names per-entry inside `resolve_list` (after parsing each `CompositionRef::Include(name)` against the registry). Don't wait for end-of-resolution. Add T-M8 test: `["fake", "!fake"]` should immediately error on "fake" inclusion. |
| C-M5 + R-M2 + S-M2 | Apply: acquire `index.lock` advisory lockfile around `harness use|remove`'s read-modify-write window. Use `crate::index::acquire_lock`. Document the lock-hold span in the module doc. |
| C-M6 | Apply: `harness::sync::write_rules_for_path` short-circuits multi-block-collapse case correctly when the canonical first block matches the new body, regardless of subsequent legacy blocks. |
| R-M3 | Apply: in `WorkspaceRefOutsideProject` emission, use the actual scope where the `[workspace]` reference was found (`ScopeKind::Project` vs `Workspace` vs `Global`). |
| R-M8 | Apply: `CompositionRef::parse` rejects malformed bracketed refs (e.g. `"[workspaces.a"`) with `CompositionErrorKind::BadExclusion` or a new `MalformedReference` variant instead of falling through to `Include`. |
| T-M1 | Apply: add test in `tests/settings_workspace_ref_outside_project.rs` (or new file): `[workspaces.unknown]` reference resolves to exit 13 via the `From<CompositionErrorKind>` boundary. |
| T-M2 | Apply: add JSON wire-shape pins for `HarnessBareEntry`, `HarnessInfoOutcome`, `HarnessUseOutcome`, `HarnessRemoveOutcome` (pattern from US1.d-2a `workspace_use_json_shape.rs`). |
| T-M3 | Apply: add `tome harness use --force` test that exercises the FR-502 wiring path. |
| T-M5 | Apply: `tome harness sync` test against missing project marker → exit 2. |
| T-M6 | Apply: tighten `remove_last_entry_leaves_empty_array` to parse the resulting TOML and assert `harnesses = []`. |
| S-M3 | Apply: extend `tests/security_hardening.rs` with mode preservation + symlink refusal tests for `settings::edit::save_settings`. |

## Majors — deferred to follow-up issue

| # | Reason for deferral |
|---|---|
| C-M3 | bare's MCP-config heuristic — cosmetic edge case; defer. |
| C-M7 | harness use informational notice differentiation — cosmetic wording; defer. |
| C-M8 | cycle path scope-kind prefix — depends on C-M1 fix which already restructures path nodes; revisit naturally in C-M1's wake. |
| C-m2 | `harness info`'s `detected_path` field is wrong for multi-word names — needs trait extension for per-user dir; defer to a small follow-up. |
| R-M4 | post-edit re-snapshot error masks settings drift — defensive concern. |
| R-M5 | redundant `home_root()` calls in bare — perf cosmetic. |
| R-M6 | typed enum for direct_scopes — superseded by C-M1 + C-B3 fixes. |
| R-M7 | dead `_for_use` aliases — cleanup. |
| T-M4 | informational-notice path untested — pairs with C-M7 deferral. |
| T-M7 | cycle-through-`[global]` untested — small gap; can be added to existing cycle test file. |
| T-M9 | use's idempotence extends to rules/MCP mtimes — extends the sync_idempotence.rs pattern; defer. |

## Minors + nits

Bulk-deferred to a tracking issue per individual reviewer files.

## Net effect of US3.d-1

- ~10 production-source touches (`harness/sync.rs`, `commands/harness/{mod,info,use_,remove}.rs`, `settings/{resolver,composition}.rs`, `error.rs`)
- ~10 new tests + ~6 modified tests (T-B2 refactor for output capture; T-M1/M2/M3/M5/M6/M8 additions)
- 1 test helper addition (`HOME_MUTEX` + `HomeGuard`)
- 2 review artefacts (`us3-findings.md`, `us3-disposition.md`)

US3.d-2 runs `/sdd:map incremental` + retro/CLAUDE.md updates.
