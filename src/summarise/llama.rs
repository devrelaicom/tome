//! `LlamaSummariser` — the production summariser implementation.
//!
//! F6 (this slice) ships a **skeleton only**. The constructor and
//! `summarise` trait method both return
//! [`SummariserFailureKind::BackendInitFailed`] with an explanatory
//! message so any call site that tries to invoke the summariser before
//! US4.a wires it up surfaces as a clearly-attributed failure rather
//! than a panic or a silent no-op.
//!
//! The reason the skeleton lives in F6 (and not in US4.a) is the
//! `Summariser` trait + module surface need to be referenced by other
//! Foundational and US-1/2 slices that wire summary regeneration into
//! `tome plugin enable / disable`, `tome catalog update`, and friends.
//! Those call sites must compile and test against the trait long
//! before the real inference path is ready.
//!
//! US4.a replaces every body below with the real load + decode + sample
//! pipeline described in `contracts/summariser.md` §"Inference invocation".

use std::path::PathBuf;

use llama_cpp_2::llama_backend::LlamaBackend;

use crate::error::{SummariserFailureKind, TomeError};
use crate::paths::Paths;

use super::{PluginSummariesInput, Summariser, SummariserOutput};

/// Production summariser. The `backend` is a static borrow of the
/// process-wide [`LlamaBackend`] singleton owned by
/// [`super::backend`]; the model + context themselves are constructed
/// inside `summarise` and dropped immediately afterwards (FR-421).
///
/// In F6 this type carries data but is never returned successfully —
/// `LlamaSummariser::new` always errors. US4.a is the first phase that
/// returns `Ok(LlamaSummariser { … })` from the constructor.
#[allow(dead_code)] // `backend` + `model_path` materialise in US4.a
pub struct LlamaSummariser {
    backend: &'static LlamaBackend,
    model_path: PathBuf,
}

impl LlamaSummariser {
    /// Construct a `LlamaSummariser` bound to the registry's summariser
    /// entry under `paths.models_dir`.
    ///
    /// **F6 behaviour**: always returns
    /// `Err(SummariserFailure { kind: BackendInitFailed { source } })`
    /// with a clear "production wiring lands in US4.a" message. This
    /// keeps the trait surface stable for call sites being written in
    /// parallel slices while making accidental reachability obvious.
    pub fn new(_paths: &Paths) -> Result<Self, TomeError> {
        Err(TomeError::SummariserFailure {
            kind: SummariserFailureKind::BackendInitFailed {
                source: "LlamaSummariser is a skeleton in F6; production wiring lands in US4.a"
                    .to_owned(),
            },
        })
    }
}

impl Summariser for LlamaSummariser {
    fn summarise(&self, _input: &PluginSummariesInput) -> Result<SummariserOutput, TomeError> {
        Err(TomeError::SummariserFailure {
            kind: SummariserFailureKind::BackendInitFailed {
                source: "LlamaSummariser::summarise is unimplemented in F6; production wiring lands in US4.a"
                    .to_owned(),
            },
        })
    }
}
