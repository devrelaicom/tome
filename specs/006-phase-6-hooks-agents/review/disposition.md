# Phase 6 — Phase-wide reviewer disposition

Decisions on the [phase-wide findings](./findings.md). **Committed before any fix is
applied** (per the per-US discipline carried into Polish). 0 BLOCKER, 2 MAJOR.

Two commits carry the dispositioned work:
- **T148 `fix(phase-6): apply phase-wide reviewer findings`** — CON-1, CON-2, CON-3,
  CON-5, SEC-1, SEC-2, TEST-2, TEST-3, TEST-4 (the 3 cheap Phase-4 pins).
- **T152 `test(phase-6): idempotence + e2e + security hardening`** — TEST-1 (multi-
  sink precedence test, lands beside T149/T150), and the T151 cap-std DEFER note.

## Fix now

| ID | Sev | Decision | Action | Commit |
|---|---|---|---|---|
| CON-1 | MAJOR | **Fix** | Wrap the `.claude/settings.local.json` symlink-refusal `Io` into `HookSettingsWriteFailed` (exit 44) at the refusal site in `hooks.rs`, reconciling with the authoritative `exit-codes-p6.md` and the parallel US3 C3-1 guardrails precedent (symlink → the sink's dedicated code, not 7). Update `contracts/hooks-integration.md:94,117` exit 7 → 44. Grep for ALL assertions of exit 7 on a settings-file symlink and flip them to 44 (`tests/hooks_merge.rs` ≈ 361/378/386, plus any e2e). Verify with `cargo test --test hooks_merge` + `--test hooks_rewrite`. | T148 |
| CON-2 | MINOR | **Fix (doc)** | Remove `GUARDRAILS.md` from the code-7 reuse example in `exit-codes-p6.md:50` — a symlinked/unreadable guardrails source now surfaces as 46 (B-1). | T148 |
| CON-3 | MINOR | **Fix (doc)** | Clarify `exit-codes-p6.md:50`: agent-source IO is code 7 only on the *indexing* path; during `harness sync`, agent source/parse failures surface as 45 (`AgentTranslationFailed`). | T148 |
| CON-5 | INFO→fix | **Fix (comment)** | One-line comment on `HarnessDecision` noting field order is merge-chronology (agents/hooks/guardrails, each appended LAST), distinct from the hooks→guardrails→agents *processing* order — pre-empts diff confusion. Trivial. | T148 |
| SEC-1 | MINOR | **Fix (comment + test)** | Add a code comment at `agents.rs::render_markdown_yaml` documenting the verbatim-body trust assumption (leading-block-only frontmatter; a later `---` is a thematic break; values YAML-escaped → body cannot inject frontmatter fields), mirroring the Codex `toml_edit`-escaping note. Add a `body_with_frontmatter_delimiter_does_not_inject_fields` regression pin. | T148 |
| SEC-2 | MINOR | **Fix (comment)** | Add a comment at `sync.rs::compute_plugins_with_hooks_json` marking the existing-DB `unwrap_or_default` swallow an *intentional* exception to the propagate-on-existing-DB rule — its failure mode is "render an extra prose region" (fail-safe), not "delete enabled state" (fail-dangerous). Keeps the two opposite error choices auditable. | T148 |
| TEST-2 | MINOR | **Fix** | Add one `DoctorReport`-with-all-Phase-6-fields-`Some` serialization pin asserting `hooks < guardrails < agents < privilege_escalation < personas`, all after `entry_counts` — the missing half of the "appended LAST" wire contract. | T148 |
| TEST-3 | MINOR | **Fix** | Convert the Guardrails + Persona report pins in `doctor_p6_json_shape.rs` from `to_value` (order-insensitive) to `to_string`-against-literal (order-sensitive), matching the other three — so all five reports are genuinely byte-pinned (NFR-011 is an ordering property). | T148 |
| TEST-4 | MINOR | **Partial fix** | Add 3-line `to_string`-against-literal pins for `SubsystemHealth`, `ProjectBindingState`, `RulesCopyState` (Phase 4 enums riding inside `DoctorReport`) — closes the WEAK backlog cheaply. GAP-1 (exit 9/26-29 e2e, needs in-process MCP harness) carries forward. | T148 |
| TEST-1 | MAJOR | **Fix** | Add a dedicated multi-sink `first_error` precedence test: malformed `hooks.json` + marker-injected `GUARDRAILS.md` + corrupt agent + one healthy plugin, all sinks driven; assert exit 43 wins (hooks precedence) AND the healthy plugin landed in all three sinks (forward progress crossed every sink). A variant dropping the hooks-malformed plugin → exit 46. Own file/function (not overloading T150's happy path). | T152 |

## Defer / accept

| ID | Decision | Rationale |
|---|---|---|
| CON-4 | **Defer (doc)** | Consistent with US4 C4-3 — behaviour fully tested; only the contracts' Tests-table filenames are stale. Optional docs sweep; non-blocking. |
| RUST-1 | **Defer + CONCERNS note** | Triple enabled-plugin enumeration + a second read-only DB open in the hooks pass is needless-work only (all read-only, deterministic, no destructive path); folding it risks the "hooks-presence set computed even when no harness participates" ordering. Note beside R5-3. |
| RUST-2 | **Defer** | Guardrails per-path dedup O(harnesses²) re-filter is bounded by the ~4 fixed harnesses; cosmetic consistency with `group_by_path`. |
| RUST-3 / RUST-4 / RUST-5 | **No action (confirmed sound)** | R-1 DB-error propagation correct at all four sites; privilege-strip borrow + independent-source-read audit sound; TD-061/TD-063/TD-064/R5-3 all confirm-deferred with the full phase assembled. |
| TD-065 | **Confirm-deferred** | (From the Polish backlog.) Doctor's prompts-collision report intentionally uses `expose_personas=false` for a stable baseline collision namespace; US5's `PersonaReport` is the persona-aware surface. No reviewer escalated it; the split-surface design holds. |
| SEC-3 / SEC-4 / SEC-5 | **No action (confirmed sound)** | Persona in-band breakout = accepted caveated limitation (S4-1); privilege audit integrity end-to-end; final-node symlink refusal correct across all four sinks. |
| **T151 / cap-std** | **DEFER (documented)** | Per the security verdict below. Constitution gate forbids the new top-level dep; residual intermediate-dir-symlink TOCTOU is narrow under the operator-owned-dir trust model; current mitigations adequate. Documented in `CONCERNS.md` + a code comment at T151/T152. |
| GAP-1 | **Carry forward** | Exit codes 9, 26-29 lack e2e CLI coverage (MCP-internal; needs an in-process MCP test harness). Phase 6+ test hardening. |

### T151 paste-ready deferral note (for `CONCERNS.md`)

> **TD-062 / SEC-019 / C-1 — intermediate-dir symlink TOCTOU (DEFERRED, v0.6.0).**
> Symlink refusal on the four Phase 6 file sinks (hooks `settings.local.json`,
> guardrails in-file regions + Cursor sibling, agent files) is final-node-only; an
> intermediate directory being a symlink is not checked, leaving a narrow TOCTOU
> window before the open/rename. Accepted for v0.6.0: the Phase 6 constitution gate
> forbids the new top-level dependency `cap-std` that the clean fix (capability-based
> `openat`/`O_NOFOLLOW` directory walk) would require, and the residual risk is low
> under Tome's trust model — the operator explicitly owns and creates the project and
> harness directory trees, plugin content never supplies directory path components
> (only the final filename, validated as a single safe segment and re-asserted against
> its parent), and an attacker able to swap an intermediate-directory symlink mid-sync
> already holds the operator's filesystem privileges. Current mitigations (final-node
> symlink refusal, atomic tempfile + same-FS rename, mode preservation, fail-closed
> non-UTF-8 handling) are adequate; the exposure matches the pre-existing Phase 4
> discipline and introduces no new symlink-exposure class. Revisit when an unrelated
> need justifies the `cap-std` dependency (with a constitution amendment) or if Tome's
> trust model extends to syncing into operator-shared or untrusted directories.
