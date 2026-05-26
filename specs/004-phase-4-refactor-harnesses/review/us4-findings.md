# US4 — Pre-Closeout Review Findings

Four reviewers dispatched in parallel against the merged US4 surface (PRs #94–#96).

Per-reviewer source files in `/tmp/tome-review-us4-{contract,rust,test,security}.md`.

Counts: **5 blockers, 21 majors, ~30 minors+nits.**

Triage in `us4-disposition.md`.

---

## Blockers (5)

| # | Source | One-line | File / cite |
|---|---|---|---|
| **C-B1** | contract | Summariser registry ships the all-zero placeholder SHA-256. `tome models download` exits 31 (ModelCorrupt) at summariser entry; `tome workspace regen-summary` always exits 24 with ModelMissing. **Entire US4 user-visible surface non-functional in production.** | `src/embedding/registry.rs:98`, `src/summarise/registry.rs:50` |
| **C-B2** | contract | Stale "exit 20" references in code + contract docs. Runtime is correct (24) but every doc surface lies, surfacing as reader-traps. | `src/error.rs:501`, `src/workspace/regen_summary.rs:16,118`, `contracts/summariser.md:93-94,172` |
| **C-B3 / R-B1** | contract + rust | `LONG_MAX_CHARS` defined as 2400 in `src/summarise/prompts.rs:73` AND 2500 in `src/workspace/regen_summary.rs:59`. Long summaries in 2401–2500 window warn ONCE; 2501+ warn TWICE. Contract says one warn at 2500. Two sources of truth. | `src/summarise/prompts.rs:73` vs `src/workspace/regen_summary.rs:59` |
| **T-B1** | test | `SummariserOverrideGuard` is dead code — never instantiated by any test. None of the 4 production trigger sites have end-to-end test coverage proving they invoke the summariser after their `workspace_skills` mutation commits. Every test calls `regenerate_for_trigger_with_summariser` directly, bypassing the production code path. | `src/summarise/trigger.rs:68-85`; 4 trigger callers untested for the SummariserOverrideGuard path |

## Majors (21)

### Contract audit (4)

| # | One-line | File ref |
|---|---|---|
| C-M1 | `format_input_descriptions` adds `"- "` bullet prefix the contract's prompt explicitly tells the model to ignore. | `src/summarise/llama.rs` |
| C-M2 | Trigger silently converts `ModelMissing` to `Ok(())` without contract authority. Pragmatic carve-out from US4.b — but no contract sentence supports it. | `src/summarise/trigger.rs:122-135` |
| C-M3 | (Various contract drift in length-window code path duplications) | multi-site |
| C-M4 | (Various — see full report) | per `/tmp/tome-review-us4-contract.md` |

### Rust-lens (8)

| # | One-line | File ref |
|---|---|---|
| R-M1 | `Server::override_search_skills_description` reaches into rmcp's `tool_router.map` private-ish field with silent fallthrough — rmcp rename → server advertises scaffold with no warning. | `src/mcp/server.rs` |
| R-M2 | `LlamaSummariser::new` uses `.expect` on `MODEL_REGISTRY.iter().find(...)` instead of helper — registry edit panics with exit 101 rather than `TomeError`. | `src/summarise/llama.rs` |
| R-M3 | `as i32` cast pattern silent on truncation. | `src/summarise/llama.rs:246-247` |
| R-M4 | `wait_for_shutdown_signal` swallows `ctrl_c()` registration errors via `.ok()` — failure returns `"SIGINT"` immediately, spurious graceful-shutdown-on-startup. | (file ref in source) |
| R-M5 | (additional rust findings) | per `/tmp/tome-review-us4-rust.md` |
| R-M6 | `tome catalog remove --force` cascade-disables enabled plugins but never calls `regenerate_for_trigger`. Workspace's cached summary still mentions removed skills. Every OTHER `workspace_skills` mutation site fires the trigger; this one gap. | `src/commands/catalog/remove.rs` |
| R-M7 | Mutex poison in `backend()` is a one-way door bricking the process; recovering with `into_inner()` is safer. | `src/summarise/mod.rs` |
| R-M8 | `=0.1.146` exact-pin defensible but undertested (no upgrade-review cadence, no tokenizer-stability fixture). | `Cargo.toml` |

### Test audit (5)

| # | One-line | File ref |
|---|---|---|
| T-M1 | `reindex_with_unchanged_hashes_can_be_gated_to_zero_calls` tests `any_changes()` predicate, not the NEGATIVE trigger counter-test asked for. | `tests/summariser_triggers.rs` |
| T-M2 | Trigger silently swallows `ModelMissing` (production behaviour by design) — no test pins this contract. | "no test for ModelMissing silent-noop" |
| T-M3 | `cross_workspace_triggers_count_independently` claims FR-365 coverage but never exercises `commands::catalog::update::run`. | `tests/summariser_triggers.rs` |
| T-M4 | `summariser_real.rs` mixes `stub_summariser_seed()` identity with a real `LlamaSummariser`. | `tests/summariser_real.rs` |
| T-M5 | MCP `oversized_description` test acknowledges it can't verify `tracing::warn!` actually fired. | `tests/mcp_tool_description.rs` |

### Security audit (4)

| # | One-line | File ref |
|---|---|---|
| S-M1 | Prompt-injection trust boundary real but acceptable per "user authored which plugins are enabled" — needs in-code documentation. | `src/summarise/llama.rs` |
| S-M2 | No per-description length cap before prompt interpolation (S2-M3 carry-over). | `src/summarise/llama.rs` |
| S-M3 | `LlamaSummariser::new` doesn't short-circuit on all-zero placeholder hash (download path does). Belt-and-braces. | `src/summarise/llama.rs` |
| S-M4 | Per-trigger SHA-256 re-hash of 400 MB GGUF (DoS + small TOCTOU window). Performance concern; clean fix: carry the model in the struct. | `src/summarise/llama.rs` |

---

## Verdict

C-B1 is the show-stopper: placeholder SHA-256 means the production path is non-functional. Must be fixed in US4.d-1. C-B2 (exit-20 doc drift) and C-B3/R-B1 (length-window double source) are reader-traps that block reviewer confidence. T-B1 (SummariserOverrideGuard dead code) means the 4 production trigger sites have no end-to-end coverage. Plan: US4.d-1 ships 4 blockers + selected majors; US4.d-2 closeout (sdd:map + retro + CLAUDE.md).

Note: B1 + B3/R-B1 + T-B1 are independent root causes. C-B2 is documentation-only.
