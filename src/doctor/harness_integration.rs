//! Per-harness rules-file + MCP-config integration check (T367).
//!
//! For each harness in the resolved effective list, two checks run:
//!
//! 1. **Rules-file**: classifies the target file at
//!    [`HarnessModule::rules_file_target`] based on the harness's
//!    [`RulesFileStrategy`]:
//!
//!    - `BlockInExistingFile`: parse the file, look for the Tome block,
//!      compute the expected body via [`BlockBodyStyle`], compare.
//!    - `StandaloneFile`: existence-only check.
//!
//! 2. **MCP-config**: parse the harness's MCP config file (JSON or
//!    TOML), look at the `tome` key under the documented parent key,
//!    classify against:
//!
//!    - Absent → `Broken`.
//!    - Present, Tome-owned, `--workspace` arg matches the resolved
//!      workspace → `Ok`.
//!    - Present, Tome-owned, `--workspace` arg is stale → `Drift`.
//!    - Present, NOT Tome-owned (developer-authored) → `UserOwned`.
//!
//! The check is read-only — never mutates anything. Errors that aren't
//! `NotFound` collapse to the most-pessimistic classification (`Broken`)
//! so doctor surfaces failures rather than blowing up on them.

use std::path::Path;

use crate::harness::{BlockBodyStyle, RulesFileStrategy, lookup, mcp_config, rules_file};
use crate::settings::resolver::EffectiveHarnessList;
use crate::workspace::WorkspaceName;

use super::report::{HarnessSubsystemReport, SubsystemHealth};

/// Run the per-harness rules-file + MCP-config checks for every harness
/// in `effective_list`. Returns `(rules, mcp)` — one entry per harness
/// in the input list, in the same order.
pub fn check_harness_integration(
    project_root: &Path,
    effective_list: &EffectiveHarnessList,
    home: &Path,
    workspace_name: &WorkspaceName,
) -> (Vec<HarnessSubsystemReport>, Vec<HarnessSubsystemReport>) {
    let mut rules = Vec::with_capacity(effective_list.harnesses.len());
    let mut mcp = Vec::with_capacity(effective_list.harnesses.len());
    for harness in &effective_list.harnesses {
        let Some(module) = lookup(&harness.name) else {
            // Harness in the effective list but not in the production
            // registry — this should be impossible (resolver validates),
            // but be defensive.
            rules.push(HarnessSubsystemReport {
                harness: harness.name.clone(),
                health: SubsystemHealth::Broken,
            });
            mcp.push(HarnessSubsystemReport {
                harness: harness.name.clone(),
                health: SubsystemHealth::Broken,
            });
            continue;
        };

        rules.push(HarnessSubsystemReport {
            harness: harness.name.clone(),
            health: check_rules_file(module, project_root),
        });
        mcp.push(HarnessSubsystemReport {
            harness: harness.name.clone(),
            health: check_mcp_config(module, project_root, home, workspace_name),
        });
    }
    (rules, mcp)
}

fn check_rules_file(
    module: &dyn crate::harness::HarnessModule,
    project_root: &Path,
) -> SubsystemHealth {
    let target = module.rules_file_target(project_root);
    match module.rules_file_strategy() {
        RulesFileStrategy::StandaloneFile => {
            if target.is_file() {
                SubsystemHealth::Ok
            } else {
                SubsystemHealth::Broken
            }
        }
        RulesFileStrategy::BlockInExistingFile => {
            let contents = match std::fs::read_to_string(&target) {
                Ok(s) => s,
                // No file → no block → broken.
                Err(_) => return SubsystemHealth::Broken,
            };
            // Look up the canonical block.
            let block = match rules_file::parse_block(&contents) {
                Ok(Some(b)) => b,
                Ok(None) => return SubsystemHealth::Broken,
                Err(_) => return SubsystemHealth::Broken,
            };
            // Expected body per the harness's BlockBodyStyle.
            let expected = expected_body(module, project_root);
            if block.body == expected {
                SubsystemHealth::Ok
            } else {
                SubsystemHealth::Drift
            }
        }
    }
}

/// Compute the expected block body for `module`. Mirrors the production
/// `harness::sync::compute_rules_body` shape but without the I/O error
/// propagation — read failures collapse to empty bodies, which makes
/// the doctor check `Drift` against any existing non-empty block (the
/// correct user-facing outcome).
fn expected_body(module: &dyn crate::harness::HarnessModule, project_root: &Path) -> String {
    match module.block_body_style() {
        BlockBodyStyle::AtInclude => {
            let rules_path = crate::paths::Paths::project_marker_rules(project_root);
            let target = module.rules_file_target(project_root);
            let parent = target.parent().unwrap_or(Path::new(""));
            format!("@{}", relative_path(parent, &rules_path).display())
        }
        BlockBodyStyle::Inline => {
            let rules_path = crate::paths::Paths::project_marker_rules(project_root);
            std::fs::read_to_string(&rules_path).unwrap_or_default()
        }
    }
}

/// Compute a `target` path relative to `base`. Falls back to an
/// absolute path when no common ancestor exists. Mirrors the shape of
/// `harness::sync::relative_path` for the AtInclude body.
fn relative_path(base: &Path, target: &Path) -> std::path::PathBuf {
    use std::path::Component;
    let base_comps: Vec<Component<'_>> = base.components().collect();
    let target_comps: Vec<Component<'_>> = target.components().collect();
    let mut shared = 0usize;
    while shared < base_comps.len()
        && shared < target_comps.len()
        && base_comps[shared] == target_comps[shared]
    {
        shared += 1;
    }
    let ups = base_comps.len() - shared;
    let mut out = std::path::PathBuf::new();
    for _ in 0..ups {
        out.push("..");
    }
    for c in &target_comps[shared..] {
        out.push(c.as_os_str());
    }
    if out.as_os_str().is_empty() {
        target.to_path_buf()
    } else {
        out
    }
}

fn check_mcp_config(
    module: &dyn crate::harness::HarnessModule,
    project_root: &Path,
    home: &Path,
    workspace_name: &WorkspaceName,
) -> SubsystemHealth {
    let path = module.mcp_config_path(project_root, home);
    let entry =
        match mcp_config::read_entry(&path, module.mcp_config_format(), module.mcp_parent_key()) {
            Ok(Some(e)) => e,
            // No file or no entry → Broken (Tome should have written one).
            Ok(None) => return SubsystemHealth::Broken,
            // Parse error or symlink refusal → Broken.
            Err(_) => return SubsystemHealth::Broken,
        };
    if !mcp_config::is_tome_owned(&entry) {
        return SubsystemHealth::UserOwned;
    }
    // Tome-owned. Check the `--workspace <name>` argument matches the
    // resolved workspace. We scan args for `--workspace` followed by
    // the value; tolerate the `=` form too (`--workspace=<name>`).
    let mut iter = entry.args.iter();
    let mut seen_workspace: Option<&str> = None;
    while let Some(arg) = iter.next() {
        if let Some(value) = arg.strip_prefix("--workspace=") {
            seen_workspace = Some(value);
            break;
        }
        if arg == "--workspace" {
            if let Some(next) = iter.next() {
                seen_workspace = Some(next.as_str());
            }
            break;
        }
    }
    match seen_workspace {
        Some(name) if name == workspace_name.as_str() => SubsystemHealth::Ok,
        Some(_) => SubsystemHealth::Drift,
        // No --workspace arg at all → stale args; classify as Drift so
        // `--fix` re-runs sync (which will add the canonical arg).
        None => SubsystemHealth::Drift,
    }
}
