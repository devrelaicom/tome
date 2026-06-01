# Feature Specification: Phase 7 — Beta Hardening and Public Release

**Feature Branch**: `007-phase-7-beta-release`
**Created**: 2026-06-01
**Status**: Draft
**Input**: User description: "Phase 7 — Beta hardening plus public release: beta-gate bug fixes, the fix-or-document trio, the cleanup bundle, mechanical symlink hardening via rustix, sync.rs decomposition, an in-process MCP test harness, plus the crates.io / cargo-dist / Homebrew release wrapper."

**Codebase Documentation**: See [.sdd/codebase/](../../.sdd/codebase/) for technical details. The eight codebase documents were refreshed at the Phase 6 closeout against the post-v0.6.0 tree and remain current for the start of Phase 7.

**Source**: There is no PRD for this phase (the planned roadmap ended at Phase 6). The authoritative inputs for WHAT are the two beta-readiness audits produced 2026-05-29/30 — `CODE-REVIEW.md` (line-level correctness/security review, 13 verified findings) and `RELEASE-READINESS.md` (public-MVP release audit) — together with the decisions taken in the planning session that disposed of every finding. Finding IDs below (e.g. `F-KNN`, `B1`, `M3`) trace to those documents.

## Overview

Phase 6 shipped Tome to a code-complete, internally-reviewed v0.6.0. Phase 7 does not add product surface; it takes that artifact across the line to a **public beta** an external developer can discover, install, and trust. The work splits cleanly into two halves that this phase deliberately treats as one release.

The first half is **hardening the existing surface**. A 31-agent code review found no blockers but eight real bugs in working features — most importantly that semantic search, the headline feature, can silently return zero rows once any workspace/searchable/catalog/plugin filter applies. None is a crash on normal input and none is a security exposure, but several would make a first-time user's experience look broken. Phase 7 fixes the five first-impression bugs, the three lower-frequency correctness bugs, and a bundle of cheap robustness and code-hygiene cleanups; closes the one class of denial-of-service (unbounded third-party reads); and finishes the long-deferred mechanical symlink hardening — now affordable because `rustix`, the capability primitive needed, is already in the dependency graph and can be promoted transitive→direct under the existing complexity-budget rule rather than pulling in a new top-level dependency. Tome's trust posture is made explicit and honest: it defends the *mechanical* boundary (no out-of-memory, no path traversal, no symlink escape, no file corruption) and tells the user plainly that it cannot vet a catalog's *content* — a plugin's instructions are executed by the user's agent, so "only add catalogs you trust" is, and remains, the rule.

The second half is the **release wrapper** the product has never had. The crate is unpublished and the name `tome` is already taken on crates.io, so Tome publishes under `tome-mcp` while keeping the command users type as `tome`. A release pipeline (cargo-dist) produces self-contained prebuilt binaries for Linux and macOS, pushes a Homebrew formula to a tap, publishes to crates.io, and attaches a third-party-license bundle. The constitution is amended first to permit release tooling, exactly as it requires. The README is rewritten as the project's front door, a security-disclosure channel is opened, internal process artifacts are removed from version control going forward, and the repository is given the metadata that makes it discoverable. The first public version stays 0.6.0 — honest about the history, conservative while a release-candidate dependency settles.

Two supporting investments make the hardening durable rather than one-off: the 1,737-line `harness/sync.rs` is decomposed into per-sink reconciler modules behind a thin orchestrator (landed first, as a behaviour-preserving refactor, so the harness fixes land in the clean structure), and an in-process MCP test harness is built so the MCP surface — the integration story external users exercise first — finally has end-to-end exit-code coverage.

What this phase explicitly does **not** do: add features or new harnesses; build the deferred retrieval-quality evaluation harness (only a one-time real-model recall check for the search fix is in scope); implement catalog discovery (that lives in the separate `tome-site` track); change the trust model from "trusted on enrol"; support Windows; or upgrade off the pinned release-candidate inference dependency. Items deferred with rationale are listed under Out of Scope.

## User Scenarios & Testing *(mandatory)*

### User Story 1 — A flawless first run (Priority: P1)

A developer installs Tome, adds a catalog (including over SSH or a tokenised URL), enables a plugin, searches their skills, and — after upgrading Tome — runs `doctor`. Nothing about this first experience looks broken: searches return the skills that match, even when a workspace or `--catalog`/`--plugin` filter narrows the corpus; private-repo catalogs added over SSH can be shown, updated, and removed; a catalog with an awkward name does not brick the workspace; user-invocable prompts never silently disappear; and `doctor` never crashes after an upgrade.

**Why this priority**: These are the bugs a brand-new beta user is most likely to hit in their first session, and the ones that most damage trust — a search that returns nothing reads as "this product is broken." They are the headline beta gate.

**Independent Test**: On a realistically-populated multi-workspace index, run searches whose nearest global vectors belong to other workspaces, non-searchable commands, or filtered-out catalogs, and confirm the matching entries are still returned up to the requested count. Add a catalog by a plain-SSH source and confirm `show`/`update`/`remove` all resolve it. Create a workspace from a catalog whose name contains a newline/control character and confirm the workspace remains operable. Enumerate MCP prompts for a plugin that produces a name collision across entry kinds and confirm no entry is dropped. Upgrade across a schema change and run read-only `doctor`; confirm it completes without taking the index lock and without aborting.

**Acceptance Scenarios**:

1. **Given** an index where at least `top_k` vectors nearer than a matching entry belong to other workspaces, non-searchable commands, or filtered-out catalogs/plugins, **When** the user runs `query` (with or without `--catalog`/`--plugin`/`--strict`) or the MCP `search_skills` tool, **Then** the result is `min(top_k, total matching entries)` — the matching entries are returned and the set is not truncated by the vector pre-filter, on a corpus large enough that a fixed-multiplier over-fetch would still miss them.
2. **Given** a catalog added from a source whose URL changes under credential scrubbing (plain SSH `git@host:owner/repo`, `ssh://`, or `https://user:token@…`), **When** the user runs `catalog show`, `catalog update`, or `catalog remove`, **Then** each command resolves the cached clone correctly, reuse is honoured, and no clone is orphaned on disk.
3. **Given** a catalog whose name contains a newline or other control character, **When** a workspace is created or re-emitted from it (including `workspace init --inherit-global`), **Then** the workspace settings remain parseable and every harness operation on that workspace succeeds.
4. **Given** a plugin that yields a prompt-name collision across entry kinds (e.g. a Command and a user-invocable Skill named `foo`, plus a Command `foo2`), **When** MCP prompts are listed and fetched, **Then** every user-invocable entry is present and resolvable and `doctor` reports the resolution truthfully.
5. **Given** a Tome upgrade that leaves the on-disk index at an older or newer schema, **When** the user runs `doctor` without `--fix`, **Then** `doctor` opens the index read-only, never runs a migration, never takes the advisory lock, and produces a (possibly degraded) report instead of aborting.
6. **Given** `doctor --fix` on a stale-schema index, **When** it runs, **Then** the lock-held migration is performed exactly as before.

---

### User Story 2 — Defended against hostile and awkward catalog content, and honest about the rest (Priority: P2)

A developer adds a catalog they did not author. Tome treats its contents as untrusted *input*: a multi-gigabyte or malformed manifest cannot exhaust memory; a symlink planted in a path component cannot redirect a Tome-managed write outside its target directory; OpenCode still receives Tome's rules even when paired with another AGENTS-based harness; and a concurrent `catalog remove --force` cannot strand a ghost-enabled plugin. At the same time, the project documents clearly that Tome cannot vet the *content* a catalog ships — its skills, commands, and agents are instructions the user's own agent will execute.

**Why this priority**: Tome's entire purpose is consuming third-party catalogs, so the read boundary must hold under hostile input and the security promise must be honest. These are real but lower-frequency than US1's first-run bugs, and the symlink work is a moderate refactor.

**Independent Test**: Feed an oversized `plugin.json` (and the sibling unbounded-read sites) through `enable`/`show`/`list`/`doctor` and confirm a bounded, named error rather than out-of-memory. Place a symlink as an intermediate directory component on a Tome-managed write path and confirm the write refuses rather than following it. Pair OpenCode with Codex/Gemini and confirm Tome's rules are delivered to OpenCode in a form it resolves. Run `catalog remove --force` racing a concurrent `plugin enable` and confirm no plugin is left enabled with its catalog enrolment deleted. Read the published security documentation and confirm it draws the mechanical-vs-semantic line and states "only add catalogs you trust."

**Acceptance Scenarios**:

1. **Given** a catalog whose `plugin.json` (or `tome-catalog.toml`, or any third-party file Tome reads) is larger than that read's per-class cap, **When** Tome reads it during any command, **Then** the read is bounded and fails with the dedicated parse/size error naming the file, never exhausting memory.
2. **Given** a Tome-managed write target whose path contains a symlinked intermediate directory component, **When** Tome writes a rules file, settings file, agent file, or guardrails file, **Then** the write does not traverse the symlink and is refused with the dedicated write-guard error; the final-node symlink refusal continues to hold.
3. **Given** OpenCode in an effective harness list that also contains an AGENTS-based harness, **When** sync reconciles the shared rules file, **Then** OpenCode receives Tome's rules in a form it can resolve (an inline body), not an unresolved include directive.
4. **Given** a `catalog remove --force` whose cascade input is computed, **When** a concurrent `plugin enable` serialises on the same advisory lock, **Then** the cascade is re-derived inside the lock so no plugin is left enabled with its catalog enrolment removed.
5. **Given** the public project documentation, **When** a user reads the security page, **Then** it enumerates the mechanical defences Tome provides and states explicitly that Tome cannot vet catalog content and that adding a catalog is trusting it.

---

### User Story 3 — Maintainable internals with the MCP surface under test (Priority: P3)

A contributor changes harness reconciliation or the MCP surface and is caught by tests rather than by users. The 1,737-line reconciliation file is decomposed into focused per-sink modules behind a thin orchestrator, so the harness fixes in US2 land in a structure a person can hold in their head; an in-process MCP test harness gives the server's error and exit-code paths real end-to-end coverage; and a bundle of small correctness/hygiene defects is cleared so the public repository does not read as half-finished.

**Why this priority**: These are quality investments, not user-visible features, but they de-risk the rest of the phase (the harness fixes ride on the decomposition; the MCP-prompt fix is verified through the harness) and remove the residue an external auditor would notice. They sequence early (the decomposition lands first) even though their user-facing priority is lower.

**Independent Test**: Confirm `harness/sync.rs` is replaced by per-sink reconciler modules behind a thin orchestrator with the existing idempotence, first-error-precedence, and wire-shape pin suites still green and unchanged in behaviour. Run the new in-process MCP test harness and confirm it exercises the previously-uncovered exit codes end-to-end, including a regression that drives the US1 prompt-collision fix through `prompts/list` + `prompts/get`. Confirm each cleanup-bundle defect has a test or a truthful message where applicable.

**Acceptance Scenarios**:

1. **Given** the reconciliation logic, **When** the decomposition lands, **Then** the three reconcile sinks live in separate per-sink modules behind a thin orchestrator, the fixed sink order and first-error precedence are preserved, and every pre-existing harness test passes without behavioural change (a pure structural refactor).
2. **Given** the MCP server, **When** the in-process test harness runs, **Then** the exit codes that previously lacked end-to-end CLI coverage are exercised against a real server instance, and the US1 prompt-collision fix is verified end-to-end through the prompt list/get path.
3. **Given** an intra-plugin duplicate `(kind, name)`, **When** the plugin is indexed, **Then** the duplicate is detected, a warning is emitted, and the reported count reflects rows actually written (no silent overwrite, no over-count).
4. **Given** a malformed `~/.tome/config.toml`, **When** Tome parses it, **Then** it fails with the specific manifest/TOML-parse error and exit code, not a generic internal error.
5. **Given** an off-spec non-array `hooks` event value or a meta row indicating corruption, **When** Tome encounters it, **Then** it fails closed / distinguishes the corruption case explicitly rather than silently coercing or mis-diagnosing.
6. **Given** the production source, **When** it is read by a contributor, **Then** there is no dead `reference_count` accessor, no stale doc-comment describing a shipped feature as a stub, and no internal spec/contract citation in user-facing `--help` text.

---

### User Story 4 — One command to install (Priority: P4)

A member of the public installs Tome with a single command — `brew install …/tome` on macOS or `cargo install tome-mcp` anywhere with a toolchain — and gets a self-contained `tome` binary with no separate runtime libraries to fetch. Prebuilt binaries exist for Linux and macOS so most users skip the heavy native build entirely.

**Why this priority**: This is the criterion that makes the beta *public* rather than source-only. It is independently demonstrable (install on a clean machine) and is the core of the distribution story, but it depends on the hardening being done first so what installs is worth installing.

**Independent Test**: From a clean machine, `cargo install tome-mcp` yields a working `tome` binary; on macOS, `brew install` from the tap yields the same. The downloaded binary runs without any sidecar inference library present (verified by inspecting its dynamic dependencies on each target). A tagged release produces per-platform archives with checksums, a Homebrew formula in the tap, a crates.io release, and an attached third-party-license file.

**Acceptance Scenarios**:

1. **Given** the published crate, **When** a user runs `cargo install tome-mcp`, **Then** a binary named `tome` is installed and runs.
2. **Given** the Homebrew tap, **When** a macOS user installs the formula, **Then** a `tome` binary is installed and runs.
3. **Given** a prebuilt release binary for any supported target (Linux and macOS, x86_64 and aarch64), **When** it is run on a clean machine of that platform, **Then** it executes without requiring a separately-installed inference runtime library (the binary is self-contained for distribution).
4. **Given** a release is cut, **When** the pipeline runs, **Then** it produces per-platform archives with checksums, updates the Homebrew tap, publishes to crates.io, and attaches an aggregated third-party-license document.
5. **Given** the binary-size budget, **When** the release binary is built, **Then** it remains under the documented cap.

---

### User Story 5 — A credible, discoverable open-source project (Priority: P5)

A developer who discovers Tome on GitHub or crates.io finds a README that explains what it is and how to install it (with prerequisites and supported platforms stated), a working way to report a vulnerability privately, an accurate changelog and licensing, and a repository whose tracked files are the project — not its internal process artifacts. The getting-started commands they copy actually work.

**Why this priority**: This is the polish that determines whether a curious visitor becomes a user and whether the project reads as trustworthy. It is the lowest user-facing priority because it does not affect whether Tome *works*, only whether it is *adopted* and *governable*.

**Independent Test**: Read the README top to bottom and run every command in its getting-started section end-to-end against the published artifacts (or a local `file://` catalog fixture) — each resolves. Confirm a security-disclosure channel exists and the placeholder address is gone. Confirm the changelog's unreleased section is correctly placed and licensing is accurate. Confirm internal process directories are no longer tracked going forward while the governance document remains tracked. Confirm the repository has a description and topics and the constitution permits the release tooling now in use.

**Acceptance Scenarios**:

1. **Given** the README, **When** a new user reads it, **Then** it leads with what Tome is (a CLI *and* MCP server), states build prerequisites and supported platforms (Linux + macOS; Windows untested), the no-telemetry guarantee, accurate model licensing, and the real install commands; every getting-started command resolves against the published artifacts and the worked example points at a real, public catalog.
2. **Given** the repository, **When** someone needs to report a vulnerability, **Then** a security policy and private reporting channel exist and no placeholder contact remains.
3. **Given** the adoption of release tooling, **When** the change lands, **Then** the constitution has first been amended (a MINOR bump → v1.4.0; no cooling-off, as the clause is not a NON-NEGOTIABLE principle) to permit it, naming the authorised set, with a recorded rationale.
4. **Given** the introduction of `rustix` as a direct dependency, **When** the change lands, **Then** it is accompanied by the complexity-budget justification and the dependency graph gains no *new* package (it is a transitive→direct promotion) and the binary-size and licence-allowlist gates still pass.
5. **Given** version control, **When** the artifact-removal change lands (in the final release slice), **Then** the internal process directories are no longer tracked, the governance document (`CONSTITUTION.md`) remains tracked, local copies are retained, and the crate tarball excludes the internal artifacts while still shipping the vendored native source.
6. **Given** the changelog and packaging metadata, **When** they are inspected, **Then** the unreleased section is at the top, the version is `0.6.0`, and the crate carries the standard discovery metadata.
7. **Given** time-sensitive CI maintenance, **When** the workflows are inspected, **Then** the deprecated checkout action has been upgraded at every call site.

---

### Edge Cases

- **Search over-fetch widening hits a ceiling.** When the bounded widen loop cannot reach `top_k` matches because the filtered corpus genuinely contains fewer than `top_k` matching entries, the result is the true (smaller) match set, not an error and not silent global-neighbourhood leakage.
- **A future-schema index under read-only `doctor`.** `doctor` reports the schema as too new and degrades that subsystem's report rather than aborting the whole run; `--fix` does not attempt a backward migration.
- **A catalog source whose raw and scrubbed URLs are identical** (plain `https://host/owner/repo`) continues to work unchanged after the cache-key fix (the common case is unaffected).
- **The Linux release binary links the system C++ runtime.** "Self-contained" means no application-specific sidecar (no `libonnxruntime`); linking the platform's own C/C++ runtime and libc is acceptable, but the build targets a portable baseline so the binary runs across mainstream distributions.
- **A symlink as an *intermediate* directory on a write path the operator created themselves.** The hardened open refuses to traverse it; because the operator owns the harness directories and plugin content never supplies path components, this is defence-in-depth, not a normal path.
- **The rustix spike fails.** If the full-path symlink-safe primitive turns out not to be reachable without a new package, FR-007 degrades to final-node `O_NOFOLLOW` + the documented trust-model mitigation; no new package is taken either way.
- **The crate name decision propagates.** Every install instruction, badge, and link that references the package name uses `tome-mcp`; everything that references the command uses `tome`.
- **A getting-started command that depends on the catalog fork.** The worked example resolves only once the referenced public catalog exists; the README/site launch is gated on that and on the crates.io/Homebrew release, while the automated check uses a `file://` fixture.

## Requirements *(mandatory)*

### Functional Requirements

**Correctness — beta gate (US1)**

- **FR-001**: Semantic search (CLI `query`, `--strict`, filters, and the MCP `search_skills` tool) MUST return exactly `min(top_k, total matching entries)` regardless of how many nearer vectors are excluded by workspace, `searchable`, `--catalog`, or `--plugin` filtering. The over-fetch MUST widen until either `top_k` post-filter matches are collected or the candidate set is exhausted; it MUST NOT under-fetch as the corpus grows. A regression test MUST place at least `top_k` nearer non-matching rows ahead of the match, on a corpus large enough that a naive fixed-multiplier over-fetch would still miss it.
- **FR-002**: Read-only `tome doctor` MUST open the index read-only, MUST NOT run schema migrations, MUST NOT take the advisory lock, and MUST degrade to a partial report (never abort) on a stale or future schema. `doctor --fix` MUST still perform the lock-held migration.
- **FR-003**: The catalog cache directory and reuse refcount MUST be keyed by the same (scrubbed) URL every reader resolves by, while cloning MUST still use the raw URL for authentication; sources whose URL changes under scrubbing (plain SSH, `ssh://`, tokenised HTTPS) MUST round-trip through `show`/`update`/`remove`/reuse without orphaning a clone.
- **FR-004**: MCP prompt names MUST be assigned against a single global taken-set so no user-invocable entry is dropped on collision across entry kinds; `doctor` MUST report the resolution truthfully.
- **FR-005**: Workspace settings MUST be emitted such that third-party catalog names containing newlines/control characters cannot produce unparsable settings; control characters MUST be rejected in catalog names at the manifest boundary.

**Robustness & honest trust posture (US2)**

- **FR-006**: Every read of a third-party file MUST be bounded by that read's existing per-class cap (e.g. `PLUGIN_MANIFEST_MAX` 256 KiB for manifests/frontmatter, `HARNESS_MCP_MAX` 1 MiB for settings/hooks) — not a single new cap — and fail with a named error rather than exhausting memory. The fix MUST cover the whole class: no unbounded `std::fs::read`/`read_to_string` on a third-party path, across at least `plugin/manifest.rs`, `catalog/manifest.rs`, `plugin/lifecycle.rs`, `plugin/components.rs`, and the `doctor` read surface.
- **FR-007**: Tome-managed writes (rules files, settings files, agent files, guardrails files) MUST NOT traverse a symlinked intermediate path component; the write MUST be refused with a dedicated write-guard error, in addition to the existing final-node symlink refusal. If the confirming spike shows the full-path primitive is unreachable without adding a *new* package, FR-007 falls back to final-node `O_NOFOLLOW` plus the documented trust-model mitigation (NFR-004 holds either way).
- **FR-008**: When OpenCode shares a rules file with an AGENTS-based harness, the shared body MUST be written in a form OpenCode can resolve — not an unresolved include directive — so Tome's rules reach every sharer; concretely, if any live sharer requires an inline body, the shared body is written inline (valid for include-capable harnesses too).
- **FR-009**: `catalog remove --force` MUST re-derive its cascade input inside the advisory-lock-held closure so a concurrent `plugin enable` cannot leave a ghost-enabled plugin.
- **FR-010**: The project MUST publish security documentation that distinguishes the mechanical boundary Tome defends from the semantic content it cannot vet, and states that adding a catalog is trusting it.

**Maintainability & test foundations (US3)**

- **FR-011**: `harness/sync.rs` MUST be decomposed into per-sink reconciler modules behind a thin orchestrator as a behaviour-preserving refactor, landed before the US2 harness fixes; the fixed sink order, first-error precedence, idempotence, and wire-shape pins MUST be unchanged.
- **FR-012**: An in-process MCP test harness MUST exist and MUST give end-to-end coverage to the MCP-internal exit codes that previously lacked it; it MUST verify the FR-004 prompt-collision fix end-to-end. (FR-004's *fix* may land first; FR-012 gates its *verification*, not its implementation.)
- **FR-013**: Intra-plugin duplicate `(kind, name)` MUST be detected and warned, and the "N indexed" message MUST count rows actually written.
- **FR-014**: A malformed `~/.tome/config.toml` MUST surface the specific TOML/manifest-parse error and exit code (reusing the existing `ManifestInvalid::TomlParse`, exit 5), not a generic internal error.
- **FR-015**: Off-spec inputs MUST fail closed rather than silently coerce: a non-array `hooks` event value MUST be rejected with the existing settings-write error (exit 44, `HookSettingsWriteFailed`), not coerced to `[]`; a meta row indicating corruption MUST be distinguished from a fresh database (a diagnostic distinction reusing existing variants — no new exit code).
- **FR-016**: Dead code (the unused `reference_count` accessor) MUST be removed and its misdirecting doc pointer corrected; stale doc-comments describing shipped features as stubs MUST be swept; and internal spec/contract citations (`FR-`/`NFR-`/`contracts/*.md`) MUST be stripped from user-facing `--help`/clap doc-comments (DOC-06).

**Public install (US4)**

- **FR-017**: The published crate MUST be `tome-mcp`; the installed binary and the Homebrew formula MUST be `tome` (`cargo install tome-mcp` and `brew install …/tome` both yield a `tome` command).
- **FR-018**: A release pipeline MUST produce self-contained prebuilt binaries for Linux and macOS (x86_64 and aarch64) that run on a clean machine of the target platform with no separately-installed inference runtime; each target's build MUST verify the absence of an application-specific sidecar library (e.g. `libonnxruntime`) by inspecting the binary's dynamic dependencies in CI.
- **FR-019**: A tagged release MUST produce per-platform archives with checksums, push a Homebrew formula to the `aaronbassett/homebrew-tap` (using a least-privilege credential with cross-owner write access), publish to crates.io, and attach an aggregated third-party-license document covering both the cargo dependency graph and the vendored/statically-linked native components (ONNX Runtime, llama.cpp, vendored sqlite-vec).
- **FR-020**: The release binary MUST remain under the documented binary-size cap.

**Credible release (US5)**

- **FR-021**: The README MUST be rewritten to lead with what Tome is (CLI *and* MCP server), state build prerequisites (C/C++ toolchain + CMake + the build-time inference-runtime download) and supported platforms (Linux + macOS; Windows untested), the no-telemetry guarantee, accurate model licensing, real install commands, absolute repository links, and a worked example pointing at a real public catalog; every getting-started command MUST resolve against the published artifacts.
- **FR-022**: A security policy MUST be added, private vulnerability reporting MUST be enabled, and the placeholder contact address MUST be removed.
- **FR-023**: The constitution MUST be amended to permit release tooling *before* that tooling is adopted — the amended clause naming the authorised set (a cargo-dist release pipeline, prebuilt-binary distribution, a Homebrew tap pushed via a cross-owner PAT, and crates.io publish under the renamed crate) — with a recorded rationale and a MINOR version bump (→ v1.4.0; the release-tooling clause lives under Development Workflow, not a NON-NEGOTIABLE principle, so no 24-hour cooling-off applies). The `rustix` promotion and the `tome`→`tome-mcp` crate rename MUST each carry a one-paragraph complexity-budget justification.
- **FR-024**: Internal process directories (`specs/`, `review/`, `retro/`, `.sdd/`, `CLAUDE.md`, and the two `*.local.json` dev-state files) MUST be removed from version-control tracking going forward and ignored, with local copies retained; `CONSTITUTION.md` MUST remain tracked; the crate tarball MUST exclude the internal artifacts via an include/exclude allowlist while still shipping `vendor/sqlite-vec`. This untracking MUST land in the final (US5) release slice, after the hardening PRs that review against those tracked artifacts have merged.
- **FR-025**: The changelog's unreleased section MUST be moved to the top; the first public version MUST remain `0.6.0`; the crate MUST carry standard discovery metadata (homepage, documentation, authors); the repository MUST have a description and topics (operator action). The docs.rs build MUST be made to succeed despite the build-time inference-runtime download (via `[package.metadata.docs.rs]` or feature-gating), or the `documentation` field MUST point at the repository.
- **FR-026**: The deprecated `actions/checkout` MUST be upgraded at all call sites (three today, across `ci.yml` and `security.yml`; time-sensitive). The cargo-dist-generated release workflow MUST be subject to the same gates as CI (fmt/clippy/`cargo-deny`, version-pinned actions including the upgraded checkout).

### Key Entities

- **rustix (dependency)**: a capability-based filesystem primitive, already present transitively with its `fs` feature enabled; promoted to a direct dependency to provide `openat`/`openat2`-based symlink-safe path resolution. No new package enters the graph; the licence stays on the allowlist.
- **Release pipeline (cargo-dist)**: the release-time tooling that builds, checksums, and distributes binaries + Homebrew formula + crates.io publish. Newly introduced; gated on the constitution amendment.
- **No schema change**: this phase introduces no SQLite schema migration and no new exit-code cluster (existing variants/codes are reused).

## Non-Functional Requirements

- **NFR-001**: No regression in the existing quality gates — `cargo fmt --check`, `cargo clippy --all-targets --all-features -D warnings`, `typos`, and the full test suite MUST pass on the CI matrix (Linux + macOS × stable + MSRV).
- **NFR-002**: The closed error set is preserved — no new `Other`/`Unknown` arm; every new failure path reuses an existing specific variant + exit code (no new exit codes this phase): e.g. config parse → `ManifestInvalid::TomlParse` (exit 5), non-array hooks → `HookSettingsWriteFailed` (exit 44).
- **NFR-003**: The sync-only-except-`src/mcp/` boundary, atomic-write discipline, credential scrubbing at every boundary, and the reconcile mass-delete safeguard MUST all be preserved by the refactor and fixes.
- **NFR-004**: The symlink hardening MUST add no *new package* to the dependency graph (`rustix` is a transitive→direct promotion; its `fs` feature is already enabled transitively). Any `rustix` feature or direct-version change MUST stay within the licence allowlist and the binary-size cap; a small size delta from compiling already-present code is acceptable, a new package is not.
- **NFR-005**: The `harness/sync.rs` decomposition MUST be strictly behaviour-preserving, evidenced by the unchanged idempotence/first-error/wire-pin suites.
- **NFR-006**: Tome MUST remain free of telemetry, analytics, and crash reporting; the only network egress remains the prompted, checksum-pinned model downloads and `git`/catalog fetches.
- **NFR-007**: Licensing MUST stay clean — `cargo-deny` (advisories, bans, licences, sources) green; the shipped binary MUST carry an aggregated third-party-licence notice covering the cargo graph *and* the statically-linked/vendored native components.
- **NFR-008**: Release and CI builds MUST be `--locked`; `Cargo.lock` is committed, authoritative, and shipped in the crate tarball, so a tagged artifact matches the audited dependency set and no transitive bump silently enters a release.
- **NFR-009**: The crate's declared MSRV (`rust-version`) MUST be verified green and left unchanged this phase.
- **NFR-010**: Release-pipeline workflows MUST be subject to the same gates as CI (fmt/clippy/`cargo-deny`, version-pinned third-party actions); the Homebrew-tap PAT MUST be least-privilege and never logged.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On a multi-workspace index where ≥`top_k` nearer vectors are filtered out, a query returns `min(top_k, total matches)` — the matching entry is present (0 → present) and the count does not shrink as the corpus grows; verified once against the real embedding models, not only the stub.
- **SC-002**: Running `doctor` immediately after an upgrade across a schema change completes successfully 100% of the time (no exit-73 abort, no unlocked migration).
- **SC-003**: A catalog added over plain SSH can be shown, updated, and removed with zero orphaned clones left on disk.
- **SC-004**: No user-invocable entry is missing from the MCP prompt list or unresolvable on get, including the command+skill+`foo2` collision case.
- **SC-005**: A hostile or malformed third-party manifest of arbitrary size produces a bounded named error in well under the memory headroom of a typical dev machine, never an out-of-memory crash, across every read site.
- **SC-006**: A symlinked intermediate directory component on a managed write path is refused 100% of the time across the supported platforms.
- **SC-007**: A clean Linux machine and a clean macOS machine can each install Tome with one command and run it with no additional runtime library install.
- **SC-008**: Every command in the README getting-started section executes successfully against the published artifacts (the automated CI check MAY use a `file://` local-catalog fixture, decoupling it from the public availability of the catalog fork).
- **SC-009**: The full quality-gate suite and `cargo-deny` pass on the CI matrix with no new warnings; the release binary stays under the size cap; release/CI builds are `--locked`.
- **SC-010**: The MCP exit codes previously lacking end-to-end coverage are exercised by the new harness (coverage gap closed).
- **SC-011**: The decomposed reconciler passes every pre-existing harness test with no behavioural diff.

## Assumptions

- The Midnight Expert catalog fork (`devrelaicom/midnight-expert-tome`) will be public by the time the README/worked-example and site launch, so getting-started commands resolve. The code-side spec does not depend on it; the README launch is gated on it, and the automated SC-008 check uses a `file://` fixture.
- `rustix` exposes the needed `openat`/`openat2` (Linux `RESOLVE_NO_SYMLINKS`) and a portable per-component `O_NOFOLLOW` path under (or one feature beyond) the feature set already enabled transitively; a short spike confirms this before the hardening lands. If it does not, FR-007's documented fallback applies (no new package either way).
- The macOS release binary is already self-contained (verified); the Linux binary is expected to be likewise after a glibc-baseline build, confirmed per-target in CI.
- `cargo-dist` can target Linux + macOS and push a cross-owner Homebrew tap given a least-privilege PAT secret; the operator provides the secret and performs the operator-only actions (repo description/topics, enabling private vulnerability reporting, providing the tap token).
- The local release binary MUST be rebuilt before tagging so smoke tests (SC-008) reflect the release version, not a stale build (HYG-7).

## Dependencies & Sequencing

- **Constitution amendment precedes release tooling** (CONSTITUTION.md release-tooling clause); the clause is not NON-NEGOTIABLE, so no cooling-off gates it.
- **`harness/sync.rs` decomposition precedes** the OpenCode rules fix (FR-008) and the symlink-guard consolidation (FR-007), so those land in the clean structure.
- **The symlink-guard consolidation (FR-007) MUST be applied across all per-sink reconcilers in one pass**, not sink-by-sink — the project has twice been bitten by an exit-code/policy fix applied to one sink and missed on its parallel; the decomposition makes one-pass discipline mandatory.
- **The in-process MCP harness precedes** end-to-end *verification* of the prompt-collision fix; FR-004's fix may land first (the harness gates verification, not implementation).
- The release wrapper (US4/US5) is the last slice and is gated on the hardening (US1–US3) being merged; FR-024's untracking is the final step.
- This phase is independent of the separate `tome-site` track (already specified and planned in the `tome-site` repository); the only coupling is launch timing and a single source of truth for install instructions.

## Out of Scope (deferred, with rationale)

- **Retrieval-quality evaluation harness** (golden-query suite / nightly real-model CI) — deferred; only the one-time real-model recall check for the search fix (SC-001) is in scope.
- **`doctor` Phase-5-surface refresh after `--fix` across a cross-major migration (F-DOCTOR-STALE-PHASE5)** — the partial finding; deferred past beta. FR-002 covers read-only-`doctor` crash-safety, not this surface-refresh sub-case.
- **Catalog discovery / gallery / registry** — owned by the `tome-site` track.
- **Hooks stale-removal on source eviction (TD-063)** — accepted for the beta and surfaced by `doctor`; revisiting the no-sidecar ownership model is out of scope.
- **MCP server live-reload of settings** — documented as a restart requirement; no live-reload this phase.
- **Exit-code allocation registry; reconciler efficiency (RUST-1/2)** — backlog; not hot.
- **Windows support** — explicitly unsupported and stated; the symlink hardening is Unix-centric.
- **Upgrading off the pinned release-candidate inference dependency** — accepted, pinned, for a 0.x beta.
- **New features, new harnesses, schema changes, new exit-code clusters** — none this phase.
