//! `tome harness session-start` — print the workspace's skill-routing
//! directive to stdout, regenerated fresh from live state.
//!
//! This is the target of the Tome-owned Claude Code SessionStart hook
//! (`src/harness/routing.rs::session_start_hook`): Claude Code runs it at the
//! start of every session and injects its stdout as `additionalContext`. It is
//! the on-demand, always-current sibling of the on-disk `RULES.md` produced by
//! [`crate::harness::routing::write_workspace_rules`] — same directive bytes,
//! but computed at session start rather than at enable/disable/tier-change time.

use std::io::Write;

use crate::cli::HarnessSessionStartArgs;
use crate::error::TomeError;
use crate::harness::{Envelope, SessionSteering, lookup};
use crate::output::Mode;
use crate::paths::Paths;
use crate::workspace::{ResolvedScope, WorkspaceName};

/// Print the routing directive to stdout for the resolved (or `--workspace`)
/// workspace. Always plain text — the Claude Code SessionStart hook captures
/// stdout as `additionalContext` regardless of the global `--json` flag, so
/// this command does not branch on `Mode`.
pub fn run(
    args: HarnessSessionStartArgs,
    scope: &ResolvedScope,
    paths: &Paths,
    _mode: Mode,
) -> Result<(), TomeError> {
    let name: WorkspaceName = match args.workspace.as_deref() {
        Some(raw) => WorkspaceName::parse(raw)?,
        None => scope.scope.name().clone(),
    };

    // Reconcile this project's files before printing, so the directive we emit
    // is consistent with freshly-synced harness files (and `.tome/RULES.md`).
    // FAIL-SOFT: a sync error must never block or fail the session-start hook —
    // warn and continue; the directive prints regardless. (No `?` on the call.)
    if let Some(project_root) = scope.project_root.as_deref() {
        let sync_args = crate::cli::SyncArgs {
            all: false,
            rules_only: false,
            harness_only: false,
            harness: vec![],
        };
        if let Err(e) =
            crate::commands::sync::sync_one_project(&name, project_root, &sync_args, paths)
        {
            tracing::warn!(
                workspace = name.as_str(),
                error = %e,
                "session-start: project reconcile failed; printing directive anyway",
            );
        }
    }

    let entries = if paths.index_db.exists() {
        let conn = crate::index::open_read_only(&paths.index_db)?;
        crate::index::skills::tiered_entries_for_workspace(&conn, name.as_str())?
    } else {
        Vec::new()
    };
    let summary = crate::harness::routing::read_cached_long_summary(paths, &name);
    let directive = crate::harness::routing::build_directive(&entries, summary.as_deref());

    let output = select_output(args.harness.as_deref(), &directive);

    if let Some(out) = output {
        std::io::stdout().lock().write_all(out.as_bytes())?;
    }
    Ok(())
}

/// Select the stdout payload from the `--harness` argument and the computed
/// `directive`. This is the REAL channel-selection `run` uses (extracted so it
/// is unit/integration testable without capturing stdout):
///
/// * ABSENT (`None`) → raw directive, byte-identical to the Phase ≤10 path (the
///   claude-code / codex hooks pass no `--harness`; this must not change).
/// * PRESENT but UNKNOWN → fail closed, emit nothing (the rules file still
///   carries the directive).
/// * PRESENT + an Open Plugins target (`generic-op`/`goose`) → wrap in
///   `ClaudeNested`. The `tome-op` bundle's `hooks.json` stamps `--harness
///   goose`/`generic-op`, and contract `open-plugins-tome-op.md` requires the
///   ClaudeNested envelope. These harnesses leave `session_steering() = None`
///   (they integrate through the atomic bundle emitter, NOT the command-hook
///   reconciler), so this case is selected by the open-plugins predicate BEFORE
///   the `session_steering` dispatch — giving them a `CommandHook` would make
///   `reconcile_command_hooks` fight the bundle.
/// * PRESENT + `CommandHook { envelope, .. }` → wrap in that envelope.
/// * PRESENT + `TsPlugin`/`None` → raw directive (the shim wraps it; `None`
///   harnesses get raw).
///
/// An EMPTY directive (empty workspace) emits nothing regardless of the channel:
/// an empty `additionalContext` envelope would inject noise.
pub fn select_output(harness: Option<&str>, directive: &str) -> Option<String> {
    match harness {
        None => Some(directive.to_string()),
        Some(_) if directive.is_empty() => None,
        Some(host) => match lookup(host) {
            None => None,
            // Open Plugins targets (`generic-op`/`goose`) integrate through the
            // atomic `tome-op` bundle, so they keep `session_steering() = None`.
            // Their hooks.json stamps `--harness <name>` expecting the
            // ClaudeNested envelope (contract open-plugins-tome-op.md). Detect
            // them by the open-plugins predicate (the path arg is unused beyond
            // `is_some()`) and wrap accordingly — never the raw directive.
            Some(module)
                if module
                    .open_plugins_root(std::path::Path::new("."))
                    .is_some() =>
            {
                Some(wrap_in_envelope(Envelope::ClaudeNested, directive))
            }
            Some(module) => match module.session_steering() {
                SessionSteering::CommandHook { envelope, .. } => {
                    Some(wrap_in_envelope(envelope, directive))
                }
                SessionSteering::TsPlugin { .. } | SessionSteering::None => {
                    Some(directive.to_string())
                }
            },
        },
    }
}

/// Wrap a routing `directive` in the harness's stdout `envelope` (contract
/// session-steering.md §Stdout envelopes). Pure + deterministic: identical
/// inputs → byte-identical output.
///
/// The JSON is built via `serde_json` so a directive containing `"` / `\n` /
/// any control char is escaped correctly — never string-concatenated. The
/// result is one compact JSON object on a single line (the convention every
/// hook stdout consumer expects).
pub fn wrap_in_envelope(envelope: Envelope, directive: &str) -> String {
    let value = match envelope {
        Envelope::ClaudeNested => serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": directive,
            }
        }),
        Envelope::FlatAdditionalContext => serde_json::json!({
            "additionalContext": directive,
        }),
        Envelope::AntigravityInjectSteps => serde_json::json!({
            "injectSteps": [ { "ephemeralMessage": directive } ]
        }),
        Envelope::CursorAdditionalContext => serde_json::json!({
            "additional_context": directive,
        }),
    };
    // `to_string` (not pretty) → compact single-line object, the stdout-hook
    // convention. Serialisation of a plain `Value` is infallible here.
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directive carrying a `"` and a `\n` proves the envelope escapes
    /// correctly (serde_json, never string concatenation): the quote becomes
    /// `\"`, the newline becomes `\n`, and the surrounding JSON stays valid.
    const TRICKY: &str = "line1 \"quoted\"\nline2";

    #[test]
    fn wrap_claude_nested_pins_exact_bytes() {
        assert_eq!(
            wrap_in_envelope(Envelope::ClaudeNested, TRICKY),
            r#"{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"line1 \"quoted\"\nline2"}}"#,
        );
    }

    #[test]
    fn wrap_flat_additional_context_pins_exact_bytes() {
        assert_eq!(
            wrap_in_envelope(Envelope::FlatAdditionalContext, TRICKY),
            r#"{"additionalContext":"line1 \"quoted\"\nline2"}"#,
        );
    }

    #[test]
    fn wrap_antigravity_inject_steps_pins_exact_bytes() {
        assert_eq!(
            wrap_in_envelope(Envelope::AntigravityInjectSteps, TRICKY),
            r#"{"injectSteps":[{"ephemeralMessage":"line1 \"quoted\"\nline2"}]}"#,
        );
    }

    /// Every wrapped envelope round-trips as valid JSON whose escaped string
    /// payload equals the original directive — the escaping is correct, not
    /// merely present.
    #[test]
    fn wrapped_envelopes_round_trip_the_directive() {
        let claude: serde_json::Value =
            serde_json::from_str(&wrap_in_envelope(Envelope::ClaudeNested, TRICKY)).unwrap();
        assert_eq!(claude["hookSpecificOutput"]["additionalContext"], TRICKY,);
        let flat: serde_json::Value =
            serde_json::from_str(&wrap_in_envelope(Envelope::FlatAdditionalContext, TRICKY))
                .unwrap();
        assert_eq!(flat["additionalContext"], TRICKY);
        let anti: serde_json::Value =
            serde_json::from_str(&wrap_in_envelope(Envelope::AntigravityInjectSteps, TRICKY))
                .unwrap();
        assert_eq!(anti["injectSteps"][0]["ephemeralMessage"], TRICKY);
    }

    /// US2 (T049): the `--harness <name>` → envelope mapping `run` applies. For
    /// each real new-harness name, `lookup(name).session_steering()` yields a
    /// `CommandHook` whose envelope is the one `run` wraps the directive in:
    /// devin → ClaudeNested, copilot-cli → FlatAdditionalContext, gemini →
    /// ClaudeNested. Antigravity (rules-only) yields `None` → raw directive.
    #[test]
    fn harness_name_selects_the_contract_envelope() {
        fn envelope_for(name: &str) -> Option<Envelope> {
            match lookup(name).unwrap().session_steering() {
                SessionSteering::CommandHook { envelope, .. } => Some(envelope),
                SessionSteering::TsPlugin { .. } | SessionSteering::None => None,
            }
        }
        assert_eq!(envelope_for("devin"), Some(Envelope::ClaudeNested));
        assert_eq!(
            envelope_for("copilot-cli"),
            Some(Envelope::FlatAdditionalContext)
        );
        assert_eq!(envelope_for("gemini"), Some(Envelope::ClaudeNested));
        // Antigravity is rules-only (US2 T047): no command-hook envelope.
        assert_eq!(envelope_for("antigravity"), None);
    }

    /// PW1 (phase-wide): the Open Plugins targets (`generic-op`/`goose`) keep
    /// `session_steering() = None` (they integrate through the atomic `tome-op`
    /// bundle, not the command-hook reconciler), yet their `hooks.json` stamps
    /// `--harness <name>` expecting the ClaudeNested envelope (contract
    /// open-plugins-tome-op.md §hooks.json). `select_output` must therefore
    /// wrap their directive in ClaudeNested — never the raw directive.
    #[test]
    fn select_output_wraps_open_plugins_targets_in_claude_nested() {
        let directive = "ROUTE skills via Tome.";
        let expected = wrap_in_envelope(Envelope::ClaudeNested, directive);
        assert_eq!(
            select_output(Some("goose"), directive).as_deref(),
            Some(expected.as_str()),
            "goose must emit the ClaudeNested envelope, not raw",
        );
        assert_eq!(
            select_output(Some("generic-op"), directive).as_deref(),
            Some(expected.as_str()),
            "generic-op must emit the ClaudeNested envelope, not raw",
        );
        // An empty directive still emits nothing (an empty envelope is noise).
        assert_eq!(select_output(Some("goose"), ""), None);
    }

    /// PW1 live-probe (manual): confirm a real Goose SessionStart hook running
    /// `tome harness session-start --harness goose` injects the ClaudeNested
    /// envelope's `additionalContext` into the model's context. Cannot run in
    /// CI (needs a live Goose host), so it is `#[ignore]`d as a release probe.
    #[test]
    #[ignore = "live-probe: confirm Goose SessionStart hook injects the ClaudeNested envelope"]
    fn goose_session_start_injects_claude_nested_live() {
        // Manual: install the tome-op bundle into a Goose project, start a
        // session, and assert the directive arrived as `additionalContext`.
    }
}
