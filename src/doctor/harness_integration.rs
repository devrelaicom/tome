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

use crate::harness::{
    BlockBodyStyle, RulesFileStrategy, mcp_config, rules_file, with_effective_modules,
};
use crate::settings::resolver::EffectiveHarnessList;
use crate::workspace::WorkspaceName;

use super::report::{HarnessSubsystemReport, SubsystemHealth};

/// Run the per-harness rules-file + MCP-config checks for every harness
/// in `effective_list`. Returns `(rules, mcp)` — one entry per harness
/// in the input list, in the same order.
///
/// Uses `with_effective_modules` rather than `lookup` so test-installed
/// overrides (`HARNESS_MODULES_OVERRIDE`) are respected: the stub
/// harness used by US5.b's `tests/doctor_fix_p4.rs` is invisible to
/// `lookup` (the production-only `&'static` registry) but visible
/// through `with_effective_modules`.
pub fn check_harness_integration(
    project_root: &Path,
    effective_list: &EffectiveHarnessList,
    home: &Path,
    workspace_name: &WorkspaceName,
) -> (Vec<HarnessSubsystemReport>, Vec<HarnessSubsystemReport>) {
    let mut rules = Vec::with_capacity(effective_list.harnesses.len());
    let mut mcp = Vec::with_capacity(effective_list.harnesses.len());
    for harness in &effective_list.harnesses {
        let (rules_health, mcp_health) = with_effective_modules(|modules| {
            match modules.iter().find(|m| m.name() == harness.name) {
                Some(module) => {
                    // The rules-body style is a GROUP decision (union across
                    // every LIVE sharer of this rules path), mirroring the
                    // writer's `harness::sync::group_body_style`. Resolve it
                    // here — inside the registry guard, where the live sharer
                    // modules are visible — and pass it down, rather than
                    // re-entering `with_effective_modules` from `expected_body`
                    // (the guard is non-reentrant; see its doc comment).
                    let style = group_body_style(*module, modules, effective_list, project_root);
                    (
                        check_rules_file(*module, project_root, style),
                        check_mcp_config(*module, project_root, home, workspace_name),
                    )
                }
                // Harness in the effective list but not in the
                // effective registry — should be impossible (resolver
                // validates) but be defensive.
                None => (SubsystemHealth::Broken, SubsystemHealth::Broken),
            }
        });
        rules.push(HarnessSubsystemReport {
            harness: harness.name.clone(),
            health: rules_health,
        });
        mcp.push(HarnessSubsystemReport {
            harness: harness.name.clone(),
            health: mcp_health,
        });
    }
    (rules, mcp)
}

fn check_rules_file(
    module: &dyn crate::harness::HarnessModule,
    project_root: &Path,
    style: BlockBodyStyle,
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
            let contents = match crate::util::bounded_read_to_string(
                &target,
                crate::util::HARNESS_RULES_MAX,
            ) {
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
            // Expected body per the GROUP's resolved BlockBodyStyle (union
            // across live sharers; see `group_body_style`). `target` is this
            // module's rules-file path; every group sharer targets the same
            // path, so it is the writer's `rules_path` grouping key — pass it
            // for the AtInclude relative-path computation.
            let expected = expected_body(style, &target, project_root);
            if block.body == expected {
                SubsystemHealth::Ok
            } else {
                SubsystemHealth::Drift
            }
        }
    }
}

/// Resolve the GROUP body style for `module`'s rules path: the union across
/// every LIVE sharer of that path. This is the doctor-side mirror of the
/// writer's [`crate::harness::sync::group_body_style`] — both must agree, or
/// `doctor` reports a permanent false-positive `Drift` (the writer lands the
/// group's inline body into a shared file while doctor expects the single
/// module's own AtInclude style, and `--fix` re-runs the same writer so it
/// never converges).
///
/// `modules` is the registry slice (already under the `with_effective_modules`
/// guard at the call site); `effective_list` names the live harnesses. A
/// sharer is any module whose `rules_file_target` equals `module`'s AND whose
/// name is in the effective list — the same path-equality + OR-of-live grouping
/// the writer applies. `Inline` wins the moment any live sharer requires it
/// (an include-incapable harness, e.g. OpenCode, would read `@.tome/RULES.md`
/// as prose); an include-only group stays `AtInclude`.
fn group_body_style(
    module: &dyn crate::harness::HarnessModule,
    modules: &[&dyn crate::harness::HarnessModule],
    effective_list: &EffectiveHarnessList,
    project_root: &Path,
) -> BlockBodyStyle {
    let target = module.rules_file_target(project_root);
    let any_inline = modules.iter().any(|m| {
        effective_list.harnesses.iter().any(|h| h.name == m.name())
            && m.rules_file_target(project_root) == target
            && m.block_body_style() == BlockBodyStyle::Inline
    });
    if any_inline {
        BlockBodyStyle::Inline
    } else {
        BlockBodyStyle::AtInclude
    }
}

/// Compute the expected block body for the resolved GROUP `style`. Mirrors the
/// production `harness::sync::compute_rules_body` shape but without the I/O
/// error propagation — read failures collapse to empty bodies, which makes the
/// doctor check `Drift` against any existing non-empty block (the correct
/// user-facing outcome).
///
/// `style` is the GROUP's lowest-common-denominator style (see
/// [`group_body_style`]), NOT the single module's own — a shared path with any
/// inline sharer renders inline, matching exactly what the writer wrote.
///
/// `rules_target` is the harness's rules-file path (the group's shared path);
/// the AtInclude relative-path is computed from its parent, mirroring the
/// writer's `compute_rules_body`, which keys the include directive off the
/// shared `rules_path`.
fn expected_body(style: BlockBodyStyle, rules_target: &Path, project_root: &Path) -> String {
    match style {
        BlockBodyStyle::AtInclude => {
            let rules_path = crate::paths::Paths::project_marker_rules(project_root);
            let parent = rules_target.parent().unwrap_or(Path::new(""));
            format!("@{}", relative_path(parent, &rules_path).display())
        }
        BlockBodyStyle::Inline => {
            let rules_path = crate::paths::Paths::project_marker_rules(project_root);
            crate::util::bounded_read_to_string(&rules_path, crate::util::HARNESS_RULES_MAX)
                .unwrap_or_default()
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
    let entry = match mcp_config::read_entry(&path, &module.mcp_dialect()) {
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
