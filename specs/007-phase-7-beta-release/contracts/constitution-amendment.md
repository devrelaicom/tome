# Contract: Constitution Amendment — Release Tooling (FR-023)

**FR**: FR-023 · **SC**: SC (US5 #3) · **Research**: §R-17
**Sequencing**: lands **first** (F1), before any cargo-dist work. The current clause literally says the constitution "gets amended first."

## What changes

Amend `CONSTITUTION.md` — rewrite the Development-Workflow **"Release tooling"** clause from a deferral to an **authorisation** of the named set, with a recorded rationale and a **MINOR** version bump to **v1.4.0**.

- The clause lives under **Development Workflow**, **not** a NON-NEGOTIABLE Core Principle. Per §Governance, the 24-hour cooling-off applies only to NON-NEGOTIABLE principles, so **no cooling-off applies** (same class as the v1.3.0 §Paths amendment).
- MINOR per §Versioning (materially expanded guidance; no principle removed or inverted).
- The amendment is **enabling** (authorises previously-deferred tooling), not a relaxation of any quality gate.

## Amendment text (proposed)

Replace:

> **Release tooling** is deferred until there is something to release beyond `cargo install --path .`. When that day comes, this constitution gets amended first.

with (illustrative — final wording lands in the F1 PR):

> **Release tooling.** Tome distributes a public beta. The following release tooling is authorised: a **`cargo-dist`-driven release pipeline** producing per-platform archives with checksums; **prebuilt-binary distribution** for Linux + macOS (x86_64 + aarch64); a **Homebrew tap** (`aaronbassett/homebrew-tap`) updated via a **least-privilege cross-owner PAT** (never logged); and **`crates.io` publish under the crate name `tome-mcp`** (the `tome` command name is preserved via `[[bin]]`, the crates.io name `tome` being permanently unavailable). Release workflows are subject to the same gates as CI (fmt, clippy `-D warnings`, `cargo-deny`, version-pinned actions) and build `--locked` against the committed `Cargo.lock`. The shipped binary carries an aggregated third-party-licence notice covering the cargo graph and the statically-linked/vendored native components. `cargo publish`, release tagging, and tap-PR merges remain deliberate maintainer actions.

## Amendment-log entry (proposed)

Add to `## Amendment log`:

> **v1.4.0 (2026-06-0X)** — Rewrote the Development-Workflow §Release tooling clause from a deferral to an authorisation of the public-beta release set (cargo-dist pipeline, prebuilt-binary distribution for Linux+macOS, a cross-owner-PAT Homebrew tap, crates.io publish under `tome-mcp`), subject to the existing CI gates + `--locked`. Driven by Phase 7 (beta hardening + public release); the crates.io name `tome` is permanently owned/yanked so the crate is renamed while the command stays `tome`. MINOR bump — materially expanded Development-Workflow guidance. The 24-hour cooling-off does not apply (Development Workflow is not a NON-NEGOTIABLE Core Principle per §Governance).

Bump the footer: `**Version**: 1.4.0 | **Ratified**: 2026-05-11 | **Last Amended**: 2026-06-0X`.

## What this amendment does NOT cover

- The **`rustix` transitive→direct promotion** and the **`tome`→`tome-mcp` crate rename** are governed by the **§Complexity budget** one-paragraph-justification rule, **not** by this amendment. Their justifications live in `plan.md` § Complexity Tracking (and the PRs that land them). Conflating them into the amendment would over-scope it.
- `CONSTITUTION.md` itself **stays tracked** in version control (FR-024 untracks process artifacts, never the governance document).

## Verification (SC US5 #3)

- The amendment is merged **before** the first cargo-dist PR (REL3).
- `CONSTITUTION.md` shows v1.4.0, the rewritten clause naming the authorised set, the rationale, the amendment-log entry, and the bumped `Last Amended` date.
- The PR body carries the brief rationale (§Governance amendment requirement (2)); green CI (§Governance (3)).

## Anti-requirements

- MUST NOT bump MAJOR (no principle removed/inverted).
- MUST NOT impose a cooling-off (not a NON-NEGOTIABLE clause).
- MUST NOT fold the rustix/rename complexity-budget notes into the amendment text.
