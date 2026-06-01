# Contract: Robustness & Honest Trust Posture (US2)

**FRs**: FR-006, FR-008, FR-009, FR-010 · **SCs**: SC-005 · **Research**: §R-7/8/9 · (FR-007 has its own contract: `symlink-guard.md`)

---

## FR-006 — Every third-party read bounded by its existing per-class cap (F-PLUGIN-MANIFEST-DOS)

**Invariant**: every read of a third-party file MUST be bounded by **that read's existing per-class cap** — `PLUGIN_MANIFEST_MAX` (256 KiB) for manifests/frontmatter, `HARNESS_MCP_MAX` (1 MiB) for settings/hooks — **not** a single new cap — and fail with a named error rather than exhausting memory. The fix MUST cover the **whole class**: no unbounded `std::fs::read`/`read_to_string` on a third-party path.

**Mechanism** (§R-7): route through `crate::util::bounded_read(path, <per-class cap>)` (the helper the sibling `SKILL.md` path already uses at `frontmatter.rs:292`).

**Site list (minimum)**:

| Site | File | Cap |
|---|---|---|
| `plugin.json` read | `plugin/manifest.rs:61` | `PLUGIN_MANIFEST_MAX` |
| `tome-catalog.toml` read | `catalog/manifest.rs:46` | `PLUGIN_MANIFEST_MAX` |
| lifecycle read | `plugin/lifecycle.rs:958` | per-class |
| components read | `plugin/components.rs:170` | per-class |
| doctor read-only/CI surface (`tome-catalog.toml`) | `doctor/checks.rs:174` | `PLUGIN_MANIFEST_MAX` |

(A grep for `fs::read`/`read_to_string` on third-party paths during implementation confirms no site is missed — "fix the class, not the instance.")

**Test obligations**: `tests/bounded_reads.rs` — feed an oversized file (well past the cap) at each site through `enable`/`show`/`list`/`doctor`; assert a **bounded named error** (the per-class parse/size error naming the file), never OOM. SC-005: bounded error in well under a typical dev machine's memory headroom, across every read site.

---

## FR-008 — OpenCode receives an inline rules body (F-RULES-OPENCODE)

**Site**: `src/harness/sync.rs::compute_rules_body` (the shared-`AGENTS.md` body-style decision).

**Invariant**: when OpenCode shares a rules file with an AGENTS-based harness, the shared body MUST be written in a form OpenCode can resolve — **not** an unresolved `@`-include directive. Concretely: if **any** live sharer requires an inline body, the shared body is written **inline** (valid for include-capable harnesses too).

**Mechanism** (§R-8): pick the lowest-common-denominator body style across live sharers (if any sharer is `Inline`, write `Inline`), mirroring the guardrails reconciler's existing union approach. Inline is a strict superset — `AtInclude` harnesses also resolve an inline body.

**Test obligations**: `tests/rules_opencode_inline.rs` — OpenCode + Codex (and OpenCode + Gemini) in the effective list: assert OpenCode's shared file contains the **inline body**, not `@.tome/RULES.md` as literal prose; assert the include-capable harness still resolves it. (Lands after the decomposition, D.)

---

## FR-009 — `catalog remove --force` re-derives its cascade inside the lock (F-REMOVE-TOCTOU)

**Site**: `src/commands/catalog/remove.rs`.

**Invariant**: `catalog remove --force` MUST re-derive its cascade input **inside** the advisory-lock-held closure so a concurrent `plugin enable` (serialising on the same `index.lock`) cannot leave a ghost-enabled plugin whose catalog enrolment is removed.

**Mechanism** (§R-9): move the enabled-plugins read inside the locked closure; don't reuse the pre-lock `Vec`. Maps `CONCERNS.md` TD-017.

**Test obligations**: `tests/catalog_remove_toctou.rs` — drive a `catalog remove --force` racing a concurrent `plugin enable` (two processes / serialised on the lock); assert no plugin is left enabled with its catalog enrolment deleted. The single-process common case is a regression guard.

---

## FR-010 — Published security documentation draws the mechanical-vs-semantic line

**Site**: the security documentation published in the README/SECURITY slice (REL4) — tracked here as the US2 trust-posture requirement.

**Invariant**: the project MUST publish security documentation that (a) **enumerates the mechanical defences** Tome provides — no OOM (bounded reads), no path traversal (path-segment validation), no symlink escape (the FR-007 guard incl. intermediate components or the documented fallback), no file corruption (atomic writes + structural-match merge) — and (b) states **explicitly** that Tome **cannot vet a catalog's content** (its skills/commands/agents are instructions the user's own agent will execute) and that **adding a catalog is trusting it** — "only add catalogs you trust."

**Test obligation**: SC-010-adjacent / SC (US2 Independent Test) — a doc-presence check that the security page draws the mechanical-vs-semantic line and states the trust rule. (Content requirement, verified by review + the README smoke check.)

---

## Cross-cutting

- No schema change, no new exit code (FR-006 uses per-class parse/size errors; FR-009 uses the existing lock/enrolment paths).
- FR-008 and FR-007 both land **after** the decomposition so they land in the clean per-sink structure.
- FR-010's text is the honest framing of the FR-006/FR-007 mechanical guarantees + the unchanged "trusted on enrol" model (out of scope to change the trust model this phase).
