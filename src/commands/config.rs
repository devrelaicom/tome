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

    let knobs = collect_knobs(&cfg, raw.as_ref(), paths);

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

/// Built-in default for `[summariser] enabled` — absent means enabled (the
/// consumer only disables on an explicit `Some(false)`, see
/// `summarise::trigger`). Local to this SSOT since the consumer's default is
/// expressed as a `!= Some(false)` check rather than a shared constant.
const DEFAULT_SUMMARISER_ENABLED: bool = true;
/// Built-in default for `[doctor] verify_by_default` — the effective `verify` is
/// `flag || config.unwrap_or(false)` (see `commands::doctor`).
const DEFAULT_DOCTOR_VERIFY: bool = false;
/// Built-in default for `[hooks] translate_plugin_hooks` — an opt-OUT toggle:
/// absent or `Some(true)` ⇒ enabled (see `HooksConfig` docs / `commands::sync`).
const DEFAULT_TRANSLATE_PLUGIN_HOOKS: bool = true;

/// Build the ordered, curated knob list with each knob's effective value +
/// provenance. Order is deliberate (grouped by section) and stable.
///
/// `paths` is threaded through so the `telemetry.enabled` knob can reuse the
/// telemetry SSOT (`resolve_enabled_with_source`), which folds in the CI
/// auto-disable + env precedence — the shown VALUE/SOURCE then reflect the true
/// effective state, not a hand-rolled guess.
fn collect_knobs(cfg: &Config, raw: Option<&toml::Value>, paths: &Paths) -> Vec<Knob> {
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
    // Defaults are single-sourced from the query consumer's constants so the
    // shown default can't drift from the effective one.
    knobs.push(plain(
        raw,
        "query.top_k",
        "query",
        "top_k",
        cfg.query
            .top_k
            .unwrap_or(crate::commands::query::DEFAULT_TOP_K)
            .to_string(),
    ));
    knobs.push(plain(
        raw,
        "query.rerank",
        "query",
        "rerank",
        // #502: the effective rerank state, single-sourced through the same
        // resolver the CLI query path uses — explicit `[query] rerank` wins,
        // else a configured `[reranker]` provider implicitly enables it, else
        // the built-in default (off). So `query.rerank` reads `true` when a
        // `[reranker]` provider is configured even though `[query] rerank` is
        // unset, matching what a query would actually do.
        crate::commands::query::resolve_effective_rerank(
            cfg.query.rerank,
            cfg.reranker.is_provider_configured(),
        )
        .to_string(),
    ));
    knobs.push(plain(
        raw,
        "query.strict_min_score",
        "query",
        "strict_min_score",
        // The real unset default is `None` — there is NO score floor unless
        // `--strict` is passed (and even then the floor is reranker-dependent).
        // Render "none" for the unset case rather than a misleading numeric.
        cfg.query
            .strict_min_score
            .map_or_else(|| "none".to_string(), |v| v.to_string()),
    ));

    // --- [summariser] ---------------------------------------------------------
    knobs.push(plain(
        raw,
        "summariser.enabled",
        "summariser",
        "enabled",
        cfg.summariser
            .enabled
            .unwrap_or(DEFAULT_SUMMARISER_ENABLED)
            .to_string(),
    ));
    knobs.push(plain(
        raw,
        "summariser.long_max_chars",
        "summariser",
        "long_max_chars",
        // Single-sourced from the summariser's own constant (was a drifted
        // literal `4000`; the real effective default is `LONG_MAX_CHARS`).
        cfg.summariser
            .long_max_chars
            .unwrap_or(crate::summarise::LONG_MAX_CHARS)
            .to_string(),
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
        // Unset means "auto" (follow the stderr TTY), not a fixed `true` — the
        // consumer treats `None`/`Some(true)` identically as TTY-auto and only
        // `Some(false)` force-hides (see `presentation::progress`). Render the
        // configured bool when present, else "auto" for the honest default.
        cfg.output
            .progress
            .map_or_else(|| "auto".to_string(), |v| v.to_string()),
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
        // Single-sourced from the `search_skills` tool's constant (was a drifted
        // literal `200`; the real effective default is 150).
        cfg.mcp
            .description_max_chars
            .unwrap_or(crate::mcp::tools::search_skills::DEFAULT_DESCRIPTION_MAX_CHARS)
            .to_string(),
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
        cfg.doctor
            .verify_by_default
            .unwrap_or(DEFAULT_DOCTOR_VERIFY)
            .to_string(),
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

    // --- [hooks] translate_plugin_hooks (opt-OUT toggle) ----------------------
    knobs.push(plain(
        raw,
        "hooks.translate_plugin_hooks",
        "hooks",
        "translate_plugin_hooks",
        cfg.hooks
            .translate_plugin_hooks
            .unwrap_or(DEFAULT_TRANSLATE_PLUGIN_HOOKS)
            .to_string(),
    ));

    // --- [telemetry] enabled — env: TOME_TELEMETRY + CI auto-disable -----------
    {
        let key = "telemetry.enabled";
        // Reuse the telemetry SSOT so the shown VALUE + SOURCE reflect the TRUE
        // effective state, including the CI auto-disable short-circuit (which a
        // hand-rolled "1"/"0" match would miss — in CI the effective value is
        // forced `false`). `resolve_enabled_with_source` returns the deciding
        // `telemetry::config::Source`; map it onto our provenance:
        //   EnvOn / EnvOff / Ci → (env)  (all three are ambient/environmental)
        //   Config              → (config)
        //   Default             → (default)
        // A malformed config only surfaces at the file-read step of the resolver;
        // `show` already ran the strict `load` and would have exited 5 before
        // reaching here, so the `.unwrap_or` fallback is defensive-only.
        use crate::telemetry::config::Source as TSource;
        let (value, source) = match crate::telemetry::config::resolve_enabled_with_source(paths) {
            Ok((enabled, tsrc)) => {
                let src = match tsrc {
                    TSource::EnvOn | TSource::EnvOff | TSource::Ci => Source::Env,
                    TSource::Config => Source::Config,
                    TSource::Default => Source::Default,
                };
                (enabled.to_string(), src)
            }
            // Defensive fallback (unreachable in `show` after strict load).
            Err(_) => (
                cfg.telemetry.enabled.unwrap_or(true).to_string(),
                if key_present(raw, "telemetry", "enabled") {
                    Source::Config
                } else {
                    Source::Default
                },
            ),
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
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let cfg = Config::default();
        let knobs = collect_knobs(&cfg, None, &paths);
        // The non-env-sensitive knobs are `(default)` on a default config; the
        // shown default equals the SOURCE constant (single-sourced, item #2).
        assert_eq!(
            knob(&knobs, "query.top_k").value,
            crate::commands::query::DEFAULT_TOP_K.to_string()
        );
        assert_eq!(knob(&knobs, "query.top_k").source, Source::Default);
        assert_eq!(knob(&knobs, "summariser.enabled").value, "true");
        assert_eq!(knob(&knobs, "summariser.enabled").source, Source::Default);
        assert_eq!(knob(&knobs, "models.profile").value, "medium");
        assert_eq!(knob(&knobs, "models.profile").source, Source::Default);
        assert_eq!(knob(&knobs, "output.color").value, "auto");
        assert_eq!(knob(&knobs, "harness.default_scope").value, "project");
        // Item #3: the unset strict_min_score renders "none" (no floor), not 0.0.
        assert_eq!(knob(&knobs, "query.strict_min_score").value, "none");
        // Item #7: hooks.translate_plugin_hooks is present (opt-out default true).
        assert_eq!(knob(&knobs, "hooks.translate_plugin_hooks").value, "true");
        // NOTE: telemetry.enabled is intentionally NOT asserted here — its
        // provenance/value flow through the telemetry SSOT, which reads ambient
        // env (TOME_TELEMETRY / CI), so it is env-dependent. The integration
        // suite exercises it with proper env isolation.
    }

    /// Item #2 (regression guard): the shown DEFAULT of the two knobs that had
    /// drifted MUST equal the source constant the consumer actually uses, so a
    /// future change to the constant can't silently desync the shown default.
    #[test]
    fn shown_defaults_track_source_constants() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let cfg = Config::default();
        let knobs = collect_knobs(&cfg, None, &paths);
        assert_eq!(
            knob(&knobs, "summariser.long_max_chars").value,
            crate::summarise::LONG_MAX_CHARS.to_string(),
            "shown long_max_chars default must equal LONG_MAX_CHARS"
        );
        assert_eq!(
            knob(&knobs, "mcp.description_max_chars").value,
            crate::mcp::tools::search_skills::DEFAULT_DESCRIPTION_MAX_CHARS.to_string(),
            "shown description_max_chars default must equal DEFAULT_DESCRIPTION_MAX_CHARS"
        );
    }

    #[test]
    fn config_present_key_marks_config_even_at_default_value() {
        // `top_k = 10` equals the built-in default, but PRESENCE in the file must
        // still read as `(config)` — the whole point of raw-doc key detection.
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let raw: toml::Value = "[query]\ntop_k = 10\n".parse().unwrap();
        let cfg: Config = toml::from_str("[query]\ntop_k = 10\n").unwrap();
        let knobs = collect_knobs(&cfg, Some(&raw), &paths);
        let k = knob(&knobs, "query.top_k");
        assert_eq!(k.value, "10");
        assert_eq!(k.source, Source::Config);
        // A sibling key not in the file stays default.
        assert_eq!(knob(&knobs, "query.rerank").source, Source::Default);
    }

    #[test]
    fn config_present_nondefault_value() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let body = "[query]\ntop_k = 25\n[models]\nprofile = \"large\"\n";
        let raw: toml::Value = body.parse().unwrap();
        let cfg: Config = toml::from_str(body).unwrap();
        let knobs = collect_knobs(&cfg, Some(&raw), &paths);
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
    fn non_env_knobs_render_config_values() {
        // Non-env-sensitive knobs read straight from the config with `(config)`
        // provenance regardless of the ambient environment. (The env-sensitive
        // knobs — logging/color/workspace/telemetry — are exercised with proper
        // `.env_remove` isolation in the integration suite, not here, to avoid a
        // dependency on the ambient TOME_LOG/RUST_LOG/NO_COLOR/… vars.)
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let body = "[summariser]\nlong_max_chars = 3333\n[mcp]\ndescription_max_chars = 42\n";
        let raw: toml::Value = body.parse().unwrap();
        let cfg: Config = toml::from_str(body).unwrap();
        let knobs = collect_knobs(&cfg, Some(&raw), &paths);
        assert_eq!(knob(&knobs, "summariser.long_max_chars").value, "3333");
        assert_eq!(
            knob(&knobs, "summariser.long_max_chars").source,
            Source::Config
        );
        assert_eq!(knob(&knobs, "mcp.description_max_chars").value, "42");
        assert_eq!(
            knob(&knobs, "mcp.description_max_chars").source,
            Source::Config
        );
    }
}
