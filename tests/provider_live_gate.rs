//! T079 / NFR-005 — the live-provider verification RELEASE GATE.
//!
//! ## What this is
//!
//! Every test in this file is `#[ignore]`d. They are **OUT of fast CI by
//! design** and exercise REAL provider behaviour over the REAL synchronous
//! transport (NOT the `set_transport_override` seam the unit/US2 tests use).
//! They are the NFR-005 gate: run them before any release that touches
//! `src/provider/`.
//!
//! ```text
//! cargo test --test provider_live_gate -- --ignored
//! ```
//!
//! In the normal suite (`cargo test --test provider_live_gate`) every test here
//! is reported as `ignored` and nothing reaches the network — so this file's
//! structural value is that the gate EXISTS, compiles, and is documented.
//!
//! ## Why these three cases
//!
//! NFR-005 mandates exercising, against a real endpoint, the three failure
//! modes the unit tests can only simulate over the seam:
//!   1. a real **429** (rate-limit) — the retry loop must honour `Retry-After`
//!      and, on exhaustion, surface `RateLimited` (retryable);
//!   2. a real **timeout** — the transport must surface `Timeout` (retryable);
//!   3. an **empty / short embedding response** — content validation must
//!      fail closed with `RemoteEmbeddingInvalid` (exit 95), never indexed.
//!
//! ## Required environment (per case)
//!
//! Each test reads its connection from env and CLEANLY SKIPS (early return +
//! an explanatory `eprintln!`) when the required vars are absent — an
//! `#[ignore]`d test invoked with `--ignored` but missing creds must NOT
//! falsely fail. Credentials are resolved by Tome's own derived-env rule
//! (`TOME_<NAME>_API_KEY`, where `<NAME>` is the uppercased registry name with
//! non-alphanumerics → `_`); these tests use the registry name `live`, so the
//! key var is always `TOME_LIVE_API_KEY`.
//!
//! ### Case 1 — 429 (`live_429_rate_limit_honours_retry_after`)
//!   - `TOME_LIVE_GATE_429_KIND`     = `openai` | `voyage` (embedding-capable)
//!   - `TOME_LIVE_GATE_429_BASE_URL` = base URL of a real rate-limited
//!     endpoint, or a local mock the release engineer stands up that returns
//!     HTTP 429 with a `Retry-After` header (e.g. a 2-line nginx/`httpbin`
//!     `status/429`). Point this at something that ALWAYS 429s.
//!   - `TOME_LIVE_GATE_429_MODEL`    = the embedding model id to send.
//!   - `TOME_LIVE_API_KEY`           = the key (may be a dummy if the mock
//!     ignores auth; required so credential resolution produces a connection).
//!
//! ### Case 2 — timeout (`live_timeout_surfaces_timeout_kind`)
//!   - `TOME_LIVE_GATE_TIMEOUT_BASE_URL` = a deliberately-slow / blackhole
//!     endpoint that accepts the TCP connect but never responds (e.g.
//!     `http://10.255.255.1:1` or an nginx location with a long `sleep`).
//!   - `TOME_LIVE_GATE_TIMEOUT_KIND`     = `openai` | `voyage` (default
//!     `openai`).
//!   - `TOME_LIVE_GATE_TIMEOUT_MODEL`    = the model id (default `m`).
//!   - `TOME_LIVE_API_KEY`               = the key (dummy is fine).
//!
//! The per-call timeout is forced low via `TOME_PROVIDER_TIMEOUT_SECS=1` so the
//! case completes quickly even after the retry loop exhausts.
//!
//! ### Case 3 — empty/short embedding (`live_empty_embedding_is_remote_invalid`)
//!   - `TOME_LIVE_GATE_EMPTY_BASE_URL` = an endpoint that returns a 200 with a
//!     well-formed envelope carrying an EMPTY embedding vector (`data:[{
//!     "embedding": [] }]`) — a local mock the release engineer points at, or a
//!     real provider/model combination known to do so.
//!   - `TOME_LIVE_GATE_EMPTY_KIND`     = `openai` | `voyage` (default `openai`).
//!   - `TOME_LIVE_GATE_EMPTY_MODEL`    = the model id (default `m`).
//!   - `TOME_LIVE_API_KEY`             = the key (dummy is fine if the mock
//!     ignores auth).
//!
//! ## Isolation
//!
//! These tests mutate process-global env (`TOME_LIVE_API_KEY`,
//! `TOME_PROVIDER_TIMEOUT_SECS`). They serialise on a file-local mutex so the
//! `--ignored` run is deterministic, and each restores the timeout var on exit.

use std::sync::Mutex;
use std::time::Duration;

use tome::config::{Config, ProviderEntry, ProviderKind, Secret};
use tome::embedding::{Embedder, RemoteEmbedder};
use tome::error::TomeError;
use tome::provider::config::{Capability, ResolvedProvider, resolve};
use tome::provider::error::ProviderErrorKind;
use tome::provider::request_with_retry;

/// Serialises the env-mutating live tests so the `--ignored` run is
/// deterministic (the timeout var is process-global).
static LIVE_SERIAL: Mutex<()> = Mutex::new(());

/// Registry name used by every live test → the derived key var is always
/// `TOME_LIVE_API_KEY`.
const REGISTRY_NAME: &str = "live";

/// Read a required env var; on absence print a skip note and return `None` so
/// the caller can early-return without failing.
fn require_env(var: &str, test: &str) -> Option<String> {
    match std::env::var(var) {
        Ok(v) if !v.trim().is_empty() => Some(v),
        _ => {
            eprintln!(
                "SKIP {test}: required env `{var}` is unset; \
                 see the module docs for the live-gate setup. \
                 (An #[ignore]d test invoked with --ignored but missing creds \
                 skips rather than fails.)"
            );
            None
        }
    }
}

/// Parse an embedding-capable provider kind from an env token, defaulting to
/// OpenAI. Only `openai`/`voyage` are legal for the embedding capability.
fn embedding_kind_from_env(var: &str) -> ProviderKind {
    match std::env::var(var).ok().as_deref() {
        Some("voyage") => ProviderKind::Voyage,
        _ => ProviderKind::Openai,
    }
}

/// Build a `Config` with one embedding provider `live` of `kind`, pointed at
/// `base_url`, with `model`. The key is resolved from `TOME_LIVE_API_KEY` via
/// Tome's normal credential-resolution path (set by the caller).
fn embedding_config(kind: ProviderKind, base_url: &str, model: &str) -> Config {
    let mut config = Config::default();
    config.providers.insert(
        REGISTRY_NAME.to_string(),
        ProviderEntry {
            kind,
            base_url: Some(base_url.to_string()),
            // Inline a placeholder; the derived env var (TOME_LIVE_API_KEY)
            // wins when set, which the caller ensures.
            api_key: Some(Secret::from("env-or-inline".to_string())),
        },
    );
    config.embedding.provider = Some(REGISTRY_NAME.to_string());
    config.embedding.model = Some(model.to_string());
    config
}

/// Resolve an embedding `ResolvedProvider` through the real `resolve` path.
fn resolve_embedding(config: &Config) -> ResolvedProvider {
    resolve(config, Capability::Embedding)
        .expect("resolve ok")
        .expect("provider referenced")
}

/// RAII guard: set `TOME_PROVIDER_TIMEOUT_SECS` for the test, restore on drop.
struct TimeoutEnvGuard {
    prev: Option<std::ffi::OsString>,
}

impl TimeoutEnvGuard {
    fn set(secs: u64) -> Self {
        let prev = std::env::var_os("TOME_PROVIDER_TIMEOUT_SECS");
        // SAFETY: the live tests serialise via LIVE_SERIAL; no concurrent env mutation.
        unsafe { std::env::set_var("TOME_PROVIDER_TIMEOUT_SECS", secs.to_string()) };
        Self { prev }
    }
}

impl Drop for TimeoutEnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            // SAFETY: serialised via LIVE_SERIAL.
            Some(v) => unsafe { std::env::set_var("TOME_PROVIDER_TIMEOUT_SECS", v) },
            None => unsafe { std::env::remove_var("TOME_PROVIDER_TIMEOUT_SECS") },
        }
    }
}

// ---------------------------------------------------------------------------
// Case 1 — a real 429: the retry loop honours `Retry-After`, then surfaces
// `RateLimited` (retryable) after exhausting attempts.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "NFR-005 release gate; run with --ignored against a real/mock 429 endpoint (see module docs)"]
fn live_429_rate_limit_honours_retry_after() {
    let _serial = LIVE_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let test = "live_429_rate_limit_honours_retry_after";

    let Some(_key) = require_env("TOME_LIVE_API_KEY", test) else {
        return;
    };
    let Some(base_url) = require_env("TOME_LIVE_GATE_429_BASE_URL", test) else {
        return;
    };
    let Some(model) = require_env("TOME_LIVE_GATE_429_MODEL", test) else {
        return;
    };
    let kind = embedding_kind_from_env("TOME_LIVE_GATE_429_KIND");

    // Hit the embeddings endpoint directly so we assert on the structured
    // `ProviderError` kind (the embed() wrapper maps it to a TomeError). A
    // `Retry-After: 0` mock keeps the gate fast; the assertion is on the
    // exhausted-retry outcome, not the wall-clock wait.
    let config = embedding_config(kind, &base_url, &model);
    let resolved = resolve_embedding(&config);
    let body = serde_json::json!({ "model": model, "input": "live gate 429 probe" });

    let err = request_with_retry(&resolved, "/embeddings", &body)
        .expect_err("a 429 endpoint must not return Ok");
    assert_eq!(
        err.kind,
        ProviderErrorKind::RateLimited,
        "expected RateLimited after the retry loop honoured Retry-After, got {err:?}"
    );
    assert!(err.retryable, "a rate-limit must be retryable: {err:?}");
    // And the redacted detail must never carry a key (defence-in-depth — the
    // mock might echo the Authorization header).
    assert!(
        !err.redacted_detail.contains("sk-") && !err.redacted_detail.contains("pa-"),
        "429 error detail leaked a key shape: {}",
        err.redacted_detail
    );
}

// ---------------------------------------------------------------------------
// Case 2 — a real timeout: surfaces `Timeout` (retryable). Point at a
// blackhole/slow endpoint; force a 1s per-call timeout so the gate is quick.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "NFR-005 release gate; run with --ignored against a slow/blackhole endpoint (see module docs)"]
fn live_timeout_surfaces_timeout_kind() {
    let _serial = LIVE_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let test = "live_timeout_surfaces_timeout_kind";

    let Some(_key) = require_env("TOME_LIVE_API_KEY", test) else {
        return;
    };
    let Some(base_url) = require_env("TOME_LIVE_GATE_TIMEOUT_BASE_URL", test) else {
        return;
    };
    let model = std::env::var("TOME_LIVE_GATE_TIMEOUT_MODEL").unwrap_or_else(|_| "m".to_string());
    let kind = embedding_kind_from_env("TOME_LIVE_GATE_TIMEOUT_KIND");

    // Force a low per-call timeout so the (3-attempt) retry loop completes in a
    // few seconds even against a hung endpoint.
    let _timeout = TimeoutEnvGuard::set(1);

    let config = embedding_config(kind, &base_url, &model);
    let resolved = resolve_embedding(&config);
    assert_eq!(
        resolved.timeout,
        Duration::from_secs(1),
        "the forced TOME_PROVIDER_TIMEOUT_SECS must be honoured"
    );

    let body = serde_json::json!({ "model": model, "input": "live gate timeout probe" });
    let err = request_with_retry(&resolved, "/embeddings", &body)
        .expect_err("a blackhole endpoint must time out, not return Ok");
    assert_eq!(
        err.kind,
        ProviderErrorKind::Timeout,
        "expected Timeout against a blackhole endpoint, got {err:?}"
    );
    assert!(err.retryable, "a timeout must be retryable: {err:?}");
}

// ---------------------------------------------------------------------------
// Case 3 — an empty/short embedding response: content validation fails closed
// with `RemoteEmbeddingInvalid` (exit 95). Driven through the REAL
// `RemoteEmbedder::embed` so the production index/query path is exercised.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "NFR-005 release gate; run with --ignored against an endpoint returning an empty embedding (see module docs)"]
fn live_empty_embedding_is_remote_invalid() {
    let _serial = LIVE_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let test = "live_empty_embedding_is_remote_invalid";

    let Some(_key) = require_env("TOME_LIVE_API_KEY", test) else {
        return;
    };
    let Some(base_url) = require_env("TOME_LIVE_GATE_EMPTY_BASE_URL", test) else {
        return;
    };
    let model = std::env::var("TOME_LIVE_GATE_EMPTY_MODEL").unwrap_or_else(|_| "m".to_string());
    let kind = embedding_kind_from_env("TOME_LIVE_GATE_EMPTY_KIND");

    let config = embedding_config(kind, &base_url, &model);
    let resolved = resolve_embedding(&config);
    // No seed dimension → the empty-vector check fires first regardless.
    let embedder = RemoteEmbedder::new(resolved, None, None);

    let err = embedder
        .embed("live gate empty-embedding probe")
        .expect_err("an empty embedding must fail closed, not return a vector");
    assert!(
        matches!(err, TomeError::RemoteEmbeddingInvalid { .. }),
        "expected RemoteEmbeddingInvalid (fail-closed), got {err:?}"
    );
    assert_eq!(
        err.exit_code(),
        95,
        "remote-embedding content validation must map to exit 95"
    );
}
