//! `tome doctor [--fix] [--verify]` CLI surface. The heavy lifting
//! (`assemble_report` + per-subsystem checks + `--fix` repairs) lives
//! in `crate::doctor`; this module is the thin arg-validation + emit +
//! exit-code layer.

use std::io::Write;
use std::path::PathBuf;

use crate::cli::DoctorArgs;
use crate::doctor::{self, DoctorClassification, DoctorReport};
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::presentation::colour;
use crate::workspace::ResolvedScope;

pub fn run(args: DoctorArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    let home = home_dir().ok_or_else(|| {
        TomeError::Io(std::io::Error::other(
            "HOME is not set — cannot probe for harness directories",
        ))
    })?;

    let mut report = doctor::assemble_report(scope, &paths, &home, args.verify)?;

    if args.fix {
        let _attempts = doctor::fixes::apply(&mut report, &paths, &scope.scope);
        doctor::fixes::re_assemble(&mut report);
    }

    emit(&report, mode)?;

    // Exit-code semantics per `contracts/doctor.md`:
    // - Overall Ok → exit 0.
    // - Overall Degraded / Unhealthy → exit 1 (the report classifies).
    // - `--fix` ran but un-fixable issues remain → exit 75 instead
    //   of 1 (communicates "fix did something, but the work isn't
    //   done").
    let remaining_manual = doctor::fixes::has_remaining_manual_fixes(&report);
    let overall_ok = matches!(report.overall, DoctorClassification::Ok);
    if overall_ok {
        return Ok(());
    }
    if args.fix && remaining_manual {
        return Err(TomeError::DoctorFixNotSafe {
            subsystem: report
                .suggested_fixes
                .iter()
                .find(|f| !f.auto_fixable)
                .map(|f| f.subsystem.clone())
                .unwrap_or_else(|| "unknown".to_owned()),
        });
    }
    std::process::exit(1);
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn emit(report: &DoctorReport, mode: Mode) -> Result<(), TomeError> {
    match mode {
        Mode::Human => emit_human(report),
        Mode::Json => write_json(report),
    }
}

fn emit_human(report: &DoctorReport) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    let tty = crate::output::stdout_is_tty();

    // Glyphs match status's idiom: green tick on TTY, ASCII fallback
    // otherwise. Stays cheap to render in pipes.
    let (ok, warn, fail, info) = if tty {
        (
            colour::success("✓").to_string(),
            colour::warning("⚠").to_string(),
            colour::error("✗").to_string(),
            "·".to_owned(),
        )
    } else {
        (
            "[ok]".to_owned(),
            "[warn]".to_owned(),
            "[fail]".to_owned(),
            "[—]".to_owned(),
        )
    };
    let model_glyph = |state: &str| -> String {
        if state == "ok" {
            format!("{ok} ok")
        } else {
            format!("{fail} {state}")
        }
    };

    writeln!(out, "Tome:            {}", report.tome_version)?;
    writeln!(out)?;

    writeln!(
        out,
        "Workspace:       {}",
        match (report.workspace.scope, report.workspace.path.as_deref(),) {
            (crate::workspace::ScopeKind::Workspace, Some(p)) => p.display().to_string(),
            _ => "(global)".to_owned(),
        }
    )?;
    writeln!(
        out,
        "  resolved via:  {}",
        match report.workspace.source {
            crate::workspace::ScopeSource::Flag => "--workspace flag",
            crate::workspace::ScopeSource::GlobalFlag => "--global flag",
            crate::workspace::ScopeSource::Env => "TOME_WORKSPACE env",
            crate::workspace::ScopeSource::CwdWalk => "CWD walk",
            crate::workspace::ScopeSource::GlobalFallback => "global fallback",
        }
    )?;
    writeln!(out, "  catalogs:      {}", report.workspace.catalogs)?;
    writeln!(
        out,
        "  plugins:       {} total, {} enabled",
        report.workspace.plugins_total, report.workspace.plugins_enabled,
    )?;
    writeln!(out)?;

    writeln!(out, "Models:")?;
    writeln!(
        out,
        "  embedder       {} ({})  {}",
        report.embedder.name,
        report.embedder.version,
        model_glyph(&report.embedder.state),
    )?;
    writeln!(
        out,
        "  reranker       {} ({})  {}",
        report.reranker.name,
        report.reranker.version,
        model_glyph(&report.reranker.state),
    )?;
    writeln!(out)?;

    if report.index.present {
        writeln!(
            out,
            "Index database:  {} ({} plugins enabled, {} skills indexed, {})",
            if report.index.integrity_ok {
                &ok
            } else {
                &fail
            },
            report.index.plugins_enabled,
            report.index.skills_indexed,
            human_size(report.index.size_bytes),
        )?;
        if let Some(v) = report.index.schema_version {
            writeln!(out, "Schema version:  {v}")?;
        }
    } else {
        writeln!(out, "Index database:  not yet bootstrapped")?;
    }
    writeln!(out, "Drift:           {}", drift_label(&report.drift))?;
    writeln!(out)?;

    writeln!(out, "Catalog caches:")?;
    if report.catalogs.is_empty() {
        writeln!(out, "  (none registered in this scope)")?;
    } else {
        for c in &report.catalogs {
            // Orphan clones render with the info glyph (not the failure
            // glyph) since they're informational per
            // `catalog-extensions-p3.md` §"Doctor reporting".
            let glyph = match c.state {
                doctor::CatalogCacheState::Ok => ok.clone(),
                doctor::CatalogCacheState::Orphan => info.clone(),
                _ => fail.clone(),
            };
            let suffix = match c.state {
                doctor::CatalogCacheState::Ok => String::new(),
                doctor::CatalogCacheState::Missing => " missing".into(),
                doctor::CatalogCacheState::NotARepo => " not a git repo".into(),
                doctor::CatalogCacheState::ManifestInvalid => " manifest invalid".into(),
                doctor::CatalogCacheState::Orphan => {
                    format!(" orphan at {}", c.cache_path.display())
                }
            };
            writeln!(out, "  {:30}     {}{}", c.name, glyph, suffix)?;
        }
    }
    writeln!(out)?;

    // FR-M-DOC-2: workspace-registry status line.
    if report.workspace_registry.present {
        writeln!(
            out,
            "Workspace registry: opt-in (file present, {} tracked)",
            report.workspace_registry.tracked,
        )?;
    } else {
        writeln!(out, "Workspace registry: opt-in (file absent)")?;
    }
    writeln!(out)?;

    writeln!(out, "Detected harnesses:")?;
    for h in &report.harnesses {
        let glyph = if h.present { &ok } else { &info };
        let label = harness_display_name(&h.name);
        if h.present {
            writeln!(out, "  {glyph} {label:14} {}", h.path.display())?;
        } else {
            writeln!(
                out,
                "  {glyph} {label:14} {}    (not detected)",
                h.path.display()
            )?;
        }
    }
    writeln!(out)?;

    if !report.suggested_fixes.is_empty() {
        writeln!(out, "Suggested fixes:")?;
        for f in &report.suggested_fixes {
            let auto = if f.auto_fixable { " (auto)" } else { "" };
            writeln!(out, "  {}: {}{auto}", f.subsystem, f.diagnosis)?;
            writeln!(out, "    {}", f.command)?;
        }
        writeln!(out)?;
    }

    let overall_label = match report.overall {
        DoctorClassification::Ok => format!("{ok} healthy"),
        DoctorClassification::Degraded => format!("{warn} degraded"),
        DoctorClassification::Unhealthy => format!("{fail} unhealthy"),
    };
    writeln!(out, "Overall:         {overall_label}")?;
    Ok(())
}

fn harness_display_name(machine_name: &str) -> &'static str {
    match machine_name {
        "claude_code" => "Claude Code",
        "codex" => "Codex",
        "cursor" => "Cursor",
        "gemini" => "Gemini CLI",
        "opencode" => "OpenCode",
        "continue" => "Continue",
        _ => "unknown",
    }
}

fn drift_label(d: &crate::index::meta::DriftStatus) -> String {
    use crate::index::meta::DriftStatus;
    match d {
        DriftStatus::None => "none".to_owned(),
        DriftStatus::EmbedderNameDrift { stored, configured } => {
            format!("embedder name drift (stored: {stored}, configured: {configured})")
        }
        DriftStatus::EmbedderVersionDrift { stored, configured } => {
            format!("embedder version drift (stored: {stored}, configured: {configured})")
        }
        DriftStatus::RerankerDrift { stored, configured } => {
            format!("reranker drift (stored: {stored}, configured: {configured})")
        }
        DriftStatus::SummariserDrift { stored, configured } => {
            format!("summariser drift (stored: {stored}, configured: {configured})")
        }
    }
}

fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let kib = bytes as f64 / 1024.0;
    if kib < 1024.0 {
        return format!("{:.1} KiB", kib);
    }
    let mib = kib / 1024.0;
    format!("{:.1} MiB", mib)
}
