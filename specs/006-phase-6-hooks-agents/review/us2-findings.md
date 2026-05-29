# US2 (Real Claude Code hooks) — Reviewer findings

4-reviewer parallel pass against the US2 surface (`fa8b25f..HEAD`). Recorded before any fix.

## Security — CLEAN (0 BLOCKER / 0 MAJOR / 0 MINOR)
Symlink refusal sound (final-node check + `rename(2)` atomic replace — a TOCTOU-planted symlink is itself replaced, not followed); `settings.json` never written (hooks sink touches only `settings.local.json`); rewrite injects only Tome-controlled paths into string values (keys untouched); removal is structural-deep-equal only (no over-broad match); new file 0600, parent `.claude/` 0700 only when Tome creates it; reads bounded at 1 MiB; `serde_json` last-wins de-dupes keys (no smuggling); existing-but-unopenable DB propagates before any write (no R-1-style mass effect).

## Rust-lens
- **[MAJOR] R2-1 Evicted/missing plugin source → stale hook entries never removed.** When a plugin is still enabled in the DB but its on-disk source is gone (catalog evicted), both the `plugin_root_dir` `Err` arm (`sync.rs:757 continue`) and the `read_rewritten_entries` `NotFound → Ok(None)` arm drop it from `prepared`. If claude-code then goes non-live / the plugin is removed, `remove_hooks_for_harness` can't re-derive its entries, so its absolute-path hook entries persist in `settings.local.json` and keep executing. The inline comment only acknowledges the `Err` arm. No clean fix exists under the no-sidecar ownership model (NFR-003) — removal requires the source to re-derive the deep-equal entry. Decision needed: document both arms + ensure US5 doctor surfaces hook orphans.
- **[MINOR] R2-2** `plugin_root.to_string_lossy()`/`plugin_data.to_string_lossy()` in the rewrite (`hooks.rs:136-144`) silently U+FFFD-corrupts a non-UTF-8 install path — and this value is a LOAD-BEARING executed command. Better to fail closed (exit 44) than emit a silently-broken hook command.
- **[MINOR] R2-3** FR-002 create-if-absent only fires when ≥1 entry is merged (the loop skips empty/absent-hook plugins). Better hygiene than emitting empty files; diverges from literal contract wording. Clarify.
- **[MINOR] R2-4** Sequential two-needle `str::replace`: a `plugin_root` dir literally named `${CLAUDE_PLUGIN_DATA}` would be re-rewritten by the second pass. Impossible for a real `~/.tome/...` path; note-only.
- Confirmed: closed-set/no-catch-all on `SyncSubsystem::Hooks`/`HooksStrategy`; atomic write mirrors `mcp_config`; R-1 avoided (`open_read_only(...)?`); `plugin_root_dir` promotion preserves semantics; deep-equal merge/remove/prune correct; forward-progress; no bad `unwrap`.

## Test
- **[MAJOR] T2-1 Symlink refusal on `settings.local.json` write (exit 7) untested** — contract lists it; implemented (`refuse_symlink_settings`) but no test. Add merge + remove variants (plant symlink, assert exit 7, target unchanged, still a symlink).
- **[MAJOR] T2-2 Hooks forward-progress untested** — one malformed + one good plugin → good plugin's entry still merges AND sync exits 43. (Agents have this test; hooks don't.)
- **[MINOR] T2-3** Multi-plugin merge into one `settings.local.json` in a single sync (all merge tests use one entry).
- **[MINOR] T2-4** Exit 44 has no behavioral test — cheapest trigger: a malformed/wrong-type existing `settings.local.json` (`load_settings`/`ensure_hooks_object` → 44); assert original left intact.
- **[MINOR] T2-5** Multi-event partial-prune: remove event A (→ pruned) while a user entry under event B survives.
- **[MINOR] T2-6** Symlinked hook *source* refusal → exit 7 (read path; lower priority).
- Well-covered: two-token rewrite + verbatim survival + keys-untouched, idempotent re-add (mtime), user-authored dedup, user-edit preservation, create-if-absent + settings.json-never-touched, single-event prune, malformed-source exit 43 (unit + e2e), removal-on-disable e2e.

## Contract — no blocker/major
- **[MINOR] C2-1** Fixed-needle `str::replace` vs the contract's "small `regex` replace": outcome-identical and safe (trailing `}` prevents prefix collisions; values-only structural; no ReDoS). Assessed and accepted; contract text is descriptive not normative.
- **[MINOR] C2-2** Create-if-absent fires only when an entry is merged (= R2-3). Clarify contract.
- **[MINOR] C2-3** Unreadable source (EACCES/oversize/non-UTF-8) → exit 43 is intentional (documented in code); note in contract.

**Overall**: security clean; no BLOCKER; 1 Rust MAJOR (stale-removal gap, no clean fix — document/defer), 2 test MAJOR (symlink refusal + forward-progress), minors.
