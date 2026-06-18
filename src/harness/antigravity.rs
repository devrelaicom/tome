//! `antigravity` — the Antigravity IDE.
//!
//! Phase 11 (US1). Baseline integration: standalone rules file + MCP dialect.
//!
//! ## Session steering: RULES-ONLY for now (US2 / T047, DE-RISK per FR-020/R14)
//!
//! Antigravity DELIBERATELY keeps `session_steering()` = [`SessionSteering::None`]
//! (rules-only). The command-hook path for Antigravity is doc-ambiguous and
//! CANNOT be confirmed without a live install: the hooks directory (`.agents/`
//! vs `.agent/`), the hook event (`PreInvocation`), and the `injectSteps` stdout
//! envelope are all unverified. Shipping a guessed hook file risks writing to
//! the wrong path / under the wrong event, so until a real Antigravity install
//! confirms the shape, Antigravity is integrated via its standalone rules file
//! only (the directive still reaches Antigravity through `.agent/rules/tome.md`).
//!
//! The foundation is READY for the flip: `HookFileSpec::AntigravityHooks`
//! (`.agents/hooks.json`, named `tome` block, `PreInvocation`) and the
//! [`Envelope::AntigravityInjectSteps`] (`{ "injectSteps": [ { "ephemeralMessage":
//! … } ] }`) envelope both exist and are exercised by the `reconcile_command_hooks`
//! unit tests. Confirming the live shape (T087, run on a real Antigravity install
//! during Polish) is the ONLY blocker. Once confirmed, flip
//! `session_steering()` to EXACTLY:
//!
//! ```ignore
//! fn session_steering(&self) -> SessionSteering {
//!     SessionSteering::CommandHook {
//!         file_spec: HookFileSpec::AntigravityHooks,
//!         event: HookEvent::PreInvocation,
//!         envelope: Envelope::AntigravityInjectSteps,
//!     }
//! }
//! ```
//!
//! [`SessionSteering::None`]: crate::harness::SessionSteering::None
//! [`Envelope::AntigravityInjectSteps`]: crate::harness::Envelope::AntigravityInjectSteps
//!
//! - Per-user dir: `~/.gemini/` — Antigravity shares the Gemini config tree.
//!   The `antigravity-cli` → `gemini` alias (in [`HARNESS_ALIASES`]) routes
//!   the *CLI* to the Gemini module; this `antigravity` module is the IDE,
//!   which has its OWN project rules sink + a GLOBAL MCP file under the
//!   shared Gemini tree.
//! - Rules-file sink: a `StandaloneFile` at `<project>/.agent/rules/tome.md`.
//! - MCP config: `~/.gemini/config/mcp_config.json` (GLOBAL, under `home`).
//! - MCP dialect: JSON `mcpServers` + `CommandArgs`, no `type`,
//!   `emit_env:true` (`"env": {}`), no extra fields.
//!
//! [`HARNESS_ALIASES`]: super::HARNESS_ALIASES

use std::path::{Path, PathBuf};

use crate::harness::{
    EntryShape, FileFormat, GuardrailsPlacement, GuardrailsTarget, HarnessModule, McpDialect,
    RulesFileStrategy,
};

/// Unit struct implementing [`HarnessModule`] for Antigravity IDE.
pub struct Antigravity;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const ANTIGRAVITY: Antigravity = Antigravity;

impl HarnessModule for Antigravity {
    fn name(&self) -> &'static str {
        "antigravity"
    }

    fn description(&self) -> &'static str {
        "Antigravity IDE"
    }

    fn detect(&self, home: &Path) -> bool {
        // Antigravity shares the `~/.gemini` detection dir with the `gemini`
        // harness (PW10): a no-arg `tome harness use` that auto-detects from
        // `~/.gemini` configures BOTH. This is benign — they own DISTINCT sinks
        // (antigravity's rules at `.agent/rules/`, its MCP at
        // `~/.gemini/config/mcp_config.json`; gemini's at its own paths), so the
        // shared detection dir never causes a write collision.
        home.join(".gemini").is_dir()
    }

    fn detect_path(&self, home: &Path) -> PathBuf {
        // `name()` is "antigravity" but the per-user dir is `~/.gemini/`.
        home.join(".gemini")
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join(".agent/rules/tome.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::StandaloneFile
    }

    fn rules_namespaced_file(&self, project_root: &Path) -> Option<PathBuf> {
        Some(project_root.join(".agent/rules/tome.md"))
    }

    /// Guardrails land in a Tome-owned standalone sibling (PW3), distinct from
    /// the standalone rules file `.agent/rules/tome.md` — without it the
    /// standalone rules writer and the in-file guardrails region would share one
    /// path and clobber each other every sync. Mirrors `cursor`.
    fn guardrails_target(&self, project_root: &Path) -> GuardrailsTarget {
        GuardrailsTarget {
            placement: GuardrailsPlacement::StandaloneSibling {
                file: project_root.join(".agent/rules/TOME_GUARDRAILS.md"),
            },
            suppress_if_hooks_present: false,
        }
    }

    fn mcp_config_path(&self, _project_root: &Path, home: &Path) -> PathBuf {
        // GLOBAL config under the shared Gemini tree.
        home.join(".gemini/config/mcp_config.json")
    }

    /// Antigravity's MCP dialect: JSON `mcpServers` + `CommandArgs`, no
    /// `type`, `emit_env:true` (`"env": {}`), no extra fields.
    fn mcp_dialect(&self) -> McpDialect {
        McpDialect {
            file_format: FileFormat::Json,
            parent_key: "mcpServers",
            entry_shape: EntryShape::CommandArgs,
            entry_type: None,
            emit_env: true,
            extra_fields: &[],
        }
    }

    // RULES-ONLY (US2 / T047): `session_steering()` is DELIBERATELY left as the
    // trait default `SessionSteering::None`. The Antigravity command-hook shape
    // (`.agents/` dir, `PreInvocation` event, `injectSteps` envelope) is
    // doc-ambiguous and cannot be confirmed without a live install — see the
    // module doc comment for the de-risk rationale and the exact `CommandHook`
    // value (`AntigravityHooks` / `PreInvocation` / `AntigravityInjectSteps`)
    // to set once T087 (the live-probe gate below) confirms it. No
    // `session_steering` override here means NO hook file is written for
    // antigravity; the directive rides its `.agent/rules/tome.md` rules file.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_and_paths() {
        assert_eq!(ANTIGRAVITY.name(), "antigravity");
        assert_eq!(
            ANTIGRAVITY.detect_path(Path::new("/h")),
            Path::new("/h/.gemini"),
        );
        assert_eq!(
            ANTIGRAVITY.rules_file_target(Path::new("/proj")),
            Path::new("/proj/.agent/rules/tome.md"),
        );
        // GLOBAL MCP path under the shared Gemini tree.
        assert_eq!(
            ANTIGRAVITY.mcp_config_path(Path::new("/proj"), Path::new("/h")),
            Path::new("/h/.gemini/config/mcp_config.json"),
        );
    }

    /// PW3 (phase-wide): guardrails land in a Tome-owned StandaloneSibling that
    /// is NOT the standalone rules-file path.
    #[test]
    fn guardrails_sibling_differs_from_rules_file() {
        let proj = Path::new("/proj");
        let rules = ANTIGRAVITY.rules_file_target(proj);
        match ANTIGRAVITY.guardrails_target(proj).placement {
            GuardrailsPlacement::StandaloneSibling { file } => {
                assert_eq!(file, PathBuf::from("/proj/.agent/rules/TOME_GUARDRAILS.md"));
                assert_ne!(file, rules);
            }
            other => panic!("expected StandaloneSibling, got {other:?}"),
        }
    }

    #[test]
    fn dialect_is_mcpservers_command_args_emit_env() {
        let d = ANTIGRAVITY.mcp_dialect();
        assert_eq!(d.parent_key, "mcpServers");
        assert_eq!(d.entry_shape, EntryShape::CommandArgs);
        assert_eq!(d.entry_type, None);
        assert!(d.emit_env);
        assert!(!ANTIGRAVITY.mcp_manual_only());
    }

    /// US2 (T047, DE-RISK): antigravity stays RULES-ONLY — its
    /// `session_steering()` is the trait default `SessionSteering::None`, so the
    /// `reconcile_command_hooks` pass never writes a hook file for it. Flipping
    /// to `CommandHook` is gated on the live probe below (T087).
    #[test]
    fn session_steering_is_none_rules_only() {
        use crate::harness::SessionSteering;
        assert_eq!(
            ANTIGRAVITY.session_steering(),
            SessionSteering::None,
            "antigravity must stay rules-only until T087 confirms the hook shape",
        );
    }

    /// Live-probe gate (T087): NOT run in CI. A human must confirm against a
    /// real Antigravity install the `.agent/` rules dir, the `.agents/` hooks
    /// dir, the `PreInvocation` event, and the `injectSteps` session-start
    /// envelope. ONLY when all four are confirmed, flip `session_steering()` to:
    ///
    /// ```ignore
    /// SessionSteering::CommandHook {
    ///     file_spec: HookFileSpec::AntigravityHooks,   // .agents/hooks.json
    ///     event: HookEvent::PreInvocation,
    ///     envelope: Envelope::AntigravityInjectSteps,
    /// }
    /// ```
    ///
    /// The `AntigravityHooks` spec + `AntigravityInjectSteps` envelope already
    /// exist in the foundation (exercised by the `reconcile_command_hooks` unit
    /// tests), so the flip is the single `session_steering()` override above —
    /// no new wiring.
    #[test]
    #[ignore = "live-probe (T087): confirm .agents/ hooks dir + PreInvocation event + injectSteps envelope, then flip session_steering to CommandHook"]
    fn antigravity_rules_hooks_dirs_and_inject_steps_live_probe() {
        // No automated body — see the doc comment for the manual checklist.
    }
}
