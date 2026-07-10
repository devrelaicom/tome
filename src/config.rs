//! Tome's unified global configuration document (`~/.tome/config.toml`).
//! One typed, strict (`deny_unknown_fields`) struct: how Tome behaves globally.
//! Env vars override these values at each consumer (see per-knob precedence in
//! the design doc); the file is the persistent middle layer.
//!
//! `CatalogEntry` lives here for historical reasons — the root `[catalogs]`
//! registry is gone (the DB `workspace_catalogs` table is authoritative), but
//! `settings::WorkspaceSettings` still embeds `CatalogEntry`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::error::TomeError;
use crate::paths::Paths;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub name: String,
    pub url: String,
    #[serde(rename = "ref")]
    pub ref_: String,
    pub path: PathBuf,
    #[serde(with = "time::serde::rfc3339")]
    pub last_synced: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub harness: HarnessConfig,
    #[serde(default)]
    pub query: QueryConfig,
    #[serde(default)]
    pub summariser: SummariserConfig,
    #[serde(default)]
    pub embedding: EmbeddingConfig,
    #[serde(default)]
    pub reranker: RerankerConfig,
    #[serde(default)]
    pub telemetry: TelemetryConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub output: OutputConfig,
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub models: ModelsConfig,
    #[serde(default)]
    pub doctor: DoctorConfig,
    #[serde(default)]
    pub hooks: HooksConfig,

    /// Phase 12 — BYOK/BYOM model providers. A registry of external providers
    /// keyed by user-chosen name; capability sections (`[summariser]`,
    /// `[embedding]`, `[reranker]`) reference an entry by name via their
    /// `provider` field. Empty by default → bundled local models everywhere.
    /// Serialises to `[providers.<name>]` tables.
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderEntry>,

    // Robustness, not migration: silently accept-and-drop a legacy [catalogs]
    // table so a pre-Phase-4 config.toml doesn't hard-fail the strict parse.
    // Never serialized back (`skip_serializing`) → dropped on the next write.
    #[serde(default, skip_serializing, rename = "catalogs")]
    _legacy_catalogs: Option<toml::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HarnessConfig {
    /// Harnesses active at the global scope (was settings.toml `harnesses`).
    /// `Option` is load-bearing: `None` = "not declared" (layer abstains),
    /// `Some([])` = "declared empty" — the composition resolver distinguishes them.
    #[serde(default)]
    pub enabled: Option<Vec<String>>,
    #[serde(default)]
    pub expose_agents_as_personas: Option<bool>,
    #[serde(default)]
    pub strip_plugin_agent_privileges: Option<bool>,
    /// Default target for `tome harness use`/`remove` when `--scope` is omitted.
    #[serde(default)]
    pub default_scope: Option<HarnessScope>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct QueryConfig {
    #[serde(default)]
    pub top_k: Option<u32>,
    #[serde(default)]
    pub rerank: Option<bool>,
    #[serde(default)]
    pub strict_min_score: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SummariserConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub long_max_chars: Option<usize>,
    /// Phase 12 — name of a `[providers.<name>]` entry to summarise with.
    /// Omitted → the bundled Qwen2.5-0.5B summariser. When set, `model` is
    /// required (validated at resolve time → exit 93).
    #[serde(default)]
    pub provider: Option<String>,
    /// Phase 12 — the remote model identifier (required when `provider` is set).
    #[serde(default)]
    pub model: Option<String>,
}

/// Phase 12 — `[embedding]`. Points the embedding capability at an external
/// provider. Absent / `provider` omitted → the bundled `bge-small` embedder
/// selected by `[models] profile`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingConfig {
    /// Name of a `[providers.<name>]` entry. Allowed kinds: openai, voyage.
    #[serde(default)]
    pub provider: Option<String>,
    /// The remote model identifier (required when `provider` is set).
    #[serde(default)]
    pub model: Option<String>,
    /// When set, the authoritative expected output vector length — a remote
    /// embedding whose length differs is rejected (`RemoteEmbeddingInvalid`).
    #[serde(default)]
    pub dimensions: Option<u32>,
}

/// Phase 12 — `[reranker]`. Points the reranking capability at an external
/// provider (Voyage only in v1). Absent / `provider` omitted → the bundled
/// `bge-reranker` selected by `[models] profile`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RerankerConfig {
    /// Name of a `[providers.<name>]` entry. Allowed kind: voyage.
    #[serde(default)]
    pub provider: Option<String>,
    /// The remote model identifier (required when `provider` is set).
    #[serde(default)]
    pub model: Option<String>,
}

impl RerankerConfig {
    /// Whether a `[reranker]` provider (with model) is configured — the
    /// "implicit enable" signal for reranking (#502). Configuring a reranker
    /// backend is a clear intent to use one, so this turns reranking on even
    /// when `[query] rerank` is unset. Requires BOTH `provider` and `model`;
    /// a half-configured `[reranker]` (provider without model) does not flip
    /// reranking on (and would surface as `ProviderConfigInvalid`/93 at resolve
    /// time if actually used). SSOT for the CLI resolver, `tome config show`,
    /// and the MCP `search_skills` default.
    pub fn is_provider_configured(&self) -> bool {
        self.provider.is_some() && self.model.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TelemetryConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Override the Gauge collector endpoint. `TOME_GAUGE_ENDPOINT` env wins
    /// over this; default is the pinned prod URL. Must be https or loopback
    /// (validated by the kernel at handle build).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LoggingConfig {
    #[serde(default)]
    pub level: Option<LogLevel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OutputConfig {
    #[serde(default)]
    pub color: Option<ColorMode>,
    #[serde(default)]
    pub progress: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceConfig {
    #[serde(default)]
    pub default: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct McpConfig {
    #[serde(default)]
    pub description_max_chars: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelsConfig {
    #[serde(default)]
    pub profile: Option<crate::embedding::Profile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorConfig {
    #[serde(default)]
    pub verify_by_default: Option<bool>,
}

/// US6 — `[hooks]`. Controls plugin-hook translation and the BYOM prompt model.
/// All fields are absent by default (opt-in semantics for the prompt sub-feature;
/// `translate_plugin_hooks` defaults to `true` when absent so the feature stays on).
///
/// `translate_plugin_hooks = false` disables the whole dispatch subsystem and
/// tears down any previously written run-hook entries + manifests on the next sync.
/// `prompt_provider`/`prompt_model` wire a BYOM LLM for `Handler::Prompt` hooks;
/// when absent, prompt handlers in the manifest are suppressed by the sync gate.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct HooksConfig {
    /// When `Some(false)`, the plugin-hook dispatch subsystem is disabled and
    /// the reconciler removes any existing Tome run-hook entries and manifests.
    /// Absent or `Some(true)` → enabled (the default).
    #[serde(default)]
    pub translate_plugin_hooks: Option<bool>,
    /// Name of a `[providers.<name>]` entry to use for prompt handler dispatch.
    /// When set, `prompt_model` is also required (validated at resolve time → exit 93).
    #[serde(default)]
    pub prompt_provider: Option<String>,
    /// Remote model identifier for prompt handler dispatch (required when `prompt_provider` is set).
    #[serde(default)]
    pub prompt_model: Option<String>,
}

/// Phase 12 — a redacting wrapper around an inline API key.
///
/// Deserialises from a plain TOML string and serialises back to its real value,
/// so a `config.toml` carrying an inline `api_key` round-trips losslessly. The
/// **`Debug` and `Display` impls are hand-written to redact** — neither ever
/// renders the inner value, so a stray `{:?}`/`{}` in a log line, an error
/// chain, or a panic message cannot leak the credential. The one consumer that
/// genuinely needs the bytes calls [`Secret::expose`] explicitly, which makes
/// every real-value access greppable.
///
/// `Debug` is intentionally NOT derived (a derive would print the inner string);
/// `Clone`/`PartialEq`/`Eq` are safe to derive (they don't render).
///
/// ## ⚠️ Serialize asymmetry — credential-leak vector
///
/// `#[serde(transparent)]` makes `Serialize` emit the **real** value (required
/// for lossless `config.toml` round-trips). So redaction protects `Debug`/
/// `Display` ONLY — not `Serialize`. Today the only `Serialize` consumer of
/// `ProviderEntry`/`Secret` is on-disk `config.toml` persistence (the
/// legitimate home for an inline key). Any NEW `Serialize` surface that reaches
/// a user-facing channel — a `tome config show --json`, a `doctor` provider
/// dump, a telemetry event — would leak the inline `api_key` silently. Such a
/// surface MUST redact (serialise a masked form / the `Credential`, never a raw
/// `ProviderEntry`).
// not-strict: `#[serde(transparent)]` newtype over a String — it has no named
// fields, so `#[serde(deny_unknown_fields)]` is inapplicable (and rejected by
// serde). Exempt from the `manifest_strictness` deny-unknown-fields gate.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)] // not-strict
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    /// The real, unredacted credential. The only path to the inner value —
    /// every call site is an explicit, auditable "I need the actual secret".
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl From<String> for Secret {
    fn from(s: String) -> Self {
        Secret(s)
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Mirror the derived tuple-struct shape but redact the payload.
        f.write_str("Secret(\"***redacted***\")")
    }
}

impl std::fmt::Display for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("***redacted***")
    }
}

/// Phase 12 — the kind of external provider a `[providers.<name>]` entry names.
/// Fixes the wire shape, default base URL, and credential placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum ProviderKind {
    Openai,
    Anthropic,
    Gemini,
    Voyage,
}

impl ProviderKind {
    /// The stable lowercase token for this kind — the `kind = "…"` wire value,
    /// reused by messages and telemetry. Byte-identical to the serde rename.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKind::Openai => "openai",
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Gemini => "gemini",
            ProviderKind::Voyage => "voyage",
        }
    }
}

/// Phase 12 — one `[providers.<name>]` registry entry.
///
/// The registry name (the map key) is documented to derive an env-var override
/// `TOME_<NAME>_API_KEY`; credential resolution is env → inline `api_key` →
/// none. `base_url` defaults per [`ProviderKind`] when omitted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderEntry {
    pub kind: ProviderKind,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<Secret>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    /// The `tracing_subscriber::EnvFilter` directive for this level.
    pub fn as_directive(self) -> &'static str {
        match self {
            LogLevel::Off => "off",
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum HarnessScope {
    Project,
    Global,
}

/// Strict load of `~/.tome/config.toml`. Missing file → defaults; a malformed
/// file → `ManifestInvalid::TomlParse` (exit 5) — the same code catalog
/// manifests use. Commands call this so a typo fails loudly.
pub fn load(paths: &Paths) -> Result<Config, TomeError> {
    match crate::util::bounded_read_to_string(
        &paths.global_config_file,
        crate::util::TOME_CONFIG_MAX,
    ) {
        Ok(text) => toml::from_str(&text).map_err(|e| {
            TomeError::ManifestInvalid(crate::error::ManifestInvalid::TomlParse {
                file: paths.global_config_file.clone(),
                message: e.to_string(),
            })
        }),
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(e),
    }
}

/// Defensive load for the telemetry silent path (reached from every command and
/// the detached flusher). Any error → defaults; never propagates, never panics,
/// so a malformed `config.toml` can't brick an unrelated command via the
/// telemetry enqueue hook. (Strict surfacing happens via `load` in commands.)
pub fn load_or_default(paths: &Paths) -> Config {
    load(paths).unwrap_or_default()
}

/// Read-only health probe for `~/.tome/config.toml`: `None` when the file is
/// absent or parses cleanly, `Some(message)` when the strict parse fails. The
/// message is the **same** legible diagnostic the strict [`load`] path surfaces
/// (file + the `toml` serde error, which names the offending key/section and its
/// line/column). It is the single source of truth so the resilient diagnostic
/// commands (`tome doctor`, `tome status`) report a malformed config identically
/// to the loud exit-5 every other command emits.
///
/// Reaching into [`load`] keeps the "what counts as malformed" rule in one
/// place: any future loosening of the strict parse (e.g. another
/// tolerate-and-drop section like `_legacy_catalogs`) is reflected here for free.
pub fn probe_error(paths: &Paths) -> Option<String> {
    // Credential-scrub EVERY message this SSOT returns. `toml` 0.8 renders a
    // source-context snippet of the offending line, so a syntax/type/value error
    // on or near an inline `api_key = "sk-…"` (or a reflected provider key) would
    // otherwise echo that credential — now via a scriptable `tome config
    // validate --json` stdout sink AND the pre-existing `doctor`/`status`
    // diagnostics. Routing through the ONE shared scrubber here fixes all of them
    // at the chokepoint. The scrubber redacts credential-SHAPED tokens
    // (`sk-…`/`pa-…`/`AIza…`, `api_key=…`, `Bearer …`, URL userinfo), NOT plain
    // identifiers, so "unknown field `nope`" survives verbatim.
    fn scrub(msg: String) -> String {
        String::from_utf8_lossy(&crate::catalog::git::scrub_credentials(msg.as_bytes()))
            .into_owned()
    }
    match load(paths) {
        Ok(_) => None,
        // A parse failure carries the legible message (key/section/line/column).
        Err(TomeError::ManifestInvalid(crate::error::ManifestInvalid::TomlParse {
            message,
            ..
        })) => Some(scrub(message)),
        // An I/O failure other than NotFound (which `load` already maps to
        // defaults) — surface it too so the user isn't left guessing; it is far
        // rarer than a typo but equally worth reporting in a diagnostic.
        Err(e) => Some(scrub(e.to_string())),
    }
}

/// Defensive config load given a known tome root (the directory holding
/// `config.toml`). Mirrors [`load_or_default`] but for callers that already
/// know the root path rather than going through a [`Paths`] struct —
/// e.g. `index::db::open`, which derives the root from the DB path. Any
/// error (missing file, I/O failure, malformed TOML) → [`Config::default`].
pub fn load_or_default_from_root(root: &std::path::Path) -> Config {
    let path = root.join("config.toml");
    match crate::util::bounded_read_to_string(&path, crate::util::TOME_CONFIG_MAX) {
        Ok(text) => toml::from_str(&text).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

#[cfg(test)]
mod load_or_default_from_root_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn present_config_file_parses_profile() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "[models]\nprofile = \"small\"\n",
        )
        .unwrap();
        let cfg = load_or_default_from_root(dir.path());
        assert_eq!(cfg.models.profile, Some(crate::embedding::Profile::Small));
    }

    #[test]
    fn absent_file_returns_default() {
        let dir = TempDir::new().unwrap();
        // No config.toml written.
        let cfg = load_or_default_from_root(dir.path());
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn malformed_file_returns_default() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("config.toml"), "this = is = broken\n").unwrap();
        let cfg = load_or_default_from_root(dir.path());
        assert_eq!(cfg, Config::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn paths_in(dir: &TempDir) -> crate::paths::Paths {
        crate::paths::Paths::from_root(dir.path().to_path_buf())
    }

    #[test]
    fn default_config_round_trips() {
        let c = Config::default();
        let s = toml::to_string(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn full_config_parses() {
        let toml = r#"
[harness]
enabled = ["claude-code", "codex"]
expose_agents_as_personas = true
strip_plugin_agent_privileges = false
default_scope = "global"

[query]
top_k = 15
rerank = false
strict_min_score = 0.7

[summariser]
enabled = false
long_max_chars = 4000
provider = "myprov"
model = "gpt-4o-mini"

[embedding]
provider = "myprov"
model = "text-embedding-3-small"
dimensions = 1536

[reranker]
provider = "voyageprov"
model = "rerank-2"

[providers.myprov]
kind = "openai"
base_url = "http://localhost:11434/v1"
api_key = "sk-test"

[providers.voyageprov]
kind = "voyage"

[telemetry]
enabled = false

[logging]
level = "info"

[output]
color = "never"
progress = false

[workspace]
default = "work"

[mcp]
description_max_chars = 300

[models]
profile = "small"

[doctor]
verify_by_default = true

[hooks]
translate_plugin_hooks = true
prompt_provider = "myprov"
prompt_model = "gpt-4o-mini"
"#;
        let c: Config = toml::from_str(toml).unwrap();
        assert_eq!(
            c.harness.enabled.as_deref(),
            Some(&["claude-code".to_string(), "codex".to_string()][..])
        );
        assert_eq!(c.harness.default_scope, Some(HarnessScope::Global));
        assert_eq!(c.query.top_k, Some(15));
        assert_eq!(c.query.rerank, Some(false));
        assert_eq!(c.summariser.long_max_chars, Some(4000));
        assert_eq!(c.telemetry.enabled, Some(false));
        assert_eq!(c.logging.level, Some(LogLevel::Info));
        assert_eq!(c.output.color, Some(ColorMode::Never));
        assert_eq!(c.mcp.description_max_chars, Some(300));
        assert_eq!(c.doctor.verify_by_default, Some(true));
        assert_eq!(c.models.profile, Some(crate::embedding::Profile::Small));

        // US6 — hooks config.
        assert_eq!(c.hooks.translate_plugin_hooks, Some(true));
        assert_eq!(c.hooks.prompt_provider.as_deref(), Some("myprov"));
        assert_eq!(c.hooks.prompt_model.as_deref(), Some("gpt-4o-mini"));
        assert_eq!(c.output.progress, Some(false));
        assert_eq!(c.summariser.enabled, Some(false));
        assert_eq!(c.harness.expose_agents_as_personas, Some(true));
        assert_eq!(c.harness.strip_plugin_agent_privileges, Some(false));
        assert_eq!(c.workspace.default.as_deref(), Some("work"));
        assert!((c.query.strict_min_score.unwrap() - 0.7_f32).abs() < 1e-6);

        // Phase 12 — summariser provider/model.
        assert_eq!(c.summariser.provider.as_deref(), Some("myprov"));
        assert_eq!(c.summariser.model.as_deref(), Some("gpt-4o-mini"));

        // Phase 12 — embedding section.
        assert_eq!(c.embedding.provider.as_deref(), Some("myprov"));
        assert_eq!(c.embedding.model.as_deref(), Some("text-embedding-3-small"));
        assert_eq!(c.embedding.dimensions, Some(1536));

        // Phase 12 — reranker section.
        assert_eq!(c.reranker.provider.as_deref(), Some("voyageprov"));
        assert_eq!(c.reranker.model.as_deref(), Some("rerank-2"));

        // Phase 12 — the provider registry parses each entry's kind/base_url/key.
        assert_eq!(c.providers.len(), 2);
        let myprov = c.providers.get("myprov").expect("myprov entry");
        assert_eq!(myprov.kind, ProviderKind::Openai);
        assert_eq!(
            myprov.base_url.as_deref(),
            Some("http://localhost:11434/v1")
        );
        assert_eq!(myprov.api_key.as_ref().map(Secret::expose), Some("sk-test"));
        let voyage = c.providers.get("voyageprov").expect("voyageprov entry");
        assert_eq!(voyage.kind, ProviderKind::Voyage);
        assert_eq!(voyage.base_url, None);
        assert_eq!(voyage.api_key, None);
    }

    #[test]
    fn unknown_section_field_rejected() {
        let err = toml::from_str::<Config>("[query]\nnope = 1\n").unwrap_err();
        assert!(err.to_string().to_lowercase().contains("unknown"));
    }

    #[test]
    fn providers_unknown_field_rejected() {
        // `ProviderEntry` is Tome-owned and strict — an unknown key fails the
        // parse (exit 5), not a silent accept.
        let err = toml::from_str::<Config>("[providers.x]\nkind=\"openai\"\nnope=1\n").unwrap_err();
        assert!(err.to_string().to_lowercase().contains("unknown"), "{err}");
    }

    #[test]
    fn secret_debug_and_display_redact() {
        let s = Secret::from("sk-abc123".to_string());
        let dbg = format!("{s:?}");
        let disp = format!("{s}");
        assert!(
            !dbg.contains("sk-abc123"),
            "Debug must redact the inner value: {dbg}"
        );
        assert!(
            !disp.contains("sk-abc123"),
            "Display must redact the inner value: {disp}"
        );
        // The redacted markers are present, and `expose()` still returns the real
        // value for the one consumer that needs it.
        assert!(dbg.contains("redacted"), "{dbg}");
        assert!(disp.contains("redacted"), "{disp}");
        assert_eq!(s.expose(), "sk-abc123");
    }

    #[test]
    fn legacy_catalogs_table_tolerated_and_dropped() {
        // A pre-Phase-4 config.toml carrying the dead [catalogs] registry must
        // not hard-fail the strict parse, and must not be written back.
        let toml = r#"
[catalogs.foo]
name = "foo"
url = "https://example/"
ref = "main"
path = "/x"
last_synced = "2026-01-01T00:00:00Z"
"#;
        let c: Config = toml::from_str(toml).unwrap();
        let back = toml::to_string(&c).unwrap();
        assert!(
            !back.contains("catalogs"),
            "legacy catalogs must be dropped on serialize: {back}"
        );
    }

    #[test]
    fn load_missing_file_is_default() {
        let dir = TempDir::new().unwrap();
        assert_eq!(load(&paths_in(&dir)).unwrap(), Config::default());
    }

    #[test]
    fn load_malformed_is_exit_5() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        std::fs::create_dir_all(paths.global_config_file.parent().unwrap()).unwrap();
        std::fs::write(&paths.global_config_file, "this = is = broken").unwrap();
        let err = load(&paths).unwrap_err();
        assert_eq!(err.exit_code(), 5);
    }

    #[test]
    fn load_or_default_swallows_malformed() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        std::fs::create_dir_all(paths.global_config_file.parent().unwrap()).unwrap();
        std::fs::write(&paths.global_config_file, "this = is = broken").unwrap();
        assert_eq!(load_or_default(&paths), Config::default()); // never panics
    }

    #[test]
    fn probe_error_none_when_absent() {
        let dir = TempDir::new().unwrap();
        assert_eq!(probe_error(&paths_in(&dir)), None);
    }

    #[test]
    fn probe_error_none_when_well_formed() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        std::fs::create_dir_all(paths.global_config_file.parent().unwrap()).unwrap();
        std::fs::write(&paths.global_config_file, "[query]\ntop_k = 5\n").unwrap();
        assert_eq!(probe_error(&paths), None);
    }

    #[test]
    fn probe_error_names_unknown_key_and_section() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        std::fs::create_dir_all(paths.global_config_file.parent().unwrap()).unwrap();
        std::fs::write(&paths.global_config_file, "[query]\nnope = 1\n").unwrap();
        let msg = probe_error(&paths).expect("malformed config probed");
        // The message names the offending key and is the same legible toml
        // diagnostic the strict `load` path surfaces (line/column included).
        assert!(msg.contains("nope"), "must name the offending key: {msg}");
        assert!(msg.to_lowercase().contains("unknown"), "{msg}");
        // It must match what strict `load` emits, byte-for-byte, so the
        // diagnostic surfaces and the loud exit-5 stay in lockstep.
        let strict = match load(&paths) {
            Err(TomeError::ManifestInvalid(crate::error::ManifestInvalid::TomlParse {
                message,
                ..
            })) => message,
            other => panic!("expected TomlParse, got {other:?}"),
        };
        assert_eq!(msg, strict);
    }

    /// Issue #286 (review item 1 — credential leak via the diagnostic): when the
    /// parse error is ON an inline `api_key = "sk-…"` line, `toml` 0.8 renders a
    /// source-context snippet that echoes that whole line verbatim — secret
    /// included. `probe_error` must route the message through the shared
    /// credential scrubber so the key never reaches the (now scriptable) `tome
    /// config validate --json` / `doctor` / `status` diagnostic sinks. A duplicate
    /// `api_key` is a deterministic way to put the error on that exact line: toml
    /// reports "duplicate key" and renders the offending `api_key = "sk-…"` line.
    #[test]
    fn probe_error_scrubs_inline_api_key_in_snippet() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        std::fs::create_dir_all(paths.global_config_file.parent().unwrap()).unwrap();
        std::fs::write(
            &paths.global_config_file,
            "[providers.p]\nkind = \"openai\"\napi_key = \"sk-leaktest0123456789abcdef\"\napi_key = \"sk-second00123456789abcdef\"\n",
        )
        .unwrap();
        let msg = probe_error(&paths).expect("malformed config probed");
        assert!(
            !msg.contains("sk-leaktest0123456789abcdef"),
            "the inline api_key must NOT leak into the diagnostic: {msg}"
        );
        assert!(
            !msg.contains("sk-second00123456789abcdef"),
            "the second inline api_key must NOT leak either: {msg}"
        );
        // The scrubbed marker is present where the key was.
        assert!(
            msg.contains("<scrubbed>"),
            "expected a redaction marker: {msg}"
        );
        // The legible diagnostic itself survives (the reason is still reported).
        assert!(
            msg.to_lowercase().contains("duplicate") || msg.to_lowercase().contains("key"),
            "the parse reason must survive scrubbing: {msg}"
        );
    }

    /// The scrubbing must NOT eat plain field names — the legible "unknown field
    /// `nope`" diagnostic (the whole point of `probe_error`) must survive.
    #[test]
    fn probe_error_scrub_preserves_plain_field_names() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        std::fs::create_dir_all(paths.global_config_file.parent().unwrap()).unwrap();
        std::fs::write(&paths.global_config_file, "[query]\nnope = 1\n").unwrap();
        let msg = probe_error(&paths).expect("malformed config probed");
        assert!(msg.contains("nope"), "plain field name must survive: {msg}");
        assert!(msg.to_lowercase().contains("unknown"), "{msg}");
    }

    /// Issue #287 (review item 1): `probe_error`'s NON-`TomlParse` fallback arm.
    /// A config file that exceeds `TOME_CONFIG_MAX` is refused by
    /// `bounded_read_to_string` with a `TomeError::Io` BEFORE any TOML parse, so
    /// `load` propagates an `Io` (not `TomlParse`). `probe_error` must still
    /// report it (`Some`, surfaced not swallowed) and must NOT panic. This pins
    /// the "I/O / unreadable error is reported" branch.
    #[test]
    fn probe_error_surfaces_io_error_for_oversize_file() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        std::fs::create_dir_all(paths.global_config_file.parent().unwrap()).unwrap();
        // One byte over the 1 MiB config cap. The content is valid UTF-8 (and
        // valid TOML comment text) in isolation; the size cap is what trips the
        // read, so the fallback arm is exercised independently of TOML parsing.
        let oversize = "#".repeat((crate::util::TOME_CONFIG_MAX as usize) + 1);
        std::fs::write(&paths.global_config_file, oversize).unwrap();

        // Confirm `load` itself returns an `Io` error (not `TomlParse`), so this
        // test genuinely covers the fallback arm and not the parse arm.
        match load(&paths) {
            Err(TomeError::Io(_)) => {}
            other => panic!("expected an Io error for an over-cap config, got {other:?}"),
        }

        let msg = probe_error(&paths).expect("an over-cap config must be reported, not swallowed");
        assert!(
            msg.to_lowercase().contains("cap") || msg.to_lowercase().contains("exceed"),
            "the I/O message should explain the size refusal: {msg}",
        );
    }
}
