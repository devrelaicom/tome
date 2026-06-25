//! The synchronous provider transport seam (`reqwest::blocking`-backed).
//!
//! [`request_with_retry`] is the one entry point every per-kind module calls:
//! it shapes the request (URL, auth placement by [`ProviderKind`], JSON body),
//! runs a bounded retry loop (FR-012), and maps the outcome onto the structured
//! [`ProviderError`]. A successful 2xx body is returned as a lenient
//! [`serde_json::Value`] for the caller to extract fields from (per-kind
//! modules detect a 200-with-error envelope themselves).
//!
//! ## Sync-only
//!
//! Everything here is `reqwest::blocking`. `tests/harness_settings/sync_boundary.rs`
//! greps this tree for async constructs; nothing under `src/provider/` may reach
//! the async runtime (the constitution's single async island is `src/mcp/`).
//!
//! ## Test seam (T013)
//!
//! [`execute`] dispatches through an injectable override
//! ([`set_transport_override`]) when one is installed, else the real blocking
//! client. The override is a `#[doc(hidden)] pub static` + RAII guard (the
//! project convention — integration tests under `tests/` can't see
//! `#[cfg(test)]`, so the seam is compiled in unconditionally). The override
//! receives the full [`RequestSpec`], so a stateful closure can return e.g. a
//! 429 then a 200 across retry attempts.

use std::sync::Mutex;
use std::time::Duration;

use crate::config::ProviderKind;
use crate::provider::config::ResolvedProvider;
use crate::provider::error::{ProviderError, ProviderErrorKind};

/// The Anthropic API version header value (the Messages API requires it).
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Maximum total attempts in the retry loop (1 initial + 2 retries) — FR-012.
const MAX_ATTEMPTS: u32 = 3;

/// Upper bound on a single backoff sleep when no `Retry-After` is present.
const MAX_BACKOFF: Duration = Duration::from_secs(5);

/// Hard ceiling on a single retry sleep, including a server-supplied
/// `Retry-After`. We honour `Retry-After` (FR-012), but a misconfigured or
/// hostile upstream returning `Retry-After: 86400` must not hang a foreground
/// command (`tome query`/`reindex`) for hours. Clamp to a bounded wait; if the
/// server genuinely needs longer, the attempt cap exhausts and we fail clean
/// with `RateLimited` (the user can re-run) rather than stalling.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

/// The HTTP method for a provider request. v1 only ever POSTs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Post,
}

/// A fully-shaped HTTP request, ready for the transport. Auth is ALREADY placed
/// (header or query) by [`build_spec`], so the transport just sends it.
///
/// `Debug` is hand-written to REDACT: this is the most credential-dense struct
/// in the tree — `url` can carry the gemini `?key=` credential and `headers`
/// carry the `Authorization: Bearer` / `x-api-key` value; `body` can carry user
/// content (skill text / queries on the embedding path). So a stray
/// `tracing::debug!(?spec)` must never leak a secret or content. The URL and
/// header values route through the SSOT scrubber; the body shows only its
/// length. (Do NOT re-derive `Debug` — that would reintroduce the leak.)
#[derive(Clone)]
pub struct RequestSpec {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl std::fmt::Debug for RequestSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let url = crate::catalog::git::scrub_to_string(self.url.as_bytes());
        let headers: Vec<(String, String)> = self
            .headers
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    crate::catalog::git::scrub_to_string(v.as_bytes()),
                )
            })
            .collect();
        f.debug_struct("RequestSpec")
            .field("method", &self.method)
            .field("url", &url)
            .field("headers", &headers)
            .field("body_len", &self.body.len())
            .finish()
    }
}

/// A completed HTTP response (status + optional `Retry-After` + raw body).
/// Produced by [`execute`] on a request that COMPLETED (any status); a
/// transport-level failure is [`TransportFailure`] instead.
#[derive(Debug, Clone)]
pub struct RawResponse {
    pub status: u16,
    pub retry_after: Option<Duration>,
    pub body: Vec<u8>,
}

/// A transport-level failure (the request never completed): a connect error, a
/// timeout, a TLS failure, etc. Distinct from a completed non-2xx response.
#[derive(Debug, Clone)]
pub enum TransportFailure {
    /// The request exceeded the per-call timeout.
    Timeout,
    /// Any other transport error (connect refused, DNS, TLS, …). `detail` is a
    /// short, already-safe description (no URL/credential is included).
    Other { detail: String },
}

// ---------------------------------------------------------------------------
// Test seam (T013): an injectable transport override.
// ---------------------------------------------------------------------------

type TransportFn = dyn Fn(&RequestSpec) -> Result<RawResponse, TransportFailure> + Send + Sync;

/// The process-global transport override. `None` → use the real blocking
/// client. Installed via [`set_transport_override`], cleared by dropping the
/// returned [`ProviderTransportGuard`].
///
/// Compiled in unconditionally (NOT `#[cfg(test)]`): integration tests under
/// `tests/` can't see `#[cfg(test)]` items, which is exactly why the project
/// uses the `#[doc(hidden)] pub static` convention for these seams.
#[doc(hidden)]
pub static TRANSPORT_OVERRIDE: Mutex<Option<Box<TransportFn>>> = Mutex::new(None);

/// Serialises every test that installs a transport override. The override slot
/// is process-global, so two concurrent override-installing tests would clobber
/// each other (observed: a timeout test seeing an `Other` injector from a
/// parallel test). [`set_transport_override`] takes this lock and the returned
/// guard holds it for the test's lifetime, so override-installing tests are
/// mutually exclusive — across BOTH lib tests and integration tests.
static TRANSPORT_TEST_SERIAL: Mutex<()> = Mutex::new(());

/// RAII guard that clears the transport override on drop, restoring real
/// transport, and releases the [`TRANSPORT_TEST_SERIAL`] lock. Returned by
/// [`set_transport_override`].
#[doc(hidden)]
#[must_use = "the override is cleared when this guard is dropped; bind it to a variable"]
pub struct ProviderTransportGuard {
    // Held for the guard's lifetime so override-installing tests serialise.
    _serial: std::sync::MutexGuard<'static, ()>,
}

impl Drop for ProviderTransportGuard {
    fn drop(&mut self) {
        // Clear the override even on lock poisoning, so a later test isn't
        // stuck with a stale injector. The serial lock is released after this
        // (it's dropped with the struct), so the clear is ordered before the
        // next test can install.
        let mut slot = TRANSPORT_OVERRIDE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *slot = None;
    }
}

/// Install a transport override for the duration of the returned guard. The
/// closure sees the full [`RequestSpec`] for each attempt, so a stateful
/// closure (capturing an `AtomicUsize`) can vary its response across retry
/// attempts. Dropping the guard restores real transport AND releases the
/// serial lock, so override-installing tests never run concurrently.
#[doc(hidden)]
pub fn set_transport_override<F>(f: F) -> ProviderTransportGuard
where
    F: Fn(&RequestSpec) -> Result<RawResponse, TransportFailure> + Send + Sync + 'static,
{
    let serial = TRANSPORT_TEST_SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut slot = TRANSPORT_OVERRIDE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *slot = Some(Box::new(f));
    ProviderTransportGuard { _serial: serial }
}

/// Dispatch one request: to the installed override if present, else the real
/// blocking client. The single point where the transport decision is made.
fn execute(spec: &RequestSpec, timeout: Duration) -> Result<RawResponse, TransportFailure> {
    {
        let slot = TRANSPORT_OVERRIDE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(override_fn) = slot.as_ref() {
            return override_fn(spec);
        }
    }
    real_execute(spec, timeout)
}

/// The ONE memoised `reqwest::blocking::Client` builder is per-call because the
/// timeout varies per provider; building a fresh client per request is the
/// price of a per-call timeout. Redirects are disabled (a 3xx must surface as a
/// non-2xx status, never silently re-POST the body — which could carry the
/// credential — to a downgraded URL).
fn real_execute(spec: &RequestSpec, timeout: Duration) -> Result<RawResponse, TransportFailure> {
    let client = match reqwest::blocking::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        // TLS backend init failure — unreachable in practice; fail closed.
        Err(_) => {
            return Err(TransportFailure::Other {
                detail: "failed to build HTTP client".to_string(),
            });
        }
    };

    let mut builder = match spec.method {
        Method::Post => client.post(&spec.url),
    };
    for (k, v) in &spec.headers {
        builder = builder.header(k.as_str(), v.as_str());
    }
    builder = builder.body(spec.body.clone());

    match builder.send() {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_retry_after);
            let body = resp.bytes().map(|b| b.to_vec()).unwrap_or_default();
            Ok(RawResponse {
                status,
                retry_after,
                body,
            })
        }
        Err(e) if e.is_timeout() => Err(TransportFailure::Timeout),
        // NEVER include the reqwest Display (it can reproduce the URL, and a
        // URL-with-key for gemini). Keep the detail generic and credential-free.
        Err(_) => Err(TransportFailure::Other {
            detail: "transport error".to_string(),
        }),
    }
}

/// Parse a `Retry-After` header value (RFC 7231): either a non-negative integer
/// number of seconds, or an HTTP-date. We support the integer-seconds form
/// exactly; an HTTP-date (rare for 429) yields `None` and the caller falls back
/// to exponential backoff.
fn parse_retry_after(raw: &str) -> Option<Duration> {
    raw.trim().parse::<u64>().ok().map(Duration::from_secs)
}

/// Build the fully-shaped [`RequestSpec`] for a resolved provider + path +
/// JSON body. Auth is placed here, driven by [`ProviderKind`] (FR-007 / the
/// per-kind placement table), so per-kind modules never re-implement it:
/// - openai/voyage → `Authorization: Bearer <k>`
/// - anthropic → `x-api-key: <k>` + `anthropic-version: <date>`
/// - gemini → `?key=<k>` query parameter
///
/// When the credential is absent, the auth is simply omitted (FR-007 case 3).
fn build_spec(resolved: &ResolvedProvider, path: &str, body: &serde_json::Value) -> RequestSpec {
    let mut url = format!("{}{}", resolved.base_url, path);
    let mut headers: Vec<(String, String)> =
        vec![("Content-Type".to_string(), "application/json".to_string())];

    let credential = resolved.credential.expose();
    match resolved.kind {
        ProviderKind::Openai | ProviderKind::Voyage => {
            if let Some(key) = credential {
                headers.push(("Authorization".to_string(), format!("Bearer {key}")));
            }
        }
        ProviderKind::Anthropic => {
            if let Some(key) = credential {
                headers.push(("x-api-key".to_string(), key.to_string()));
            }
            headers.push((
                "anthropic-version".to_string(),
                ANTHROPIC_VERSION.to_string(),
            ));
        }
        ProviderKind::Gemini => {
            if let Some(key) = credential {
                let separator = if url.contains('?') { '&' } else { '?' };
                url.push(separator);
                url.push_str("key=");
                url.push_str(key);
            }
        }
    }

    let body_bytes = serde_json::to_vec(body).unwrap_or_default();
    RequestSpec {
        method: Method::Post,
        url,
        headers,
        body: body_bytes,
    }
}

/// Compute the backoff sleep for `attempt` (0-based). When a `Retry-After` is
/// present we honour it (FR-012: the server's explicit ask), clamped to
/// [`MAX_RETRY_AFTER`] so a pathological value can't stall a foreground command.
/// Without one we fall back to exponential `2^attempt` seconds capped at
/// [`MAX_BACKOFF`].
fn backoff_for(attempt: u32, retry_after: Option<Duration>) -> Duration {
    if let Some(ra) = retry_after {
        return ra.min(MAX_RETRY_AFTER); // honour, but bound the foreground wait
    }
    let secs = 1u64 << attempt.min(4); // 1, 2, 4, 8, 16 …
    Duration::from_secs(secs).min(MAX_BACKOFF)
}

/// Make a remote provider request with the bounded retry loop (FR-012).
///
/// Retries ONLY transport errors/timeouts, 429, and 5xx (≤ [`MAX_ATTEMPTS`]
/// total). Honours `Retry-After` on a 429, else exponential backoff capped at
/// [`MAX_BACKOFF`]. NEVER retries a non-429 4xx. On a 2xx the body is parsed as
/// a lenient [`serde_json::Value`] and returned; a parse failure →
/// `MalformedResponse`. Per-kind modules extract fields and detect a
/// 200-with-error envelope from the returned `Value`.
///
/// The full failure → [`ProviderError`] mapping (with credentials scrubbed via
/// the constructor) lives here, so callers get a single structured error type.
pub fn request_with_retry(
    resolved: &ResolvedProvider,
    path: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, ProviderError> {
    let spec = build_spec(resolved, path, body);
    let provider = resolved.name.as_str();

    // Track the last retryable failure so, on exhaustion, we surface the right
    // kind (RateLimited / Unreachable / Timeout) rather than a generic one.
    let mut last_retryable: Option<ProviderErrorKind> = None;

    for attempt in 0..MAX_ATTEMPTS {
        match execute(&spec, resolved.timeout) {
            Ok(resp) => {
                let status = resp.status;
                if (200..300).contains(&status) {
                    return parse_success_body(provider, &resp.body);
                }
                // 401/403 → Auth (no retry).
                if status == 401 || status == 403 {
                    return Err(ProviderError::new(
                        provider,
                        ProviderErrorKind::Auth,
                        false,
                        format!("HTTP {status}: {}", body_snippet(&resp.body)),
                    ));
                }
                // 404 → ModelNotFound (no retry).
                if status == 404 {
                    return Err(ProviderError::new(
                        provider,
                        ProviderErrorKind::ModelNotFound,
                        false,
                        format!("HTTP {status}: {}", body_snippet(&resp.body)),
                    ));
                }
                // 429 → RateLimited (retryable: honour Retry-After).
                if status == 429 {
                    last_retryable = Some(ProviderErrorKind::RateLimited);
                    if attempt + 1 < MAX_ATTEMPTS {
                        std::thread::sleep(backoff_for(attempt, resp.retry_after));
                        continue;
                    }
                    return Err(ProviderError::new(
                        provider,
                        ProviderErrorKind::RateLimited,
                        true,
                        format!(
                            "HTTP 429 after {MAX_ATTEMPTS} attempts: {}",
                            body_snippet(&resp.body)
                        ),
                    ));
                }
                // 5xx → Unreachable (retryable).
                if (500..600).contains(&status) {
                    last_retryable = Some(ProviderErrorKind::Unreachable);
                    if attempt + 1 < MAX_ATTEMPTS {
                        std::thread::sleep(backoff_for(attempt, resp.retry_after));
                        continue;
                    }
                    return Err(ProviderError::new(
                        provider,
                        ProviderErrorKind::Unreachable,
                        true,
                        format!(
                            "HTTP {status} after {MAX_ATTEMPTS} attempts: {}",
                            body_snippet(&resp.body)
                        ),
                    ));
                }
                // Other 4xx → BadRequest (no retry).
                return Err(ProviderError::new(
                    provider,
                    ProviderErrorKind::BadRequest,
                    false,
                    format!("HTTP {status}: {}", body_snippet(&resp.body)),
                ));
            }
            Err(TransportFailure::Timeout) => {
                last_retryable = Some(ProviderErrorKind::Timeout);
                if attempt + 1 < MAX_ATTEMPTS {
                    std::thread::sleep(backoff_for(attempt, None));
                    continue;
                }
                return Err(ProviderError::new(
                    provider,
                    ProviderErrorKind::Timeout,
                    true,
                    format!("request timed out after {MAX_ATTEMPTS} attempts"),
                ));
            }
            Err(TransportFailure::Other { detail }) => {
                last_retryable = Some(ProviderErrorKind::Unreachable);
                if attempt + 1 < MAX_ATTEMPTS {
                    std::thread::sleep(backoff_for(attempt, None));
                    continue;
                }
                return Err(ProviderError::new(
                    provider,
                    ProviderErrorKind::Unreachable,
                    true,
                    format!("{detail} after {MAX_ATTEMPTS} attempts"),
                ));
            }
        }
    }

    // Unreachable: the loop always returns. Belt-and-braces map the last seen
    // retryable kind so the compiler is satisfied without an `unwrap`.
    let kind = last_retryable.unwrap_or(ProviderErrorKind::Unreachable);
    Err(ProviderError::new(
        provider,
        kind,
        true,
        format!("exhausted {MAX_ATTEMPTS} attempts"),
    ))
}

/// Parse a 2xx body as a lenient JSON value; a parse failure →
/// `MalformedResponse`.
fn parse_success_body(provider: &str, body: &[u8]) -> Result<serde_json::Value, ProviderError> {
    serde_json::from_slice::<serde_json::Value>(body).map_err(|e| {
        ProviderError::new(
            provider,
            ProviderErrorKind::MalformedResponse,
            false,
            format!("2xx body was not valid JSON: {e}"),
        )
    })
}

/// A bounded, UTF-8-lossy snippet of a response body for an error detail. The
/// `ProviderError` constructor scrubs it, but we also bound length so a large
/// error page can't bloat the message.
fn body_snippet(body: &[u8]) -> String {
    const MAX: usize = 512;
    let text = String::from_utf8_lossy(body);
    if text.len() <= MAX {
        text.into_owned()
    } else {
        // Truncate on a char boundary within MAX.
        let mut end = MAX;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &text[..end])
    }
}

/// Log a response body at DEBUG ONLY, scrubbed. Provided for the per-kind
/// modules so they never log a raw body. Never logs above DEBUG.
#[allow(dead_code)]
pub(crate) fn debug_log_body(context: &str, body: &[u8]) {
    if tracing::enabled!(tracing::Level::DEBUG) {
        let scrubbed = crate::catalog::git::scrub_to_string(body);
        tracing::debug!(context, body = %scrubbed, "provider response body");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Secret;
    use crate::provider::config::{Capability, Credential};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn resolved(kind: ProviderKind, key: Option<&str>) -> ResolvedProvider {
        // Build a ResolvedProvider directly for transport tests. (resolve() is
        // exercised in config.rs; here we want a controlled connection.)
        ResolvedProvider {
            name: "testprov".to_string(),
            kind,
            base_url: "https://example.test/api".to_string(),
            credential: credential_for(key),
            model: "test-model".to_string(),
            timeout: Duration::from_secs(1),
        }
    }

    /// Build a `Credential` via the public resolve path so the test doesn't
    /// depend on private constructors: a tiny config + env override.
    fn credential_for(key: Option<&str>) -> Credential {
        // We can't call the private Credential constructors from here, but
        // resolve() produces one. Use a throwaway config; the env var carries
        // the key when present.
        use crate::config::{Config, ProviderEntry};
        let mut config = Config::default();
        config.providers.insert(
            "testprov".into(),
            ProviderEntry {
                kind: ProviderKind::Openai,
                base_url: None,
                api_key: key.map(|k| Secret::from(k.to_string())),
            },
        );
        config.embedding.provider = Some("testprov".into());
        config.embedding.model = Some("m".into());
        // Ensure no env override interferes.
        crate::provider::config::resolve(&config, Capability::Embedding)
            .unwrap()
            .unwrap()
            .credential
    }

    fn ok_json(value: serde_json::Value) -> RawResponse {
        RawResponse {
            status: 200,
            retry_after: None,
            body: serde_json::to_vec(&value).unwrap(),
        }
    }

    #[test]
    fn request_spec_debug_redacts_url_key_and_auth_headers() {
        // The most credential-dense struct: gemini `?key=` in the URL + a
        // Bearer / x-api-key header. A stray `{:?}` must leak neither, and must
        // not dump the body.
        let spec = RequestSpec {
            method: Method::Post,
            url: "https://generativelanguage.googleapis.com/v1beta/models/m:generateContent?key=AIzaSyABCDEFGHIJKLMNOPQRSTUVWXYZ012345".to_string(),
            headers: vec![
                (
                    "Authorization".to_string(),
                    "Bearer sk-ABCDEFGHIJKLMNOPQRSTUVWX".to_string(),
                ),
                (
                    "x-api-key".to_string(),
                    "sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUV".to_string(),
                ),
            ],
            body: b"{\"model\":\"m\",\"messages\":[]}".to_vec(),
        };
        let rendered = format!("{spec:?}");
        assert!(
            !rendered.contains("AIzaSyABCDEFGHIJKLMNOPQRSTUVWXYZ012345"),
            "gemini url key leaked in Debug: {rendered}",
        );
        assert!(
            !rendered.contains("sk-ABCDEFGHIJKLMNOPQRSTUVWX"),
            "bearer key leaked in Debug: {rendered}",
        );
        assert!(
            !rendered.contains("sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUV"),
            "x-api-key value leaked in Debug: {rendered}",
        );
        // Body is summarised by length, not dumped.
        assert!(
            rendered.contains("body_len"),
            "expected body_len: {rendered}"
        );
    }

    #[test]
    fn build_spec_openai_uses_bearer() {
        let r = resolved(ProviderKind::Openai, Some("sk-key"));
        let spec = build_spec(&r, "/embeddings", &serde_json::json!({"x": 1}));
        assert_eq!(spec.url, "https://example.test/api/embeddings");
        assert!(
            spec.headers
                .iter()
                .any(|(k, v)| k == "Authorization" && v == "Bearer sk-key")
        );
        assert!(
            spec.headers
                .iter()
                .any(|(k, v)| k == "Content-Type" && v == "application/json")
        );
    }

    #[test]
    fn build_spec_anthropic_uses_x_api_key_and_version() {
        let r = resolved(ProviderKind::Anthropic, Some("ant-key"));
        let spec = build_spec(&r, "/v1/messages", &serde_json::json!({}));
        assert!(
            spec.headers
                .iter()
                .any(|(k, v)| k == "x-api-key" && v == "ant-key")
        );
        assert!(
            spec.headers
                .iter()
                .any(|(k, v)| k == "anthropic-version" && v == ANTHROPIC_VERSION)
        );
        // No Bearer for anthropic.
        assert!(!spec.headers.iter().any(|(k, _)| k == "Authorization"));
    }

    #[test]
    fn build_spec_gemini_appends_key_query() {
        let r = resolved(ProviderKind::Gemini, Some("g-key"));
        let spec = build_spec(
            &r,
            "/v1beta/models/x:generateContent",
            &serde_json::json!({}),
        );
        assert!(spec.url.ends_with("?key=g-key"), "{}", spec.url);
        // No auth header for gemini.
        assert!(!spec.headers.iter().any(|(k, _)| k == "Authorization"));
        assert!(!spec.headers.iter().any(|(k, _)| k == "x-api-key"));
    }

    #[test]
    fn build_spec_absent_credential_omits_auth() {
        let r = resolved(ProviderKind::Openai, None);
        let spec = build_spec(&r, "/embeddings", &serde_json::json!({}));
        assert!(!spec.headers.iter().any(|(k, _)| k == "Authorization"));
    }

    #[test]
    fn success_2xx_returns_parsed_value() {
        let _guard = set_transport_override(|_spec| Ok(ok_json(serde_json::json!({"ok": true}))));
        let r = resolved(ProviderKind::Openai, Some("k"));
        let value = request_with_retry(&r, "/embeddings", &serde_json::json!({})).unwrap();
        assert_eq!(value["ok"], serde_json::json!(true));
    }

    #[test]
    fn malformed_2xx_body_is_malformed_response() {
        let _guard = set_transport_override(|_spec| {
            Ok(RawResponse {
                status: 200,
                retry_after: None,
                body: b"not json{".to_vec(),
            })
        });
        let r = resolved(ProviderKind::Openai, Some("k"));
        let err = request_with_retry(&r, "/embeddings", &serde_json::json!({})).unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::MalformedResponse);
        assert!(!err.retryable);
    }

    #[test]
    fn auth_401_no_retry() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let _guard = set_transport_override(move |_spec| {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(RawResponse {
                status: 401,
                retry_after: None,
                body: b"{\"error\":\"bad key\"}".to_vec(),
            })
        });
        let r = resolved(ProviderKind::Openai, Some("k"));
        let err = request_with_retry(&r, "/x", &serde_json::json!({})).unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Auth);
        assert!(!err.retryable);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "401 must not retry");
    }

    #[test]
    fn not_found_404_is_model_not_found_no_retry() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let _guard = set_transport_override(move |_spec| {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(RawResponse {
                status: 404,
                retry_after: None,
                body: Vec::new(),
            })
        });
        let r = resolved(ProviderKind::Openai, Some("k"));
        let err = request_with_retry(&r, "/x", &serde_json::json!({})).unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::ModelNotFound);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn other_4xx_is_bad_request_no_retry() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let _guard = set_transport_override(move |_spec| {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(RawResponse {
                status: 400,
                retry_after: None,
                body: b"{\"error\":\"context length exceeded\"}".to_vec(),
            })
        });
        let r = resolved(ProviderKind::Openai, Some("k"));
        let err = request_with_retry(&r, "/x", &serde_json::json!({})).unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::BadRequest);
        assert!(!err.retryable);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "non-429 4xx must not retry"
        );
    }

    #[test]
    fn rate_limit_then_success_across_attempts() {
        // First attempt 429 (Retry-After 0 to keep the test fast), second 200.
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let _guard = set_transport_override(move |_spec| {
            let n = c.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Ok(RawResponse {
                    status: 429,
                    retry_after: Some(Duration::from_secs(0)),
                    body: Vec::new(),
                })
            } else {
                Ok(ok_json(serde_json::json!({"ok": 1})))
            }
        });
        let r = resolved(ProviderKind::Openai, Some("k"));
        let value = request_with_retry(&r, "/x", &serde_json::json!({})).unwrap();
        assert_eq!(value["ok"], serde_json::json!(1));
        assert_eq!(calls.load(Ordering::SeqCst), 2, "should have retried once");
    }

    #[test]
    fn rate_limit_exhausted_is_rate_limited_retryable() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let _guard = set_transport_override(move |_spec| {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(RawResponse {
                status: 429,
                retry_after: Some(Duration::from_secs(0)),
                body: Vec::new(),
            })
        });
        let r = resolved(ProviderKind::Openai, Some("k"));
        let err = request_with_retry(&r, "/x", &serde_json::json!({})).unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::RateLimited);
        assert!(err.retryable);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            MAX_ATTEMPTS as usize,
            "429 should retry up to MAX_ATTEMPTS"
        );
    }

    #[test]
    fn server_5xx_exhausted_is_unreachable_retryable() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let _guard = set_transport_override(move |_spec| {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(RawResponse {
                status: 503,
                retry_after: Some(Duration::from_secs(0)),
                body: Vec::new(),
            })
        });
        let r = resolved(ProviderKind::Openai, Some("k"));
        let err = request_with_retry(&r, "/x", &serde_json::json!({})).unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Unreachable);
        assert!(err.retryable);
        assert_eq!(calls.load(Ordering::SeqCst), MAX_ATTEMPTS as usize);
    }

    #[test]
    fn transport_timeout_exhausted_is_timeout_retryable() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let _guard = set_transport_override(move |_spec| {
            c.fetch_add(1, Ordering::SeqCst);
            Err(TransportFailure::Timeout)
        });
        let r = resolved(ProviderKind::Openai, Some("k"));
        let err = request_with_retry(&r, "/x", &serde_json::json!({})).unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Timeout);
        assert!(err.retryable);
        assert_eq!(calls.load(Ordering::SeqCst), MAX_ATTEMPTS as usize);
    }

    #[test]
    fn transport_other_exhausted_is_unreachable() {
        let _guard = set_transport_override(|_spec| {
            Err(TransportFailure::Other {
                detail: "connection refused".to_string(),
            })
        });
        let r = resolved(ProviderKind::Openai, Some("k"));
        let err = request_with_retry(&r, "/x", &serde_json::json!({})).unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Unreachable);
        assert!(err.retryable);
    }

    #[test]
    fn guard_drop_restores_real_transport() {
        // Install a real guard; while it is alive the serial lock is held, so
        // the `is_some` check is race-free.
        let guard = set_transport_override(|_spec| Ok(ok_json(serde_json::json!({}))));
        assert!(
            TRANSPORT_OVERRIDE
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_some(),
            "override should be installed while the guard is alive"
        );
        // Dropping the guard clears the override AND releases the serial lock.
        // Re-acquire the serial lock immediately so no other override test can
        // install before we observe the cleared slot.
        drop(guard);
        let _serial = TRANSPORT_TEST_SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            TRANSPORT_OVERRIDE
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none(),
            "guard drop must clear the override"
        );
    }

    #[test]
    fn parse_retry_after_seconds_form() {
        assert_eq!(parse_retry_after("3"), Some(Duration::from_secs(3)));
        assert_eq!(parse_retry_after("  10 "), Some(Duration::from_secs(10)));
        assert_eq!(parse_retry_after("Wed, 21 Oct 2026 07:28:00 GMT"), None);
    }
}
