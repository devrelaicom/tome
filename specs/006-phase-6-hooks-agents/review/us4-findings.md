# US4 (Agent personas via MCP prompts) — Reviewer findings

4-reviewer parallel pass against the US4 surface (`9fac54f..HEAD`). Recorded before any fix. No BLOCKER; security PASS.

## Contract
- **[MAJOR] C4-1** `${TOME_PLUGIN_VERSION}` renders EMPTY in persona bodies. `enabled_agents_for_workspace` does not SELECT `plugin_version` (the command/skill registry query does), so the persona `PromptEntry` is built with `plugin_version: String::new()` and the documented built-in silently resolves to empty (FR-062 step 3 fidelity gap). Fix: add `plugin_version` to the `EnabledAgent` projection + thread onto the persona `PromptEntry`.
- **[MINOR] C4-2** Persona name derivation routes through `derive_name(base, "persona", Some("{base}-persona"))` → `sanitise_trunc(override, 48)`. A long clash-prefixed `<plugin>-<name>` can truncate so the `-persona` suffix is amputated (e.g. `…-perso`). Still routes correctly (keyed on final name) but breaks the user-facing `<name>-persona` shape. Fix: protect the suffix (truncate the base portion, leave room for `-persona`).
- **[MINOR] C4-3** Contract § Tests names `tests/persona_prompts*.rs`; tests landed as `tests/personas*.rs`; `toggle_from_startup_scope` has no equivalent (see T4-1). Reconcile the contract table.

## Rust-lens
- **[MAJOR] R4-1** Every persona-agent `EntryIdentity` is pushed with `indexed_at: String::new()` (same as the drop-persona seed). `resolve_collisions` tie-breaks on `indexed_at ASC`, and command/skill identities carry a real RFC3339 timestamp, so a `<name>-persona` colliding with a command/skill **unconditionally wins the base name** — contradicting FR-062 (Phase 5 tie-break applies to regular personas; only `drop-persona` is reserved). The empty seed is documented only for `drop-persona`; this looks like an over-broad copy. Fix: carry the agent's real `indexed_at` into the persona `EntryIdentity` (add `indexed_at` to `EnabledAgent`); keep `String::new()` ONLY for `drop-persona`.
- **[MAJOR] R4-2** `resolve_expose_personas` (`mcp/mod.rs`) duplicates the three scope-loaders (`load_project_marker`/`load_workspace_settings`/`load_global_settings`) verbatim from `commands/harness/list.rs`; `harness/sync.rs` has a third copy (same NotFound/parse-error arms + identical reason strings). Violates the SSOT-at-second-consumer convention; drift hazard. Fix: promote the three loaders to a shared `settings` module and have all consumers call them.
- **[MINOR] R4-3** `doctor/checks.rs:210` hard-codes `expose_personas = false`, so the doctor prompts/collision report under-reports a flag-on server's real persona collisions. US5 follow-up (PersonaReport).
- Confirmed: async-island boundary clean; `PersonaRole` dispatch exhaustive no-catch-all; `drop-persona` reservation sound (empty catalog/plugin/indexed_at sorts first); `resolve_scalar_with` cleanly reusable for US5's second scalar; no bad unwrap/panic.

## Test
- **[MAJOR] T4-1** FR-067 startup-scope toggle resolution is UNTESTED end-to-end. `tests/settings_p6.rs` tests only the pure `resolve_scalar` closure; `tests/personas.rs` passes a literal bool. `resolve_expose_personas` + `build_prompt_registry` (on-disk project→workspace→global at the server startup scope) have zero coverage — the load-bearing FR-067 claim. Add an integration test: write settings files, assert the resolved toggle reflects the startup scope irrespective of the project marker.
- **[MINOR] T4-2** `get_wraps_and_substitutes` doesn't assert the frontmatter is actually STRIPPED (a regression wrapping the raw file would pass). Add `assert!(!text.contains("description: ..."))`.
- **[MINOR] T4-3** Persona-path warn-and-skip (unresolvable/unparsable agent) untested (the existing skip tests run with personas off). Add: delete one agent's `.md`, assert the registry builds, surviving personas + `drop-persona` present, broken one absent.
- **[MINOR] T4-4** Persona name length/sanitisation backstop (FR-061/066) untested (pairs with C4-2).

## Security — PASS (0 BLOCKER / 0 MAJOR)
- **[MINOR] S4-1** Conversational closing-tag / `display_name` breakout: the agent body (verbatim) sits between `<{persona_name}>…</{persona_name}>` and `display_name` is interpolated into the prose; a body containing `</…-persona>` could confuse the LLM's perception of the region. LOW — conversational text fed to an LLM, not a re-parsed file (unlike US3's validated marker case); double operator opt-in; advisory caveat present. Accept + document as an in-band limitation (parity with the Phase 5 `$ARGUMENTS` caveat).
- Confirmed clean: no new/bypassed substitution path (NFR-007 — reuses Phase 5 `render` + `map_caller_arguments`); `args` opaque, no shell/path sink; body re-read path-validated (`validate_db_stored_path`) + bounded; `drop-persona` reservation unhijackable; nothing auto-enables (double opt-in); no DoS.

**Overall**: no blocker; 2 contract/rust correctness MAJORs (empty `${TOME_PLUGIN_VERSION}`; persona collision bias), an SSOT refactor MAJOR, an FR-067 test MAJOR, minors. Security PASS.
