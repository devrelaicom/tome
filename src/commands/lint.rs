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
    let strict = args.strict;
    // Run the lint compute into a Result<LintReport>. A pre-report failure
    // (parse / level-mismatch / autofix I/O) short-circuits with an Err and is
    // telemetered as `Errors`; a produced report is telemetered by its verdict.
    let result = (|| -> Result<LintReport, TomeError> {
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
        Ok(report)
    })();

    // One `tome.authoring_action{verb=Lint}` emit with the REAL outcome.
    // source_format is `Unknown`: lint runs over a NATIVE Tome artifact, there
    // is no foreign source format.
    emit_lint_telemetry(level, strict, &result);

    let report = result?;
    report.into_result(strict)
}

/// Emit one `tome.authoring_action{verb=Lint}` event with the REAL outcome:
/// errors → `Errors`; strict + warnings → `StrictRefused`; warnings →
/// `Warnings`; clean → `Ok`. A pre-report failure (the `Err` arm) is `Errors`.
fn emit_lint_telemetry(level: ArtifactLevel, strict: bool, result: &Result<LintReport, TomeError>) {
    use crate::telemetry::event::{
        AuthoringActionEvent, AuthoringOutcome, AuthoringVerb, SourceFormat,
    };
    let outcome = match result {
        Ok(report) => {
            if report.errors > 0 {
                AuthoringOutcome::Errors
            } else if strict && report.warnings > 0 {
                AuthoringOutcome::StrictRefused
            } else if report.warnings > 0 {
                AuthoringOutcome::Warnings
            } else {
                AuthoringOutcome::Ok
            }
        }
        Err(_) => AuthoringOutcome::Errors,
    };
    crate::telemetry::enqueue(AuthoringActionEvent {
        verb: AuthoringVerb::Lint,
        artifact: crate::commands::convert::artifact_of(level),
        source_format: SourceFormat::Unknown,
        outcome,
    });
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
        Mode::Json => write_json(&lint_json(report, fixed))?,
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

/// Build the `--json` report object: a single `{ findings[], summary }`
/// (distinct from `convert`'s JSONL stream).
fn lint_json(report: &LintReport, fixed: usize) -> serde_json::Value {
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
    json!({
        "findings": findings,
        "summary": {
            "errors": report.errors,
            "warnings": report.warnings,
            "infos": report.infos,
            "fixed": fixed,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring::ir::{Diagnostic, Provenance};

    #[test]
    fn check_artifact_level_matches_and_rejects() {
        let cat = Artifact::Catalog(crate::authoring::ir::CatalogIr {
            name: "c".into(),
            version: "1.0.0".into(),
            description: "d".into(),
            owner: crate::catalog::manifest::Owner {
                name: "o".into(),
                email: "o@x.io".into(),
            },
            plugins: Vec::new(),
            provenance: Provenance::local("tome", std::path::PathBuf::from("c")),
            diagnostics: Vec::new(),
        });
        assert!(check_artifact_level(&cat, ArtifactLevel::Catalog).is_ok());
        // `plugin lint <catalog>` → Usage(2).
        let err = check_artifact_level(&cat, ArtifactLevel::Plugin).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn lint_json_has_findings_and_summary_with_keys() {
        let report = LintReport::from_diagnostics(vec![Diagnostic::error("lint/x", "boom").at(
            crate::authoring::ir::Location::file(std::path::PathBuf::from("a/SKILL.md")),
        )]);
        let v = lint_json(&report, 3);
        assert_eq!(v["summary"]["errors"], 1);
        assert_eq!(v["summary"]["fixed"], 3);
        let f = &v["findings"][0];
        assert_eq!(f["rule"], "lint/x");
        assert_eq!(f["severity"], "error");
        assert_eq!(f["file"], "a/SKILL.md");
        assert_eq!(f["autofixable"], false);
    }
}
