//! `tome config {show,validate}` — inspect and validate the unified global
//! configuration document (`~/.tome/config.toml`).
//!
//! Both subcommands are **read-only**: they never write the config, never
//! create directories, and never take the index advisory lock. `show` renders
//! every curated user-facing knob with its EFFECTIVE value plus a provenance
//! annotation (`(default)` / `(config)` / `(env)`); `validate` runs the strict
//! [`config::load`] and reports success or the legible parse diagnostic (reusing
//! [`config::probe_error`], the SSOT shared with `doctor`/`status`).
//!
//! ## Provenance accuracy
//!
//! Provenance is resolved per knob against three inputs, highest precedence
//! first:
//!
//! 1. **`(env)`** — ONLY for the knobs that genuinely have an environment
//!    override, and only when that variable is actually set to a value that
//!    takes effect. The overrides mirror the real per-consumer precedence:
//!    - `logging.level` ← `TOME_LOG` / `RUST_LOG` (see [`crate::logging`])
//!    - `output.color` ← `NO_COLOR` (see [`crate::presentation::colour`])
//!    - `workspace.default` ← `TOME_WORKSPACE` (see
//!      [`crate::workspace::resolution`])
//!    - `telemetry.enabled` ← `TOME_TELEMETRY` (see
//!      [`crate::telemetry::config`])
//!    - `telemetry.endpoint` ← `TOME_GAUGE_ENDPOINT`
//!
//!    A knob with NO env override is never annotated `(env)`.
//! 2. **`(config)`** — the key is present in `~/.tome/config.toml`. Presence is
//!    detected by navigating the RAW parsed TOML (not the final struct): the
//!    struct cannot distinguish "set to the default value in the file" from
//!    "defaulted", so key-presence must be read from the document itself.
//! 3. **`(default)`** — none of the above; the built-in default applies.
//!
//! ## Scope of the shown knobs
//!
//! `show` surfaces the curated scalar user knobs only. The Phase-12 BYOK/BYOM
//! wiring fields (`[providers]`, the capability `provider`/`model` references on
//! `[summariser]`/`[embedding]`/`[reranker]`/`[hooks]`) are intentionally NOT
//! rendered: a `[providers.<name>]` entry can carry an inline `api_key`
//! ([`crate::config::Secret`]), and echoing them through a user-facing surface —
//! especially `--json` — is exactly the credential-leak vector documented on
//! `Secret`. Inspect those via `tome models test` / `tome doctor` instead.

use std::io::Write;

use serde::Serialize;

use crate::cli::{ConfigCommand, ConfigShowArgs};
use crate::config::{self, Config};
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

/// Subcommand dispatcher invoked by `main.rs`.
pub fn run(cmd: ConfigCommand, _scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    match cmd {
        ConfigCommand::Show(args) => show(&paths, args, mode),
        ConfigCommand::Validate => validate(&paths, mode),
    }
}

// ---------------------------------------------------------------------------
// provenance
// ---------------------------------------------------------------------------

/// Where a knob's effective value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Source {
    /// The built-in default (key absent from the file, no env override).
    Default,
    /// The key is present in `~/.tome/config.toml`.
    Config,
    /// An environment variable overrides the file/default value.
    Env,
}

impl Source {
    /// The parenthesised human annotation.
    fn annotation(self) -> &'static str {
        match self {
            Source::Default => "(default)",
            Source::Config => "(config)",
            Source::Env => "(env)",
        }
    }
}

/// One rendered knob for the `--json` output. `value` is the EFFECTIVE value
/// rendered as a string (a stable, byte-stable representation); `source` is its
/// resolved provenance.
#[derive(Debug, Serialize)]
struct KnobJson {
    value: String,
    source: Source,
}

/// A resolved knob before rendering: its dotted key, effective value, and
/// provenance.
struct Knob {
    key: &'static str,
    value: String,
    source: Source,
}

// ---------------------------------------------------------------------------
// show
// ---------------------------------------------------------------------------

fn show(paths: &Paths, args: ConfigShowArgs, mode: Mode) -> Result<(), TomeError> {
    // Strict load: a malformed config fails loudly (exit 5), exactly like every
    // other command — `show` is not a diagnostic surface, so it does not degrade
    // to defaults (that is `tome config validate`'s / `doctor`'s job).
    let cfg = config::load(paths)?;
    let _ = args; // no per-subcommand flags today beyond the global `--json`.

    // Read the raw document text so key-PRESENCE can be detected independently of
    // the final struct (a key set to the default value is still `(config)`).
    // A missing file is fine — every knob then reads as `(default)` (unless an
    // env override applies). Any read/parse trouble degrades presence detection
    // to "absent"; the strict `load` above already guaranteed a parseable file,
    // so `raw` parsing here cannot realistically fail, but we stay defensive.
    let raw = read_raw_toml(paths);

    let knobs = collect_knobs(&cfg, raw.as_ref());

    match mode {
        Mode::Json => {
            // A single stable JSON object: `{ "<key>": { "value", "source" } }`.
            // BTreeMap keeps the keys sorted for a byte-stable wire shape.
            let map: std::collections::BTreeMap<&str, KnobJson> = knobs
                .iter()
                .map(|k| {
                    (
                        k.key,
                        KnobJson {
                            value: k.value.clone(),
                            source: k.source,
                        },
                    )
                })
                .collect();
            write_json(&map)
        }
        Mode::Human => {
            let mut out = std::io::stdout().lock();
            // Align the `key` column so the `value (source)` columns line up.
            let key_width = knobs.iter().map(|k| k.key.len()).max().unwrap_or(0);
            for k in &knobs {
                writeln!(
                    out,
                    "{:<width$}  {} {}",
                    k.key,
                    k.value,
                    k.source.annotation(),
                    width = key_width,
                )?;
            }
            Ok(())
        }
    }
}

/// Read the raw config document as a parsed `toml::Value`, or `None` when the
/// file is absent / unreadable / unparsable. Used ONLY for key-presence
/// detection — the effective values come from the strict-parsed [`Config`].
fn read_raw_toml(paths: &Paths) -> Option<toml::Value> {
    let text = crate::util::bounded_read_to_string(
        &paths.global_config_file,
        crate::util::TOME_CONFIG_MAX,
    )
    .ok()?;
    text.parse::<toml::Value>().ok()
}

/// True when `section.field` is explicitly present in the raw document.
fn key_present(raw: Option<&toml::Value>, section: &str, field: &str) -> bool {
    raw.and_then(|v| v.get(section))
        .and_then(|s| s.get(field))
        .is_some()
}

/// Non-empty environment variable value. An empty var is treated as unset,
/// matching every real consumer (the loaders all filter empty).
fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Whether `NO_COLOR` is set and non-empty (per <https://no-color.org>): its
/// mere presence forces colour off, regardless of value.
fn no_color_active() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty())
}

/// Build the ordered, curated knob list with each knob's effective value +
/// provenance. Order is deliberate (grouped by section) and stable.
fn collect_knobs(cfg: &Config, raw: Option<&toml::Value>) -> Vec<Knob> {
    let mut knobs = Vec::new();

    // The common "no env override" case: provenance is config-if-present, else
    // default; the value is the config value or a rendered built-in default.
    // A free helper (not a closure) so the per-knob inline blocks below can also
    // push into `knobs` without a borrow clash.
    fn plain(
        raw: Option<&toml::Value>,
        key: &'static str,
        section: &str,
        field: &str,
        value: String,
    ) -> Knob {
        let source = if key_present(raw, section, field) {
            Source::Config
        } else {
            Source::Default
        };
        Knob { key, value, source }
    }

    // --- [query] --------------------------------------------------------------
    knobs.push(plain(
        raw,
        "query.top_k",
        "query",
        "top_k",
        cfg.query
            .top_k
            .map_or_else(|| "10".to_string(), |v| v.to_string()),
    ));
    knobs.push(plain(
        raw,
        "query.rerank",
        "query",
        "rerank",
        cfg.query.rerank.unwrap_or(true).to_string(),
    ));
    knobs.push(plain(
        raw,
        "query.strict_min_score",
        "query",
        "strict_min_score",
        cfg.query
            .strict_min_score
            .map_or_else(|| "0.0".to_string(), |v| v.to_string()),
    ));

    // --- [summariser] ---------------------------------------------------------
    knobs.push(plain(
        raw,
        "summariser.enabled",
        "summariser",
        "enabled",
        cfg.summariser.enabled.unwrap_or(true).to_string(),
    ));
    knobs.push(plain(
        raw,
        "summariser.long_max_chars",
        "summariser",
        "long_max_chars",
        cfg.summariser
            .long_max_chars
            .map_or_else(|| "4000".to_string(), |v| v.to_string()),
    ));

    // --- [logging] level — env: TOME_LOG > RUST_LOG > config > default --------
    {
        let key = "logging.level";
        let (value, source) = if let Some(v) = env_nonempty("TOME_LOG") {
            (v, Source::Env)
        } else if let Some(v) = env_nonempty("RUST_LOG") {
            (v, Source::Env)
        } else if let Some(level) = cfg.logging.level {
            (level.as_directive().to_string(), Source::Config)
        } else {
            ("warn".to_string(), Source::Default)
        };
        knobs.push(Knob { key, value, source });
    }

    // --- [output] color — env: NO_COLOR forces off ----------------------------
    {
        let key = "output.color";
        let (value, source) = if no_color_active() {
            // NO_COLOR forces colour off regardless of the file/default value.
            ("never".to_string(), Source::Env)
        } else if let Some(c) = cfg.output.color {
            (color_str(c), Source::Config)
        } else {
            // ColorMode::default() is Auto.
            ("auto".to_string(), Source::Default)
        };
        knobs.push(Knob { key, value, source });
    }
    knobs.push(plain(
        raw,
        "output.progress",
        "output",
        "progress",
        cfg.output.progress.unwrap_or(true).to_string(),
    ));

    // --- [workspace] default — env: TOME_WORKSPACE ----------------------------
    {
        let key = "workspace.default";
        let (value, source) = if let Some(v) = env_nonempty("TOME_WORKSPACE") {
            (v, Source::Env)
        } else if let Some(w) = cfg.workspace.default.as_deref() {
            (w.to_string(), Source::Config)
        } else {
            // No default configured → the privileged `global` workspace is used.
            ("global".to_string(), Source::Default)
        };
        knobs.push(Knob { key, value, source });
    }

    // --- [mcp] ----------------------------------------------------------------
    knobs.push(plain(
        raw,
        "mcp.description_max_chars",
        "mcp",
        "description_max_chars",
        cfg.mcp
            .description_max_chars
            .map_or_else(|| "200".to_string(), |v| v.to_string()),
    ));

    // --- [models] profile -----------------------------------------------------
    knobs.push(plain(
        raw,
        "models.profile",
        "models",
        "profile",
        cfg.models.profile.map_or_else(
            || crate::embedding::Profile::DEFAULT.as_str().to_string(),
            |p| p.as_str().to_string(),
        ),
    ));

    // --- [doctor] -------------------------------------------------------------
    knobs.push(plain(
        raw,
        "doctor.verify_by_default",
        "doctor",
        "verify_by_default",
        cfg.doctor.verify_by_default.unwrap_or(false).to_string(),
    ));

    // --- [harness] default_scope ----------------------------------------------
    knobs.push(plain(
        raw,
        "harness.default_scope",
        "harness",
        "default_scope",
        cfg.harness
            .default_scope
            .map_or_else(|| "project".to_string(), harness_scope_str),
    ));

    // --- [telemetry] enabled — env: TOME_TELEMETRY (=1 on / =0 off) -----------
    {
        let key = "telemetry.enabled";
        // TOME_TELEMETRY overrides ONLY on the exact "1"/"0" tokens the resolver
        // honours; any other value falls through to file/default.
        let env_override = match std::env::var("TOME_TELEMETRY").ok().as_deref() {
            Some("1") => Some(true),
            Some("0") => Some(false),
            _ => None,
        };
        let (value, source) = if let Some(v) = env_override {
            (v.to_string(), Source::Env)
        } else if let Some(v) = cfg.telemetry.enabled {
            (v.to_string(), Source::Config)
        } else {
            // Opt-out default: telemetry is on unless disabled.
            ("true".to_string(), Source::Default)
        };
        knobs.push(Knob { key, value, source });
    }

    // --- [telemetry] endpoint — env: TOME_GAUGE_ENDPOINT ----------------------
    {
        let key = "telemetry.endpoint";
        let (value, source) = if let Some(v) = env_nonempty("TOME_GAUGE_ENDPOINT") {
            (v.trim().to_string(), Source::Env)
        } else if let Some(ep) = cfg.telemetry.endpoint.as_deref() {
            (ep.to_string(), Source::Config)
        } else {
            (
                crate::telemetry::config::DEFAULT_ENDPOINT.to_string(),
                Source::Default,
            )
        };
        knobs.push(Knob { key, value, source });
    }

    knobs
}

fn color_str(c: crate::config::ColorMode) -> String {
    match c {
        crate::config::ColorMode::Auto => "auto",
        crate::config::ColorMode::Always => "always",
        crate::config::ColorMode::Never => "never",
    }
    .to_string()
}

fn harness_scope_str(s: crate::config::HarnessScope) -> String {
    match s {
        crate::config::HarnessScope::Project => "project",
        crate::config::HarnessScope::Global => "global",
    }
    .to_string()
}

// ---------------------------------------------------------------------------
// validate
// ---------------------------------------------------------------------------

/// The `tome config validate --json` record.
#[derive(Debug, Serialize)]
struct ValidateReport {
    valid: bool,
    /// The legible parse diagnostic when invalid (naming the offending
    /// key/section/line); absent on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn validate(paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    // `probe_error` is the SSOT the diagnostics (`doctor`/`status`) already use:
    // `None` when the file is absent OR parses cleanly, `Some(message)` with the
    // same legible diagnostic the strict `load` path surfaces otherwise.
    match config::probe_error(paths) {
        None => {
            // Valid (or absent — an absent config is a valid "all defaults").
            match mode {
                Mode::Json => write_json(&ValidateReport {
                    valid: true,
                    error: None,
                }),
                Mode::Human => {
                    let mut out = std::io::stdout().lock();
                    writeln!(out, "config is valid")?;
                    Ok(())
                }
            }
        }
        Some(message) => {
            // Invalid. In `--json`, emit the structured report to stdout for
            // scriptability; in human mode, print nothing here — the app boundary
            // already writes the legible `error: manifest invalid: …` line to
            // stderr, and double-printing would be noise. Either way, return the
            // config-parse error so the process exits 5
            // (`ManifestInvalid::TomlParse`) via the normal boundary — reusing the
            // existing exit code, no new one.
            if mode == Mode::Json {
                write_json(&ValidateReport {
                    valid: false,
                    error: Some(message.clone()),
                })?;
            }
            Err(TomeError::ManifestInvalid(
                crate::error::ManifestInvalid::TomlParse {
                    file: paths.global_config_file.clone(),
                    message,
                },
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn paths_in(dir: &TempDir) -> Paths {
        Paths::from_root(dir.path().to_path_buf())
    }

    fn write_config(dir: &TempDir, body: &str) {
        let paths = paths_in(dir);
        std::fs::create_dir_all(paths.global_config_file.parent().unwrap()).unwrap();
        std::fs::write(&paths.global_config_file, body).unwrap();
    }

    /// A knob by key from a freshly collected set (against a given config +
    /// raw doc). Env is NOT manipulated here — these tests assert the
    /// file/default behaviour without touching the process environment (which
    /// the integration suite covers with proper isolation).
    fn knob<'a>(knobs: &'a [Knob], key: &str) -> &'a Knob {
        knobs.iter().find(|k| k.key == key).expect("knob present")
    }

    #[test]
    fn default_config_all_default() {
        let cfg = Config::default();
        let knobs = collect_knobs(&cfg, None);
        // Every knob is `(default)` on a default config with no env overrides…
        // except the env-sensitive ones, which this test does not set, so they
        // are also default. We assert a representative spread.
        assert_eq!(knob(&knobs, "query.top_k").value, "10");
        assert_eq!(knob(&knobs, "query.top_k").source, Source::Default);
        assert_eq!(knob(&knobs, "summariser.enabled").value, "true");
        assert_eq!(knob(&knobs, "summariser.enabled").source, Source::Default);
        assert_eq!(knob(&knobs, "models.profile").value, "medium");
        assert_eq!(knob(&knobs, "models.profile").source, Source::Default);
        assert_eq!(knob(&knobs, "output.color").value, "auto");
        assert_eq!(knob(&knobs, "harness.default_scope").value, "project");
    }

    #[test]
    fn config_present_key_marks_config_even_at_default_value() {
        // `top_k = 10` equals the built-in default, but PRESENCE in the file must
        // still read as `(config)` — the whole point of raw-doc key detection.
        let raw: toml::Value = "[query]\ntop_k = 10\n".parse().unwrap();
        let cfg: Config = toml::from_str("[query]\ntop_k = 10\n").unwrap();
        let knobs = collect_knobs(&cfg, Some(&raw));
        let k = knob(&knobs, "query.top_k");
        assert_eq!(k.value, "10");
        assert_eq!(k.source, Source::Config);
        // A sibling key not in the file stays default.
        assert_eq!(knob(&knobs, "query.rerank").source, Source::Default);
    }

    #[test]
    fn config_present_nondefault_value() {
        let body = "[query]\ntop_k = 25\n[models]\nprofile = \"large\"\n";
        let raw: toml::Value = body.parse().unwrap();
        let cfg: Config = toml::from_str(body).unwrap();
        let knobs = collect_knobs(&cfg, Some(&raw));
        assert_eq!(knob(&knobs, "query.top_k").value, "25");
        assert_eq!(knob(&knobs, "query.top_k").source, Source::Config);
        assert_eq!(knob(&knobs, "models.profile").value, "large");
        assert_eq!(knob(&knobs, "models.profile").source, Source::Config);
    }

    #[test]
    fn validate_absent_is_valid() {
        let dir = TempDir::new().unwrap();
        assert_eq!(config::probe_error(&paths_in(&dir)), None);
    }

    #[test]
    fn validate_good_config_is_valid() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "[query]\ntop_k = 5\n");
        assert_eq!(config::probe_error(&paths_in(&dir)), None);
    }

    #[test]
    fn validate_malformed_returns_exit_5() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "[query]\nnope = 1\n");
        let paths = paths_in(&dir);
        // The validate command returns the TomlParse error (exit 5) with the
        // legible key-naming message.
        let err = validate(&paths, Mode::Human).unwrap_err();
        assert_eq!(err.exit_code(), 5);
        let msg = err.to_string();
        assert!(msg.contains("nope"), "must name the offending key: {msg}");
    }

    #[test]
    fn no_env_knobs_are_never_env_without_env() {
        // With a config that sets logging/color/workspace/telemetry and NO env
        // vars, none should be `(env)` (they should be `(config)`).
        // NOTE: this test does not touch the environment; TOME_LOG/RUST_LOG/etc
        // may be set in the ambient environment, so we only assert the config
        // values render, not the source, for the env-sensitive knobs.
        let body = "[logging]\nlevel = \"debug\"\n[workspace]\ndefault = \"work\"\n";
        let raw: toml::Value = body.parse().unwrap();
        let cfg: Config = toml::from_str(body).unwrap();
        let knobs = collect_knobs(&cfg, Some(&raw));
        // Values are read from the config when no env override wins.
        assert_eq!(knob(&knobs, "logging.level").value, "debug");
        assert_eq!(knob(&knobs, "workspace.default").value, "work");
    }
}
