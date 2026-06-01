# Tome Constitution

Tome is a Rust CLI (and, eventually, MCP server) that makes Claude Code's plugin ecosystem work across other agentic coding harnesses. This constitution governs how we build it. It is binding on all contributors and on every code change.

## Core Principles

### I. Unix Philosophy
Tome is a CLI. Every command does one thing, reads/writes plain text, and composes with other tools. Human-readable output is the default; `--json` produces structured output suitable for `jq` and scripts. Errors go to stderr; success output goes to stdout. Respect `NO_COLOR` and auto-disable colour when stdout is not a TTY. If a command's behaviour cannot be described in one sentence, it should probably be two commands.

### II. Predictable Exit Codes (NON-NEGOTIABLE)
Exit codes are part of the public contract. `0` for success, `2` for usage errors, and a documented integer for every named failure class (catalog not found, manifest invalid, Git failure, etc.). Once an exit code is documented and shipped, changing its meaning is a breaking change. New failure classes get new codes; we don't repurpose old ones.

### III. Scriptable by Default
Every interactive prompt has a non-interactive flag equivalent. When stdin is not a TTY and a command would prompt, it errors loudly with a clear message rather than hanging or silently auto-confirming. Destructive operations require explicit opt-in (`--force`) in non-interactive contexts. CI must be able to drive any Tome command without a human in the loop.

### IV. Strict Schemas, Helpful Errors
All declarative input (catalog manifests, config files, future plugin specs) is parsed strictly: unknown fields are rejected, not ignored. Every parse failure names the offending field, the file, and points at the expected schema. Strictness now makes evolution easier later — silent acceptance of typos becomes someone else's debugging problem.

### V. Fail Fast, Fail Clear
Errors crash early with full context. Every user-facing error names what failed, where, and — when possible — what to try next. "API connection failed: github.com returned 503. Try again or check status.github.com." is the bar. "ECONNREFUSED" is not. Bubble up upstream errors (e.g. Git's stderr) prefixed with Tome context rather than swallowing them. No silent failures, no spinners that pretend everything is fine.

### VI. KISS / YAGNI
Build the smallest thing that solves Phase N. Defer Phase N+1 until it has a concrete user. No speculative abstractions, no "we might need this later" hooks. The Rule of Three applies: don't generalise until the third real repetition. Boring, idiomatic Rust beats novel and clever.

### VII. Modular by Boundary
Modules are organised around capabilities (catalog, config, paths, error), not layers. Each module has an explicit public surface; cross-module access goes through that surface. No circular dependencies. Library-shaped modules use `thiserror` for typed errors; application code uses `anyhow`. Workspace splitting is deferred until there's enough code to justify the friction.

### VIII. Test What Matters
Integration tests cover every shipped CLI command against real fixtures (real Git repos, real TOML, real filesystems where practical). Unit tests cover parsers, error paths, and anything subtle enough that "it compiles" isn't proof. Trivial getters and pass-through code don't need tests. Mocks are a last resort and never for the things they hide (Git, the filesystem) — use real binaries and `tempfile`.

### IX. Conventional Commits
Every commit follows `type(scope): subject`. Enforced by `cocogitto` in the `commit-msg` hook. This is non-decorative: the format powers changelog generation and lets reviewers triage diffs by intent. Squash-and-merge PRs must still produce a conventional message.

### X. CI Gates Every Merge
No change lands on `main` without green CI: fmt, clippy `-D warnings`, build, and test on stable and MSRV, plus weekly security checks (`cargo-audit`, `cargo-deny`). Maintainers walk each PR diff end-to-end before merging — small batches, no rubber-stamping. Automation (Renovate, trivial bumps) follows the same rules: green CI and a deliberate merge, not an auto-merge.

### XI. Documentation Is Part of the Change
A change isn't done until its documentation is. README, command help text, and the changelog are updated in the same PR that changes the behaviour. Comments explain *why*, not *what*. The reader knows Rust; assume that.

### XII. Inherit, Don't Reimplement
Where the host system already does the job, shell out. Git is the canonical example: every dev machine has it, and `libgit2` is megabytes of binary bloat for capability we don't otherwise need. Same rule applies to credential management — we inherit whatever the user's `git` is configured for and never store, prompt for, or manage credentials ourselves.

### XIII. Never Log Secrets
Tokens, SSH keys, credential-helper output, and anything Git emits that looks credential-shaped never lands in Tome's logs, error messages, or `--json` output. When we surface upstream errors, scrub them.

## Operational Constraints

**Toolchain.** Stable Rust, edition pinned in `Cargo.toml`. MSRV declared and verified in CI. `rust-toolchain.toml` pins channel + `rustfmt` + `clippy`.

**Lints.** `cargo clippy --all-targets --all-features -- -D warnings` is enforced in pre-commit and CI. `cargo fmt --check` is enforced. `typos` runs in pre-commit. No `#[allow(clippy::...)]` without a comment explaining why.

**Dependencies.** Minimum viable set. Each new dependency justifies itself: what does it do that we couldn't do in a screen of code? `cargo-deny` enforces the licence allowlist (`MIT`, `Apache-2.0`, `MIT-0`, `BSD-{2,3}-Clause`, `ISC`, `Unicode-DFS-2016`, `Zlib`) and bans GPL-family licences. `cargo-audit` runs weekly. Renovate proposes updates; humans review them.

**Async.** Synchronous only until there's a concrete reason otherwise (the MCP server is the expected forcing function). Don't pull in `tokio` "in case".

**Binary size.** Release builds stay under 50 MB stripped. Adding a dependency that pushes us over this requires a written justification in the PR. The cap was 10 MB at ratification; Phase 2 linked ONNX Runtime (via `fastembed` → `ort`) which is intrinsically larger than the worst-case projection. The cap is now sized to current reality (~30 MB) with headroom for query, reindex, and the MCP server.

**Paths.** Tome-owned paths resolve under `<home>/.tome/`. The home directory is resolved via raw environment-variable inspection (`HOME` on Unix; the `std::env::home_dir` standard-library helper is an acceptable alternative since its un-deprecation in Rust 1.85). All Tome state lives under this root; the XDG-style separation of config / data / cache / state is deliberately collapsed into a single tree (rationale: simpler discovery, atomic backup/wipe, parallel evolution with the workspace and project-binding model introduced in Phase 4). Cache directories under the root are content-addressed (sha256 of source URL) to prevent collisions. The v1.2.0 §Paths wording referenced the `directories` crate; that wording was aspirational rather than implemented (Tome's Phase 1 / Phase 2 / Phase 3 code used raw env-var inspection throughout). The v1.3.0 amendment closes that documentation/code mismatch in addition to changing the on-disk layout.

**Licensing.** MIT OR Apache-2.0 dual licence. Both files committed at the repo root. New contributors implicitly licence their contributions under both per the standard Rust convention.

## Development Workflow

**Local setup.** Clone, run `git config core.hooksPath .githooks`, run `cargo build`. A new contributor should be able to submit a green PR within 10 minutes of `git clone`. If that ceases to be true, fix the setup, not the rule.

**Pre-commit.** `cargo fmt --check`, `typos`, `cargo clippy -D warnings`. Fast feedback before the commit lands. Implemented as a versioned shell script under `.githooks/pre-commit`; no external hooks manager (principle XII).

**Commit-msg.** `cog verify` enforces Conventional Commits.

**Pre-push.** `cargo test` runs the full suite before a branch is shared.

**CI.** Matrix `{macos-latest, ubuntu-latest} × {stable, MSRV}`. Required checks: fmt, clippy, build, test. Security workflow runs weekly + on PR: `cargo-audit` and `cargo-deny check`.

**Branching.** Trunk-based. Short-lived feature branches off `main`. Merge frequently.

**PRs.** Small batches. A PR that changes more than ~400 lines or touches more than two modules should be split unless there's a clear reason it can't be.

**Release tooling.** Tome distributes a public beta. The following release tooling is authorised: a **`cargo-dist`-driven release pipeline** producing per-platform archives with checksums; **prebuilt-binary distribution** for Linux + macOS (x86_64 + aarch64); a **Homebrew tap** (`aaronbassett/homebrew-tap`) updated via a **least-privilege cross-owner PAT** (never logged, per principle XIII); and **`crates.io` publish under the crate name `tome-mcp`** (the `tome` command name is preserved via `[[bin]]`, the crates.io name `tome` being permanently unavailable). Release workflows are subject to the same gates as CI (fmt, clippy `-D warnings`, `cargo-deny`, version-pinned actions) and build `--locked` against the committed `Cargo.lock`. The shipped artifacts carry an aggregated third-party-licence notice covering the cargo dependency graph and the statically-linked / vendored native components. `cargo publish`, release tagging, and tap-PR merges remain deliberate maintainer actions, never automated.

## Governance

This constitution supersedes ad-hoc practice. Where it conflicts with PRD detail, the constitution wins on *how* and the PRD wins on *what*.

**Compliance.** Every PR must be compatible with the constitution. Reviewers reject changes that violate principles without an accompanying amendment.

**Amendments.** Changes to this document require: (1) a PR that edits `CONSTITUTION.md`, (2) a brief rationale in the PR body, (3) green CI, (4) the `Last Amended` date bumped. Amendments to NON-NEGOTIABLE principles additionally require a 24-hour cooling-off period between drafting and merging — no same-session changes to bedrock rules.

**Versioning.** Semantic: MAJOR for removed/inverted principles, MINOR for new principles or materially expanded guidance, PATCH for clarifications and typo fixes.

**Complexity budget.** Any PR that introduces a new dependency, a new top-level module, or a new public CLI surface includes a one-paragraph justification. "It seemed nice" is not justification.

**Runtime guidance.** Day-to-day conventions (naming, error message tone, help-text style) are documented separately from this constitution. When that documentation and this constitution disagree, the constitution wins and the runtime guidance gets fixed.

## Amendment log

- **v1.4.0 (2026-06-01)** — Rewrote the Development-Workflow §Release tooling clause from a deferral to an authorisation of the public-beta release set (cargo-dist pipeline, prebuilt-binary distribution for Linux+macOS, a cross-owner-PAT Homebrew tap, crates.io publish under `tome-mcp`), subject to the existing CI gates + `--locked`. Driven by Phase 7 (beta hardening + public release); the crates.io name `tome` is permanently owned/yanked so the crate is renamed while the command stays `tome`. MINOR bump — materially expanded Development-Workflow guidance. The 24-hour cooling-off does not apply (Development Workflow is not a NON-NEGOTIABLE Core Principle per §Governance).

- **v1.3.0 (2026-05-22)** — Rewrote the `## Operational Constraints` §Paths block to drop the `directories` crate citation and consolidate Tome-owned paths under `<home>/.tome/`. Driven by Phase 4 spec §FR-300 / FR-302 / FR-303 (central-architecture refactor: single home root, single SQLite DB, named workspaces, project-binding pointers) and the v1.2.0 aspirational/implemented mismatch (no Phase 1/2/3 code ever called `directories::ProjectDirs`). MINOR bump — materially expanded guidance in an Operational Constraint. The 24-hour cooling-off period does not apply (Operational Constraints are not NON-NEGOTIABLE principles per §Governance).

**Version**: 1.4.0 | **Ratified**: 2026-05-11 | **Last Amended**: 2026-06-01
