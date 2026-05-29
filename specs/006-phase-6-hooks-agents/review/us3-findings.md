# US3 (Guardrails + rules-file correction) — Reviewer findings

4-reviewer parallel pass against the US3 surface (`a991312..HEAD`). Recorded before any fix.

## BLOCKER (independently confirmed by Rust-lens, test, AND security)

- **B-1 Marker injection / verbatim-body poisoning.** `GUARDRAILS.md` bodies are copied **verbatim** between managed markers (`guardrails.rs` `compose_in_file`/`format_marker_region`) with **no validation that the body contains no marker lines**. Re-parsed on every sync (`find_marker_regions`). Consequences, all on the NEXT reconciliation:
  1. **Region escape → permanent prose injection**: a body containing `<!-- END GUARDRAILS: c:p -->` … `<!-- START GUARDRAILS: c:p -->` lands attacker prose OUTSIDE Tome's markers in `CLAUDE.md`/shared `AGENTS.md`, surviving all future syncs AND the plugin's own disable (removal only drops bytes BETWEEN markers). A hostile catalog plants LLM-trusted instructions in the rules file.
  2. **File wedge (DoS)**: a stray `<!-- END GUARDRAILS: … -->` body line → next `find_marker_regions` returns `Err` → exit 46 → that file never reconciles again (the malicious region can't even be removed).
  3. **Rules-block corruption**: escaped content can plant a bare `<!-- tome:begin -->` → wedges the Phase-4 rules subsystem too.
  No malicious intent required — a plugin author *documenting* the marker syntax self-wedges. **Fix:** fail closed in `read_guardrails_source` — reject (exit 46, naming the source, forward-progress) any body line matching the guardrails START/END regex or the `tome:(begin|end)` block regex. Escaping is wrong (body is contractually verbatim); refusal is the honest option (matches US2's UTF-8 fail-closed pattern). Add negative tests for all three crafted bodies.

## Rust-lens
- **[MAJOR] R3-1** `compose_in_file` builds `GuardrailsWriteFailed { path: PathBuf::new() }` (empty path) on a parse error; the caller propagates `?` without substituting the real target → unusable diagnostic for the most likely failure (B-1). Fix: wrap with the real `target` (as the other failure arms do) or thread `target` in.
- **[MINOR] R3-2** `classify` re-parses `prior` with `find_marker_regions(...).unwrap_or(false)` — a second full parse purely for Updated-vs-Removed + a swallowed error. Thread `had_regions`/`seen_keys` out of `compose_in_file`; drop the swallow.
- **[MINOR] R3-3** Clone-heavy rebuild: `compose_in_file` allocates a `String` per file line every sync before the byte-equality short-circuit. Pre-existing pattern (`compose_block_write`); US3 multiplies it across in-file targets. Optional `push_str` fast-path. Defer.
- **[MINOR] R3-4** `reconcile_standalone_sibling` `remove_file`s the Cursor sibling unconditionally when empty, with no ownership check — deletes a user file that happens to sit at `.cursor/rules/TOME_GUARDRAILS.md`. Symlink-refused but not content-checked. Low risk (Tome-specific filename, contractually fully-owned).
- Confirmed: marker-region engine families don't cross; suppression filesystem-presence-only computed pre-guardrails; `SyncSubsystem::Guardrails` no-catch-all; `pub(crate)` promotions correct; sink order hooks→guardrails→agents; read-only DB propagates.

## Test
- **[MAJOR] T3-1** Marker-spoofing body untested (= B-1). Add round-trip + resync-survives tests.
- **[MAJOR] T3-2** Atomicity (injected write failure → exit 46, no partial region) NOT tested — the only exit-46 test is the pre-write symlink refusal, which never reaches the atomic-write path. Add an injected-mid-write-failure test asserting exit 46 + file byte-unchanged.
- **[MAJOR] T3-3** "AGENTS.md region unaffected during a suppression transition" untested — `both_transitions_in_one_sync` installs only claude-code. Add codex; assert both plugins' regions persist on AGENTS.md across both transitions.
- **[MINOR] T3-4** Five-harness render tested with 3; gemini's `AGENTS.md`-else-`GEMINI.md` branch untested for guardrails. Add gemini (no AGENTS.md → GEMINI.md; with AGENTS.md → shared).
- **[MINOR] T3-5** Two-plugin distinct regions tested only on shared AGENTS.md, not the Cursor sibling (separate rebuild path). Add cursor.
- **[MINOR] T3-6** Overwrite-in-place with CHANGED source content untested at integration level (only unit + unchanged-idempotence). Add: change `GUARDRAILS.md` body between two syncs, assert region updated in place, one START.
- **[MINOR] T3-7** Verbatim-body fidelity for parseable-looking content (frontmatter/headings/`@include`/trailing ws) unasserted.

## Contract
- **[MINOR] C3-1 Exit-code contradiction (doc).** `guardrails.md:87,109` say symlink refusal → **exit 7**, but the impl + e2e test + the AUTHORITATIVE `exit-codes-p6.md` (code 7 explicitly excludes "a guardrails target"; code 46 = guardrails render/write) say **46**. The impl follows the authoritative pin. Fix the `guardrails.md` prose 7 → 46. No code change.
- Otherwise conformant: correction (CLAUDE.md not AGENTS.md, both resolve `.tome/RULES.md`), markers, per-harness targets, suppression CLAUDE.md-only + both transitions, deterministic lex placement, Cursor sibling delete, idempotence.

**Overall**: 1 BLOCKER (marker injection), majors (empty error path; untested atomicity + cross-file transition), minors + a doc exit-code fix. Security otherwise clean (symlink refusal, sibling-delete target, path composition, perms all sound).
