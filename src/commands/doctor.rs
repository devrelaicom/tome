//! `tome doctor [--fix] [--verify]` CLI surface. The heavy lifting
//! (`assemble_report` + per-subsystem checks + `--fix` repairs) lives
//! in `crate::doctor`; this module is the thin arg-validation + emit +
//! exit-code layer.

use std::io::Write;

use crate::cli::DoctorArgs;
use crate::doctor::{self, DoctorClassification, DoctorReport};
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::presentation::colour;
use crate::workspace::ResolvedScope;

pub fn run(
    args: DoctorArgs,
    scope: &ResolvedScope,
    mode: Mode,
    verbose: bool,
) -> Result<(), TomeError> {
    // `--force` without `--fix` is a user error: there is nothing for
    // `--force` to override during a read-only report pass. Surface as
    // exit 2 (Usage) rather than exit 7 (Io) — per US5 reviewer R-M1
    // this is a usage-shape failure, not an I/O failure, and the
    // closed-set error vocabulary already encodes it.
    if args.force && !args.fix {
        return Err(TomeError::Usage(
            "`--force` requires `--fix`; rerun as `tome doctor --fix --force`".to_owned(),
        ));
    }
    // `--dry-run` previews what `--fix` would apply; without `--fix` there is
    // nothing to preview (the read-only report already IS the no-fix pass) —
    // same usage-shape classification as the bare `--force` above (issue #430).
    if args.dry_run && !args.fix {
        return Err(TomeError::Usage(
            "`--dry-run` requires `--fix`; rerun as `tome doctor --fix --dry-run`".to_owned(),
        ));
    }
    // Every mutating path below gates on this, NOT on `args.fix` directly: a
    // `--fix --dry-run` run must behave exactly like the read-only pass on
    // disk while still listing what a real `--fix` would apply.
    let apply_fixes = args.fix && !args.dry_run;

    let paths = Paths::resolve()?;
    // Route HOME through the validated `home_root()` (rejects empty/non-absolute,
    // exit 2) — the SAME resolver every meta write path uses, so doctor's
    // harness-detection home and the installer's never diverge. `home_root()`
    // already errors when HOME is unset, replacing the bespoke check here.
    let home = crate::commands::harness::home_root()?;

    // Phase 8 cutover (US1 closeout MAJOR-1): migrate any legacy model
    // `manifest.json` → `manifest.toml` BEFORE classification, so a pre-cutover
    // install (model bytes present, only the legacy JSON) is seen as healthy
    // and the model `--fix` never spuriously `remove_dir_all`s + re-downloads
    // it — honouring the contract's "No re-download" guarantee. Only under
    // `--fix` (read-only doctor must not mutate). A migration failure is
    // warned, not propagated (FR-561); the legacy manifest then simply stays
    // surfaced by `assemble_report` below.
    if apply_fixes {
        match doctor::cutover::migrate_model_manifests(&paths) {
            Ok(migrated) if !migrated.is_empty() => tracing::info!(
                count = migrated.len(),
                "doctor --fix: migrated legacy model manifest.json → manifest.toml",
            ),
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "doctor --fix: model manifest migration failed");
            }
        }
    }

    // `verify_by_default` in config.toml: effective verify = flag OR config.
    // DEFENSIVE load (issue #287): doctor is the command you run to diagnose a
    // broken setup — a malformed `~/.tome/config.toml` must not exit 5 before
    // the report renders. The parse problem is surfaced as a `SuggestedFix`
    // below (and flips `overall` to Unhealthy), so it is reported loudly, not
    // swallowed; a malformed config simply means `verify_by_default` reads as
    // the default (false). Every NON-diagnostic command still loads strictly.
    let cfg = crate::config::load_or_default(&paths);
    let verify = args.verify || cfg.doctor.verify_by_default.unwrap_or(false);

    // Read-only probe for a malformed config (issue #287). `None` when absent or
    // well-formed; `Some(message)` carries the legible diagnostic (offending
    // key/section + line/column) — identical to the strict exit-5 every other
    // command emits. Captured before `assemble_report` so the finding is folded
    // into both the report and `--fix`'s re-assembly path below.
    let config_error = crate::config::probe_error(&paths);

    let mut report = doctor::assemble_report(scope, &paths, &home, verify)?;
    apply_config_finding(&mut report, config_error.as_deref());

    if apply_fixes {
        let ctx = doctor::fixes::FixContext {
            paths: &paths,
            scope,
            home: &home,
            force: args.force,
        };
        let _attempts = doctor::fixes::apply(&mut report, &ctx);

        // Phase 9 / US4 (FR-032): repair meta-skill drift by re-running the
        // idempotent `meta::install_skill` for every stale / missing-but-
        // expected candidate (the SAME safe atomic symlink-checked path — NOT
        // a bespoke writer). Forward-progress: a per-location failure is
        // warned inside `repair` and the loop continues; the returned first
        // error is logged but never aborts the doctor pass (FR-561). The
        // report's `meta_skills` field is then re-projected (gated on "the
        // repair ran") so it reflects post-repair on-disk state.
        let meta_repair = doctor::meta_drift::repair(&home, scope);
        match &meta_repair {
            Ok(n) if *n > 0 => {
                tracing::info!(count = n, "doctor --fix: (re)installed meta skills");
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "doctor --fix: meta-skill repair had failures");
            }
        }
        // `MetaAction::Fix` is doctor's meta `--fix` repair (FR-026). Forward-
        // progress: `repair` returns the first error after continuing past
        // per-location failures → all-ok = Ok, an Err carries at least one
        // failure (some may have still landed) = Partial. One infallible emit.
        crate::telemetry::emit(crate::telemetry::event::MetaActionEvent {
            action: crate::telemetry::event::MetaAction::Fix,
            outcome: if meta_repair.is_ok() {
                crate::telemetry::event::Outcome::Ok
            } else {
                crate::telemetry::event::Outcome::Partial
            },
        });
        report.meta_skills = doctor::meta_drift::check(&home, scope);

        // FR-410: sweep orphan `.tome.tmp.*` staging directories. Best-
        // effort — per-fix failures are warn'd inside `cleanup_stale_
        // staging_dirs`. We don't propagate the count to the report
        // (no contract field for it) but it surfaces in the trace log.
        let cleaned = doctor::orphan_cleanup::cleanup_stale_staging_dirs(&paths).unwrap_or(0);
        if cleaned > 0 {
            tracing::info!(
                count = cleaned,
                "doctor --fix: removed stale `.tome.tmp.*` staging directories",
            );
        }
        doctor::fixes::re_assemble(&mut report);

        // Phase 12 / US4 (FR-017): `re_assemble` rebuilds `suggested_fixes` via
        // the shared fixes SSOT, which doesn't know about the corrupt-remote-index
        // fix. Re-check it read-only and re-append when it still holds. A
        // BUNDLED-local repair (`reindex --force`) clears
        // `meta.embedder_dimension`, so `check_corrupt_index` then reads
        // "meta absent → N/A → no finding": nothing is re-appended, the
        // re-assembled `index.integrity_ok` is back to true, and `overall`
        // returns to Ok → exit 0 (self-healed). A REMOTE mismatch is never
        // auto-fixed (paid API cost), so it re-appears as an
        // `auto_fixable: false` fix while `index.integrity_ok` stays false from
        // assembly — `overall` remains non-Ok and `has_remaining_manual_fixes`
        // is true → `DoctorFixNotSafe` (exit 75).
        doctor::reappend_corrupt_index_fix(&mut report, &paths, &cfg);

        // Issue #291: `re_assemble` rebuilds `suggested_fixes` + `overall` via
        // the shared fixes SSOT, which doesn't know about the provider-credential
        // findings. `doctor --fix` NEVER sets a user's env var, so a
        // still-missing `TOME_<NAME>_API_KEY` is never auto-fixed — re-apply the
        // findings read-only so they persist through `--fix` as non-auto-fixable
        // manual findings (→ exit 75 via `has_remaining_manual_fixes`) and keep
        // escalating `overall`, exactly like the corrupt-index / config cases.
        // Uses the strictly-loaded `cfg` (defensive-loaded in the CLI wrapper);
        // a provider referenced with a resolve error is reported by the
        // exit-93 path, not here.
        doctor::apply_provider_credential_findings(
            &mut report.suggested_fixes,
            &mut report.overall,
            &cfg,
        );

        // Issue #433: `re_assemble` rebuilds `suggested_fixes` via the shared fixes
        // SSOT, which doesn't know about the orphan-data pointers. `doctor
        // --fix` never deletes persistent data (and refreshes no orphan
        // report), so re-apply from the report's own (still-current) orphan
        // surface. These are user-clearable manual findings — they
        // deliberately remain in the exit-75 gate (see
        // `apply_orphan_data_findings`'s GATE DECISION).
        doctor::apply_orphan_data_findings(
            &mut report.suggested_fixes,
            report.orphan_data_dirs.as_ref(),
        );

        // Issue #283: `re_assemble` rebuilds `suggested_fixes` via the shared fixes
        // SSOT, which doesn't know about the fresh-install onboarding nudges.
        // `doctor --fix` never enrols a catalog / enables a plugin / configures
        // a harness (those are user product decisions), so a still-not-set-up
        // install keeps its onboarding guidance through `--fix`. `Onboarding`
        // fixes are informational (excluded from the exit-75 gate), so they
        // never change the exit code.
        doctor::reappend_onboarding_fixes(&mut report);

        // Issue #431: `re_assemble` doesn't know about hook-dispatch drift
        // either. `fixes::apply`'s harness branch refreshed
        // `report.hook_translation` after the sync, so a healed dispatch
        // surface re-appends nothing here and a still-drifted one keeps its
        // finding + the Degraded escalation.
        doctor::reappend_hook_drift_fixes(&mut report);

        // `re_assemble` rebuilds `suggested_fixes` + `overall` via the SSOT,
        // which doesn't know about the config-parse finding. `doctor --fix`
        // NEVER rewrites the user-authored `config.toml`, so a malformed config
        // is never auto-fixed — re-probe read-only and re-apply so it persists
        // through `--fix` as a non-auto-fixable manual finding (→ exit 75 via
        // `has_remaining_manual_fixes`), exactly like the corrupt-remote-index
        // case above.
        apply_config_finding(&mut report, crate::config::probe_error(&paths).as_deref());
    }

    emit(&report, mode, &paths, verbose)?;

    // Issue #430: with `--fix --dry-run`, list the repairs a real `--fix`
    // would apply — the SAME snapshot predicate `fixes::apply` uses
    // (`auto_fixable`, plus user-owned harness-MCP fixes under `--force`) —
    // then fall through to the read-only exit semantics below. Human-mode
    // only: the JSON report already carries `auto_fixable` per fix.
    if args.fix && args.dry_run && mode == Mode::Human {
        emit_fix_dry_run(&report, args.force)?;
    }

    // `tome.doctor_run`: emit AFTER the report renders but BEFORE any of the
    // exit paths below (one of which is a hard `std::process::exit` with a
    // health code that would otherwise skip the emit). `findings` is the raw number of
    // suggested-fix issues the report surfaced (the kernel buckets it). A
    // `--fix --dry-run` records `fix: false` — nothing was applied.
    crate::telemetry::emit(crate::telemetry::event::DoctorRun {
        fix: apply_fixes,
        findings: report.suggested_fixes.len() as u32,
    });

    // Exit-code semantics per `contracts/doctor.md` (issue #282 — three
    // distinct health codes, matching `tome status`):
    // - Overall Ok → exit 0.
    // - Overall Degraded → exit `EXIT_HEALTH_DEGRADED` (10): the report found
    //   a non-fatal issue (queries still serve). Distinct from Unhealthy so a
    //   CI gate can fail-on-unhealthy-only; still non-zero.
    // - Overall Unhealthy → exit `EXIT_HEALTH_UNHEALTHY` (1).
    // - `--fix` ran but un-fixable issues remain → exit 75
    //   (`DoctorFixNotSafe`) INSTEAD of the health code (communicates "fix did
    //   something, but the work isn't done"). This takes precedence over the
    //   Degraded/Unhealthy split — it is the more actionable verdict after a
    //   repair attempt, and it routes through the closed `TomeError` map.
    let remaining_manual = doctor::fixes::has_remaining_manual_fixes(&report);
    let health_code = match report.overall {
        DoctorClassification::Ok => return Ok(()),
        DoctorClassification::Degraded => crate::error::EXIT_HEALTH_DEGRADED,
        DoctorClassification::Unhealthy => crate::error::EXIT_HEALTH_UNHEALTHY,
    };
    if apply_fixes && remaining_manual {
        return Err(TomeError::DoctorFixNotSafe {
            // Issue #283: this scan and `has_remaining_manual_fixes` share the
            // one `is_blocking_manual_fix` predicate. Excluded fixes (onboarding
            // nudges, manual/unverified MCP pointers) sort arbitrarily among the
            // genuine ones, so a bare `.find(|f| !f.auto_fixable)` could label
            // the error with a subsystem that is explicitly not supposed to
            // block. Sharing the predicate keeps the label pointing at the fix
            // that actually triggered the exit-75 gate.
            subsystem: report
                .suggested_fixes
                .iter()
                .find(|f| doctor::fixes::is_blocking_manual_fix(&report, f))
                .map(|f| f.subsystem.to_wire_string())
                .unwrap_or_else(|| "unknown".to_owned()),
        });
    }
    // Raw exit (not a `TomeError`) so the report always renders first.
    std::process::exit(health_code);
}

/// Fold a malformed-`config.toml` parse error (issue #287) into the report: push
/// a non-auto-fixable `Config` `SuggestedFix` carrying the legible diagnosis, and
/// flip `overall` to `Unhealthy` so `tome doctor` exits non-zero while it holds.
///
/// A no-op when `config_error` is `None` (absent or well-formed config), so the
/// byte-stable minimal-report JSON pin is unchanged for a clean system. The
/// finding is `auto_fixable: false` — Tome never rewrites the user-authored
/// `config.toml`; the user fixes the named key by hand (the diagnosis quotes the
/// `toml` error, which names the offending key/section + line/column).
fn apply_config_finding(report: &mut DoctorReport, config_error: Option<&str>) {
    let Some(message) = config_error else {
        return;
    };
    report.suggested_fixes.push(doctor::SuggestedFix {
        subsystem: doctor::Subsystem::Config,
        // "could not be loaded" rather than "is malformed": `probe_error`'s
        // fallback arm also covers a pure I/O / unreadable / over-cap failure
        // (not only a TOML parse error), so the wording must not mislabel those.
        diagnosis: format!("`~/.tome/config.toml` could not be loaded: {message}"),
        // Pointer, not a runnable repair — Tome must not rewrite a user-owned
        // config; `auto_fixable: false` is what keeps `--fix` from "fixing" it.
        command: "edit ~/.tome/config.toml to correct the reported problem".to_owned(),
        auto_fixable: false,
    });
    report.overall = DoctorClassification::Unhealthy;
}

fn emit(
    report: &DoctorReport,
    mode: Mode,
    paths: &crate::paths::Paths,
    verbose: bool,
) -> Result<(), TomeError> {
    match mode {
        Mode::Human => emit_human(report, paths, verbose),
        Mode::Json => write_json(report),
    }
}

/// Issue #430: the `--fix --dry-run` listing — the repairs a real `--fix`
/// would apply, selected by the SAME predicate `doctor::fixes::apply` snapshots
/// (`auto_fixable: true`, plus user-owned harness-MCP fixes under `--force`).
/// Printed after the report so the preview reads as a footer, applied nothing.
fn emit_fix_dry_run(report: &DoctorReport, force: bool) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    let would: Vec<&doctor::SuggestedFix> = report
        .suggested_fixes
        .iter()
        .filter(|f| {
            f.auto_fixable || (force && matches!(&f.subsystem, doctor::Subsystem::HarnessMcp(_)))
        })
        .collect();
    if would.is_empty() {
        writeln!(out, "Fix dry run: nothing to apply automatically.")?;
        return Ok(());
    }
    writeln!(
        out,
        "Fix dry run: `tome doctor --fix` would apply {} repair{}:",
        would.len(),
        if would.len() == 1 { "" } else { "s" },
    )?;
    for f in &would {
        writeln!(out, "  {}: {}", f.subsystem, f.command)?;
    }
    Ok(())
}

/// The health of ONE human-report section, for the leading verdict's counts
/// and the failing-first ordering (issue #430). `Ok` covers the informational
/// sections too — anything that is not a warning or a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionState {
    Ok,
    Warn,
    Fail,
}

/// One rendered human-report section: its computed state plus the exact bytes
/// the pre-#430 renderer produced for it (each body ends with its own trailing
/// blank line, preserving the historical spacing).
struct Section {
    state: SectionState,
    body: String,
}

/// The four state glyphs the doctor renderer uses, resolved once per run
/// (TTY → coloured glyphs, pipe → ASCII tags).
struct Glyphs {
    ok: String,
    warn: String,
    fail: String,
    info: String,
}

fn glyphs() -> Glyphs {
    let tty = crate::output::stdout_is_tty();
    if tty {
        Glyphs {
            ok: colour::success("✓").to_string(),
            warn: colour::warning("⚠").to_string(),
            fail: colour::error("✗").to_string(),
            info: "·".to_owned(),
        }
    } else {
        Glyphs {
            ok: "[ok]".to_owned(),
            warn: "[warn]".to_owned(),
            fail: "[fail]".to_owned(),
            info: "[—]".to_owned(),
        }
    }
}

/// Issue #430: the human report leads with a one-line verdict, orders the body
/// failing → warnings, and collapses the all-ok subsystems into one line
/// (`--verbose` restores the full listing). `--json` is byte-identical to
/// before — this restructure is human-mode only.
fn emit_human(
    report: &DoctorReport,
    paths: &crate::paths::Paths,
    verbose: bool,
) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    let g = glyphs();
    let sections = build_sections(report, &g);

    let fails = sections
        .iter()
        .filter(|s| s.state == SectionState::Fail)
        .count();
    let warns = sections
        .iter()
        .filter(|s| s.state == SectionState::Warn)
        .count();
    let oks = sections
        .iter()
        .filter(|s| s.state == SectionState::Ok)
        .count();

    // The verdict FIRST: classification + per-section counts. The
    // classification is the report's own `overall` (the `--json` gating
    // source); the counts are this renderer's section states.
    let (verdict_glyph, verdict_label) = match report.overall {
        DoctorClassification::Ok => (&g.ok, "healthy"),
        DoctorClassification::Degraded => (&g.warn, "degraded"),
        DoctorClassification::Unhealthy => (&g.fail, "unhealthy"),
    };
    writeln!(
        out,
        "{verdict_glyph} {verdict_label} — {fails} failing, {warns} warning{}, {oks} ok",
        if warns == 1 { "" } else { "s" },
    )?;
    writeln!(out)?;

    // Context header (identity, not a subsystem): always rendered.
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
            crate::workspace::ScopeSource::Env => "TOME_WORKSPACE env",
            crate::workspace::ScopeSource::Config => "config.toml [workspace] default",
            crate::workspace::ScopeSource::ProjectMarker => "project marker walk",
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

    // Issue #433: name the MCP server's log file — the natural first stop
    // for "MCP tools aren't appearing in my harness". Resolved through the
    // same `TOME_MCP_LOG` policy the server itself applies. Rendered at
    // HEADER level (not a collapsible section) so it survives the all-ok
    // collapse — it is a pointer, not a health state. Human-only: the
    // byte-stable `--json` envelope stays untouched.
    let mcp_log = match crate::mcp::log::resolve_sink(
        &paths.mcp_log,
        std::env::var(crate::mcp::log::LOG_ENV).ok().as_deref(),
    ) {
        crate::mcp::log::LogSink::Off => {
            format!("disabled ({}=off)", crate::mcp::log::LOG_ENV)
        }
        crate::mcp::log::LogSink::File(path) => path.display().to_string(),
    };
    writeln!(out, "MCP server log:  {mcp_log}")?;
    writeln!(out)?;

    if verbose {
        // `--verbose`: today's full listing, in the historical order.
        for s in &sections {
            out.write_all(s.body.as_bytes())?;
        }
    } else {
        // Failing sections first, then warnings, then one collapsed line for
        // everything ok. Relative order within each bucket is the historical
        // section order.
        for s in sections.iter().filter(|s| s.state == SectionState::Fail) {
            out.write_all(s.body.as_bytes())?;
        }
        for s in sections.iter().filter(|s| s.state == SectionState::Warn) {
            out.write_all(s.body.as_bytes())?;
        }
        if oks > 0 {
            writeln!(
                out,
                "{oks} subsystem{} ok (run with --verbose for detail)",
                if oks == 1 { "" } else { "s" },
            )?;
            writeln!(out)?;
        }
    }

    // Suggested fixes are the actionable tail — never collapsed.
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
        DoctorClassification::Ok => format!("{} healthy", g.ok),
        DoctorClassification::Degraded => format!("{} degraded", g.warn),
        DoctorClassification::Unhealthy => format!("{} unhealthy", g.fail),
    };
    writeln!(out, "Overall:         {overall_label}")?;
    Ok(())
}

/// Render every report section (in the pre-#430 order) into an owned body plus
/// its computed [`SectionState`]. Sections that would not have rendered before
/// (absent `Option`s, empty lists) are simply not built, so they neither print
/// nor count.
#[allow(clippy::too_many_lines)]
fn build_sections(report: &DoctorReport, g: &Glyphs) -> Vec<Section> {
    use std::fmt::Write as _;

    let (ok, warn, fail, info) = (&g.ok, &g.warn, &g.fail, &g.info);
    let model_glyph = |state: &str| -> String {
        if state == "ok" {
            format!("{ok} ok")
        } else {
            format!("{fail} {state}")
        }
    };

    let mut sections: Vec<Section> = Vec::new();

    // ---- Models -----------------------------------------------------------
    {
        let mut s = String::new();
        let _ = writeln!(s, "Models:");
        let _ = writeln!(
            s,
            "  embedder       {} ({})  {}",
            report.embedder.name,
            report.embedder.version,
            model_glyph(&report.embedder.state),
        );
        let _ = writeln!(
            s,
            "  reranker       {} ({})  {}",
            report.reranker.name,
            report.reranker.version,
            model_glyph(&report.reranker.state),
        );
        // #480: the summariser renders in the Models section and counts
        // toward its state as a WARNING (never Fail) — matching its Degraded
        // classification (#470/#429). Before this, a summariser-only-broken
        // install printed a "degraded" verdict whose section counts read
        // "0 failing, 0 warnings", with nothing in the body to substantiate
        // it. The warn glyph (not fail) keeps the severity vocabulary aligned
        // with `classify`.
        let summariser_glyph = if report.summariser.state == "ok" {
            format!("{ok} ok")
        } else {
            format!("{warn} {}", report.summariser.state)
        };
        let _ = writeln!(
            s,
            "  summariser     {} ({})  {}",
            report.summariser.name, report.summariser.version, summariser_glyph,
        );
        let _ = writeln!(s);
        let state = if report.embedder.state != "ok" || report.reranker.state != "ok" {
            SectionState::Fail
        } else if report.summariser.state != "ok" {
            SectionState::Warn
        } else {
            SectionState::Ok
        };
        sections.push(Section { state, body: s });
    }

    // ---- Index database + drift -------------------------------------------
    {
        let mut s = String::new();
        if report.index.present {
            let _ = writeln!(
                s,
                "Index database:  {} ({} plugins enabled, {} skills indexed, {})",
                if report.index.integrity_ok { ok } else { fail },
                report.index.plugins_enabled,
                report.index.skills_indexed,
                human_size(report.index.size_bytes),
            );
            if let Some(v) = report.index.schema_version {
                let _ = writeln!(s, "Schema version:  {v}");
            }
        } else {
            let _ = writeln!(s, "Index database:  not yet bootstrapped");
        }
        let _ = writeln!(s, "Drift:           {}", drift_label(&report.drift));
        let _ = writeln!(s);
        let state = if report.index.present && !report.index.integrity_ok {
            SectionState::Fail
        } else if report.drift != crate::index::meta::DriftStatus::None {
            SectionState::Warn
        } else {
            SectionState::Ok
        };
        sections.push(Section { state, body: s });
    }

    // ---- Catalog caches -----------------------------------------------------
    {
        let mut s = String::new();
        let _ = writeln!(s, "Catalog caches:");
        let mut state = SectionState::Ok;
        if report.catalogs.is_empty() {
            let _ = writeln!(s, "  (none registered in this scope)");
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
                match c.state {
                    doctor::CatalogCacheState::Ok => {}
                    doctor::CatalogCacheState::Orphan => {
                        if state == SectionState::Ok {
                            state = SectionState::Warn;
                        }
                    }
                    _ => state = SectionState::Fail,
                }
                let _ = writeln!(s, "  {:30}     {}{}", c.name, glyph, suffix);
            }
        }
        let _ = writeln!(s);
        sections.push(Section { state, body: s });
    }

    // Issue #432: the Phase 3 opt-in workspace registry file is gone —
    // `check_workspace_registry` is a permanent `present: false` stub kept
    // only for the byte-stable JSON envelope (see its doc comment). The
    // human section it fed read like something was missing on every healthy
    // install, so it is no longer rendered; the JSON field is untouched.

    // ---- Detected harnesses (informational) --------------------------------
    {
        let mut s = String::new();
        let _ = writeln!(s, "Detected harnesses:");
        for h in &report.harnesses {
            let glyph = if h.present { ok } else { info };
            let label = harness_display_name(&h.name);
            if h.present {
                let _ = writeln!(s, "  {glyph} {label:14} {}", h.path.display());
            } else {
                let _ = writeln!(
                    s,
                    "  {glyph} {label:14} {}    (not detected)",
                    h.path.display()
                );
            }
        }
        let _ = writeln!(s);
        sections.push(Section {
            state: SectionState::Ok,
            body: s,
        });
    }

    // ---- Prompts surface (Phase 5 / US5.b) ----------------------------------
    if let Some(p) = &report.prompts {
        let mut s = String::new();
        let workspace_label = report
            .workspace
            .path
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "global".to_owned());
        let _ = writeln!(s, "Prompts surface for workspace \"{}\":", workspace_label);
        if p.prompts.is_empty() {
            let _ = writeln!(s, "  (no user-invocable entries enrolled)");
        } else {
            for d in &p.prompts {
                let _ = writeln!(s, "  {}{}", crate::mcp::MCP_SLASH_PREFIX, d.name);
            }
        }
        if !p.collisions.is_empty() {
            let _ = writeln!(s);
            let _ = writeln!(s, "  Prompt name collisions:");
            for c in &p.collisions {
                let _ = writeln!(
                    s,
                    "    base `{}` → {} entries (winner takes the base name; losers suffixed)",
                    c.base_name,
                    c.entries.len(),
                );
                for entry in &c.entries {
                    let _ = writeln!(
                        s,
                        "      - {}/{} ({}) → {}",
                        entry.identity.catalog,
                        entry.identity.plugin,
                        entry.identity.name,
                        entry.final_name,
                    );
                }
            }
        }
        let _ = writeln!(s);
        let state = if p.collisions.is_empty() {
            SectionState::Ok
        } else {
            SectionState::Warn
        };
        sections.push(Section { state, body: s });
    }

    // ---- Orphan persistent-data directories (Phase 5 / US5.b) ---------------
    if let Some(o) = &report.orphan_data_dirs
        && (!o.plugin_data.is_empty() || !o.workspace_data.is_empty())
    {
        let mut s = String::new();
        let _ = writeln!(s, "Orphan persistent data directories:");
        if !o.plugin_data.is_empty() {
            let _ = writeln!(s, "  plugin-data (no longer enabled in any workspace):");
            for p in &o.plugin_data {
                let _ = writeln!(s, "    {}", p.display());
            }
        }
        if !o.workspace_data.is_empty() {
            let _ = writeln!(s, "  workspace-data:");
            for p in &o.workspace_data {
                let _ = writeln!(s, "    {}", p.display());
            }
        }
        let _ = writeln!(s);
        let _ = writeln!(
            s,
            "  Cleanup: not auto-fixable — see `Suggested fixes` below for the per-directory command.",
        );
        let _ = writeln!(s);
        sections.push(Section {
            state: SectionState::Warn,
            body: s,
        });
    }

    // ---- Entry counts (Phase 5 / US5.b, informational) ----------------------
    if let Some(c) = &report.entry_counts {
        let mut s = String::new();
        let _ = writeln!(s, "Entry counts:");
        let _ = writeln!(s, "  Skills:               {}", c.skills);
        let _ = writeln!(s, "  Commands:             {}", c.commands);
        let _ = writeln!(s, "  Agents:               {}", c.agents);
        let _ = writeln!(s, "  Pending re-embedding: {}", c.pending_re_embedding,);
        let _ = writeln!(s);
        sections.push(Section {
            state: SectionState::Ok,
            body: s,
        });
    }

    // ---- Hooks contribution + drift (Phase 6 / US5, Claude Code) ------------
    if let Some(h) = &report.hooks
        && !h.plugins.is_empty()
    {
        let mut s = String::new();
        let mut state = SectionState::Ok;
        let _ = writeln!(s, "Hooks (Claude Code):");
        for p in &h.plugins {
            for c in &p.contributed {
                let _ = writeln!(
                    s,
                    "  {ok} {}:{} {} x{} contributed",
                    p.catalog, p.plugin, c.event, c.count,
                );
            }
            for m in &p.missing {
                state = SectionState::Warn;
                let _ = writeln!(
                    s,
                    "  {warn} {}:{} {} x{} expected but missing (drift; re-merges on next sync)",
                    p.catalog, p.plugin, m.event, m.count,
                );
            }
        }
        let _ = writeln!(s);
        sections.push(Section { state, body: s });
    }

    // ---- Guardrails regions (Phase 6 / US5) ----------------------------------
    if let Some(gr) = &report.guardrails
        && !gr.files.is_empty()
    {
        let mut s = String::new();
        let mut state = SectionState::Ok;
        let _ = writeln!(s, "Guardrails regions:");
        for f in &gr.files {
            let _ = writeln!(s, "  {}", f.path.display());
            for cp in &f.present {
                let suppressed = f
                    .suppressed
                    .iter()
                    .any(|sp| sp.catalog == cp.catalog && sp.plugin == cp.plugin);
                let orphaned = f
                    .orphaned
                    .iter()
                    .any(|o| o.catalog == cp.catalog && o.plugin == cp.plugin);
                let (glyph, suffix) = if orphaned {
                    state = SectionState::Warn;
                    (info, "  (orphaned)")
                } else if suppressed {
                    (info, "  (suppressed by hooks)")
                } else {
                    (ok, "")
                };
                let _ = writeln!(s, "    {glyph} {}:{}{suffix}", cp.catalog, cp.plugin);
            }
        }
        let _ = writeln!(s);
        sections.push(Section { state, body: s });
    }

    // ---- Native agents (Phase 6 / US5) ---------------------------------------
    if let Some(a) = &report.agents
        && !a.harnesses.is_empty()
    {
        let mut s = String::new();
        let mut state = SectionState::Ok;
        let _ = writeln!(s, "Native agents:");
        for h in &a.harnesses {
            let _ = writeln!(
                s,
                "  {} ({} present, {} orphaned)",
                harness_display_name(&h.harness),
                h.present.len(),
                h.orphaned.len(),
            );
            for orphan in &h.orphaned {
                state = SectionState::Warn;
                let _ = writeln!(s, "    {warn} {orphan} (orphaned)");
            }
            for d in &h.dropped_fields {
                let _ = writeln!(s, "    {info} {} dropped: {:?}", d.agent, d.fields);
            }
        }
        let _ = writeln!(s);
        sections.push(Section { state, body: s });
    }

    // ---- Unrepresented agents (native-agent expansion Phase 2) ---------------
    if let Some(u) = &report.unrepresented_agents {
        let mut s = String::new();
        let count = u.agents.len();
        let harness_list = u.rules_only_harnesses.join(", ");
        let _ = writeln!(
            s,
            "Agents without native form on rules-only harnesses ({harness_list}):"
        );
        let _ = writeln!(
            s,
            "  {warn} {count} agent{} — reachable via MCP persona when expose_agents_as_personas is enabled",
            if count == 1 { "" } else { "s" },
        );
        for a in &u.agents {
            let _ = writeln!(s, "    {}:{}/{}", a.catalog, a.plugin, a.name);
        }
        let _ = writeln!(s);
        sections.push(Section {
            state: SectionState::Warn,
            body: s,
        });
    }

    // ---- Hook translation (US11) ---------------------------------------------
    if let Some(ht) = &report.hook_translation {
        let mut s = String::new();
        let mut state = SectionState::Ok;
        let _ = writeln!(s, "Hook translation:");
        for h in &ht.per_harness {
            let hstate = if h.enabled { "on" } else { "off" };
            let events = if h.registered_events.is_empty() {
                "—".to_string()
            } else {
                h.registered_events.join(", ")
            };
            let _ = writeln!(s, "  {} [{}]  registered: {}", h.harness, hstate, events);
            if !h.dropped_to_guardrails.is_empty() {
                let dropped = h.dropped_to_guardrails.join(", ");
                let _ = writeln!(s, "    {info} dropped to GUARDRAILS: {dropped}");
            }
            if h.manifest_stale {
                state = SectionState::Warn;
                let _ = writeln!(
                    s,
                    "    {warn} manifest stale — run `tome sync` to reconcile",
                );
            }
            // Issue #431: the probed dispatch drift verdict (project scope
            // only). `ok` stays silent — the row's registered list already
            // says everything. A non-`ok` state marks the SECTION Warn
            // (mirroring `manifest_stale` above) so the drift line survives
            // the #430 all-ok collapse.
            if let Some(probe_state) = h.state.as_deref()
                && probe_state != "ok"
            {
                state = SectionState::Warn;
                let missing = if h.missing_events.is_empty() {
                    String::new()
                } else {
                    format!(" (unregistered: {})", h.missing_events.join(", "))
                };
                let _ = writeln!(
                    s,
                    "    {warn} dispatch {}{missing} — run `tome sync` (or `tome doctor --fix`)",
                    probe_state.replace('_', " "),
                );
            }
            if h.trust_prompt_note {
                let _ = writeln!(
                    s,
                    "    {info} prompt-model configured (first execution may request trust)",
                );
            }
        }
        // Issue #439: translated hooks fail open by design, so a misfiring
        // hook is silent — point at the debugging tools whenever any harness
        // has translation active. Human output only; the JSON report is
        // untouched.
        if ht.per_harness.iter().any(|h| h.enabled) {
            let _ = writeln!(
                s,
                "  debug: TOME_HOOK_DEBUG=1 or `tome harness run-hook --explain --event <event> --harness <name>`",
            );
        }
        let _ = writeln!(s);
        sections.push(Section { state, body: s });
    }

    // ---- Unrepresented hooks (issue #292) -------------------------------------
    if let Some(u) = &report.unrepresented_hooks {
        let mut s = String::new();
        let count = u.hooks.len();
        let harness_list = u.rules_only_harnesses.join(", ");
        let _ = writeln!(
            s,
            "Hooks without native form on rules-only harnesses ({harness_list}):"
        );
        let _ = writeln!(
            s,
            "  {warn} {count} plugin hook{} — rendered as GUARDRAILS.md prose only (not enforced)",
            if count == 1 { "" } else { "s" },
        );
        for h in &u.hooks {
            let _ = writeln!(s, "    {}:{}  {}", h.catalog, h.plugin, h.event);
        }
        let _ = writeln!(s);
        sections.push(Section {
            state: SectionState::Warn,
            body: s,
        });
    }

    // ---- Privilege-escalation audit (Phase 6 / US5, FR-051) -------------------
    if let Some(p) = &report.privilege_escalation
        && !p.plugins.is_empty()
    {
        let mut s = String::new();
        let _ = writeln!(
            s,
            "Privileged agents (carry hooks/mcpServers/permissionMode):"
        );
        for plug in &p.plugins {
            for ag in &plug.agents {
                let _ = writeln!(
                    s,
                    "  {warn} {}:{}/{} → {:?}",
                    plug.catalog, plug.plugin, ag.name, ag.fields,
                );
            }
        }
        let _ = writeln!(s);
        sections.push(Section {
            state: SectionState::Warn,
            body: s,
        });
    }

    // ---- Personas (Phase 6 / US5, informational) ------------------------------
    if let Some(p) = &report.personas {
        let mut s = String::new();
        let _ = writeln!(s, "Agent personas (expose_agents_as_personas on):");
        for persona in &p.personas {
            let clash = if persona.clash_prefixed {
                "  (clash-prefixed)"
            } else {
                ""
            };
            let _ = writeln!(
                s,
                "  {}{}{}",
                crate::mcp::MCP_SLASH_PREFIX,
                persona.resolved_persona_name,
                clash,
            );
        }
        let _ = writeln!(s, "  {}{}", crate::mcp::MCP_SLASH_PREFIX, p.drop_persona);
        let _ = writeln!(s);
        sections.push(Section {
            state: SectionState::Ok,
            body: s,
        });
    }

    // ---- Phase 8 cutover surfaces ----------------------------------------------
    if !report.legacy_model_manifests.is_empty() {
        let mut s = String::new();
        let _ = writeln!(s, "Legacy model manifests (pre-cutover manifest.json):");
        for name in &report.legacy_model_manifests {
            let _ = writeln!(
                s,
                "  {warn} {name}  (run `tome doctor --fix` to migrate to manifest.toml)",
            );
        }
        let _ = writeln!(s);
        sections.push(Section {
            state: SectionState::Warn,
            body: s,
        });
    }
    if !report.unconverted_plugins.is_empty() {
        let mut s = String::new();
        let _ = writeln!(
            s,
            "Unconverted plugins (legacy plugin.json, no tome-plugin.toml):"
        );
        for p in &report.unconverted_plugins {
            let _ = writeln!(s, "  {warn} {p}  (run `tome plugin convert <source>`)");
        }
        let _ = writeln!(s);
        sections.push(Section {
            state: SectionState::Warn,
            body: s,
        });
    }

    // ---- Meta-skill drift (Phase 9 / US4) ----------------------------------------
    if !report.meta_skills.is_empty() {
        let mut s = String::new();
        let _ = writeln!(s, "Meta skills (drift):");
        for m in &report.meta_skills {
            let _ = writeln!(
                s,
                "  {warn} {} @ {} ({})  {}  (run `tome doctor --fix`)",
                m.skill_id, m.harness, m.scope, m.state,
            );
        }
        let _ = writeln!(s);
        sections.push(Section {
            state: SectionState::Warn,
            body: s,
        });
    }

    // ---- Telemetry (Phase 10 / US5, FR-064) ---------------------------------------
    if let Some(t) = &report.telemetry {
        let mut s = String::new();
        let on_off = if t.enabled { "enabled" } else { "disabled" };
        let _ = writeln!(s, "Telemetry:");
        let _ = writeln!(
            s,
            "  state:     {on_off} ({})",
            telemetry_source_label(t.source),
        );
        if let Some(err) = &t.config_error {
            let _ = writeln!(s, "  {warn} config error: {err}");
        }
        if t.install_id.present {
            let mode = t
                .install_id
                .mode
                .map(|m| format!(" mode {m:04o}"))
                .unwrap_or_default();
            let age = t
                .install_id
                .age_seconds
                .map(|sec| format!(" age {sec}s"))
                .unwrap_or_default();
            let _ = writeln!(s, "  install:   {}{mode}{age}", t.install_id.path);
        } else {
            let _ = writeln!(s, "  install:   (none) {}", t.install_id.path);
        }
        let oldest = t
            .queue
            .oldest_age_seconds
            .map(|sec| format!(", oldest {sec}s"))
            .unwrap_or_default();
        let _ = writeln!(
            s,
            "  queue:     {} pending, {} unparsable{oldest}",
            t.queue.pending, t.queue.corrupt,
        );
        match &t.last_flush {
            Some(lf) => match lf.status {
                Some(st) => {
                    let _ = writeln!(s, "  last flush: {} (status {st})", lf.timestamp);
                }
                None => {
                    let _ = writeln!(s, "  last flush: {} (no successful delivery)", lf.timestamp);
                }
            },
            None => {
                let _ = writeln!(s, "  last flush: never");
            }
        }
        let _ = writeln!(s, "  endpoint:  {}", t.endpoint);
        if t.allowlist.is_empty() {
            let _ = writeln!(s, "  allowlist: (empty)");
        } else {
            let _ = writeln!(s, "  allowlist:");
            for e in &t.allowlist {
                let _ = writeln!(s, "    {} -> {}", e.short_id, e.canonical_source);
            }
        }
        let _ = writeln!(s);
        let state = if t.config_error.is_some() {
            SectionState::Warn
        } else {
            SectionState::Ok
        };
        sections.push(Section { state, body: s });
    }

    // ---- MCP probe (issue #434, --verify / [doctor] verify_by_default) -------
    // One line per (workspace, server) probed. A FAILED probe marks the
    // section Fail so it renders ahead of the warnings and can never be
    // collapsed away; an all-ok probe participates in the normal ok collapse
    // (the JSON `mcp_probe` field still carries the detail).
    if let Some(probe) = &report.mcp_probe {
        let mut s = String::new();
        let mut state = SectionState::Ok;
        let _ = writeln!(s, "MCP probe (end-to-end, --verify):");
        for p in probe {
            if p.ok {
                let tools = p.tools.map(|t| format!(" ({t} tools)")).unwrap_or_default();
                let _ = writeln!(
                    s,
                    "  {ok} {} (workspace {}) — initialize + tools/list ok{tools}",
                    p.harness, p.workspace,
                );
            } else {
                state = SectionState::Fail;
                let _ = writeln!(
                    s,
                    "  {fail} {} (workspace {}) — {}",
                    p.harness,
                    p.workspace,
                    p.error.as_deref().unwrap_or("failed"),
                );
            }
        }
        let _ = writeln!(s);
        sections.push(Section { state, body: s });
    }

    // ---- Remote providers (Phase 12 / US4) ------------------------------------------
    if !report.providers.is_empty() {
        let mut s = String::new();
        let mut state = SectionState::Ok;
        let _ = writeln!(s, "Providers:");
        for p in &report.providers {
            let cred = if p.credential_resolvable {
                "credential resolved".to_owned()
            } else {
                if state == SectionState::Ok {
                    state = SectionState::Warn;
                }
                let env_var = crate::provider::config::derive_env_var_name(&p.name);
                format!("no credential (set {env_var})")
            };
            let reach = match p.reachable {
                Some(true) => format!("  {ok} reachable"),
                Some(false) => {
                    state = SectionState::Fail;
                    format!("  {fail} unreachable")
                }
                None => String::new(),
            };
            let _ = writeln!(
                s,
                "  {} ({}) [{}] — {}{}",
                p.name,
                p.kind,
                p.capabilities.join(", "),
                cred,
                reach,
            );
        }
        let _ = writeln!(s);
        sections.push(Section { state, body: s });
    }

    // ---- Model registry (Phase 13, native-agent model-registry) -------------------------
    {
        let mut s = String::new();
        let mr = &report.model_registry;
        let _ = writeln!(
            s,
            "Model registry:  {} ({} models, fetched {})",
            mr.source, mr.model_count, mr.fetched_at,
        );
        if mr.override_corrupt {
            let _ = writeln!(
                s,
                "  {warn} override file is corrupt (active registry fell back to baked)",
            );
            let _ = writeln!(
                s,
                "    run `tome models update --include-registry` to refresh",
            );
        }
        let _ = writeln!(s);
        sections.push(Section {
            state: if mr.override_corrupt {
                SectionState::Warn
            } else {
                SectionState::Ok
            },
            body: s,
        });
    }

    sections
}

/// Human label for the telemetry enabled-state provenance. Mirrors the
/// `commands::telemetry` status labels so the two surfaces read identically.
fn telemetry_source_label(source: crate::telemetry::config::Source) -> &'static str {
    use crate::telemetry::config::Source;
    match source {
        Source::EnvOn => "TOME_TELEMETRY=1",
        Source::EnvOff => "TOME_TELEMETRY=0",
        Source::Ci => "CI auto-off",
        Source::Config => "config file",
        Source::Default => "default",
    }
}

/// Human label for a harness machine name (issue #428).
///
/// Resolved from the harness module registry so a newly-added harness can
/// never render as "unknown": [`crate::harness::lookup`] covers every
/// registered module (supported + opt-in targets + aliases), and its
/// `description()` is the same compact label `tome harness` prints. The
/// explicit arms keep the shorter labels this section has always used for
/// the original harnesses — their registry descriptions carry vendor
/// prefixes ("Anthropic's Claude Code CLI") too wide for the aligned
/// column — plus `continue`, which doctor probes but the registry does not
/// know. Anything else missing from the registry falls back to the raw
/// machine name, never a lying "unknown".
fn harness_display_name(machine_name: &str) -> &str {
    match machine_name {
        "claude-code" => "Claude Code",
        "codex" => "Codex",
        "cursor" => "Cursor",
        "gemini" => "Gemini CLI",
        "opencode" => "OpenCode",
        "continue" => "Continue",
        other => crate::harness::lookup(other)
            .map(|m| m.description())
            .unwrap_or(other),
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

#[cfg(test)]
mod tests {
    use super::harness_display_name;

    /// Issue #428: a Phase 11 harness resolves a real label from the module
    /// registry instead of "unknown".
    #[test]
    fn phase_11_harness_renders_registry_label() {
        assert_eq!(harness_display_name("zed"), "Zed editor");
        assert_eq!(harness_display_name("kiro"), "AWS Kiro IDE");
        assert_eq!(
            harness_display_name("jetbrains-ai"),
            "JetBrains AI Assistant"
        );
    }

    /// The original six keep their historical short labels (the registry
    /// descriptions carry vendor prefixes too wide for the aligned column).
    #[test]
    fn original_harnesses_keep_short_labels() {
        assert_eq!(harness_display_name("claude-code"), "Claude Code");
        assert_eq!(harness_display_name("gemini"), "Gemini CLI");
        assert_eq!(harness_display_name("continue"), "Continue");
    }

    /// Every registered module (supported + opt-in) resolves to SOME real
    /// label — the registry is the source of truth, so a future harness
    /// cannot go stale here. A name missing from the registry falls back to
    /// the raw string; nothing ever renders "unknown".
    #[test]
    fn every_registered_harness_has_a_label_and_fallback_is_raw() {
        for m in crate::harness::SUPPORTED_HARNESSES
            .iter()
            .chain(crate::harness::OPT_IN_TARGETS)
        {
            let label = harness_display_name(m.name());
            assert!(!label.is_empty(), "{}: empty label", m.name());
            assert_ne!(label, "unknown", "{}: stale label", m.name());
        }
        assert_eq!(
            harness_display_name("some-future-harness"),
            "some-future-harness"
        );
    }
}
