# Contract: Repository Hygiene & Credibility (US5)

**FRs**: FR-021, FR-022, FR-024, FR-025, FR-026 · **SCs**: SC-008, SC-009 · **Research**: §R-16/18/21

The polish that determines whether a curious visitor becomes a user and whether the project reads as trustworthy and governable.

---

## FR-021 — README rewritten as the front door

**Invariant**: the README MUST lead with **what Tome is (a CLI *and* MCP server)**, and state: build **prerequisites** (C/C++ toolchain + CMake + the build-time inference-runtime download), **supported platforms** (Linux + macOS; **Windows untested**), the **no-telemetry** guarantee, **accurate model licensing**, the **real install commands** (`cargo install tome-mcp`; `brew install …/tome`; `--path .` as fallback), **absolute** repository links, and a **worked example pointing at a real public catalog**. Every getting-started command MUST resolve against the published artifacts.

**Fixes folded in** (from the audits): line 3 "(and eventually an MCP server)" → "A Rust CLI **and** MCP server…" (DOC-05/HYG-6); Qwen2.5-0.5B relabelled **Apache-2.0** (not MIT) (LIC-002); repo-relative links → absolute `https://github.com/...` (PKG-3); a Privacy/Network no-telemetry note (ADD-4); the 404 worked example (`midnight/midnight-experts`) → a real public catalog or a `file://` fixture (B2/DOC-01).

**Test/SC**: SC-008 — every command in the getting-started section executes successfully against the published artifacts; the **automated CI check MAY use a `file://` local-catalog fixture**, decoupling it from the public availability of the catalog fork (spec Assumption). HYG-7: rebuild the local binary before smoke-testing so it reflects the release version.

---

## FR-022 — Security disclosure channel

**Invariant**: a `SECURITY.md` MUST be added, **private vulnerability reporting** enabled, and the placeholder contact (`security@example.invalid` in `CONTRIBUTING.md:66`) **removed**.

**Mechanism**: add `SECURITY.md` (incl. the FR-010 mechanical-vs-semantic trust framing — see `robustness-trust.md`); enabling GitHub private vulnerability reporting is an **operator action** (Settings → Code security). Replace the placeholder email.

**SC**: a working private-reporting channel exists; no placeholder address remains.

---

## FR-024 — Untrack internal process artifacts (THE FINAL STEP)

**Invariant**: `specs/`, `review/`, `retro/`, `.sdd/`, `CLAUDE.md`, and the two `*.local.json` dev-state files MUST be removed from version-control tracking going forward and **ignored**, with **local copies retained**; `CONSTITUTION.md` MUST **remain tracked**; the crate tarball MUST exclude the internal artifacts via an `include`/`exclude` allowlist while still shipping `vendor/sqlite-vec`.

**Mechanism** (§R-18): `git rm --cached` the listed paths + add them to `.gitignore`; add a `Cargo.toml include`/`exclude` allowlist (`src/**`, `build.rs`, `vendor/**`, `Cargo.toml`, `Cargo.lock`, `README.md`, `CHANGELOG.md`, `LICENSE-*`); re-run `cargo package --list` to confirm `vendor/sqlite-vec` + `build.rs` still ship. `git rm --cached` the two `.local.json` (one is already dirty in the tree).

**Sequencing (load-bearing)**: this lands **last**, in the US5 release slice, **after** every hardening PR that reviews against those tracked artifacts has merged — and after the phase retro + the final `CLAUDE.md` update. The SDD/closeout workflow reads these dirs **from the working tree**; untracking ≠ deletion, and local copies must remain so planning/execution keeps its working context.

**SC**: the internal dirs are no longer tracked, `CONSTITUTION.md` stays tracked, local copies retained, the tarball excludes the internal artifacts and still ships the vendored native source.

---

## FR-025 — Changelog + discovery metadata + docs.rs

**Invariant**: the changelog's `[Unreleased]` section MUST be moved to the **top**; the first public version MUST remain **`0.6.0`**; the crate MUST carry standard discovery metadata (`homepage`, `documentation`, `authors`); the repository MUST have a **description and topics** (operator action). The docs.rs build MUST be made to succeed despite the build-time inference-runtime download (`[package.metadata.docs.rs]` or feature-gating), **or** the `documentation` field MUST point at the repository.

**Mechanism** (§R-16): reorder `CHANGELOG.md`; add `authors`/`homepage`/`documentation` to `Cargo.toml`; add `[package.metadata.docs.rs]` (feature-gate the embedder off for docs) or set `documentation` → repo as the fallback; add a v0.6.0 row to `RELEASE-BINARY-SIZE.md`. `gh repo edit --description … --add-topic rust,cli,mcp,claude-code,ai,plugins` is the **operator action** (ADD-6).

---

## FR-026 — Deprecated checkout upgraded; release workflow gated

**Invariant**: `actions/checkout` MUST be upgraded at **all call sites** (three today: `ci.yml:22`, `security.yml:20`, `security.yml:38`) — **time-sensitive** (Node-20 forced to Node-24 ~2026-06-02). The cargo-dist-generated release workflow MUST be subject to the same gates as CI (fmt/clippy/`cargo-deny`, version-pinned actions including the upgraded checkout).

**Mechanism** (§R-21): bump `@v4 → @v5` at the three sites as an **early standalone trivial PR** (F3) so the deadline is met and every Phase 7 PR stays green; the release-workflow-gating half lands with REL3.

---

## Cross-cutting

- Operator-only actions (cannot be done by the orchestrator): enable GitHub private vulnerability reporting; set repo description + topics; provide the Homebrew-tap PAT; run `cargo publish` + tag + tap-PR merge.
- SC-009: the full quality-gate suite + `cargo-deny` pass on the CI matrix with no new warnings; release/CI builds `--locked`; binary under the cap.
- Decisions only the user makes (from RELEASE-READINESS §🧭): crate name (decided: `tome-mcp`), version (decided: 0.6.0), comfort with the already-public internal artifacts, distribution scope (crates.io + prebuilt binaries), GitHub licence display.
