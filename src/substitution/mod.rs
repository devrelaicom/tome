//! Phase 5 — variable substitution layer.
//!
//! Renders entry bodies (skills + commands) through a four-stage pipeline:
//! built-ins → env passthrough → arguments → optional ARGUMENTS tail.
//! Contract: `specs/005-phase-5-commands-prompts/contracts/substitution-engine.md`.
//!
//! F3 ships the module skeleton + override seams; consumers wire the
//! production behaviour in US1/US2/US3.
//!
//! ## Module layout
//!
//! - [`context`] — public `SubstitutionContext` + `SubstitutionContextBuilder`
//!   + `ArgumentValues` enum.
//! - [`builtins`] — `{{TOME_*}}` placeholder stage (stub in F3).
//! - [`env`] — `{{$VAR}}` env-passthrough stage (stub in F3).
//! - [`arguments`] — Claude Code `$ARGUMENTS` / `$N` / `$NAME` stage
//!   (stub in F3).
//! - [`data_dir`] — lazy plugin/workspace data-dir creation (stub in F3).
//! - [`regex_sets`] — `OnceLock<Regex>` slots for compiled stage regexes
//!   (uncompiled in F3; populated by US2/US3). Named with the `_sets`
//!   suffix to avoid shadowing the `regex` crate inside this module.

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
/// Stages 1 (built-ins) and 2 (env passthrough) are scanned in a
/// SINGLE regex pass per US2.d B2 — the resolved value is emitted
/// directly into the output buffer and never re-enters the scanner.
/// This is the structural enforcement of the no-rescan invariant
/// (NFR-007 / FR-051) and closes the exfiltration vector where a
/// hostile plugin's `"version": "${TOME_ENV_GITHUB_TOKEN}"` could leak
/// the operator's host env var into the LLM context via a body
/// referencing `${TOME_PLUGIN_VERSION}`.
///
/// Stages 3 and 4 (argument substitution + `ARGUMENTS:` tail) land in
/// US3. See `contracts/substitution-engine.md` for the full pipeline
/// shape.
pub fn render(body: &str, context: &SubstitutionContext) -> Result<String, SubstitutionError> {
    let re = regex_sets::combined_regex();
    // Fast-path: bodies with no `${TOME_…}` references short-circuit
    // without allocating an owned output buffer.
    if !re.is_match(body) {
        return Ok(body.to_owned());
    }
    let mut out = String::with_capacity(body.len());
    let mut last_end = 0;
    for caps in re.captures_iter(body) {
        // Group 0 is guaranteed to exist for any captures_iter match.
        let m = caps.get(0).expect("regex group 0 present on every match");
        out.push_str(&body[last_end..m.start()]);
        // Group 3 is the optional `:-default` (applies to whichever
        // branch matched).
        let default = caps.get(3).map(|c| c.as_str());

        // Per the unified pattern in `regex_sets::combined_regex`,
        // exactly one of group 1 (env branch) or group 2 (built-in
        // branch) is set on any successful match. Leftmost alternation
        // guarantees the env branch wins on `TOME_ENV_*` references.
        if let Some(env_name) = caps.get(1) {
            // Stage 2 — env passthrough. Pure function: never errors.
            let value = env::resolve_env(env_name.as_str(), default);
            out.push_str(&value);
        } else {
            // Stage 1 — built-in.
            let builtin_name = caps
                .get(2)
                .expect("combined_regex always sets group 1 or group 2 on a match")
                .as_str();
            match builtins::resolve_builtin(builtin_name, context, default)? {
                Some(value) => out.push_str(&value),
                None => {
                    tracing::debug!(
                        target: "tome::substitution",
                        builtin = builtin_name,
                        "unknown TOME_ built-in; leaving verbatim",
                    );
                    out.push_str(m.as_str());
                }
            }
        }
        last_end = m.end();
    }
    out.push_str(&body[last_end..]);
    Ok(out)
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
