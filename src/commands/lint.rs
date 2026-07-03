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

/// Lint one or more Tome artifacts at the given `level` (catalog/plugin/skill).
///
/// Each source is processed independently through [`lint_one`] with **never-halt
/// forward-progress**: a per-source failure (parse / level-mismatch / autofix
/// I/O) is captured as that source's failed outcome and does NOT abort the
/// remaining sources. The command exit code is the WORST verdict across all
/// sources — any source with errors (or a pre-report failure) → 85; else `--strict`
/// with any warnings → 86; else 0.
///
/// Output keeps single-source shapes byte-identical: for exactly one source the
/// `--json` stream is the pre-#326 single `{ findings, summary }` object and the
/// human output is unchanged. For multiple sources the JSON becomes JSONL — one
/// `{ source, findings, summary }` record per source — and human output prints a
/// per-source section (a source header + its report).
pub fn run(
    args: LintArgs,
    _scope: &ResolvedScope,
    mode: Mode,
    level: ArtifactLevel,
) -> Result<(), TomeError> {
    let strict = args.strict;

    // SINGLE-SOURCE (the pre-#326 shape, kept byte-identical): compute, emit the
    // single `{ findings, summary }` object (or unchanged human output), and —
    // critically — PROPAGATE a pre-report failure as-is. A parse error stays
    // `Usage`/2, an autofix I/O error stays `Io`/7; only a produced report maps
    // to the lint verdict codes (0/85/86). Converting these to the aggregate 85
    // would break back-compat, so the single path never reaches the aggregate.
    if args.sources.len() == 1 {
        let source = &args.sources[0];
        let result = lint_one(source, level, args.autofix, args.dry_run);
        emit_lint_telemetry(level, strict, &result.as_ref().map(|o| &o.report));
        let outcome = result?;
        emit_report(
            &outcome.report,
            outcome.fixed,
            args.autofix,
            args.dry_run,
            mode,
        )?;
        return outcome.report.into_result(strict);
    }

    // MULTI-SOURCE: never-halt forward-progress. Each source is linted
    // independently; a per-source failure (parse / level-mismatch / autofix I/O)
    // is captured, emitted as that source's failed record, and does NOT abort the
    // rest. The exit code is the WORST verdict across all sources.
    let mut any_errors = false;
    let mut any_warnings = false;

    for source in &args.sources {
        // A produced report is telemetered by its verdict; an `Err` is
        // telemetered as `Errors` — one emit PER source, mirroring single-source.
        let result = lint_one(source, level, args.autofix, args.dry_run);
        emit_lint_telemetry(level, strict, &result.as_ref().map(|o| &o.report));

        // Track the aggregate verdict. A pre-report failure counts as an error
        // for the aggregate (it is surfaced as an error record below), so
        // `Err` ⇒ `any_errors` — never-halt: we record it and continue.
        match &result {
            Ok(outcome) => {
                if outcome.report.errors > 0 {
                    any_errors = true;
                }
                if outcome.report.warnings > 0 {
                    any_warnings = true;
                }
            }
            Err(_) => any_errors = true,
        }

        emit_source_multi(source, &result, args.autofix, args.dry_run, mode)?;
    }

    // Aggregate exit code, computed AFTER the loop (never early-return on the
    // first source's verdict): worst-of. Reuse the closed-set variants —
    // `ValidationFoundErrors`/85, `ValidationStrictWarnings`/86. The
    // `errors`/`warnings` payloads are aggregate flags (≥ 1), not exact counts:
    // the exact per-source tallies live in each emitted record.
    if any_errors {
        Err(TomeError::ValidationFoundErrors { errors: 1 })
    } else if strict && any_warnings {
        Err(TomeError::ValidationStrictWarnings { warnings: 1 })
    } else {
        Ok(())
    }
}

/// One source's lint compute output: the report plus the applied-fix count
/// (`--autofix`; 0 otherwise) needed to render the per-source report.
struct LintOneOutcome {
    report: LintReport,
    fixed: usize,
}

/// The per-source lint compute: parse (through the `UntrustedRoot`-guarded
/// [`parse_artifact`], which refuses a symlinked component before any read) →
/// level-check → autofix-or-lint. Returns the produced report + fix count, or
/// the pre-report failure (parse / level-mismatch / autofix I/O). No I/O beyond
/// the artifact reads + `--autofix` writes; emission is the caller's so a
/// failure here is captured, not halted.
fn lint_one(
    source: &Path,
    level: ArtifactLevel,
    autofix_on: bool,
    dry_run: bool,
) -> Result<LintOneOutcome, TomeError> {
    // Parse once to detect + validate the artifact matches the command level.
    let artifact = parse_artifact(source)?;
    check_artifact_level(&artifact, level)?;

    let (report, fixed) = if autofix_on {
        let outcome = autofix(source, dry_run)?;
        (outcome.report, outcome.fixed)
    } else {
        (lint::run(&artifact, &rules::all()), 0)
    };
    Ok(LintOneOutcome { report, fixed })
}

/// Emit one `tome.authoring_action{verb=Lint}` event PER source with the REAL
/// outcome: errors → `Errors`; strict + warnings → `StrictRefused`; warnings →
/// `Warnings`; clean → `Ok`. A pre-report failure (the `Err` arm) is `Errors`.
/// Takes the report by reference so the caller keeps ownership of the outcome
/// for rendering.
fn emit_lint_telemetry(
    level: ArtifactLevel,
    strict: bool,
    result: &Result<&LintReport, &TomeError>,
) {
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
    crate::telemetry::emit(AuthoringActionEvent {
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
/// (distinct from `convert`'s JSONL stream). Each finding uses the shared
/// [`crate::authoring::ir::finding_json`] serializer that `convert --json` also
/// calls, so the two verbs' per-finding fields cannot drift (issue #299).
fn lint_json(report: &LintReport, fixed: usize) -> serde_json::Value {
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(crate::authoring::ir::finding_json)
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

/// Emit ONE source's record in the multi-source stream.
///
/// JSON (`--json`): a JSONL line — the single-source `{ findings, summary }`
/// object with a leading `source` attribution field added ONLY here (the
/// single-source path never wraps, keeping its shape byte-identical). A failed
/// source (parse / level-mismatch / autofix I/O) still emits a record: its error
/// is surfaced as a synthetic `lint/source-failed` error finding so the stream
/// accounts for EVERY source and the reader sees why it failed.
///
/// Human: a `== <source> ==` section header, then this source's report (the
/// same body as the single-source human output), or a one-line failure notice.
fn emit_source_multi(
    source: &Path,
    result: &Result<LintOneOutcome, TomeError>,
    autofix_on: bool,
    dry_run: bool,
    mode: Mode,
) -> Result<(), TomeError> {
    let display = source.display();
    match (mode, result) {
        (Mode::Json, Ok(outcome)) => {
            let mut value = lint_json(&outcome.report, outcome.fixed);
            prepend_source(&mut value, source);
            write_json(&value)?;
        }
        (Mode::Json, Err(e)) => {
            // A pre-report failure: emit a record whose sole finding IS the
            // failure, so the JSONL stream has one object per source.
            let finding = json!({
                "rule": "lint/source-failed",
                "severity": "error",
                "message": e.to_string(),
                "file": display.to_string(),
                "line": serde_json::Value::Null,
                "autofixable": false,
            });
            let value = json!({
                "source": display.to_string(),
                "error": e.to_string(),
                "findings": [finding],
                "summary": { "errors": 1, "warnings": 0, "infos": 0, "fixed": 0 },
            });
            write_json(&value)?;
        }
        (Mode::Human, Ok(outcome)) => {
            println!("== {display} ==");
            emit_report(&outcome.report, outcome.fixed, autofix_on, dry_run, mode)?;
        }
        (Mode::Human, Err(e)) => {
            println!("== {display} ==");
            println!("[error] lint/source-failed: {e}");
            println!("Summary: 1 error(s), 0 warning(s), 0 info(s)");
        }
    }
    Ok(())
}

/// Insert a leading `source` attribution field into a single-source lint JSON
/// object. Used ONLY on the multi-source JSONL path; `serde_json`'s
/// `preserve_order` keeps `source` first, then the untouched `findings`/`summary`.
fn prepend_source(value: &mut serde_json::Value, source: &Path) {
    if let Some(obj) = value.as_object_mut() {
        let mut with_source = serde_json::Map::new();
        with_source.insert("source".to_owned(), json!(source.display().to_string()));
        for (k, v) in std::mem::take(obj) {
            with_source.insert(k, v);
        }
        *obj = with_source;
    }
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

    #[test]
    fn single_source_lint_json_carries_no_source_field() {
        // #326: the single-source object stays byte-identical — top-level keys
        // are exactly `findings`/`summary`, no `source` attribution.
        let report = LintReport::from_diagnostics(vec![Diagnostic::warning("lint/w", "hmm")]);
        let v = lint_json(&report, 0);
        let obj = v.as_object().unwrap();
        assert!(obj.contains_key("findings"));
        assert!(obj.contains_key("summary"));
        assert!(
            !obj.contains_key("source"),
            "single-source object must not gain a `source` field: {v}"
        );
    }

    #[test]
    fn prepend_source_wraps_multi_record_with_source_first() {
        // #326: the multi-source JSONL record is the single object with a leading
        // `source` field; `findings`/`summary` are unchanged and follow it.
        let report = LintReport::from_diagnostics(vec![Diagnostic::error("lint/x", "boom")]);
        let mut v = lint_json(&report, 0);
        prepend_source(&mut v, std::path::Path::new("plugins/b"));

        // `source` is present and first (preserve_order), then the unchanged body.
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["source", "findings", "summary"]);
        assert_eq!(v["source"], "plugins/b");
        // The body is byte-identical to the single-source object.
        let bare = lint_json(&report, 0);
        assert_eq!(v["findings"], bare["findings"]);
        assert_eq!(v["summary"], bare["summary"]);
    }
}
