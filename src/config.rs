//! Tome's unified global configuration document (`~/.tome/config.toml`).
//! One typed, strict (`deny_unknown_fields`) struct: how Tome behaves globally.
//! Env vars override these values at each consumer (see per-knob precedence in
//! the design doc); the file is the persistent middle layer.
//!
//! `CatalogEntry` lives here for historical reasons — the root `[catalogs]`
//! registry is gone (the DB `workspace_catalogs` table is authoritative), but
//! `settings::WorkspaceSettings` still embeds `CatalogEntry`.

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
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TelemetryConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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
        assert_eq!(c.output.progress, Some(false));
        assert_eq!(c.summariser.enabled, Some(false));
        assert_eq!(c.harness.expose_agents_as_personas, Some(true));
        assert_eq!(c.harness.strip_plugin_agent_privileges, Some(false));
        assert_eq!(c.workspace.default.as_deref(), Some("work"));
        assert!((c.query.strict_min_score.unwrap() - 0.7_f32).abs() < 1e-6);
    }

    #[test]
    fn unknown_section_field_rejected() {
        let err = toml::from_str::<Config>("[query]\nnope = 1\n").unwrap_err();
        assert!(err.to_string().to_lowercase().contains("unknown"));
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
}
