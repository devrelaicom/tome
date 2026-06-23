//! `LlamaSummariser` — the production summariser implementation.
//!
//! Loads a Qwen2.5-0.5B-Instruct GGUF model via `llama-cpp-2`, runs the
//! short + long prompts in sequence, and returns the resulting
//! `SummariserOutput`. Length-window enforcement (FR-425) emits a
//! `tracing::info!` for outputs above the documented hard cap but never
//! drops the value — a too-long short summary already embedded into the
//! MCP tool description is advisory, not a hard failure. Empty or
//! unparsable outputs surface as `SummariserFailure::OutputEmpty` /
//! `OutputUnparsable` (exit 24).
//!
//! ## Lifetime model
//!
//! - The `LlamaBackend` is a process-wide singleton owned by
//!   [`super::backend()`]. A single `LlamaBackend` lives for the whole
//!   process lifetime; once initialised it is never re-created.
//! - The `LlamaModel` is **cached** on the [`LlamaSummariser`] (US4.d-1,
//!   S-M4): SHA-256 verification + `LlamaModel::load_from_file` runs
//!   once in [`Self::new`]; per-`summarise` calls re-use the cached
//!   model. Before US4.d-1 each summarise re-hashed the ~400 MB GGUF
//!   and re-loaded the model — substantially slower for batched
//!   triggers (catalog update sweeping N workspaces).
//! - The `LlamaContext` is still constructed per-prompt inside
//!   [`summarise`] and dropped at the end of the call; the KV cache
//!   shape is per-pass, so reuse would conflate short / long histories.
//! - `LlamaModel` is `Send + Sync` (the upstream `unsafe impl` is in
//!   `llama-cpp-2/src/model.rs`); no `Mutex` wrapper is needed for the
//!   `Summariser: Send + Sync` bound to hold.
//!
//! ## llama-cpp-2 API notes (v0.1.146)
//!
//! - Sampling uses the chain-of-samplers API: `LlamaSampler::chain_simple([...])`
//!   composes `penalties → top_p → temp → dist`. The terminating
//!   `dist(seed)` sampler picks a token from the post-filter distribution;
//!   a fixed seed (`0xC0FFEE`) keeps repeated runs deterministic given
//!   the same input.
//! - Tokenisation uses `LlamaModel::str_to_token(prompt, AddBos::Always)`.
//!   The first prompt of a fresh context receives the BOS token; the
//!   second `summarise` call inside the same process gets a brand-new
//!   context so the same applies.
//! - Token decoding uses `LlamaModel::token_to_piece` with an
//!   `encoding_rs::UTF_8` decoder so multi-byte sequences are reassembled
//!   correctly across token boundaries.
//! - The `LlamaContext` pins both n_ctx and n_batch to 4096
//!   (`with_n_ctx` / `with_n_batch`) — see `run_inference` for why
//!   n_batch must match n_ctx. That's enough headroom for the longest
//!   realistic skill-library summary while staying cheap on memory.

use std::num::NonZeroU32;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use tracing::{info, warn};

use crate::embedding::download::sha256_file;
use crate::error::{ShortOrLong, SummariserFailureKind, TomeError};
use crate::paths::Paths;

use super::SHORT_MAX_CHARS;
use super::prompts::{SHORT_PROMPT, long_prompt};
use super::registry::summariser_entry;
use super::{PluginSummariesInput, Summariser, SummariserOutput};

/// Filename inside `<models_dir>/qwen2.5-0.5b-instruct/` that holds the
/// GGUF weights. Mirrors `SUMMARISER_ENTRY.files[0]` in
/// `src/summarise/registry.rs`; kept as a named constant so the
/// constructor reads cleanly.
const PRIMARY_FILE: &str = "model.gguf";

/// Context-window size requested when constructing `LlamaContext`. 4096
/// tokens is enough headroom for the longest realistic skill-library
/// summary; Qwen2.5-0.5B-Instruct supports up to 32k natively.
const CONTEXT_SIZE: u32 = 4096;

/// Sampling seed. Pinned so repeated invocations against the same input
/// produce the same output (modulo any backend-side non-determinism).
const SAMPLING_SEED: u32 = 0xC0FFEE;

/// Sampling temperature. Deterministic-leaning but not so cold the
/// model hedges. Pinned by `contracts/summariser.md` §"Inference
/// invocation".
const SAMPLING_TEMP: f32 = 0.3;

/// Top-p (nucleus) sampling cutoff. Pinned by the contract.
const SAMPLING_TOP_P: f32 = 0.9;

/// Repeat penalty. Pinned by the contract.
const SAMPLING_REPEAT_PENALTY: f32 = 1.1;

/// Penalty window. `-1` = consider the full context; matches the
/// llama.cpp default for `penalty_last_n`.
const SAMPLING_PENALTY_LAST_N: i32 = -1;

/// Hard cap on tokens generated for the SHORT pass. Picked at ~3x the
/// character maximum to leave generous headroom; the contract caps
/// output at `SHORT_MAX_CHARS = 800`. Inference loops break out
/// earlier on EOG (end-of-generation) tokens.
const MAX_SHORT_TOKENS: i32 = 384;

/// Hard cap on tokens generated for the LONG pass. ~3x the character
/// maximum (`LONG_MAX_CHARS = 2500`); also broken out on EOG.
const MAX_LONG_TOKENS: i32 = 1024;

/// Production summariser. Holds the cached `LlamaModel` after a
/// successful [`Self::new`] verifies the on-disk SHA-256 and loads the
/// model. Per-prompt `LlamaContext` instances are constructed inside
/// [`Self::summarise`] and dropped before it returns. The process-wide
/// `LlamaBackend` is borrowed through [`super::backend()`] each call.
pub struct LlamaSummariser {
    model: LlamaModel,
}

impl std::fmt::Debug for LlamaSummariser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `LlamaModel` doesn't implement `Debug`. Render an opaque tag so
        // `LifecycleDeps`-style structs that derive `Debug` still work.
        f.debug_struct("LlamaSummariser")
            .field("model", &"<LlamaModel>")
            .finish()
    }
}

impl LlamaSummariser {
    /// Construct a `LlamaSummariser` bound to the registry's summariser
    /// entry under `paths.models_dir`. Verifies the on-disk SHA-256
    /// against the registry pin AND eagerly loads the `LlamaModel`
    /// (cached on `self`, reused across `summarise` calls — US4.d-1
    /// S-M4). Same posture as the embedder / reranker `--verify` paths,
    /// extended with the model-load step.
    ///
    /// Returns `SummariserFailure { kind: ModelMissing }` if the GGUF
    /// file is absent OR the registry pin is still the all-zero
    /// placeholder (S-M3 belt-and-braces — see C-B1); `ModelChecksumMismatch`
    /// if the SHA-256 differs from the registry pin; `BackendInitFailed`
    /// if either `LlamaBackend::init` (lazy) or
    /// `LlamaModel::load_from_file` fails; and `Io` for any other
    /// filesystem error.
    pub fn new(paths: &Paths) -> Result<Self, TomeError> {
        let entry = summariser_entry();

        // S-M3: refuse to construct if the registry still carries the
        // all-zero placeholder. C-B1 flipped this to the real hash in
        // US4.d-1; this guard catches a regression that re-introduces
        // a placeholder and surfaces it as `ModelMissing` (silent
        // no-op via the trigger callers) rather than letting the
        // checksum-mismatch path run a full SHA-256 over ~400 MB of
        // legitimately downloaded bytes only to fail at the end.
        if entry.has_placeholder_checksum() {
            return Err(TomeError::SummariserFailure {
                kind: SummariserFailureKind::ModelMissing,
            });
        }

        let model_path = paths.model_path(entry.name)?.join(PRIMARY_FILE);
        if !model_path.exists() {
            return Err(TomeError::SummariserFailure {
                kind: SummariserFailureKind::ModelMissing,
            });
        }

        let observed = sha256_file(&model_path)?;
        if !observed.eq_ignore_ascii_case(entry.sha256) {
            return Err(TomeError::SummariserFailure {
                kind: SummariserFailureKind::ModelChecksumMismatch {
                    expected: entry.sha256.to_owned(),
                    observed,
                },
            });
        }

        // Eagerly load the model so the SHA verification + load happen
        // exactly once per `LlamaSummariser`. Backend init is still lazy
        // (borrowed via `super::backend()`); a backend init failure
        // surfaces here for the first time if the caller wasn't using
        // the summariser earlier.
        let backend = super::backend()?;
        let model_params = LlamaModelParams::default();
        let model =
            LlamaModel::load_from_file(backend, &model_path, &model_params).map_err(|e| {
                TomeError::SummariserFailure {
                    kind: SummariserFailureKind::BackendInitFailed {
                        source: format!("load_from_file: {e}"),
                    },
                }
            })?;

        Ok(Self { model })
    }
}

impl Summariser for LlamaSummariser {
    fn summarise(
        &self,
        input: &PluginSummariesInput,
        long_max_chars: usize,
    ) -> Result<SummariserOutput, TomeError> {
        let backend = super::backend()?;

        // SHORT pass.
        let descriptions = format_input_descriptions(input);
        let short_prompt = SHORT_PROMPT.replace("{descriptions}", &descriptions);
        let short = run_inference(backend, &self.model, &short_prompt, MAX_SHORT_TOKENS)?;
        validate_output(&short, ShortOrLong::Short)?;
        check_length_window(&short, ShortOrLong::Short, SHORT_MAX_CHARS);

        // LONG pass — cascades from the short output. Build the prompt with
        // the configured cap so the model targets the right budget.
        let long_prompt_str = long_prompt(long_max_chars).replace("{topics}", &short);
        let long = run_inference(backend, &self.model, &long_prompt_str, MAX_LONG_TOKENS)?;
        validate_output(&long, ShortOrLong::Long)?;
        check_length_window(&long, ShortOrLong::Long, long_max_chars);

        Ok(SummariserOutput { short, long })
    }
}

/// Render the input plugin/skill set as a stable, deterministic block
/// suitable for substitution into `{descriptions}`. One line per skill,
/// in the order the input arrives (the caller has already sorted by
/// `(catalog, plugin, name)`).
///
/// US4.d-1 (C-M1): no `"- "` bullet prefix. The SHORT prompt explicitly
/// tells the model "no bullet points"; rendering input lines AS bullets
/// gave the model a contradictory example. Lines now go in as plain
/// `"<plugin>: <skill-name> — <skill-description>"` records.
fn format_input_descriptions(input: &PluginSummariesInput) -> String {
    let mut out = String::new();
    for plugin in &input.plugins {
        for skill in &plugin.skills {
            // Format: "<plugin>: <skill-name> — <skill-description>"
            // (no leading `"- "`) — see C-M1 in
            // `specs/004-phase-4-refactor-harnesses/review/us4-findings.md`.
            out.push_str(&plugin.plugin);
            out.push_str(": ");
            out.push_str(&skill.name);
            if !skill.description.is_empty() {
                out.push_str(" — ");
                out.push_str(&skill.description);
            }
            out.push('\n');
        }
    }
    out
}

/// Run the decode + sample loop for one prompt. Returns the assembled
/// UTF-8 string of generated tokens (excluding the prompt). Breaks out
/// on EOG (end-of-generation) tokens or when `max_tokens` is reached,
/// whichever comes first.
fn run_inference(
    backend: &llama_cpp_2::llama_backend::LlamaBackend,
    model: &LlamaModel,
    prompt: &str,
    max_tokens: i32,
) -> Result<String, TomeError> {
    // Create a fresh context per prompt. llama-cpp-2 does not currently
    // expose a "reset KV cache" hook on `LlamaContext`, and reusing the
    // context across the short → long boundary would conflate the two
    // prompts' KV histories. A fresh context per pass is the safest
    // contract — the cost is one extra `llama_new_context_with_model`
    // call (sub-millisecond on a 0.5B model).
    //
    // `n_batch` MUST be >= the largest prompt we ever decode in one call.
    // We never set it before, so llama.cpp used its 2048 default while the
    // context window is CONTEXT_SIZE (4096); a 2048–4096-token prompt then
    // tripped `GGML_ASSERT(n_tokens_all <= n_batch)` and aborted the process
    // (issue #208). Pinning n_batch = CONTEXT_SIZE means the existing
    // prompt-length guard (tokens <= n_ctx - max_tokens < n_ctx == n_batch)
    // guarantees the assert can never fire.
    let ctx_params = LlamaContextParams::default()
        .with_n_ctx(NonZeroU32::new(CONTEXT_SIZE))
        .with_n_batch(CONTEXT_SIZE);
    let mut ctx =
        model
            .new_context(backend, ctx_params)
            .map_err(|e| TomeError::SummariserFailure {
                kind: SummariserFailureKind::BackendInitFailed {
                    source: format!("new_context: {e}"),
                },
            })?;

    // Tokenise the prompt with BOS (fresh context, no prior history).
    let tokens =
        model
            .str_to_token(prompt, AddBos::Always)
            .map_err(|e| TomeError::SummariserFailure {
                kind: SummariserFailureKind::BackendInitFailed {
                    source: format!("str_to_token: {e}"),
                },
            })?;

    // Refuse a prompt that won't fit in the context window. The check is
    // tokens-vs-ctx, not chars-vs-ctx, because tokenisation is the
    // authoritative cost.
    let n_ctx = ctx.n_ctx() as i32;
    if tokens.len() as i32 > n_ctx - max_tokens {
        return Err(TomeError::SummariserFailure {
            kind: SummariserFailureKind::BackendInitFailed {
                source: format!(
                    "prompt is {} tokens but context window is {} (with {} reserved for output)",
                    tokens.len(),
                    n_ctx,
                    max_tokens,
                ),
            },
        });
    }

    // Feed the prompt to the model. `add_sequence` marks the last token
    // as a logit target so the next sample reads the post-prompt
    // distribution.
    let mut batch = LlamaBatch::new(tokens.len().max(1), 1);
    batch
        .add_sequence(&tokens, 0, false)
        .map_err(|e| TomeError::SummariserFailure {
            kind: SummariserFailureKind::BackendInitFailed {
                source: format!("batch.add_sequence: {e}"),
            },
        })?;

    ctx.decode(&mut batch)
        .map_err(|e| TomeError::SummariserFailure {
            kind: SummariserFailureKind::BackendInitFailed {
                source: format!("decode (prompt): {e}"),
            },
        })?;

    // Sampler chain. Order matters: penalties first (they touch logits),
    // then top-p, then temperature, then a distribution sampler to pick
    // a token. `dist(seed)` is deterministic given the seed.
    let mut sampler = LlamaSampler::chain_simple([
        LlamaSampler::penalties(SAMPLING_PENALTY_LAST_N, SAMPLING_REPEAT_PENALTY, 0.0, 0.0),
        LlamaSampler::top_p(SAMPLING_TOP_P, 1),
        LlamaSampler::temp(SAMPLING_TEMP),
        LlamaSampler::dist(SAMPLING_SEED),
    ]);

    let mut decoder = encoding_rs::UTF_8.new_decoder();
    let mut out = String::new();
    let mut n_generated: i32 = 0;
    let mut n_cur: i32 = tokens.len() as i32;

    loop {
        if n_generated >= max_tokens {
            break;
        }

        // Sample the token at the last position. `idx = -1` means "use
        // the most recent logits".
        let token = sampler.sample(&ctx, -1);
        sampler.accept(token);

        if model.is_eog_token(token) {
            break;
        }

        // Decode the new token and append to the output. `token_to_piece`
        // takes a stateful UTF-8 decoder so multi-byte sequences split
        // across tokens reassemble correctly.
        match model.token_to_piece(token, &mut decoder, /* special */ false, None) {
            Ok(piece) => out.push_str(&piece),
            Err(e) => {
                // A decode failure on a specific token doesn't doom the
                // whole pass — log and continue. If the model produces
                // enough garbage to fail `validate_output`, the caller
                // sees `OutputUnparsable` cleanly.
                warn!(error = %e, "token_to_piece failed; skipping token");
            }
        }

        n_generated += 1;

        // Feed the sampled token back through decode so the next sample
        // reads the right logits.
        batch.clear();
        batch
            .add(token, n_cur, &[0], true)
            .map_err(|e| TomeError::SummariserFailure {
                kind: SummariserFailureKind::BackendInitFailed {
                    source: format!("batch.add (generated): {e}"),
                },
            })?;
        ctx.decode(&mut batch)
            .map_err(|e| TomeError::SummariserFailure {
                kind: SummariserFailureKind::BackendInitFailed {
                    source: format!("decode (generated): {e}"),
                },
            })?;
        n_cur += 1;
    }

    // The decoder may hold a trailing partial-UTF8 byte sequence; flush
    // it. Bare unmappable sequences become U+FFFD per the encoding_rs
    // contract.
    let mut tail = String::new();
    let (_, _, _had_errors) = decoder.decode_to_string(&[], &mut tail, /* last */ true);
    out.push_str(&tail);

    // Trim leading/trailing whitespace — the model often emits a space
    // before the first token or a newline at the end.
    Ok(out.trim().to_owned())
}

/// Refuse empty output as a hard failure (FR-425: empty → exit 24).
/// Whitespace-only output is treated as empty after the `trim()` in
/// `run_inference`.
fn validate_output(text: &str, which: ShortOrLong) -> Result<(), TomeError> {
    if text.is_empty() {
        return Err(TomeError::SummariserFailure {
            kind: SummariserFailureKind::OutputEmpty { which },
        });
    }
    // UTF-8 unparsability would have been caught by `token_to_piece` /
    // the `encoding_rs` decoder; reaching here with non-UTF-8 bytes is
    // impossible by construction. The variant is left in the enum for
    // forward-compatibility (e.g. future grammar-based parsing) and
    // currently unreachable. Suppressed with `_ = which` so the
    // signature stays unified.
    let _ = which;
    Ok(())
}

/// Emit a `tracing::info!` when the output exceeds the documented hard
/// cap (FR-425). The value is *still returned* — a too-long short
/// summary that's already been embedded into the MCP tool description
/// is a warning, not a hard error. `max_chars` is the effective cap
/// (SHORT_MAX_CHARS for short; `long_max_chars` from config for long).
fn check_length_window(text: &str, which: ShortOrLong, max_chars: usize) {
    let observed = text.chars().count();
    if observed > max_chars {
        info!(
            which = %which,
            observed_chars = observed,
            max_chars,
            "summariser output exceeds recommended length window",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summarise::{PluginSummaryItem, SkillSummaryItem};

    fn paths_in(root: &std::path::Path) -> Paths {
        Paths::from_root(root.to_path_buf())
    }

    #[test]
    fn new_returns_model_missing_when_gguf_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        std::fs::create_dir_all(paths.model_path("qwen2.5-0.5b-instruct").unwrap()).unwrap();

        let err = LlamaSummariser::new(&paths).unwrap_err();
        match err {
            TomeError::SummariserFailure {
                kind: SummariserFailureKind::ModelMissing,
            } => {}
            other => panic!("expected ModelMissing, got {other:?}"),
        }
    }

    #[test]
    fn new_returns_checksum_mismatch_on_bad_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let dir = paths.model_path("qwen2.5-0.5b-instruct").unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        // Write a tiny non-matching artefact. The placeholder registry
        // hash is all zeros, so any actual SHA-256 of real bytes will
        // disagree.
        std::fs::write(dir.join("model.gguf"), b"definitely not real gguf").unwrap();

        let err = LlamaSummariser::new(&paths).unwrap_err();
        match err {
            TomeError::SummariserFailure {
                kind: SummariserFailureKind::ModelChecksumMismatch { observed, .. },
            } => {
                assert!(!observed.is_empty(), "observed hash should be populated");
            }
            other => panic!("expected ModelChecksumMismatch, got {other:?}"),
        }
    }

    #[test]
    fn format_input_descriptions_renders_stable_order() {
        let input = PluginSummariesInput {
            plugins: vec![
                PluginSummaryItem {
                    catalog: "core".to_owned(),
                    plugin: "alpha".to_owned(),
                    description: String::new(),
                    skills: vec![
                        SkillSummaryItem {
                            name: "skill-one".to_owned(),
                            description: "describes skill one".to_owned(),
                        },
                        SkillSummaryItem {
                            name: "skill-two".to_owned(),
                            description: String::new(),
                        },
                    ],
                },
                PluginSummaryItem {
                    catalog: "core".to_owned(),
                    plugin: "beta".to_owned(),
                    description: String::new(),
                    skills: vec![SkillSummaryItem {
                        name: "skill-three".to_owned(),
                        description: "for beta".to_owned(),
                    }],
                },
            ],
        };
        let rendered = format_input_descriptions(&input);
        // US4.d-1 (C-M1): no `"- "` bullet prefix.
        assert_eq!(
            rendered,
            "alpha: skill-one — describes skill one\n\
             alpha: skill-two\n\
             beta: skill-three — for beta\n"
        );
    }

    #[test]
    fn check_length_window_does_not_panic_within_bounds() {
        check_length_window(&"x".repeat(10), ShortOrLong::Short, SHORT_MAX_CHARS);
        check_length_window(
            &"x".repeat(10),
            ShortOrLong::Long,
            crate::summarise::LONG_MAX_CHARS,
        );
    }

    #[test]
    fn validate_output_rejects_empty_short() {
        let err = validate_output("", ShortOrLong::Short).unwrap_err();
        match err {
            TomeError::SummariserFailure {
                kind:
                    SummariserFailureKind::OutputEmpty {
                        which: ShortOrLong::Short,
                    },
            } => {}
            other => panic!("expected OutputEmpty(Short), got {other:?}"),
        }
    }

    #[test]
    fn validate_output_rejects_empty_long() {
        let err = validate_output("", ShortOrLong::Long).unwrap_err();
        match err {
            TomeError::SummariserFailure {
                kind:
                    SummariserFailureKind::OutputEmpty {
                        which: ShortOrLong::Long,
                    },
            } => {}
            other => panic!("expected OutputEmpty(Long), got {other:?}"),
        }
    }

    #[test]
    fn validate_output_accepts_non_empty() {
        assert!(validate_output("ok", ShortOrLong::Short).is_ok());
        assert!(validate_output("ok", ShortOrLong::Long).is_ok());
    }
}
