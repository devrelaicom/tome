//! `tome plugin convert <SOURCE>` — convert a Claude Code plugin into a native
//! Tome plugin (a copy; the source is never modified). Thin
//! arg→`authoring::convert`→emit wrapper following the silent-compute / emit
//! split: [`crate::authoring::convert::run`] does the I/O-bounded compute and
//! returns a structured outcome; this wrapper renders it per output mode.
//!
//! Slice scope (US2): **local** plugin sources. Remote `SOURCE` fetching and
//! `--into` injection land in later slices and currently return a clear usage
//! error rather than a silent partial behaviour.

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::authoring::convert::{self, ConvertConfig};
use crate::authoring::detect::ArtifactLevel;
use crate::cli::ConvertArgs;
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::workspace::ResolvedScope;

pub fn run(args: ConvertArgs, _scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    if args.into.is_some() {
        return Err(TomeError::Usage(
            "`--into` injection lands in a later slice; use `--output` for now".to_owned(),
        ));
    }

    // Local sources only this slice. A non-existent path is treated as a remote
    // SOURCE, which is not yet fetched.
    let source_path = Path::new(&args.source);
    if !source_path.exists() {
        return Err(TomeError::Usage(format!(
            "source `{}` is not a local path; remote sources (owner/repo, git URL) land in a later slice",
            args.source
        )));
    }

    let new_name =
        convert::resolve_requested_name(args.name.as_deref(), args.name_flag.as_deref())?;
    let output_dir = args.output.unwrap_or_else(|| PathBuf::from("."));

    let cfg = ConvertConfig {
        level: ArtifactLevel::Plugin,
        from: args.from,
        new_name,
        strict: args.strict,
        force: args.force,
        dry_run: args.dry_run,
        output_dir,
    };

    let outcome = convert::run(source_path, &cfg)?;
    emit_report(&outcome, mode)
}

/// Render the conversion outcome: a human summary, or a JSONL action stream
/// (one diagnostic per line + a final `result` line) under `--json` (FR-015).
fn emit_report(outcome: &convert::ConvertOutcome, mode: Mode) -> Result<(), TomeError> {
    match mode {
        Mode::Json => {
            for d in &outcome.report.diagnostics {
                write_json(&json!({
                    "type": "diagnostic",
                    "severity": d.severity.as_str(),
                    "rule": d.rule_id,
                    "message": d.message,
                }))?;
            }
            write_json(&json!({
                "type": "result",
                "harness": outcome.harness.as_str(),
                "level": outcome.level.as_str(),
                "source_name": outcome.source_name,
                "final_name": outcome.final_name,
                "target": outcome.target.display().to_string(),
                "dry_run": outcome.dry_run,
                "written": outcome.written.len(),
                "errors": outcome.report.errors,
                "warnings": outcome.report.warnings,
                "infos": outcome.report.infos,
            }))?;
        }
        Mode::Human => {
            let verb = if outcome.dry_run {
                "Would convert"
            } else {
                "Converted"
            };
            println!(
                "{verb} {} plugin `{}` → `{}`",
                outcome.harness.as_str(),
                outcome.source_name,
                outcome.final_name
            );
            for d in &outcome.report.diagnostics {
                println!("  [{}] {}: {}", d.severity.as_str(), d.rule_id, d.message);
            }
            println!(
                "{} {} file(s) to {}  ({} warning(s), {} info(s))",
                if outcome.dry_run { "Dry run:" } else { "Done:" },
                outcome.written.len(),
                outcome.target.display(),
                outcome.report.warnings,
                outcome.report.infos,
            );
        }
    }
    Ok(())
}
