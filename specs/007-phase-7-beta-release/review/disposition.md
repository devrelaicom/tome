# Phase 7 — phase-wide review disposition (T171)

Triage of `findings.md`. 0 blockers; both majors are APPLIED (fixed before the beta is declared done — the phase-wide pass's purpose). Minors are documented; the cheap doc-staleness ones are folded into the P9 closeout docs PR, the rest deferred with rationale.

## APPLIED (themed fix PRs, T173 — findings committed first, per discipline)

### PR-A — `fix(harness): agents cleanup-removal symlink refusal returns its dedicated code 45 (CON-1)` [MAJOR-1]
Branch `007-p7-p9-fix-agents-exit-code`. In `cleanup_all_owned_agents` (and any other orphan-cleanup removal of a Tome-owned agent file), run `crate::util::refuse_symlinked_component(&path)` and map a refusal to `TomeError::AgentTranslationFailed` (exit 45) **before** `rules_file::remove_standalone`, mirroring the live-removal path (agents.rs:318). Add an integration test in `tests/symlink_intermediate_guard.rs` driving the agents removal/cleanup path through a symlinked Tome-owned agent file and asserting exit **45** (closes the contract minor + the test minor too). Reuse closed-set variant; no new code/schema/dep.

### PR-B — `fix(models): bound + accurately document aux model-file downloads` [MAJOR-2 + security byte-cap minor]
Branch `007-p7-p9-fix-aux-verify-doc`. (1) Correct `SECURITY.md`: state precisely that the **primary** model artefact (.onnx / .gguf) is SHA-256-pinned-and-verified while the auxiliary tokenizer/config files are fetched from **compile-time-pinned URLs but not hash-verified** (and that catalog *content* is never vetted — keep the mechanical-vs-semantic line). (2) Add a **byte cap** to the aux stream loop in `download.rs::stream_url_to_partial` (a generous per-file ceiling, reusing the bounded-read discipline) so a compromised/MITM pinned host can't serve an unbounded sidecar; map an over-cap to an existing error. Keeps the no-new-variant invariant.

## FOLDED into the P9 closeout docs PR (T174-T176, cheap doc-staleness)
- `tests/search_knn_recall.rs` header/inline: `k=top_k*8` → reflect the shipped `*4` (`OVER_FETCH_MULTIPLIER`).
- `tests/prompt_collision_global.rs:16`: drop "the in-process MCP harness does not exist yet" (it does — `tests/common/mcp_harness.rs`).

## DEFERRED (documented, post-beta or out-of-scope)
- **FR-006 frontmatter over-cap test gap** — the frontmatter read IS bounded (frontmatter.rs:292, pre-Phase-7) and other sites are tested; adding a frontmatter-specific over-cap test is a nice-to-have, not a correctness gap. Candidate fast-follow.
- **`relative_path` two-impl divergence (sync.rs vs doctor)** — both correct in all tested cases; consolidating to one SSOT helper is a refactor, out of scope for beta (note for a future tidy; tracked alongside the P5 "3 union impls" note).
- **`FORCE_CONTEXT_BUILD_FAILURE` per-render atomic** — accepted in the T1 review (no-op in prod, one relaxed atomic load); not worth a hot-path change.
- **ENOTDIR→symlink-refusal category on a regular-file intermediate component** — benign (invalid path refused either way; matches the F2 errno-set design). Note only.
- **agents 45-mapping via the private writer vs the orchestrated path** — the PR-A integration test narrows this; full orchestrated-path coverage needs the DB/registry seam, partially addressed.

## Carry-forward (NOT Phase-7 review findings; recorded in retros for the handoff)
- **Scoped catalog-discovery commands read deprecated `config.toml [catalogs]`** (P8 retro) — `plugin list --catalog`/`plugin show`/`reindex <catalog>` exit-3 on an enrolled catalog. Medium-severity, workarounds exist; recommended fast-follow (config→DB discovery-path migration).
