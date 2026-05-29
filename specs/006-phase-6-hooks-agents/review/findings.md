# Phase 6 — Phase-wide reviewer findings

**Branch**: `006-polish` · **Baseline**: US5 merged @ de699d7 (PR #136).
**Surface**: the COMPLETE Phase 6 source (real Claude Code hooks + guardrails prose
fallback + native agent translation across four harnesses + agent personas via MCP
prompts + the Phase 4 rules-file correction). ~6.7k src LOC across 35 files; ~40 test
files.

Four read-only reviewers dispatched in one message (contract / Rust-lens =
`devs:rust-dev` / test / security). Each read the five `us{1..5}-disposition.md`
docs first to avoid re-litigating settled items; this pass exists to catch
**cross-US drift the per-US passes structurally could not see** (the Phase 5 P8
lesson).

**Tally**: 0 BLOCKER · 2 MAJOR (CON-1, TEST-1) · 8 MINOR · INFO/confirmations.
Security: 0 findings; **T151 cap-std verdict = DEFER**.

---

## Contract reviewer — 0 BLOCKER, 1 MAJOR, 3 MINOR, 1 INFO

All 9 contracts mapped to code; implementation conforms with high fidelity. The
per-US cross-US fixes (C4-1/R4-1 column projection, R-1 DB-error propagation, B-1
marker defence, C3-1 guardrails exit-code reconciliation, S-1 path validation,
C5-1 doctor refresh, FR-067 startup-scope, Claude-Code-only strip) are all applied
and internally consistent. Numeric codes 43–46 are correct and used consistently
in `src/error.rs` and every sink call site.

- **CON-1 — MAJOR — cross-US — new.** `contracts/hooks-integration.md:94,117` +
  `exit-codes-p6.md:50` vs `src/harness/hooks.rs` (`refuse_symlink_settings`) +
  `tests/hooks_merge.rs:343-387`. A symlinked `.claude/settings.local.json` write
  target surfaces **exit 7** (raw `Io` propagates), hooks-integration.md documents
  7, and the test asserts 7. But the **authoritative** `exit-codes-p6.md:50` says
  code 7 covers IO that is "**not** the local Claude settings file," and code **44**
  (`HookSettingsWriteFailed`) is "read, merge, or **write** failure on the project's
  local Claude settings file." A symlink refusal is a pre-write guard on that exact
  file → should be **44**. The directly parallel guardrails case was *deliberately*
  reconciled in US3's C3-1 (guardrails symlink → 46, not 7). The hooks path was left
  on 7, so the two contracts now give conflicting codes for the same operation class
  (symlink refusal on a dedicated sink). Not a correctness/security defect (exit 7 is
  a sane deterministic IO signal; the target is provably untouched) → MAJOR not
  BLOCKER. **Rec**: wrap the settings symlink-refusal `Io` into
  `HookSettingsWriteFailed` (44); update hooks-integration.md:94,117 → 44; flip the
  `tests/hooks_merge.rs` assertions (≈ lines 361/378/386). Keeps the symlink-refusal
  exit-code policy uniform across all three Phase 6 sinks.

- **CON-2 — MINOR — cross-US — new.** `exit-codes-p6.md:50` lists reading a plugin's
  `GUARDRAILS.md` under code **7**, but the B-1 fix maps a symlinked / unreadable
  source to `GuardrailsWriteFailed` (**46**). Stale example. **Rec**: remove
  `GUARDRAILS.md` from the code-7 example list. Doc-only.

- **CON-3 — MINOR — cross-US — new.** `exit-codes-p6.md:50` lists reading
  `agents/*.md` under code **7**; true at *index* time (`lifecycle.rs` → `Io`/7) but
  at *sync* time `prepare_agent` maps it to `AgentTranslationFailed` (**45**).
  **Rec**: clarify the code is 7 only on indexing; sync surfaces 45. Doc-only.

- **CON-4 — MINOR — no — tracked (US4 C4-3).** Contracts' Tests tables reference
  stale test filenames (`tests/persona_prompts.rs`, etc.); real files differ. C4-3
  dispositioned the persona case; staleness extends to hooks/guardrails/doctor Tests
  tables. **Rec**: defer (consistent with C4-3); optional docs sweep. Non-blocking.

- **CON-5 — INFO — cross-US — new (note).** `HarnessDecision` JSON field order is
  `agents_action`,`hooks_action`,`guardrails_action` (merge chronology — each
  "appended LAST"), NOT the hooks→guardrails→agents *processing* order. Internally
  consistent and pin discipline intact; only a latent reader-confusion hazard.
  **Rec**: no action; optionally one-line comment noting field order = merge
  chronology, distinct from processing order.

---

## Rust-lens reviewer (`devs:rust-dev`) — 0 BLOCKER, 0 MAJOR, 2 MINOR, 3 INFO

The assembled phase is clean. The three-way reconciler composition (the part no
per-US pass could see) is correct: all three share the `reconcile_<sink>` template,
propagate read-only-DB-open errors on an existing DB (never `.ok()`-swallow), fixed
sink order hooks→guardrails→agents, and multi-sink errors surface the earliest
sink's `first_error` first. SSOT discipline holds; closed error set; no reachable
panic; no async leakage; no stringly-typed `kind` dispatch.

- **RUST-1 — MINOR — cross-US — new.** `sync.rs::reconcile_hooks` (765 + 867-888) vs
  `reconcile_guardrails` (1013-1022). Enabled-plugin enumeration computed 3× per
  sync; `compute_plugins_with_hooks_json` opens a *second* independent read-only
  handle within the hooks pass. Needless-work only (all read-only, deterministic, no
  destructive path). **Rec**: defer; CONCERNS note alongside R5-3. Folding it risks
  the carefully-ordered "hooks-presence set computed even when no harness
  participates" logic.

- **RUST-2 — MINOR — no — new.** `sync.rs::reconcile_guardrails:1067-1071`. Per-path
  dedup re-filters all snapshots per unprocessed path — O(harnesses²), bounded by the
  ~4 fixed harnesses; duplicates the `group_by_path` the rules/MCP loop already does.
  **Rec**: defer; cosmetic, fold into Polish if touching the function.

- **RUST-3 — INFO — cross-US — confirms US1 R-1.** All four `open_read_only` sites
  verified: reconcilers PROPAGATE on existing-but-unopenable DB (the `if exists()
  {open()?} else {None}` form); doctor/list surfaces intentionally degrade-to-None
  (read-only projections, no destructive reconciliation). The asymmetry is correct
  per FR-561/FR-124. No action.

- **RUST-4 — INFO — cross-US — confirms US5.** Privilege-strip borrow discipline
  sound: strip on a per-emission clone; audit (`build_privilege_escalation_report`)
  independently re-reads each agent's SOURCE `.md`, so it sees unstripped fields
  regardless of the setting. FR-051 holds. No action.

- **RUST-5 — INFO — tracked (TD-061/063/064, R5-3).** All confirm-deferred with the
  full phase assembled: TD-064 `compose_in_file` rebuild (byte-stable, bounded by
  `HARNESS_RULES_MAX`); TD-063 stale-hook-removal gap (no clean fix under no-sidecar
  NFR-003; `HooksReport` surfacing adequate; both arms documented inline); R5-3
  `--fix` heuristic (idempotent+safe); TD-061 per-agent shrink (US5 `--fix` removes
  orphans). No escalation.

---

## Test reviewer — 0 BLOCKER, 1 MAJOR, 4 MINOR, 2 INFO

All five new doctor reports + sub-records and `SyncOutcome`'s three new action fields
carry byte-stable pins; every spot-checked dispositioned test addition (T-1, T2-1,
B-1, T4-1, T4-3, T5-1, T5-2) actually landed with bug-defeating assertions.

- **TEST-1 — MAJOR — cross-US — new.** `sync.rs:424-432` (fixed-order `first_error`
  surfacing) has NO test exercising MULTIPLE sinks failing in one sync. Every
  per-sink forward-progress test malforms exactly one sink in isolation; total
  assertions on exit 43/45/46 across the suite = 2. A refactor reordering the three
  `if let Some(..err)` checks, or early-returning from a reconciler, would pass every
  existing test and silently change the operator-visible exit code / which sinks get
  abandoned. **Rec**: add a dedicated test — seed malformed `hooks.json` + a
  marker-injected `GUARDRAILS.md` + a corrupt agent source + one healthy plugin under
  a harness driving all three sinks; assert `exit_code() == 43` (hooks wins) AND the
  healthy plugin's hook entry + guardrails region + agent file all landed (forward
  progress crossed all three sinks). A second variant removing the hooks-malformed
  plugin → exit 46 (guardrails next), proving precedence is real.

- **TEST-2 — MINOR — cross-US — new.** The only top-level `DoctorReport` envelope pin
  sets all Phase 6 fields to `None` (elided), so it asserts nothing about their
  "appended LAST" relative order; `doctor_p6_json_shape.rs` pins each report in
  isolation. Reordering `personas` before `hooks` in the struct would break no test.
  **Rec**: add one all-`Some` envelope pin asserting
  `hooks < guardrails < agents < privilege_escalation < personas`, all after
  `entry_counts`.

- **TEST-3 — MINOR — no — new.** Two of five report pins (Guardrails, Persona) use
  `serde_json::to_value` comparison (key-order-INSENSITIVE) not `to_string`, so they
  don't actually pin field ORDER (NFR-011 is an ordering property). The other three
  use order-sensitive `to_string`. **Rec**: convert the two to `to_string`-against-
  literal (matching the hooks/agents/privilege style), or add `find()`-ordering
  asserts.

- **TEST-4 — MINOR — tracked (GAP-1, WEAK).** Phase 6 closed neither Phase 5 deferred
  test item: exit codes 9/26-29 still lack e2e CLI coverage (MCP-internal; need an
  in-process MCP harness); `SubsystemHealth`/`ProjectBindingState`/`RulesCopyState`
  still have no byte-stable wire pin (Phase 4 types riding inside `DoctorReport`).
  Out of Phase 6 NFR-011 scope (not new Phase 6 types) but must not silently age out.
  **Rec**: carry GAP-1 + WEAK forward in CONCERNS; if cheap, add 3-line `to_string`
  pins for the three Phase 4 enums (they ride the doctor envelope).

- **TEST-5 — INFO — cross-US — new (gap T150 fills).** The three persona test files
  have ZERO hooks/guardrails coverage; the closest whole-flow test (`doctor_p6.rs`)
  is claude-code-only (guardrails suppressed) and asserts nothing about personas. No
  single test proves enable → sync (hooks + rendered guardrails region + agent file)
  → persona registry exposed from one live flow. That is T150's mandate; not
  redundant with any existing test.

- **TEST-6 — INFO — no — confirmation.** v3→v4 marker migration, FR-070
  (agents non-searchable / absent from search / two-row name collision), and the
  privilege strip/audit-separation tests all landed clean. No gaps.

**Guidance for T149/T150** (captured for the implementer): both use the library API
(`harness::sync::sync_project`), hold `OVERRIDE_MUTEX` + `HarnessModulesGuard`, reuse
the `harness_sync_stub.rs` fixture scaffold + `mtime()` helper + 1100ms sleep.
- **T149 idempotence** — NOT covered today (existing idempotence tests are per-sink).
  Use TWO harnesses (claude-code + codex) so guardrails actually RENDERS a region
  (not just suppresses); seed one plugin shipping `hooks.json` (with
  `${CLAUDE_PLUGIN_ROOT}`), valid `GUARDRAILS.md`, one agent. Sync 1 → capture mtimes
  of all sink outputs; sleep 1100ms; sync 2 → assert `added`/`updated` EMPTY for every
  `SyncSubsystem` AND mtimes unchanged (the load-bearing check).
- **T150 e2e** — happy whole-flow: enable plugin (hooks + GUARDRAILS.md + agent),
  set `expose_agents_as_personas=true`, `sync_project`, build persona registry with
  expose=true. Assert all FOUR: hooks command rewritten in `settings.local.json`;
  guardrails region body present between markers in the rendered target; agent file
  present with translated frontmatter; persona list contains `<agent>-persona` +
  `drop-persona`. Keep the failure/precedence path in TEST-1's dedicated test.

---

## Security reviewer — 0 BLOCKER, 0 MAJOR, 2 MINOR, 3 INFO · T151 = DEFER

Audited all four file sinks + three cross-cutting trust boundaries. The two per-US
security blockers (S-1 path traversal, B-1 marker injection) remain sound under
full-phase assembly. The cross-US verbatim-body injection vectors hunted for in the
agent sinks are NOT exploitable.

- **SEC-1 — MINOR — cross-US — new.** `agents.rs::render_markdown_yaml` (327-347):
  agent body appended verbatim after the YAML frontmatter for claude-code/cursor/
  opencode. Hunted the "hostile body smuggles a second `---` frontmatter block to
  inject privileged fields" vector — **not exploitable**: only the *leading* `---…---`
  is frontmatter (a later `---` is a thematic break), and frontmatter values are
  YAML-escaped by `serde_yaml`. Residual: the guarantee rests on destination harnesses
  using leading-block-only frontmatter semantics (true for all four targets today).
  **Rec**: no code change for v0.6.0; add a one-line comment documenting the
  verbatim-body trust assumption (mirroring the Codex `toml_edit`-escaping note) so a
  future harness re-evaluates; optionally a regression test.

- **SEC-2 — MINOR — cross-US — new (adjacent to TD-063).** `sync.rs::
  compute_plugins_with_hooks_json` (867-888): an existing-but-unopenable DB has its
  error swallowed by the caller (`unwrap_or_default` → empty), the OPPOSITE
  fail-direction from the R-1/reconciler propagate rule. Here the consequence is
  benign (Claude Code renders an extra prose guardrails region; next sync corrects),
  fail-safe not fail-dangerous. **Rec**: no behavioral change; add a comment marking
  this an *intentional* exception (failure mode = "render an extra region", not
  "delete enabled state") so the two opposite error choices are auditable side by
  side.

- **SEC-3 — INFO — no — tracked (S4-1).** Persona wrapper interpolates the body
  between `<{name}>…</{name}>` tags; a body with `</name>` closes the tag early
  in-band — conversational-context confusion, NOT a re-parsed-file vuln. Double
  opt-in (default off), same `COMBINED_RE` single-sweep (no new substitution surface),
  caveated description, `drop-persona` appended last via sorted iteration. No action.

- **SEC-4 — INFO — cross-US — tracked (SEC-024/FR-051).** Audit integrity verified
  end-to-end: strip clears all three fields (`hooks`/`mcp_servers`/`permission_mode`)
  on a per-emission clone; `PrivilegeEscalationReport` re-reads each agent's SOURCE
  `.md` and lists all three regardless of the strip setting. Strip is claude-code-only.
  No action.

- **SEC-5 — INFO — cross-US — tracked (TD-062/SEC-019/C-1).** Symlink refusal across
  all four sinks is final-node-only (intermediate-dir symlink unchecked → narrow
  TOCTOU). Final-node discipline + mode preservation + atomic tempfile+rename + fail-
  closed non-UTF-8 all present and correct on every sink. Subject of T151 (below).

### T151 cap-std verdict — DEFER

Do NOT add `cap-std` (or any intermediate-dir-symlink hardening needing a new
top-level dep) for v0.6.0. Rationale across all four sinks:
1. **Constitution gate is decisive** — Phase 6 forbids any new top-level dependency;
   `cap-std` would be exactly that, requiring an amendment Phase 6 deliberately
   avoided (leanest phase since Phase 1).
2. **Residual is narrow + misaligned with the trust model** — the intermediate-dir
   TOCTOU needs an attacker who can plant/swap a symlink *inside* the operator-owned
   `.claude/`/`.codex/`/`.cursor/`/`.opencode/`/`.tome/` tree in the check→open
   window; such an attacker already holds the operator's filesystem privileges. Plugin
   content never supplies directory components (only the final filename, validated as
   a single safe segment + re-asserted via `target.parent() == Some(dir)`).
3. **Current mitigations adequate** — final-node symlink refusal blocks the realistic
   "hostile catalog symlinks the target at `~/.ssh/id_rsa`" vector; atomic
   tempfile+same-FS rename → no partial/corrupt file; mode preservation; fail-closed
   non-UTF-8. No NEW symlink-exposure class vs the pre-existing Phase 4 discipline.
4. **A future phase would need** either a constitution amendment to take `cap-std`
   (capability `Dir`-relative `openat`/`O_NOFOLLOW` — the clean fix), or a no-dep
   `O_NOFOLLOW` + `openat`-walk on `std` (which only hardens the final node — already
   covered — so buys little). Take the trade only when an unrelated need justifies the
   dep or the trust model extends to shared/untrusted dirs.

**Paste-ready deferral note** drafted in `disposition.md` / to be added to
`CONCERNS.md` at T151.

### Confirmed-clean (security)
Agent name traversal (S-1) enforced at index time + defence-in-depth at write;
guardrails marker injection (B-1) scan-regex == parse-regex, fail-closed 46; Codex
TOML body escaped by `toml_writer` (`"""` breakout impossible); hooks settings sink
(symlink refusal, 1 MiB bounded reads, exit-44 non-UTF-8, deep-equality merge/remove,
atomic + mode-preserving, only `settings.local.json`); privilege strip + audit
integrity; persona double-opt-in; `--fix` write safety (single-path `remove_file`,
inherits writer safety, read-only by default FR-124).
