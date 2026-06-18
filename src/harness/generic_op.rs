//! `generic-op` — the portable Open Plugins `tome-op` write target.
//!
//! Phase 11 (US4). An OPT-IN target (never auto-detected, never in `--all`):
//! the user opts in by name via `tome harness use generic-op`. Unlike `generic`
//! (which writes AGENTS.md + `./mcp.json` through the standard sinks),
//! `generic-op` emits a self-contained Open Plugins `tome-op` bundle via
//! [`crate::harness::open_plugins::emit_tome_op`] — dispatched by the
//! orchestrator (and `tome harness use`) because [`open_plugins_root`] returns
//! `Some`. The bundle is all-or-nothing, so it is NEVER routed through the
//! per-sink rules/MCP loop (that would double-write the bundle's `AGENTS.md` /
//! `.mcp.json`).
//!
//! Registered in [`super::OPT_IN_TARGETS`].
//!
//! [`open_plugins_root`]: HarnessModule::open_plugins_root

use std::path::{Path, PathBuf};

use crate::harness::open_plugins::TOME_OP_NAME;
use crate::harness::{
    BlockBodyStyle, EntryShape, FileFormat, HarnessModule, McpDialect, RulesFileStrategy,
};

/// Unit struct implementing [`HarnessModule`] for the `generic-op` target.
pub struct GenericOp;

/// Static instance used by the [`OPT_IN_TARGETS`] registry.
///
/// [`OPT_IN_TARGETS`]: super::OPT_IN_TARGETS
pub const GENERIC_OP: GenericOp = GenericOp;

/// The bundle root for `generic-op`: `<project>/<tome-op>` (the explicit project
/// default; the contract allows an explicit `--output` location to override at
/// the command boundary).
fn bundle_root(project_root: &Path) -> PathBuf {
    project_root.join(TOME_OP_NAME)
}

impl HarnessModule for GenericOp {
    fn name(&self) -> &'static str {
        "generic-op"
    }

    fn description(&self) -> &'static str {
        "Generic Open Plugins (tome-op) target"
    }

    fn detect(&self, _home: &Path) -> bool {
        // Inert: opt-in by name only. Never auto-detected, never in `--all`.
        false
    }

    fn detect_path(&self, home: &Path) -> PathBuf {
        home.join(".tome/generic-op-target")
    }

    fn is_opt_in_target(&self) -> bool {
        true
    }

    /// `Some` → the orchestrator / `tome harness use` dispatch to the
    /// `open_plugins` emitter instead of the per-sink loop.
    fn open_plugins_root(&self, project_root: &Path) -> Option<PathBuf> {
        Some(bundle_root(project_root))
    }

    // The methods below describe the bundle-INTERNAL sinks. They are reported by
    // `tome harness info generic-op` (which resolves opt-in targets via `lookup`)
    // but are NOT consulted on the open-plugins dispatch path (the emitter owns
    // the whole bundle atomically).

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        bundle_root(project_root).join("AGENTS.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }

    fn block_body_style(&self) -> BlockBodyStyle {
        BlockBodyStyle::Inline
    }

    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        bundle_root(project_root).join(".mcp.json")
    }

    /// JSON `mcpServers` + `CommandArgs` + `"env": {}` — the shape the bundle's
    /// `.mcp.json` carries (informational; the emitter writes the file).
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_and_opt_in() {
        assert_eq!(GENERIC_OP.name(), "generic-op");
        assert!(GENERIC_OP.is_opt_in_target());
    }

    #[test]
    fn never_detected() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".generic-op")).unwrap();
        assert!(!GENERIC_OP.detect(tmp.path()));
    }

    #[test]
    fn open_plugins_root_is_project_tome_op() {
        assert_eq!(
            GENERIC_OP.open_plugins_root(Path::new("/proj")),
            Some(PathBuf::from("/proj/tome-op")),
        );
    }

    #[test]
    fn internal_sinks_point_inside_the_bundle() {
        assert_eq!(
            GENERIC_OP.rules_file_target(Path::new("/proj")),
            Path::new("/proj/tome-op/AGENTS.md"),
        );
        assert_eq!(
            GENERIC_OP.mcp_config_path(Path::new("/proj"), Path::new("/h")),
            Path::new("/proj/tome-op/.mcp.json"),
        );
    }
}
