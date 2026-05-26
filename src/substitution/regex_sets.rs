//! Compiled-regex cache slots for the three substitution stages.
//!
//! Compiled lazily on first use via `OnceLock::get_or_init` (US2/US3).
//! F3 ships the slots uncompiled — the production pipeline still
//! returns the body unchanged at this stage.
//!
//! Filename note: this module is `regex_sets` rather than `regex` so it
//! doesn't shadow the [`regex`] crate inside the `substitution` module
//! tree, which would force every reference to the crate to use the
//! awkward `::regex::Regex` absolute path.

use regex::Regex;
use std::sync::OnceLock;

/// Compiled regex for the `{{TOME_*}}` built-ins stage. Populated by
/// US2.
#[allow(dead_code)]
pub(super) static BUILTINS_RE: OnceLock<Regex> = OnceLock::new();

/// Compiled regex for the `{{$VAR}}` env-passthrough stage. Populated
/// by US2.
#[allow(dead_code)]
pub(super) static ENV_RE: OnceLock<Regex> = OnceLock::new();

/// Compiled regex for the `$ARGUMENTS` / `$N` / `$NAME` arguments
/// stage. Populated by US3.
#[allow(dead_code)]
pub(super) static ARGUMENTS_RE: OnceLock<Regex> = OnceLock::new();
