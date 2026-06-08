//! Shared wrapper for `tome {catalog,plugin,skill} lint` — parse a native Tome
//! artifact, run the rule registry, optionally `--autofix`, and report per mode
//! (`--json` is a single `{ findings, summary }` object, distinct from
//! `convert`'s JSONL stream). Read-only by default; never opens or locks the
//! central index.

use std::path::Path;

use serde_json::json;

use crate::authoring::detect::ArtifactLevel;
use crate::authoring::ir::Artifact;
use crate::authoring::lint::autofix::autofix;
use crate::authoring::lint::parse::parse_artifact;
use crate::authoring::lint::{self, LintReport, rules};
use crate::cli::LintArgs;
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::workspace::ResolvedScope;

/// Lint a Tome artifact at the given `level` (catalog/plugin/skill).
pub fn run(
    args: LintArgs,
    _scope: &ResolvedScope,
    mode: Mode,
    level: ArtifactLevel,
) -> Result<(), TomeError> {
    let source = Path::new(&args.source);

    // Parse once to detect + validate the artifact matches the command level.
    let artifact = parse_artifact(source)?;
    check_artifact_level(&artifact, level)?;

    let (report, fixed) = if args.autofix {
        let outcome = autofix(source, args.dry_run)?;
        (outcome.report, outcome.fixed)
    } else {
        (lint::run(&artifact, &rules::all()), 0)
    };

    emit_report(&report, fixed, args.autofix, args.dry_run, mode)?;
    report.into_result(args.strict)
}

/// Reject `plugin lint <catalog>` / `skill lint <plugin>` etc.
fn check_artifact_level(artifact: &Artifact, level: ArtifactLevel) -> Result<(), TomeError> {
    let actual = match artifact {
        Artifact::Catalog(_) => ArtifactLevel::Catalog,
        Artifact::Plugin(_) => ArtifactLevel::Plugin,
        Artifact::Skill(_) => ArtifactLevel::Skill,
    };
    if actual == level {
        Ok(())
    } else {
        Err(TomeError::Usage(format!(
            "`{} lint` expected a {} but found a {}",
            level.as_str(),
            level.as_str(),
            actual.as_str()
        )))
    }
}

fn emit_report(
    report: &LintReport,
    fixed: usize,
    autofix_on: bool,
    dry_run: bool,
    mode: Mode,
) -> Result<(), TomeError> {
    match mode {
        Mode::Json => {
            let findings: Vec<_> = report
                .diagnostics
                .iter()
                .map(|d| {
                    json!({
                        "rule": d.rule_id,
                        "severity": d.severity.as_str(),
                        "message": d.message,
                        "file": d.location.as_ref().map(|l| l.file.display().to_string()),
                        "line": d.location.as_ref().and_then(|l| l.line),
                        "autofixable": d.autofix.is_some(),
                    })
                })
                .collect();
            write_json(&json!({
                "findings": findings,
                "summary": {
                    "errors": report.errors,
                    "warnings": report.warnings,
                    "infos": report.infos,
                    "fixed": fixed,
                },
            }))?;
        }
        Mode::Human => {
            for d in &report.diagnostics {
                let loc = d
                    .location
                    .as_ref()
                    .map(|l| format!(" ({})", l.file.display()))
                    .unwrap_or_default();
                println!(
                    "[{}] {}: {}{loc}",
                    d.severity.as_str(),
                    d.rule_id,
                    d.message
                );
            }
            if autofix_on {
                println!(
                    "{} {fixed} fix(es){}",
                    if dry_run { "Would apply" } else { "Applied" },
                    if dry_run { " (dry-run)" } else { "" }
                );
            }
            println!(
                "Summary: {} error(s), {} warning(s), {} info(s)",
                report.errors, report.warnings, report.infos
            );
        }
    }
    Ok(())
}
