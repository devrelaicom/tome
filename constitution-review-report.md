# Constitution Compliance Review — Tome Phase 1 PRD

**Reviewed:** Phase 1 PRD (pre-implementation)
**Date:** 2026-05-11
**Constitution Version:** 1.0.0
**Scope:** PRD design validation — no Rust code exists yet

---

## Summary

| Severity | Count |
|----------|-------|
| CRITICAL | 0 |
| HIGH | 2 |
| MEDIUM | 4 |
| LOW | 3 |
| INFO (compliant) | 9 |

---

## HIGH Findings

### H-1 — Exit code 1 is an under-specified catch-all (Principle II — NON-NEGOTIABLE)

**Principle:** II — Predictable Exit Codes (NON-NEGOTIABLE)

The PRD's exit code table defines code `1` as "generic error." The constitution requires "a documented integer for every named failure class." A catch-all code is directly contrary to this: it will absorb network-unavailable, filesystem permission errors, unexpected panics, and other distinct conditions that callers cannot distinguish programmatically. Phase 2's `tome doctor` is explicitly noted as relying on predictable exit codes — a generic bucket undermines that from day one.

**Fix:** Either (a) add a note specifying exactly what conditions collapse into `1` (e.g. "internal/unexpected errors only; classified failures always use 3–6") and treat `1` as the true last-resort fallback, or (b) introduce additional named codes now (`7 — I/O / filesystem error` is an obvious candidate). The PRD should make the decision explicit.

---

### H-2 — Git stderr bubbling is specified; credential scrubbing is not (Principle XIII)

**Principle:** XIII — Never Log Secrets

The Git plumbing section says: "let git's stderr bubble up, prefixed with Tome context." Constitution XIII says: "when we surface upstream errors, scrub them." These are in direct conflict. Git's credential helpers, SSH agent negotiation, and HTTPS error paths can emit partial tokens, URLs with embedded credentials (`https://user:token@github.com/…`), or credential-helper names. The PRD gives implementors no signal that scrubbing is required.

**Fix:** Add an explicit scrubbing requirement to the Git plumbing section and to the `src/catalog/git.rs` module notes. At minimum: strip URL-embedded credentials (regex `https://[^@]+@`) before forwarding stderr. Note that `RUST_LOG`-exposed tracing spans (see L-3) share the same risk.

---

## MEDIUM Findings

### M-1 — `git reset --hard` on update is a destructive operation without documented opt-in status (Principle III)

**Principle:** III — Scriptable by Default

The update plumbing calls `git reset --hard origin/<ref>`, which discards any local state in the cache directory. Principle III requires destructive operations to have explicit opt-in in non-interactive contexts. The PRD is silent on whether this qualifies. In the normal operational model the cache is Tome-owned and manual edits would be unusual, but a developer testing a local manifest edit could be surprised.

**Fix:** Add one explicit statement: either "the cache directory is owned exclusively by Tome; local modifications are not preserved on update — this is intentional and does not require `--force`" (and is therefore not a destructive operation under Principle III), or require `--prune-local-changes` / `--force` if the cache may hold meaningful local state.

---

### M-2 — Partial-failure behaviour for `tome catalog update` (all catalogs) is unspecified (Principles II / V)

**Principles:** II — Predictable Exit Codes; V — Fail Fast, Fail Clear

`tome catalog update` with no `<name>` updates all registered catalogs. The PRD does not specify what happens when one of N catalogs fails (e.g. network error on catalog 2 of 3). Options:

- Abort immediately on first failure (Principle V — fail fast).
- Attempt all, report failures, exit non-zero if any failed.

Both are valid choices; the absence of a decision leaves exit code semantics ambiguous and CI pipelines unable to reason about partial updates.

**Fix:** Add a "failure behaviour" note to the `update` command description. Recommended: fail immediately (consistent with Principle V), exit `6`, and print which catalog failed and why.

---

### M-3 — `config.toml` strict-parse constraint is never stated (Principle IV)

**Principle:** IV — Strict Schemas, Helpful Errors

The PRD explicitly states that the catalog manifest rejects unknown top-level fields. The `config.toml` schema is shown in the on-disk layout section but no equivalent strictness constraint is mentioned. Principle IV applies to "all declarative input (catalog manifests, config files)" — the config file is not exempt.

**Fix:** Add one sentence to the config section: "`config.toml` is parsed strictly; unknown fields are rejected with an error that names the offending field."

---

### M-4 — `plugins[].source` path validation scope is unspecified (Principles IV / V)

**Principles:** IV — Strict Schemas; V — Fail Fast, Fail Clear

The manifest schema states `plugins[].source` MUST be a relative path within the catalog repo, and other source kinds are deferred. The PRD does not define what the parser does when it receives:

- An absolute path (`/etc/passwd`)
- A path-traversal sequence (`../../.ssh/id_rsa`)
- A URL (`https://evil.example.com/payload`)
- A Windows-style absolute path (`C:\…`)

At Phase 1, plugins cannot be installed so the immediate risk is limited. However, the manifest parser built here will be reused in Phase 2 when `tome install` lands. Leaving validation unspecified means either the parser is silent about these inputs now (violating IV/V) or it has to be revisited under time pressure in Phase 2.

**Fix:** Specify in the manifest constraints section: "The parser rejects any `source` value that is not a normalised relative path (no `..` components, no leading `/`, no URL scheme). The error names the field, the offending value, and the file."

---

## LOW Findings

### L-1 — Phase 2 preview names specific embedding libraries that may breach the 10 MB binary limit (Principle VI)

**Principle:** VI — KISS / YAGNI; Operational Constraint — Binary Size

The Phase 2 preview names `sqlite-vec` and `bge-small-en-v1.5 INT8`. A bundled embedding model will almost certainly push the stripped binary well over the 10 MB limit defined in the constitution. This is not a Phase 1 violation, but if these names carry forward unchanged into a Phase 2 PRD, they will need a written justification for the binary size exceedance.

**Recommendation:** Keep the Phase 2 preview vague at this stage. When drafting the Phase 2 PRD, include a binary-size and dependency justification for the embedding model as required by the Operational Constraints section.

---

### L-2 — SHA-pinned `--ref` behaviour on `tome catalog update` is undefined (Principles V / II)

**Principles:** V — Fail Fast, Fail Clear; II — Predictable Exit Codes

`tome catalog add` accepts `--ref <branch|tag|sha>`. If a user pins a commit SHA, `tome catalog update` will attempt `git reset --hard origin/<sha>` — but a SHA is not a valid remote ref name and this will produce a confusing Git error rather than a clear Tome message.

**Fix:** Document that `--ref <sha>` means "pinned; does not track updates." `tome catalog update` on a SHA-pinned catalog should either: (a) no-op with an informational message (`catalog is pinned to <sha>; use catalog add --ref to change`), or (b) exit with a clear error. Whichever is chosen, it should not pass a raw SHA as a remote ref to Git.

---

### L-3 — `tracing-subscriber` with `RUST_LOG=debug` may emit credential-bearing URLs (Principle XIII)

**Principle:** XIII — Never Log Secrets

`tracing` + `tracing-subscriber` are listed as dependencies. In their default configuration, setting `RUST_LOG=debug` emits all instrumented spans to stderr. If any span captures a raw Git URL or `std::process::Command` invocation that includes a credential-bearing URL (`https://user:token@host`), debug logging will leak credentials.

**Fix:** Add a note to the tracing/logging section: "Spans must use sanitised forms of Git URLs (host + path only, credentials stripped before instrumentation). Raw `Command` arguments that may contain auth material must not be captured in spans."

---

## Compliant Items

The following areas were reviewed and are fully compliant with the constitution.

| Area | Principle | Notes |
|------|-----------|-------|
| Module layout | VII | `catalog/`, `config.rs`, `paths.rs`, `error.rs`, `commands/` are capability-based; `thiserror` for library modules, `anyhow` for application code |
| KISS / YAGNI | VI | Non-goals list is explicit; no speculative abstractions in Phase 1 surface |
| Git shell-out | XII | `libgit2` explicitly rejected with rationale; `directories` used for XDG paths |
| Dependency set | VI / Constraints | All nine dependencies are justified; no `tokio` |
| Manifest strict parse | IV | "Reject unknown top-level fields" explicitly stated for the manifest |
| Scriptable by default | III | `--force` on `remove`, error-not-hang on non-TTY — both specified |
| Exit codes 0, 2–6 | II | Named codes map cleanly to distinct failure classes (see H-1 for code `1`) |
| Async constraint | Constraints | Synchronous only; `tokio` deferred to MCP server phase |
| Binary size | Constraints | `< 10 MB stripped` in success criteria |
| Dual licence | Constraints | `MIT OR Apache-2.0` in `Cargo.toml`, both files at root |
| CI matrix | X | `{macos-latest, ubuntu-latest} × {stable, MSRV}` + weekly security scans matches constitution exactly |

---

## Finding Index

| ID | Severity | Principle | Short description |
|----|----------|-----------|-------------------|
| H-1 | HIGH | II (NON-NEGOTIABLE) | Exit code 1 is an under-specified catch-all |
| H-2 | HIGH | XIII | Git stderr bubbling without credential scrubbing |
| M-1 | MEDIUM | III | `git reset --hard` on update — destructive opt-in status undocumented |
| M-2 | MEDIUM | II / V | Partial-failure behaviour for update-all is unspecified |
| M-3 | MEDIUM | IV | `config.toml` strict-parse constraint never stated |
| M-4 | MEDIUM | IV / V | `plugins[].source` path traversal / absolute path validation unspecified |
| L-1 | LOW | VI / Constraints | Phase 2 embedding model will likely breach 10 MB binary limit |
| L-2 | LOW | V / II | SHA-pinned `--ref` produces confusing Git error on update |
| L-3 | LOW | XIII | `RUST_LOG=debug` spans may capture credential-bearing Git URLs |

---

*Report generated by constitution:reviewer against CONSTITUTION.md v1.0.0.*
