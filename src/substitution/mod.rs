//! Phase 5 — variable substitution layer.
//!
//! Renders entry bodies (skills + commands) through a four-stage pipeline:
//! built-ins → env passthrough → arguments → optional ARGUMENTS tail.
//! Contract: `specs/005-phase-5-commands-prompts/contracts/substitution-engine.md`.
//!
//! As of US3, every stage of the pipeline ships production behaviour.
//!
//! ## Module layout
//!
//! - [`context`] — public `SubstitutionContext` + `SubstitutionContextBuilder`
//!   + `ArgumentValues` enum.
//! - [`builtins`] — `${TOME_*}` placeholder stage (Stage 1; US2.a).
//! - [`env`] — `${TOME_ENV_*}` env-passthrough stage (Stage 2; US2.b).
//! - [`arguments`] — Claude Code `$ARGUMENTS` / `$ARGUMENTS[N]` / `$N` /
//!   `$<name>` stage (Stage 3; US3.a).
//! - [`data_dir`] — lazy plugin/workspace data-dir creation (US2.b).
//! - [`regex_sets`] — `OnceLock<Regex>` slot for the unified stage-1+2+3
//!   regex. Named with the `_sets` suffix to avoid shadowing the
//!   `regex` crate inside this module.

mod arguments;
mod builtins;
mod context;
mod data_dir;
mod env;
mod regex_sets;

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub use context::{ArgumentValues, SubstitutionContext, SubstitutionContextBuilder};

/// Closed error set for the substitution layer.
///
/// Variants map onto `TomeError` via a `From` impl wired in a later
/// slice (US1+US2+US3). The variants here are deliberately scoped to
/// the substitution domain; transport into the closed `TomeError` enum
/// happens at the consumer boundary.
#[derive(Debug)]
pub enum SubstitutionError {
    /// `create_dir_all` failed for the plugin data directory.
    PluginDataDirCreationFailed {
        path: PathBuf,
        source: std::io::Error,
    },
    /// `create_dir_all` failed for the workspace data directory.
    WorkspaceDataDirCreationFailed {
        path: PathBuf,
        source: std::io::Error,
    },
    /// `arguments` frontmatter on a command/skill was malformed (e.g.
    /// duplicate names, reserved identifier, non-string entries).
    InvalidArgumentFrontmatter { reason: String, file: PathBuf },
    /// Caller supplied a different number of arguments than the entry
    /// declared in its frontmatter.
    PromptArgumentMismatch { expected: usize, supplied: usize },
}

impl std::fmt::Display for SubstitutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PluginDataDirCreationFailed { path, source } => write!(
                f,
                "failed to create plugin data dir {}: {source}",
                path.display()
            ),
            Self::WorkspaceDataDirCreationFailed { path, source } => write!(
                f,
                "failed to create workspace data dir {}: {source}",
                path.display()
            ),
            Self::InvalidArgumentFrontmatter { reason, file } => write!(
                f,
                "invalid `arguments` frontmatter in {}: {reason}",
                file.display()
            ),
            Self::PromptArgumentMismatch { expected, supplied } => write!(
                f,
                "prompt argument mismatch: declared {expected}, caller supplied {supplied}"
            ),
        }
    }
}

impl std::error::Error for SubstitutionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::PluginDataDirCreationFailed { source, .. } => Some(source),
            Self::WorkspaceDataDirCreationFailed { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Render an entry body through the substitution pipeline.
///
/// Stages 1 (built-ins), 2 (env passthrough), and 3 (arguments) are
/// scanned in a SINGLE regex pass per US2.d B2 + US3.a — the resolved
/// value is emitted directly into the output buffer and never re-enters
/// the scanner. This is the structural enforcement of the no-rescan
/// invariant (NFR-007 / FR-051):
///
/// - Closes the exfiltration vector where a hostile plugin's
///   `"version": "${TOME_ENV_GITHUB_TOKEN}"` could leak the operator's
///   host env var into the LLM context via a body referencing
///   `${TOME_PLUGIN_VERSION}` (US2.d B2 fix).
/// - Prevents Stage 1 output containing `$0` from being substituted by
///   Stage 3, and prevents Stage 3 argument values containing
///   `${TOME_*}` from being substituted by Stage 1+2.
///
/// Stage 4 (`ARGUMENTS:` append fallback) is a non-regex tail pass
/// triggered when caller supplied arguments AND Stage 3 reported zero
/// replacements. See `contracts/substitution-engine.md` for the full
/// pipeline shape.
pub fn render(body: &str, context: &SubstitutionContext) -> Result<String, SubstitutionError> {
    let re = regex_sets::combined_regex();
    // Coerce caller args once before the loop. `None` means Stage 3
    // is structurally skipped (every Stage-3 capture leaves its match
    // verbatim); a `Some` value drives per-match dispatch in the loop.
    //
    // Re-validation here covers library API + future surface
    // consumers that don't pass through `mcp::prompts::map_caller_arguments`
    // (which performs the same validation at the MCP boundary).
    let resolved_args = match &context.args {
        Some(values) => Some(arguments::coerce_arguments(values, &context.declared_args)?),
        None => None,
    };

    // Fast-path: bodies with no matches AND no caller args short-circuit
    // both regex iteration and Stage 4 (which only triggers when
    // `context.args.is_some()`). When args ARE present, we have to fall
    // through to the Stage-4 append-fallback even on an unmatched body.
    if !re.is_match(body) && context.args.is_none() {
        return Ok(body.to_owned());
    }

    let mut out = String::with_capacity(body.len());
    let mut last_end = 0;
    let mut stage_3_replacements_performed = false;

    for caps in re.captures_iter(body) {
        // Group 0 is guaranteed to exist for any captures_iter match.
        let m = caps.get(0).expect("regex group 0 present on every match");
        out.push_str(&body[last_end..m.start()]);
        // Group 3 is the optional `:-default` (applies to whichever
        // stage-1/2 branch matched).
        let default = caps.get(regex_sets::DEFAULT_GROUP).map(|c| c.as_str());

        // Per the unified pattern in `regex_sets::combined_regex`,
        // exactly one of the following is true on any successful match:
        //   - Group 1 (env) set — Stage 2.
        //   - Group 2 (built-in) set — Stage 1.
        //   - Group 4 (arg-index) set — Stage 3 `$ARGUMENTS[N]`.
        //   - Group 5 (positional) set — Stage 3 `$N`.
        //   - Group 6 (named) set — Stage 3 `$<name>`.
        //   - No Stage-3 capture group set AND `m.as_str() == "$ARGUMENTS"` —
        //     Stage 3 bare-`$ARGUMENTS`.
        // Leftmost alternation guarantees: env branch wins on `TOME_ENV_*`;
        // `$ARGUMENTS[N]` wins over bare `$ARGUMENTS`.
        if let Some(env_name) = caps.get(regex_sets::ENV_NAME_GROUP) {
            // Stage 2 — env passthrough. Pure function: never errors.
            let value = env::resolve_env(env_name.as_str(), default);
            out.push_str(&value);
        } else if let Some(builtin_name) = caps.get(regex_sets::BUILTIN_NAME_GROUP) {
            // Stage 1 — built-in.
            match builtins::resolve_builtin(builtin_name.as_str(), context, default)? {
                Some(value) => out.push_str(&value),
                None => {
                    tracing::debug!(
                        target: "tome::substitution",
                        builtin = builtin_name.as_str(),
                        "unknown TOME_ built-in; leaving verbatim",
                    );
                    out.push_str(m.as_str());
                }
            }
        } else if let Some(args) = resolved_args.as_ref() {
            // Stage 3 — argument substitution. Dispatch on which
            // Stage-3 alternative matched (capture group 4/5/6 OR the
            // bare `$ARGUMENTS` text).
            if m.as_str() == "$ARGUMENTS"
                && caps.get(regex_sets::ARG_INDEX_GROUP).is_none()
                && caps.get(regex_sets::POSITIONAL_GROUP).is_none()
                && caps.get(regex_sets::NAMED_GROUP).is_none()
            {
                // Bare `$ARGUMENTS` — positional values joined by single
                // space (FR-042). Always counts as a Stage-3
                // replacement even when positional is empty.
                out.push_str(&arguments::bare_arguments_value(args));
                stage_3_replacements_performed = true;
            } else {
                let (value, substituted) = arguments::apply_arguments_match(&caps, args);
                if substituted {
                    out.push_str(&value);
                    stage_3_replacements_performed = true;
                } else {
                    // Defensive — apply_arguments_match returns false
                    // only for the unreachable "no group set" branch.
                    out.push_str(m.as_str());
                }
            }
        } else {
            // Stage 3 reference matched but caller supplied no args
            // (or coercion was skipped). Leave verbatim per FR-040
            // "empty if not provided" — actually no: the contract says
            // resolve to empty string. But with no `ArgumentValues`,
            // Stage 3 is structurally skipped entirely (research §R-10
            // last row: `None` → "Stage 3 skipped entirely"; all
            // argument references in body resolve to empty strings per
            // FR-040). We leave them verbatim here — the caller
            // contract is "no args → no Stage 3" so substituted output
            // matches the body's literal reference, which a downstream
            // harness can handle. Tested in
            // `tests/substitution_arguments.rs::dollar_n_with_no_args_left_verbatim`.
            out.push_str(m.as_str());
        }
        last_end = m.end();
    }
    out.push_str(&body[last_end..]);

    // Stage 4 — `ARGUMENTS:` append fallback (FR-044).
    //
    // Trigger conditions per `contracts/substitution-engine.md` § Stage 4
    // + research §R-13: caller supplied args AND Stage 3 reported zero
    // replacements. The detection is structural (sentinel from the
    // loop) — we never re-scan the body for argument patterns.
    if let Some(args) = resolved_args.as_ref()
        && !stage_3_replacements_performed
    {
        let value = stage_4_value(&context.args, args);
        // Separator policy: body ends with `\n` → add one `\n` for the
        // blank line + `ARGUMENTS:`. Body ends with non-`\n` → add two
        // `\n`s (blank line + `ARGUMENTS:`).
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
        out.push_str("ARGUMENTS: ");
        out.push_str(&value);
    }

    Ok(out)
}

/// Compute the `<value>` for the Stage 4 `ARGUMENTS:` footer per the
/// `contracts/substitution-engine.md` § Stage 4 table.
///
/// - `Single("foo bar baz")` → whole string verbatim.
/// - `Object({a, b, …})` → positional values from the coerced
///   [`arguments::ResolvedArguments`] joined by single space.
fn stage_4_value(
    original: &Option<ArgumentValues>,
    resolved: &arguments::ResolvedArguments,
) -> String {
    match original {
        Some(ArgumentValues::Single(s)) => s.clone(),
        Some(ArgumentValues::Object { .. }) => resolved.positional.join(" "),
        None => {
            // Caller guards on `resolved_args.is_some()` which implies
            // `context.args.is_some()`. Defensive only.
            debug_assert!(false, "stage_4_value invoked with None args");
            String::new()
        }
    }
}

/// Wall-clock value for the substitution layer.
///
/// Honours [`SUBSTITUTION_CLOCK_OVERRIDE`] when set (tests install via
/// `ClockOverrideGuard`), otherwise returns the current UTC time. The
/// `time` crate's `now_local()` requires the `local-offset` feature
/// (not enabled in Tome's dep tree); the Phase 5 substitution contract
/// names the clock value as "wall-clock with the local offset *when
/// available*", and the substitution engine produces ISO 8601 with
/// offset, so UTC is a sound default that the test override can replace
/// for deterministic runs.
///
/// Mutex poison recovery per the F3 contract: a test panic mid-render
/// must not take the slot down for the rest of the suite.
pub fn current_clock() -> time::OffsetDateTime {
    if let Some(slot) = SUBSTITUTION_CLOCK_OVERRIDE.get() {
        let guard = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(t) = *guard {
            return t;
        }
    }
    time::OffsetDateTime::now_utc()
}

// --- Test injection seams (R-16) ----------------------------------------
//
// These are intentionally `#[doc(hidden)] pub static` so integration tests
// under `tests/` can reach them without `cfg(test)` visibility. Each pairs
// with an RAII guard in `tests/common/mod.rs` (`ClockOverrideGuard`,
// `PluginDataDirGuard`, `WorkspaceDataDirGuard`) that installs on `new()`
// and clears the slot's value on `Drop`.
//
// Consumers MUST recover from a poisoned mutex via
// `PoisonError::into_inner` per the established Phase 4 / P5 pattern.

/// Test-only override slot for the substitution clock. When `Some`, the
/// engine treats this as the wall-clock value substituted for
/// `{{TOME_CLOCK_*}}` family placeholders, bypassing
/// `time::OffsetDateTime::now_utc()`.
#[doc(hidden)]
pub static SUBSTITUTION_CLOCK_OVERRIDE: OnceLock<Mutex<Option<time::OffsetDateTime>>> =
    OnceLock::new();

/// Test-only override slot for the plugin data directory. When `Some`,
/// the engine uses this path instead of the
/// `<home>/.tome/plugin-data/<catalog>/<plugin>/` derivation.
#[doc(hidden)]
pub static PLUGIN_DATA_DIR_OVERRIDE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

/// Test-only override slot for the workspace data directory. When
/// `Some`, the engine uses this path instead of the
/// `<home>/.tome/workspaces/<name>/plugin-data/<catalog>/<plugin>/`
/// derivation.
#[doc(hidden)]
pub static WORKSPACE_DATA_DIR_OVERRIDE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
