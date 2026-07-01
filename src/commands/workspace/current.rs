//! `tome workspace current [--json]` — print the workspace bound to the
//! current directory, for shell prompts and scripting.
//!
//! The lightweight counterpart to `tome workspace info` / `tome status`:
//! human mode prints JUST the workspace name on one line with no
//! decoration, so `$(tome workspace current 2>/dev/null)` yields the name
//! (bound) or the empty string (unbound). `--json` emits a stable record.
//!
//! "Bound" reuses the one resolution SSOT
//! ([`crate::workspace::resolution::resolve`]): the resolved
//! [`ResolvedScope`] carries both the active workspace name and *how* it
//! was determined ([`ScopeSource`]). Every source except
//! [`ScopeSource::GlobalFallback`] is an explicit selection or binding
//! (a `--workspace` flag, `TOME_WORKSPACE`, a `[workspace] default`, or a
//! project-marker `.tome/config.toml`) and prints the name. The lone
//! `GlobalFallback` case — no flag, no env, no config default, no marker —
//! is "nothing is bound to this directory" and exits non-zero
//! ([`TomeError::WorkspaceNotBound`], exit 12) with a clear, actionable
//! stderr message and NO stray stdout.
//!
//! Read-only; never acquires the advisory lock, never touches the DB (the
//! scope was already resolved + membership-checked before dispatch).

use std::io::Write;

use serde::Serialize;

use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::workspace::{ResolvedScope, ScopeSource};

/// The `--json` wire record for `tome workspace current`. Mirrors how
/// `tome status` exposes the active scope (`current_workspace` /
/// `current_scope`) and reuses the `snake_case` [`ScopeSource`]
/// serialisation already pinned by `tome workspace info --json`'s `source`
/// field. Emit-only, so no `deny_unknown_fields`.
#[derive(Debug, Serialize)]
struct CurrentRecord<'a> {
    /// The active workspace name (e.g. `"my-project"`).
    workspace: &'a str,
    /// `"global"` or `"project"` — deliberately the same two-value
    /// labelling `tome status` uses for `current_scope`, for scripting
    /// consistency. This intentionally differs from `tome workspace info
    /// --json`'s `scope` field, which serialises the `ScopeKind` enum as
    /// `"global"` / `"workspace"`.
    scope: &'static str,
    /// How the binding was resolved (`flag` / `env` / `config` /
    /// `project_marker`). Never `global_fallback` here — that case exits
    /// non-zero before this record is built.
    source: ScopeSource,
}

pub fn run(scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    // `GlobalFallback` is the ONLY "not bound" source: no flag, no env, no
    // config default, no project marker reached the resolver, so it
    // defaulted to `global`. Every other source is an explicit selection
    // or a project binding.
    if scope.source == ScopeSource::GlobalFallback {
        // `WorkspaceNotBound` (exit 12) is the dedicated failure class for
        // "no workspace resolves to this directory" — genuinely distinct
        // from `WorkspaceNotFound` (exit 13, "a *named* workspace is absent
        // from the registry"), which the strict 1:1 closed set forbids
        // sharing a code with. Its Display is actionable ("bind one with
        // `tome workspace use <name>`, or select one with `--workspace
        // <name>`"), unlike `WorkspaceNotFound`'s registry-oriented `init`
        // hint. The app boundary renders it to stderr and (in `--json`) the
        // structured error envelope; stdout stays empty so
        // `$(tome workspace current 2>/dev/null)` is the empty string.
        return Err(TomeError::WorkspaceNotBound);
    }

    let name = scope.scope.name().as_str();
    let scope_label = if scope.scope.is_global() {
        "global"
    } else {
        "project"
    };

    match mode {
        Mode::Human => {
            // Just the name, one line, no decoration — the prompt/script
            // contract.
            let mut out = std::io::stdout().lock();
            writeln!(out, "{name}")?;
            Ok(())
        }
        Mode::Json => write_json(&CurrentRecord {
            workspace: name,
            scope: scope_label,
            source: scope.source,
        }),
    }
}
