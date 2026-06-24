//! The structured `ProviderError`, mapped once onto the closed `TomeError` set.
//!
//! Every remote-provider failure is funnelled through exactly one structured
//! value ([`ProviderError`]) before it crosses back into the closed
//! [`TomeError`] enum. That single mapping point (FR-013a / FR-014a) guarantees
//! three things:
//!
//! 1. **Credentials never survive.** The constructor runs the raw detail through
//!    the ONE shared scrubber ([`crate::catalog::git::scrub_to_string`]) before
//!    storing it, so a key reflected in a response body can never reach a log,
//!    an error chain, or the `--json` envelope.
//! 2. **`kind` + `retryable` are surfaced.** [`Display`](std::fmt::Display)
//!    embeds both, and [`ProviderError::into_tome_error`] uses that Display
//!    output as the `detail` carrier — so the human message AND the `--json`
//!    error envelope (which serialises the `TomeError` message) both carry them.
//! 3. **The exit code is determined once.** `EmbeddingInvalid` →
//!    [`TomeError::RemoteEmbeddingInvalid`] (95); every other kind →
//!    [`TomeError::ProviderRequestFailed`] (94).

use crate::error::TomeError;

/// The closed taxonomy of remote-provider failure classes. Maps to two exit
/// codes via [`ProviderError::into_tome_error`]: `EmbeddingInvalid` → 95, all
/// others → 94. `as_str` is the wire-stable lowercase token surfaced in
/// messages and the `--json` envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorKind {
    /// 401/403 — bad or missing credential. Not retryable.
    Auth,
    /// 429 — rate limited after exhausting retries. Retryable.
    RateLimited,
    /// 404 / unknown-model. Not retryable.
    ModelNotFound,
    /// Other 4xx (e.g. context-length, malformed request). Not retryable.
    BadRequest,
    /// Transport timeout after exhausting retries. Retryable.
    Timeout,
    /// Connect failure / 5xx after exhausting retries. Retryable.
    Unreachable,
    /// A 2xx body that could not be parsed into the expected shape (or a
    /// 200-with-error-envelope the per-kind module detected). Not retryable.
    MalformedResponse,
    /// A remote embedding failed content validation (empty / non-finite /
    /// wrong dimension / wrong count). Routes to exit 95, not 94.
    EmbeddingInvalid,
}

impl ProviderErrorKind {
    /// The stable lowercase token for this kind — surfaced in messages and the
    /// `--json` envelope. Keep in sync with the contract's failure-mapping table.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderErrorKind::Auth => "auth",
            ProviderErrorKind::RateLimited => "rate_limited",
            ProviderErrorKind::ModelNotFound => "model_not_found",
            ProviderErrorKind::BadRequest => "bad_request",
            ProviderErrorKind::Timeout => "timeout",
            ProviderErrorKind::Unreachable => "unreachable",
            ProviderErrorKind::MalformedResponse => "malformed_response",
            ProviderErrorKind::EmbeddingInvalid => "embedding_invalid",
        }
    }
}

impl std::fmt::Display for ProviderErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A structured, already-scrubbed remote-provider failure.
///
/// `redacted_detail` is guaranteed safe to log/print/serialise: the only
/// constructor ([`ProviderError::new`]) runs its `raw_detail` argument through
/// the shared credential scrubber before storing it. Construct via `new`; never
/// build the struct field-by-field with an unscrubbed detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    /// The registry name of the provider that failed (e.g. `"myprov"`).
    pub provider: String,
    /// The failure class — drives the exit code and the retry decision.
    pub kind: ProviderErrorKind,
    /// Whether the failure was retryable (the retry loop already exhausted its
    /// attempts; this records the *class* for the operator).
    pub retryable: bool,
    /// The human-readable detail, ALREADY scrubbed of any credential. Safe to
    /// log at any level, print, or serialise.
    pub redacted_detail: String,
}

impl ProviderError {
    /// Build a `ProviderError`, scrubbing `raw_detail` through the shared
    /// credential scrubber ([`crate::catalog::git::scrub_to_string`]) so a key
    /// reflected in a response body or error string never survives into
    /// `redacted_detail`. This is the ONLY way to construct the value.
    pub fn new(
        provider: impl Into<String>,
        kind: ProviderErrorKind,
        retryable: bool,
        raw_detail: impl AsRef<str>,
    ) -> Self {
        let redacted_detail = crate::catalog::git::scrub_to_string(raw_detail.as_ref().as_bytes());
        Self {
            provider: provider.into(),
            kind,
            retryable,
            redacted_detail,
        }
    }

    /// Map this structured error onto the closed [`TomeError`] set exactly once.
    ///
    /// `EmbeddingInvalid` → [`TomeError::RemoteEmbeddingInvalid`] (exit 95);
    /// every other kind → [`TomeError::ProviderRequestFailed`] (exit 94). The
    /// `detail` carrier is the [`Display`](std::fmt::Display) output, so the
    /// kind + retryable flag ride along into both the human message and the
    /// `--json` error envelope (FR-013a).
    pub fn into_tome_error(self) -> TomeError {
        let detail = self.to_string();
        match self.kind {
            ProviderErrorKind::EmbeddingInvalid => TomeError::RemoteEmbeddingInvalid { detail },
            _ => TomeError::ProviderRequestFailed { detail },
        }
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "provider `{}` request failed [kind={}, retryable={}]: {}",
            self.provider, self.kind, self.retryable, self.redacted_detail
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_as_str_tokens() {
        assert_eq!(ProviderErrorKind::Auth.as_str(), "auth");
        assert_eq!(ProviderErrorKind::RateLimited.as_str(), "rate_limited");
        assert_eq!(ProviderErrorKind::ModelNotFound.as_str(), "model_not_found");
        assert_eq!(ProviderErrorKind::BadRequest.as_str(), "bad_request");
        assert_eq!(ProviderErrorKind::Timeout.as_str(), "timeout");
        assert_eq!(ProviderErrorKind::Unreachable.as_str(), "unreachable");
        assert_eq!(
            ProviderErrorKind::MalformedResponse.as_str(),
            "malformed_response"
        );
        assert_eq!(
            ProviderErrorKind::EmbeddingInvalid.as_str(),
            "embedding_invalid"
        );
    }

    #[test]
    fn embedding_invalid_maps_to_95() {
        let err = ProviderError::new("p", ProviderErrorKind::EmbeddingInvalid, false, "bad dim");
        let tome = err.into_tome_error();
        assert_eq!(tome.exit_code(), 95);
        assert!(matches!(tome, TomeError::RemoteEmbeddingInvalid { .. }));
    }

    #[test]
    fn every_non_embedding_kind_maps_to_94() {
        for kind in [
            ProviderErrorKind::Auth,
            ProviderErrorKind::RateLimited,
            ProviderErrorKind::ModelNotFound,
            ProviderErrorKind::BadRequest,
            ProviderErrorKind::Timeout,
            ProviderErrorKind::Unreachable,
            ProviderErrorKind::MalformedResponse,
        ] {
            let err = ProviderError::new("p", kind, false, "boom");
            let tome = err.into_tome_error();
            assert_eq!(tome.exit_code(), 94, "kind {kind:?} must map to 94");
            assert!(
                matches!(tome, TomeError::ProviderRequestFailed { .. }),
                "kind {kind:?} must map to ProviderRequestFailed"
            );
        }
    }

    #[test]
    fn detail_embeds_kind_and_retryable() {
        let err = ProviderError::new("acme", ProviderErrorKind::RateLimited, true, "slow down");
        let detail = err.to_string();
        assert!(detail.contains("kind=rate_limited"), "{detail}");
        assert!(detail.contains("retryable=true"), "{detail}");
        assert!(detail.contains("acme"), "{detail}");
        assert!(detail.contains("slow down"), "{detail}");
    }

    #[test]
    fn into_tome_error_carries_kind_and_retryable_in_message() {
        // FR-013a: the surfaced TomeError message (which the --json envelope
        // serialises) must carry kind + retryable.
        let err = ProviderError::new("acme", ProviderErrorKind::Auth, false, "401");
        let msg = err.into_tome_error().to_string();
        assert!(msg.contains("kind=auth"), "{msg}");
        assert!(msg.contains("retryable=false"), "{msg}");
    }

    #[test]
    fn credential_in_raw_detail_is_scrubbed() {
        // A reflected OpenAI key (sk-…) in the raw body must not survive into
        // redacted_detail or the Display output. The bare-key-format scrubber
        // step (T010) catches it even with no surrounding header context.
        let leaked = "request failed: invalid api key sk-abc123def456ghi789jkl provided";
        let err = ProviderError::new("acme", ProviderErrorKind::Auth, false, leaked);
        assert!(
            !err.redacted_detail.contains("sk-abc123def456ghi789jkl"),
            "redacted_detail leaked the key: {}",
            err.redacted_detail
        );
        assert!(
            !err.to_string().contains("sk-abc123def456ghi789jkl"),
            "Display leaked the key: {err}"
        );
        // And once mapped onto TomeError, still gone.
        let msg = err.into_tome_error().to_string();
        assert!(
            !msg.contains("sk-abc123def456ghi789jkl"),
            "TomeError message leaked the key: {msg}"
        );
    }

    #[test]
    fn bearer_credential_in_raw_detail_is_scrubbed() {
        let leaked = "upstream said: Authorization: Bearer sk-secrettokenvalue1234567 rejected";
        let err = ProviderError::new("acme", ProviderErrorKind::Auth, false, leaked);
        assert!(
            !err.redacted_detail.contains("sk-secrettokenvalue1234567"),
            "{}",
            err.redacted_detail
        );
    }
}
