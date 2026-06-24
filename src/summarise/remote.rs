//! `RemoteSummariser` — the BYOK/BYOM summariser (Phase 12 / US1).
//!
//! Mirrors [`crate::summarise::llama::LlamaSummariser`]'s two-pass structure
//! (SHORT then LONG, the LONG cascading from the SHORT output) but dispatches
//! each pass to a remote provider's chat endpoint instead of the bundled Qwen
//! model. The provider kind (`openai`/`anthropic`/`gemini`) fixes the wire
//! shape; the per-kind `chat` functions handle request shaping + response
//! extraction.
//!
//! ## Error mapping
//!
//! - A provider request failure ([`crate::provider::error::ProviderError`])
//!   maps once via `into_tome_error()` → [`TomeError::ProviderRequestFailed`]
//!   (exit 94).
//! - An EMPTY (after trim) short or long output is a content failure →
//!   [`TomeError::SummariserFailure`] `{ OutputEmpty }` (exit 24), the SAME
//!   class the bundled path raises. Empty content is NOT a 94 — the provider
//!   answered, the answer was just unusable.
//!
//! ## Input rendering SSOT
//!
//! The `{descriptions}` block is rendered by
//! [`crate::summarise::llama::format_input_descriptions`] — the SAME function
//! the bundled path uses — so the catalogue is rendered byte-identically
//! regardless of which summariser runs.

use crate::config::ProviderKind;
use crate::error::{ShortOrLong, SummariserFailureKind, TomeError};
use crate::provider::config::ResolvedProvider;
use crate::provider::error::ProviderError;
use crate::provider::{anthropic, gemini, openai};

use super::llama::format_input_descriptions;
use super::prompts::{SHORT_PROMPT, long_prompt};
use super::{PluginSummariesInput, Summariser, SummariserOutput};

/// A summariser backed by an external chat provider. Holds the resolved
/// connection; each `summarise` call makes two chat requests (short, long).
#[derive(Debug)]
pub struct RemoteSummariser {
    resolved: ResolvedProvider,
}

impl RemoteSummariser {
    /// Construct from a resolved provider connection (produced by
    /// [`crate::provider::config::resolve`] for the summariser capability).
    pub fn new(resolved: ResolvedProvider) -> Self {
        Self { resolved }
    }

    /// Dispatch one chat request to the kind-appropriate per-kind module. The
    /// summariser uses NO system prompt (the instruction is the whole user
    /// turn, mirroring how the bundled path feeds a single prompt).
    fn chat(&self, prompt: &str) -> Result<String, ProviderError> {
        match self.resolved.kind {
            ProviderKind::Openai => openai::chat(&self.resolved, None, prompt),
            ProviderKind::Anthropic => anthropic::chat(&self.resolved, None, prompt),
            ProviderKind::Gemini => gemini::chat(&self.resolved, None, prompt),
            // resolve() rejects voyage for the summariser capability (FR-005),
            // so this arm is unreachable through the supported config path. Fail
            // closed with a BadRequest rather than panic.
            ProviderKind::Voyage => Err(ProviderError::new(
                &self.resolved.name,
                crate::provider::error::ProviderErrorKind::BadRequest,
                false,
                "voyage is not a valid summariser provider kind",
            )),
        }
    }
}

impl Summariser for RemoteSummariser {
    fn summarise(
        &self,
        input: &PluginSummariesInput,
        long_max_chars: usize,
    ) -> Result<SummariserOutput, TomeError> {
        // SHORT pass — same prompt + same input rendering as the bundled path.
        let descriptions = format_input_descriptions(input);
        let short_prompt = SHORT_PROMPT.replace("{descriptions}", &descriptions);
        let short = self
            .chat(&short_prompt)
            .map_err(ProviderError::into_tome_error)?
            .trim()
            .to_owned();
        validate_non_empty(&short, ShortOrLong::Short)?;

        // LONG pass — cascades from the (trimmed) short output, with the
        // configured cap, exactly as the bundled path does.
        let long_prompt_str = long_prompt(long_max_chars).replace("{topics}", &short);
        let long = self
            .chat(&long_prompt_str)
            .map_err(ProviderError::into_tome_error)?
            .trim()
            .to_owned();
        validate_non_empty(&long, ShortOrLong::Long)?;

        Ok(SummariserOutput { short, long })
    }
}

/// Empty (after trim) output is a content failure → exit 24, NOT a provider
/// request failure (94). The provider answered; the answer is unusable.
fn validate_non_empty(text: &str, which: ShortOrLong) -> Result<(), TomeError> {
    if text.is_empty() {
        return Err(TomeError::SummariserFailure {
            kind: SummariserFailureKind::OutputEmpty { which },
        });
    }
    Ok(())
}
