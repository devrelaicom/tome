//! Provider-registry resolution: `Config` + capability → `ResolvedProvider`.
//!
//! [`resolve`] turns a capability's `provider`/`model` config into a
//! [`ResolvedProvider`] — the connection a `Remote*` impl uses — or `Ok(None)`
//! when no provider is referenced (the default: use the bundled local model).
//!
//! All failures here are *semantic* config errors surfaced at resolve time
//! ([`TomeError::ProviderConfigInvalid`], exit 93): an undefined provider
//! reference, a kind illegal for the capability, or a `provider` set without a
//! `model`. A malformed config *field* is a distinct parse-time failure (exit 5
//! / `ManifestInvalid::TomlParse`) handled in [`crate::config::load`].
//!
//! Credential resolution (FR-007): the derived env var `TOME_<NAME>_API_KEY`
//! wins over an inline `api_key`, which wins over no credential. Tome NEVER
//! reads a generic provider env var (`OPENAI_API_KEY`, …) — the derived name is
//! the only env source (FR-008).

use std::time::Duration;

use crate::config::{Config, ProviderKind, Secret};
use crate::error::TomeError;

/// The default per-call timeout for a remote provider request. Overridable via
/// the `TOME_PROVIDER_TIMEOUT_SECS` env var (parsed as whole seconds; a missing
/// or unparsable value falls back to this default).
pub const DEFAULT_PROVIDER_TIMEOUT: Duration = Duration::from_secs(30);

/// The env var consulted to override [`DEFAULT_PROVIDER_TIMEOUT`].
const TIMEOUT_ENV_VAR: &str = "TOME_PROVIDER_TIMEOUT_SECS";

/// Which model capability a resolution targets. Each capability pulls its
/// `provider`/`model` from a different config section and accepts a different
/// set of provider kinds (the FR-005 matrix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Summariser,
    Embedding,
    Reranker,
    /// US6.2 — BYOM provider for `Handler::Prompt` hook execution. Reads its
    /// `provider`/`model` from `[hooks] prompt_provider`/`prompt_model`.
    /// Legal kinds: openai, anthropic, gemini (same matrix as Summariser).
    HookPrompt,
}

impl Capability {
    /// The lowercase token for this capability, used in error messages and (in
    /// later phases) telemetry. Matches the config section name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Capability::Summariser => "summariser",
            Capability::Embedding => "embedding",
            Capability::Reranker => "reranker",
            Capability::HookPrompt => "hook_prompt",
        }
    }

    /// Whether `kind` is a legal provider kind for this capability (FR-005):
    /// summariser ∈ {openai, anthropic, gemini}; embedding ∈ {openai, voyage};
    /// reranker ∈ {voyage}.
    fn allows_kind(&self, kind: ProviderKind) -> bool {
        match self {
            Capability::Summariser | Capability::HookPrompt => matches!(
                kind,
                ProviderKind::Openai | ProviderKind::Anthropic | ProviderKind::Gemini
            ),
            Capability::Embedding => {
                matches!(kind, ProviderKind::Openai | ProviderKind::Voyage)
            }
            Capability::Reranker => matches!(kind, ProviderKind::Voyage),
        }
    }
}

impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A resolved credential for a provider. `None` means no credential is sent —
/// a legitimate state for a local OpenAI-compatible server that needs no auth.
///
/// `Debug` is hand-written to redact: it prints `Credential(<present>)` /
/// `Credential(<absent>)`, never the inner secret. (The inner [`Secret`] also
/// redacts, but a hand-written impl keeps the shape obvious and greppable.)
pub struct Credential(Option<Secret>);

impl Credential {
    /// Construct a present credential from a [`Secret`].
    fn present(secret: Secret) -> Self {
        Credential(Some(secret))
    }

    /// Construct the absent credential.
    fn absent() -> Self {
        Credential(None)
    }

    /// The real, unredacted credential value, if present. The single auditable
    /// path to the bytes (the underlying [`Secret::expose`]). Returns `None`
    /// when no credential is configured.
    pub fn expose(&self) -> Option<&str> {
        self.0.as_ref().map(Secret::expose)
    }

    /// Whether a credential is present (without exposing it).
    pub fn is_present(&self) -> bool {
        self.0.is_some()
    }
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_some() {
            f.write_str("Credential(<present>)")
        } else {
            f.write_str("Credential(<absent>)")
        }
    }
}

/// A fully-resolved provider connection — everything a `Remote*` impl needs to
/// make a request. Produced by [`resolve`].
///
/// `Debug` is hand-written to redact the credential (mirroring the `RequestSpec`
/// guard in `http.rs`): it renders `name`/`kind`/`base_url`/`model`/`timeout`
/// and routes the [`Credential`] through ITS OWN redacting `Debug`, never the
/// raw secret. This is credential-bearing, so a stray `tracing::debug!(?resolved)`
/// must never leak a key. (Do NOT re-derive `Debug` — that would reintroduce the
/// leak. The deriving impl is safe TODAY because `Credential` redacts and
/// `base_url` is key-free, but the carve-out is latent: a future `api_key`-in-url
/// kind or a Debug-printing inner type would expose it. The hand-written impl
/// makes the redaction explicit and greppable.)
pub struct ResolvedProvider {
    /// The registry name (the `[providers.<name>]` key).
    pub name: String,
    /// The provider kind — fixes the wire shape and credential placement.
    pub kind: ProviderKind,
    /// The resolved base URL: the entry's explicit `base_url` if set, else the
    /// per-kind default, with any trailing `/` trimmed.
    pub base_url: String,
    /// The resolved credential (env → inline → none).
    pub credential: Credential,
    /// The remote model identifier (required when a provider is referenced).
    pub model: String,
    /// The per-call timeout (default [`DEFAULT_PROVIDER_TIMEOUT`], overridable
    /// via `TOME_PROVIDER_TIMEOUT_SECS`).
    pub timeout: Duration,
}

impl std::fmt::Debug for ResolvedProvider {
    /// Redacting `Debug`: prints every field EXCEPT the raw credential, which is
    /// rendered via [`Credential`]'s own redacting `Debug`. Mirrors the
    /// `RequestSpec` guard in `http.rs`. Do NOT replace this with `#[derive]`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedProvider")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("base_url", &self.base_url)
            // `Credential`'s Debug renders `<present>`/`<absent>`, never the key.
            .field("credential", &self.credential)
            .field("model", &self.model)
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// The default base URL for a provider kind when the entry omits `base_url`.
/// A trailing `/` is NOT included here (and is trimmed from explicit values in
/// [`resolve`]) so URL composition (`{base_url}{path}`) is unambiguous.
fn default_base_url(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Openai => "https://api.openai.com/v1",
        ProviderKind::Anthropic => "https://api.anthropic.com",
        ProviderKind::Gemini => "https://generativelanguage.googleapis.com",
        ProviderKind::Voyage => "https://api.voyageai.com/v1",
    }
}

/// Derive the credential-override env var name for a registry name (FR-007):
/// uppercase the name, replace every non-alphanumeric byte with `_`, then wrap
/// as `TOME_<NAME>_API_KEY`. Shared by [`resolve`] and (in a later phase) the
/// doctor's provider report so the two never diverge.
///
/// Example: `my-prov.2` → `TOME_MY_PROV_2_API_KEY`.
pub fn derive_env_var_name(name: &str) -> String {
    let sanitised: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("TOME_{sanitised}_API_KEY")
}

/// Pull the per-call timeout from the env override, falling back to the
/// default. An unparsable or non-positive value is ignored (logged at debug).
fn resolve_timeout() -> Duration {
    match std::env::var(TIMEOUT_ENV_VAR) {
        Ok(v) => match v.trim().parse::<u64>() {
            Ok(secs) if secs > 0 => Duration::from_secs(secs),
            _ => {
                tracing::debug!(
                    value = %v,
                    "ignoring unparsable {TIMEOUT_ENV_VAR}; using default provider timeout"
                );
                DEFAULT_PROVIDER_TIMEOUT
            }
        },
        Err(_) => DEFAULT_PROVIDER_TIMEOUT,
    }
}

/// Resolve a capability's credential (FR-007): the derived env var
/// `TOME_<NAME>_API_KEY` (if set & non-empty) wins; else the inline `api_key`;
/// else no credential. Tome NEVER reads a generic provider env var (FR-008).
fn resolve_credential(name: &str, inline: Option<&Secret>) -> Credential {
    let env_var = derive_env_var_name(name);
    if let Ok(value) = std::env::var(&env_var)
        && !value.is_empty()
    {
        return Credential::present(Secret::from(value));
    }
    match inline {
        Some(secret) => Credential::present(secret.clone()),
        None => Credential::absent(),
    }
}

/// Warn (once, non-fatally) if another registry name derives the SAME
/// `TOME_<NAME>_API_KEY` env var as `name` (FR-007 edge): a collision means an
/// env override is ambiguous between two providers. This is a config warning,
/// never an error.
fn warn_on_env_var_collision(config: &Config, name: &str) {
    let env_var = derive_env_var_name(name);
    for other in config.providers.keys() {
        if other != name && derive_env_var_name(other) == env_var {
            tracing::warn!(
                provider = %name,
                collides_with = %other,
                env_var = %env_var,
                "two providers derive the same API-key env var; the env override is ambiguous"
            );
        }
    }
}

/// Resolve a capability to a [`ResolvedProvider`], or `Ok(None)` when no
/// provider is referenced (use the bundled local model — the default path).
///
/// Errors (all [`TomeError::ProviderConfigInvalid`], exit 93):
/// - the referenced provider name is not defined in `[providers]`;
/// - the provider's kind is not legal for the capability (FR-005 matrix);
/// - `provider` is set but `model` is not.
pub fn resolve(
    config: &Config,
    capability: Capability,
) -> Result<Option<ResolvedProvider>, TomeError> {
    // Pull (provider, model) from the capability's section.
    let (provider, model) = match capability {
        Capability::Summariser => (
            config.summariser.provider.as_deref(),
            config.summariser.model.as_deref(),
        ),
        Capability::Embedding => (
            config.embedding.provider.as_deref(),
            config.embedding.model.as_deref(),
        ),
        Capability::Reranker => (
            config.reranker.provider.as_deref(),
            config.reranker.model.as_deref(),
        ),
        Capability::HookPrompt => (
            config.hooks.prompt_provider.as_deref(),
            config.hooks.prompt_model.as_deref(),
        ),
    };

    // No provider referenced → use the bundled local model (the default).
    let Some(name) = provider else {
        return Ok(None);
    };

    // 1. The provider must be defined in the registry.
    let entry = config
        .providers
        .get(name)
        .ok_or_else(|| TomeError::ProviderConfigInvalid {
            detail: format!("{capability} provider `{name}` is not defined in [providers]"),
        })?;

    // 2. The kind must be legal for the capability (FR-005).
    if !capability.allows_kind(entry.kind) {
        return Err(TomeError::ProviderConfigInvalid {
            detail: format!(
                "{capability} provider `{name}` has kind `{}`, which is not valid for the {capability} capability",
                entry.kind.as_str()
            ),
        });
    }

    // 3. A referenced provider requires a model.
    let Some(model) = model else {
        return Err(TomeError::ProviderConfigInvalid {
            detail: format!("[{capability}] sets provider `{name}` but no model"),
        });
    };

    // 4. Resolve the base URL: explicit (trailing-/ trimmed) or per-kind default.
    let base_url = match entry.base_url.as_deref() {
        Some(explicit) => explicit.trim_end_matches('/').to_string(),
        None => default_base_url(entry.kind).to_string(),
    };

    // 5. Resolve the credential (env → inline → none), warning on collision.
    warn_on_env_var_collision(config, name);
    let credential = resolve_credential(name, entry.api_key.as_ref());

    Ok(Some(ResolvedProvider {
        name: name.to_string(),
        kind: entry.kind,
        base_url,
        credential,
        model: model.to_string(),
        timeout: resolve_timeout(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProviderEntry;
    use std::sync::Mutex;

    /// Serialises every test that mutates the process-global env vars this
    /// module consults (`TOME_<NAME>_API_KEY`, `TOME_PROVIDER_TIMEOUT_SECS`).
    /// `cargo test` runs a module's tests on multiple threads; the vars are
    /// process-global. Mirrors the `ENV_MUTEX` idiom used across the codebase
    /// (see `telemetry::config`).
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    /// RAII guard: snapshot the named env vars, clear them, restore on drop.
    /// Holds `ENV_MUTEX` for its lifetime so a restore can't interleave with
    /// another test's set.
    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        saved: Vec<(String, Option<std::ffi::OsString>)>,
    }

    impl EnvGuard {
        fn new(vars: &[&str]) -> Self {
            let lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            let saved = vars
                .iter()
                .map(|&k| (k.to_string(), std::env::var_os(k)))
                .collect::<Vec<_>>();
            // SAFETY: ENV_MUTEX held for the guard's lifetime; no other test in
            // this module mutates these vars concurrently.
            for &k in vars {
                unsafe { std::env::remove_var(k) };
            }
            EnvGuard { _lock: lock, saved }
        }

        fn set(&self, key: &str, val: &str) {
            // SAFETY: guarded by ENV_MUTEX (held via `_lock`).
            unsafe { std::env::set_var(key, val) };
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: still holding ENV_MUTEX (dropped after this).
            for (k, v) in &self.saved {
                match v {
                    Some(val) => unsafe { std::env::set_var(k, val) },
                    None => unsafe { std::env::remove_var(k) },
                }
            }
        }
    }

    fn entry(kind: ProviderKind, base_url: Option<&str>, api_key: Option<&str>) -> ProviderEntry {
        ProviderEntry {
            kind,
            base_url: base_url.map(|s| s.to_string()),
            api_key: api_key.map(|s| Secret::from(s.to_string())),
        }
    }

    /// Build a Config with one provider entry and the named capability pointed
    /// at it (or not, when `provider`/`model` are None).
    fn config_with(
        provider_name: &str,
        provider_entry: ProviderEntry,
        capability: Capability,
        ref_name: Option<&str>,
        model: Option<&str>,
    ) -> Config {
        let mut config = Config::default();
        config
            .providers
            .insert(provider_name.to_string(), provider_entry);
        match capability {
            Capability::Summariser => {
                config.summariser.provider = ref_name.map(String::from);
                config.summariser.model = model.map(String::from);
            }
            Capability::Embedding => {
                config.embedding.provider = ref_name.map(String::from);
                config.embedding.model = model.map(String::from);
            }
            Capability::Reranker => {
                config.reranker.provider = ref_name.map(String::from);
                config.reranker.model = model.map(String::from);
            }
            Capability::HookPrompt => {
                config.hooks.prompt_provider = ref_name.map(String::from);
                config.hooks.prompt_model = model.map(String::from);
            }
        }
        config
    }

    // --- derive_env_var_name --------------------------------------------------

    #[test]
    fn derive_env_var_name_uppercases_and_sanitises() {
        assert_eq!(derive_env_var_name("myprov"), "TOME_MYPROV_API_KEY");
        assert_eq!(derive_env_var_name("my-prov"), "TOME_MY_PROV_API_KEY");
        assert_eq!(derive_env_var_name("my.prov.2"), "TOME_MY_PROV_2_API_KEY");
        assert_eq!(derive_env_var_name("Voyage AI"), "TOME_VOYAGE_AI_API_KEY");
    }

    // --- no provider referenced → Ok(None) -----------------------------------

    #[test]
    fn no_provider_referenced_is_ok_none() {
        let config = Config::default();
        for cap in [
            Capability::Summariser,
            Capability::Embedding,
            Capability::Reranker,
            Capability::HookPrompt,
        ] {
            assert!(resolve(&config, cap).unwrap().is_none(), "{cap}");
        }
    }

    // --- undefined provider reference → 93 -----------------------------------

    #[test]
    fn undefined_provider_reference_is_93() {
        let mut config = Config::default();
        // Reference a provider that isn't in the registry.
        config.summariser.provider = Some("ghost".into());
        config.summariser.model = Some("gpt-4o".into());
        let err = resolve(&config, Capability::Summariser).unwrap_err();
        assert_eq!(err.exit_code(), 93);
        let msg = err.to_string();
        assert!(msg.contains("ghost"), "{msg}");
        assert!(msg.contains("summariser"), "{msg}");
    }

    // --- illegal kind per capability → 93 ------------------------------------

    #[test]
    fn illegal_kind_summariser_voyage_is_93() {
        // voyage is not legal for summariser.
        let config = config_with(
            "p",
            entry(ProviderKind::Voyage, None, None),
            Capability::Summariser,
            Some("p"),
            Some("model"),
        );
        let err = resolve(&config, Capability::Summariser).unwrap_err();
        assert_eq!(err.exit_code(), 93);
        let msg = err.to_string();
        assert!(msg.contains("voyage"), "{msg}");
        assert!(msg.contains("summariser"), "{msg}");
    }

    #[test]
    fn illegal_kind_embedding_anthropic_is_93() {
        // anthropic is not legal for embedding (embedding ∈ {openai, voyage}).
        let config = config_with(
            "p",
            entry(ProviderKind::Anthropic, None, None),
            Capability::Embedding,
            Some("p"),
            Some("model"),
        );
        let err = resolve(&config, Capability::Embedding).unwrap_err();
        assert_eq!(err.exit_code(), 93);
    }

    #[test]
    fn illegal_kind_embedding_gemini_is_93() {
        let config = config_with(
            "p",
            entry(ProviderKind::Gemini, None, None),
            Capability::Embedding,
            Some("p"),
            Some("model"),
        );
        assert_eq!(
            resolve(&config, Capability::Embedding)
                .unwrap_err()
                .exit_code(),
            93
        );
    }

    #[test]
    fn illegal_kind_reranker_openai_is_93() {
        // reranker ∈ {voyage} only.
        for kind in [
            ProviderKind::Openai,
            ProviderKind::Anthropic,
            ProviderKind::Gemini,
        ] {
            let config = config_with(
                "p",
                entry(kind, None, None),
                Capability::Reranker,
                Some("p"),
                Some("model"),
            );
            assert_eq!(
                resolve(&config, Capability::Reranker)
                    .unwrap_err()
                    .exit_code(),
                93,
                "kind {kind:?} must be illegal for reranker"
            );
        }
    }

    #[test]
    fn legal_kinds_resolve_ok() {
        // Sanity: every legal (capability, kind) pair resolves to Some.
        let _g = EnvGuard::new(&["TOME_P_API_KEY", TIMEOUT_ENV_VAR]);
        let legal = [
            (Capability::Summariser, ProviderKind::Openai),
            (Capability::Summariser, ProviderKind::Anthropic),
            (Capability::Summariser, ProviderKind::Gemini),
            (Capability::Embedding, ProviderKind::Openai),
            (Capability::Embedding, ProviderKind::Voyage),
            (Capability::Reranker, ProviderKind::Voyage),
            (Capability::HookPrompt, ProviderKind::Openai),
            (Capability::HookPrompt, ProviderKind::Anthropic),
            (Capability::HookPrompt, ProviderKind::Gemini),
        ];
        for (cap, kind) in legal {
            let config = config_with("p", entry(kind, None, None), cap, Some("p"), Some("model"));
            assert!(
                resolve(&config, cap).unwrap().is_some(),
                "({cap}, {kind:?}) should resolve"
            );
        }
    }

    // --- provider without model → 93 -----------------------------------------

    #[test]
    fn provider_without_model_is_93() {
        let config = config_with(
            "p",
            entry(ProviderKind::Openai, None, None),
            Capability::Embedding,
            Some("p"),
            None, // no model
        );
        let err = resolve(&config, Capability::Embedding).unwrap_err();
        assert_eq!(err.exit_code(), 93);
        assert!(err.to_string().contains("no model"), "{err}");
    }

    // --- base_url default vs explicit ----------------------------------------

    #[test]
    fn base_url_defaults_per_kind() {
        let _g = EnvGuard::new(&["TOME_P_API_KEY", TIMEOUT_ENV_VAR]);
        let cases = [
            (
                ProviderKind::Openai,
                Capability::Summariser,
                "https://api.openai.com/v1",
            ),
            (
                ProviderKind::Anthropic,
                Capability::Summariser,
                "https://api.anthropic.com",
            ),
            (
                ProviderKind::Gemini,
                Capability::Summariser,
                "https://generativelanguage.googleapis.com",
            ),
            (
                ProviderKind::Voyage,
                Capability::Reranker,
                "https://api.voyageai.com/v1",
            ),
        ];
        for (kind, cap, expected) in cases {
            let config = config_with("p", entry(kind, None, None), cap, Some("p"), Some("model"));
            let resolved = resolve(&config, cap).unwrap().unwrap();
            assert_eq!(resolved.base_url, expected, "default for {kind:?}");
        }
    }

    #[test]
    fn explicit_base_url_trailing_slash_trimmed() {
        let _g = EnvGuard::new(&["TOME_P_API_KEY", TIMEOUT_ENV_VAR]);
        let config = config_with(
            "p",
            entry(
                ProviderKind::Openai,
                Some("http://localhost:11434/v1/"),
                None,
            ),
            Capability::Embedding,
            Some("p"),
            Some("model"),
        );
        let resolved = resolve(&config, Capability::Embedding).unwrap().unwrap();
        assert_eq!(resolved.base_url, "http://localhost:11434/v1");
    }

    // --- credential resolution: env wins, inline, none -----------------------

    #[test]
    fn env_var_wins_over_inline() {
        let g = EnvGuard::new(&["TOME_P_API_KEY", TIMEOUT_ENV_VAR]);
        g.set("TOME_P_API_KEY", "env-secret");
        let config = config_with(
            "p",
            entry(ProviderKind::Openai, None, Some("inline-secret")),
            Capability::Embedding,
            Some("p"),
            Some("model"),
        );
        let resolved = resolve(&config, Capability::Embedding).unwrap().unwrap();
        assert_eq!(resolved.credential.expose(), Some("env-secret"));
        assert!(resolved.credential.is_present());
    }

    #[test]
    fn inline_used_when_env_absent() {
        let _g = EnvGuard::new(&["TOME_P_API_KEY", TIMEOUT_ENV_VAR]);
        let config = config_with(
            "p",
            entry(ProviderKind::Openai, None, Some("inline-secret")),
            Capability::Embedding,
            Some("p"),
            Some("model"),
        );
        let resolved = resolve(&config, Capability::Embedding).unwrap().unwrap();
        assert_eq!(resolved.credential.expose(), Some("inline-secret"));
    }

    #[test]
    fn empty_env_falls_back_to_inline() {
        let g = EnvGuard::new(&["TOME_P_API_KEY", TIMEOUT_ENV_VAR]);
        g.set("TOME_P_API_KEY", ""); // present but empty → ignored
        let config = config_with(
            "p",
            entry(ProviderKind::Openai, None, Some("inline-secret")),
            Capability::Embedding,
            Some("p"),
            Some("model"),
        );
        let resolved = resolve(&config, Capability::Embedding).unwrap().unwrap();
        assert_eq!(resolved.credential.expose(), Some("inline-secret"));
    }

    #[test]
    fn no_credential_when_neither_env_nor_inline() {
        let _g = EnvGuard::new(&["TOME_P_API_KEY", TIMEOUT_ENV_VAR]);
        let config = config_with(
            "p",
            entry(ProviderKind::Openai, None, None),
            Capability::Embedding,
            Some("p"),
            Some("model"),
        );
        let resolved = resolve(&config, Capability::Embedding).unwrap().unwrap();
        assert_eq!(resolved.credential.expose(), None);
        assert!(!resolved.credential.is_present());
    }

    #[test]
    fn generic_provider_env_var_is_never_read() {
        // FR-008: a generic OPENAI_API_KEY must NOT be picked up — only the
        // derived TOME_<NAME>_API_KEY.
        let g = EnvGuard::new(&["TOME_P_API_KEY", "OPENAI_API_KEY", TIMEOUT_ENV_VAR]);
        g.set("OPENAI_API_KEY", "should-be-ignored");
        let config = config_with(
            "p",
            entry(ProviderKind::Openai, None, None),
            Capability::Embedding,
            Some("p"),
            Some("model"),
        );
        let resolved = resolve(&config, Capability::Embedding).unwrap().unwrap();
        assert_eq!(resolved.credential.expose(), None);
    }

    // --- timeout default / override ------------------------------------------

    #[test]
    fn timeout_defaults_when_env_unset() {
        let _g = EnvGuard::new(&["TOME_P_API_KEY", TIMEOUT_ENV_VAR]);
        let config = config_with(
            "p",
            entry(ProviderKind::Openai, None, None),
            Capability::Embedding,
            Some("p"),
            Some("model"),
        );
        let resolved = resolve(&config, Capability::Embedding).unwrap().unwrap();
        assert_eq!(resolved.timeout, DEFAULT_PROVIDER_TIMEOUT);
    }

    #[test]
    fn timeout_env_override_is_honoured() {
        let g = EnvGuard::new(&["TOME_P_API_KEY", TIMEOUT_ENV_VAR]);
        g.set(TIMEOUT_ENV_VAR, "5");
        let config = config_with(
            "p",
            entry(ProviderKind::Openai, None, None),
            Capability::Embedding,
            Some("p"),
            Some("model"),
        );
        let resolved = resolve(&config, Capability::Embedding).unwrap().unwrap();
        assert_eq!(resolved.timeout, Duration::from_secs(5));
    }

    #[test]
    fn timeout_unparsable_env_falls_back_to_default() {
        let g = EnvGuard::new(&["TOME_P_API_KEY", TIMEOUT_ENV_VAR]);
        g.set(TIMEOUT_ENV_VAR, "not-a-number");
        let config = config_with(
            "p",
            entry(ProviderKind::Openai, None, None),
            Capability::Embedding,
            Some("p"),
            Some("model"),
        );
        let resolved = resolve(&config, Capability::Embedding).unwrap().unwrap();
        assert_eq!(resolved.timeout, DEFAULT_PROVIDER_TIMEOUT);
    }

    // --- env-var collision warning (non-fatal) -------------------------------

    #[test]
    fn env_var_collision_does_not_fail_resolution() {
        // `my-prov` and `my.prov` both derive TOME_MY_PROV_API_KEY. The
        // collision warns (we can't easily assert the log here) but must NOT
        // fail resolution.
        let _g = EnvGuard::new(&["TOME_MY_PROV_API_KEY", TIMEOUT_ENV_VAR]);
        assert_eq!(
            derive_env_var_name("my-prov"),
            derive_env_var_name("my.prov"),
            "precondition: both names collide"
        );
        let mut config = Config::default();
        config
            .providers
            .insert("my-prov".into(), entry(ProviderKind::Openai, None, None));
        config
            .providers
            .insert("my.prov".into(), entry(ProviderKind::Openai, None, None));
        config.embedding.provider = Some("my-prov".into());
        config.embedding.model = Some("model".into());
        // Resolution succeeds despite the collision.
        assert!(resolve(&config, Capability::Embedding).unwrap().is_some());
    }

    // --- Credential Debug redacts --------------------------------------------

    #[test]
    fn credential_debug_redacts() {
        let present = Credential::present(Secret::from("topsecret".to_string()));
        let absent = Credential::absent();
        let dbg_present = format!("{present:?}");
        let dbg_absent = format!("{absent:?}");
        assert!(
            !dbg_present.contains("topsecret"),
            "Credential Debug leaked the secret: {dbg_present}"
        );
        assert_eq!(dbg_present, "Credential(<present>)");
        assert_eq!(dbg_absent, "Credential(<absent>)");
    }

    #[test]
    fn resolved_provider_debug_does_not_leak_credential() {
        // An INLINE api_key must NOT appear in the hand-written redacting Debug;
        // the credential renders via `Credential`'s own `<present>` token.
        let _g = EnvGuard::new(&["TOME_P_API_KEY", TIMEOUT_ENV_VAR]);
        let config = config_with(
            "p",
            entry(ProviderKind::Openai, None, Some("leaky-secret")),
            Capability::Embedding,
            Some("p"),
            Some("model"),
        );
        let resolved = resolve(&config, Capability::Embedding).unwrap().unwrap();
        let dbg = format!("{resolved:?}");
        assert!(
            !dbg.contains("leaky-secret"),
            "ResolvedProvider Debug leaked the credential: {dbg}"
        );
        // The redaction is positive (the credential is rendered, just as the
        // redacting `<present>` token) and the other fields are still present.
        assert!(
            dbg.contains("Credential(<present>)"),
            "credential must render via its redacting Debug: {dbg}"
        );
        assert!(dbg.contains("ResolvedProvider"), "{dbg}");
        assert!(dbg.contains("\"p\""), "name should be present: {dbg}");
    }
}
