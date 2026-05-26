# US4 — Disposition

Maps findings in `us4-findings.md` to actions.

## Blockers — all applied in US4.d-1

| # | Disposition |
|---|---|
| C-B1 | Apply: record real SHA-256 for Qwen2.5-0.5B-Instruct GGUF. Steps: download the model file from the registry URL via `embedding::download::download_model` (or curl); compute SHA-256 via `embedding::download::sha256_file` or `shasum -a 256`; update `MODEL_REGISTRY` entry in both `src/embedding/registry.rs` AND `src/summarise/registry.rs` (or unify into one source). Add a regression test that asserts the placeholder hash is NOT in the registry. |
| C-B2 | Apply: grep + scrub stale `exit 20` references. Fix sites: `src/error.rs:501` (docstring), `src/workspace/regen_summary.rs:16,118` (docstrings), `contracts/summariser.md:93-94,172` (contract amendment). All should say exit 24. |
| C-B3 / R-B1 | Apply: collapse two sources to one. Move `LONG_MAX_CHARS` constant to a single location (likely `src/summarise/mod.rs` or `src/summarise/prompts.rs`); export and consume from both `llama.rs` and `regen_summary.rs`. Set the unified value to 2500 per contract. Verify only ONE warn fires for >2500 outputs (`tracing_test` or stub-based capture). |
| T-B1 | Apply: rewrite at least one trigger test to use the production code path. Option A: add tests that invoke `commands::plugin::enable::run` (CLI binary path via assert_cmd) with `SummariserOverrideGuard` installing a deterministic test summariser; assert that the override summariser was invoked. Option B: refactor `regenerate_for_trigger` to expose its outcome on a return value the test can introspect WITHOUT calling `_with_summariser` directly. Pick A — closer to production semantics; tests `tests/summariser_triggers_end_to_end.rs` (new file). |

## Majors — applied in US4.d-1 (selected)

| # | Disposition |
|---|---|
| C-M1 | Apply: change `format_input_descriptions` to NOT add `"- "` bullet prefix (or escape it explicitly). Match the contract's prompt rendering. |
| C-M2 | Apply (documentation): add explicit doc-comment + contract clarification that `ModelMissing` is a silent no-op in trigger callers per pragmatic carve-out (matches `ensure_models_present` posture). Amend `contracts/summariser.md` to document the carve-out so future readers don't see it as a contract violation. |
| R-M2 | Apply: replace `.expect` on `MODEL_REGISTRY.iter().find(...)` with `summariser_entry()` helper that returns `Result<&Entry, TomeError>`. |
| R-M6 | Apply: `tome catalog remove --force` cascade now calls `regenerate_for_trigger` for each affected workspace after the cascade-disable commits. Mirror the pattern from `plugin disable`. |
| R-M7 | Apply: in `backend()`, swap `Mutex::lock` + `?` panic-on-poison to `into_inner()` recovery. Document the rationale. |
| S-M3 | Apply: `LlamaSummariser::new` checks for the all-zero placeholder SHA-256 explicitly and returns `ModelMissing` with a "registry placeholder; download not configured yet" message. Defensive belt-and-braces against C-B1 regressions. |
| S-M4 | Apply: cache the loaded `LlamaModel` in `LlamaSummariser` so per-trigger SHA-256 re-hash is skipped after the first verify. Hash once at `new()`; subsequent `summarise()` calls reuse the cached model. |
| T-M2 | Apply: add `tests/summariser_triggers.rs::model_missing_trigger_is_silent_noop` test verifying the carve-out. |
| T-M5 | Apply: use `tracing_test = "0.3"` (or similar) to capture warn output. If the dep is too heavy, deflate to a per-test in-memory `tracing_subscriber::registry` adapter. |

## Majors — deferred to follow-up issue

| # | Reason for deferral |
|---|---|
| C-M3, C-M4 | Various contract drift items per `/tmp/tome-review-us4-contract.md` — cosmetic. |
| R-M1 | Server `tool_router.map` private-field reach — defer to a rmcp upgrade pass. |
| R-M3 | `as i32` cast pattern silent truncation — defensive concern. |
| R-M4 | `wait_for_shutdown_signal` swallowed errors — small attack surface. |
| R-M5 | Various rust cosmetic. |
| R-M8 | `=0.1.146` exact-pin upgrade-review cadence — process concern. |
| T-M1, T-M3, T-M4 | Test gaps — pair with T-B1 fix where they overlap, defer the rest. |
| S-M1 | Prompt-injection trust boundary — documentation only. |
| S-M2 | Per-description length cap before prompt interpolation — pairs with S2-M3 deferral. |

## Net effect of US4.d-1

- 5 production-source touches (`embedding/registry.rs`, `summarise/{registry,llama,trigger}.rs`, `commands/catalog/remove.rs`)
- 2 contract amendments (`contracts/summariser.md` for exit-24 + ModelMissing carve-out)
- 3-5 new tests (T-B1 end-to-end trigger test, T-M2 ModelMissing carve-out test, regression for C-B1 SHA pinning)
- 2 review artefacts (this + findings.md)

US4.d-2 runs `/sdd:map incremental` + retro/CLAUDE.md updates.

Note on C-B1: this requires actually downloading the Qwen GGUF file once to compute the real SHA-256. The implementing agent should either (a) document the steps to compute the hash and leave a clear TODO, OR (b) compute it during the fix (requires ~400 MB download). The agent should pick (a) if the network is unreliable in the dispatch environment.
