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

/// Render an entry body through the four-stage substitution pipeline.
///
/// Stage 1 (built-ins) shipped in US2.a; Stage 2 (env passthrough)
/// lights up in US2.b. Stages 3 and 4 (argument substitution +
/// `ARGUMENTS:` tail) land in US3. See
/// `contracts/substitution-engine.md` for the full pipeline shape.
pub fn render(body: &str, context: &SubstitutionContext) -> Result<String, SubstitutionError> {
    let s = builtins::apply_builtins(body, context)?;
    let s = env::apply_env(&s).into_owned();
    // Stages 3 and 4 (argument substitution + ARGUMENTS tail) land in US3.
    Ok(s)
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
