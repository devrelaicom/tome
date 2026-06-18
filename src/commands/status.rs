//! `tome status [--verify] [--json]`.
//!
//! Per-subsystem health check. See `contracts/status.md`. Read-only. Never
//! acquires the advisory lock; never triggers a model download.
//!
//! Exit semantics:
//!
//! * Overall health == Ok → exit 0
//! * Overall health == Degraded (reranker-only drift) → exit 1
//! * Overall health == Unhealthy (anything else) → exit 1
//!
//! The non-zero cases are NOT propagated as `TomeError` variants — that
//! would prevent the report from rendering. Instead, `run` emits the report
//! and then calls `std::process::exit(1)` for non-Ok cases. Library-API
//! tests bypass `run` and call `assemble_report` directly.

use std::io::Write;

use serde::Serialize;

use crate::cli::StatusArgs;
use crate::embedding::download::sha256_file;
use crate::embedding::registry::ModelEntry;
use crate::error::TomeError;
use crate::index::meta::{DriftStatus, ModelIdent, detect_drift};
use crate::index::{self, integrity};
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::workspace::{ResolvedScope, Scope};

use crate::commands::models::{ModelState, cheap_state};
use crate::commands::plugin::{embedder_entry, reranker_entry};

mod art;

pub fn run(args: StatusArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    let mut report = assemble_report(&paths, &scope.scope, args.verify)?;
    // Phase 11 / US5 (T065): augment with per-harness MCP integration state
    // (needs the ResolvedScope's project root, which `assemble_report` lacks).
    fill_harness_mcp(&mut report, scope, &paths);
    emit(&report, mode)?;
    if !matches!(report.overall, OverallHealth::Ok) {
        std::process::exit(1);
    }
    Ok(())
}

// ---- Status data model (mirrors data-model.md §11) -------------------------

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OverallHealth {
    Ok,
    Degraded,
    Unhealthy,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModelHealth {
    pub name: String,
    pub version: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct IndexHealth {
    pub present: bool,
    pub schema_version: Option<u32>,
    pub plugins_enabled: u32,
    pub skills_indexed: u32,
    pub size_bytes: u64,
    pub integrity_ok: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct EntryCounts {
    pub skills: u32,
    pub commands: u32,
    pub agents: u32,
}

/// Phase 11 / US5 (T065): one configured harness's MCP integration state for
/// `tome status`. `state` is `ok` / `manual` / `unverified` / `drift` (plus the
/// `broken` / `user_owned` / `not_applicable` states the shared doctor check can
/// also yield) — the SAME [`crate::doctor::report::SubsystemHealth`] vocabulary
/// the doctor reports, so the two surfaces cannot diverge.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HarnessMcpStatus {
    pub harness: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StatusReport {
    pub tome: String,
    pub embedder: ModelHealth,
    pub reranker: ModelHealth,
    pub summariser: ModelHealth,
    pub index: IndexHealth,
    pub drift: DriftStatus,
    pub overall: OverallHealth,
    pub workspaces_total: u32,
    pub current_workspace: String,
    pub current_scope: String,
    pub entries: EntryCounts,
    pub catalogs_enrolled: u32,
    pub reindexed_at: Option<i64>,
    pub models_on_disk_bytes: u64,
    /// Phase 11 / US5 (T065): per-harness MCP integration state for every
    /// harness in the resolved effective list. Empty when no project/scope
    /// resolves a harness list. Appended LAST + `skip_serializing_if`-gated so
    /// the pre-Phase-11 byte-stable `--json` pins don't move (empty ⇒ absent).
    /// `assemble_report` leaves it empty (it lacks the project root); `run`
    /// populates it via [`fill_harness_mcp`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub harness_mcp: Vec<HarnessMcpStatus>,
}

// ---- Assembly --------------------------------------------------------------

/// Build a `StatusReport` from the on-disk state. Read-only; does not take
/// the advisory lock. With `verify = true`, each model's primary artefact is
/// rehashed against its pinned SHA-256.
///
/// This is the library-API entry point that tests should call directly —
/// the surrounding `run()` adds the `std::process::exit(1)` semantics that
/// terminate the test runner.
pub fn assemble_report(
    paths: &Paths,
    scope: &Scope,
    verify: bool,
) -> Result<StatusReport, TomeError> {
    let tome = env!("CARGO_PKG_VERSION").to_owned();
    let embedder_entry = embedder_entry();
    let reranker_entry = reranker_entry();
    let summariser_entry = crate::summarise::registry::summariser_entry();

    let embedder = check_model(paths, embedder_entry, verify)?;
    let reranker = check_model(paths, reranker_entry, verify)?;
    let summariser = check_model(paths, summariser_entry, verify)?;

    let index = check_index(paths, scope)?;
    let drift = check_drift(paths, scope, embedder_entry, reranker_entry)?;

    let overall = classify(&embedder, &reranker, &index, &drift);

    let current_workspace = scope.name().as_str().to_owned();
    let current_scope = if scope.is_global() {
        "global"
    } else {
        "project"
    }
    .to_owned();
    let models_on_disk_bytes =
        models_on_disk(paths, &[embedder_entry, reranker_entry, summariser_entry]);

    // DB-derived workspace/global stats.
    let db = gather_db_stats(paths, scope)?;

    Ok(StatusReport {
        tome,
        embedder,
        reranker,
        summariser,
        index,
        drift,
        overall,
        workspaces_total: db.workspaces_total,
        current_workspace,
        current_scope,
        entries: db.entries,
        catalogs_enrolled: db.catalogs_enrolled,
        reindexed_at: db.reindexed_at,
        models_on_disk_bytes,
        // `run` fills this via `fill_harness_mcp` (needs the project root /
        // effective list, which `Scope` alone doesn't carry).
        harness_mcp: Vec::new(),
    })
}

/// Phase 11 / US5 (T065): populate `report.harness_mcp` with each effective
/// harness's MCP integration state. Read-only — never writes, never takes the
/// lock. Resolves the effective harness list for the scope, then reuses the
/// SAME shared doctor check (`doctor::harness_integration::check_harness_integration`)
/// so `status` and `doctor` cannot diverge. Silently leaves the field empty when
/// no project root resolves (the integration check needs a project root) or the
/// effective list can't be computed (status must always render).
fn fill_harness_mcp(report: &mut StatusReport, scope: &ResolvedScope, paths: &Paths) {
    let Some(project_root) = scope.project_root.as_deref() else {
        return;
    };
    let Ok(home) = crate::commands::harness::home_root() else {
        return;
    };
    let Some(effective) = resolve_effective_for_status(scope, paths) else {
        return;
    };
    let (_rules, mcp) = crate::doctor::harness_integration::check_harness_integration(
        project_root,
        &effective,
        &home,
        scope.scope.name(),
    );
    report.harness_mcp = mcp
        .into_iter()
        .map(|m| HarnessMcpStatus {
            harness: m.harness,
            state: m.health.as_str().to_owned(),
        })
        .collect();
}

/// Resolve the effective harness list for `status` (read-only), or `None` on any
/// failure (status must always render). Mirrors the harness-command loaders.
fn resolve_effective_for_status(
    scope: &ResolvedScope,
    paths: &Paths,
) -> Option<crate::settings::resolver::EffectiveHarnessList> {
    use crate::settings::resolver::resolve_effective_list;

    let marker = crate::commands::harness::list::load_project_marker_for_use(scope).ok()?;
    let workspace_settings =
        crate::commands::harness::list::load_workspace_settings_for_use(scope, paths).ok()?;
    let global_settings =
        crate::commands::harness::list::load_global_settings_for_use(paths).ok()?;
    let provider = crate::commands::harness::CentralDbScopeProvider::new(paths);
    resolve_effective_list(
        marker.as_ref(),
        workspace_settings.as_ref(),
        &global_settings,
        &provider,
    )
    .ok()
}

pub fn check_model(
    paths: &Paths,
    entry: &ModelEntry,
    verify: bool,
) -> Result<ModelHealth, TomeError> {
    let (mut state, _manifest) = cheap_state(paths, entry)?;
    if verify && matches!(state, ModelState::Ok) {
        let dir = paths.model_path(entry.name)?;
        if let Some(primary) = entry.files.first() {
            let actual = sha256_file(&dir.join(primary))?;
            if actual.eq_ignore_ascii_case(entry.sha256) {
                // ok
            } else {
                state = ModelState::ChecksumMismatched;
            }
        }
    }
    Ok(ModelHealth {
        name: entry.name.to_owned(),
        version: entry.version.to_owned(),
        state: state.as_str().to_owned(),
    })
}

pub fn check_index(paths: &Paths, scope: &Scope) -> Result<IndexHealth, TomeError> {
    let workspace_name = scope.name().as_str();
    let index_db = paths.index_db.clone();
    if !index_db.is_file() {
        return Ok(IndexHealth {
            present: false,
            schema_version: None,
            plugins_enabled: 0,
            skills_indexed: 0,
            size_bytes: 0,
            integrity_ok: false,
        });
    }
    let size_bytes = std::fs::metadata(&index_db).map(|m| m.len()).unwrap_or(0);

    // Phase 3 slice F5: `status` never writes; use the read-only open
    // path so a concurrent writer can't be racing us through the WAL
    // pragmas and the bootstrap re-application that `index::open` does.
    let conn = index::open_read_only(&index_db)?;

    let schema_version = match index::current_schema_version(&conn) {
        Ok(Some(v)) => Some(v),
        Ok(None) => Some(index::SCHEMA_VERSION),
        Err(_) => None,
    };

    let mut integrity_ok = integrity::check(&conn).is_ok();

    // Schema-version gate. The v2-shaped queries below (`JOIN
    // workspace_skills`) target tables that don't exist in an older
    // on-disk schema. A stale-schema DB is not an integrity failure
    // here — the doctor's schema-fix suggestion is the user-facing
    // repair path. Return zeros for the workspace-aware counts and
    // let `build_suggested_fixes` emit `subsystem: "schema"` based on
    // the reported `schema_version` field below.
    if let Some(v) = schema_version
        && v < index::SCHEMA_VERSION
    {
        return Ok(IndexHealth {
            present: true,
            schema_version,
            plugins_enabled: 0,
            skills_indexed: 0,
            size_bytes,
            integrity_ok,
        });
    }

    // A query_row failure here is rare (the schema bootstrap created the
    // table), but if it happens it indicates a corrupt index. Treat that
    // as an integrity failure rather than reporting `(0, 0)` which would
    // look like an empty install. The numeric fields stay at 0 because
    // we genuinely don't know.
    let plugins_enabled: i64 = match conn.query_row(
        "SELECT COUNT(DISTINCT s.plugin)
         FROM skills AS s
         JOIN workspace_skills AS ws ON ws.skill_id = s.id
         JOIN workspaces       AS w  ON w.id = ws.workspace_id
         WHERE w.name = ?1",
        rusqlite::params![workspace_name],
        |r| r.get(0),
    ) {
        Ok(n) => n,
        Err(_) => {
            integrity_ok = false;
            0
        }
    };
    let skills_indexed: i64 = match conn.query_row(
        "SELECT COUNT(*)
         FROM skills AS s
         JOIN workspace_skills AS ws ON ws.skill_id = s.id
         JOIN workspaces       AS w  ON w.id = ws.workspace_id
         WHERE w.name = ?1",
        rusqlite::params![workspace_name],
        |r| r.get(0),
    ) {
        Ok(n) => n,
        Err(_) => {
            integrity_ok = false;
            0
        }
    };

    Ok(IndexHealth {
        present: true,
        schema_version,
        plugins_enabled: u32::try_from(plugins_enabled).unwrap_or(u32::MAX),
        skills_indexed: u32::try_from(skills_indexed).unwrap_or(u32::MAX),
        size_bytes,
        integrity_ok,
    })
}

pub fn check_drift(
    paths: &Paths,
    _scope: &Scope,
    embedder_entry: &ModelEntry,
    reranker_entry: &ModelEntry,
) -> Result<DriftStatus, TomeError> {
    let index_db = paths.index_db.clone();
    if !index_db.is_file() {
        return Ok(DriftStatus::None);
    }
    let conn = index::open_read_only(&index_db)?;
    let embedder = ModelIdent {
        name: embedder_entry.name.to_owned(),
        version: embedder_entry.version.to_owned(),
    };
    let reranker = ModelIdent {
        name: reranker_entry.name.to_owned(),
        version: reranker_entry.version.to_owned(),
    };
    // Phase 4 / F9: third identity row to compare. The summariser
    // registry entry comes from the bundled F6 module; until US4.a
    // flips the placeholder hash, stored + configured both carry the
    // F6 placeholder identity, so drift stays `None`.
    let summariser_entry = crate::summarise::registry::summariser_entry();
    let summariser = ModelIdent {
        name: summariser_entry.name.to_owned(),
        version: summariser_entry.version.to_owned(),
    };
    detect_drift(&conn, &embedder, &reranker, &summariser)
}

fn classify(
    embedder: &ModelHealth,
    reranker: &ModelHealth,
    index: &IndexHealth,
    drift: &DriftStatus,
) -> OverallHealth {
    if embedder.state != "ok" || (!index.integrity_ok && index.present) {
        return OverallHealth::Unhealthy;
    }
    if matches!(
        drift,
        DriftStatus::EmbedderNameDrift { .. } | DriftStatus::EmbedderVersionDrift { .. }
    ) {
        return OverallHealth::Unhealthy;
    }
    if reranker.state != "ok" {
        // A reranker that's missing or corrupt still allows the embedder /
        // index to serve queries (degraded by skipping the rerank step).
        return OverallHealth::Degraded;
    }
    if matches!(drift, DriftStatus::RerankerDrift { .. }) {
        return OverallHealth::Degraded;
    }
    OverallHealth::Ok
}

// ---- Output ----------------------------------------------------------------

fn emit(report: &StatusReport, mode: Mode) -> Result<(), TomeError> {
    match mode {
        Mode::Human => emit_human(report),
        Mode::Json => write_json(report),
    }
}

fn emit_human(report: &StatusReport) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    let panel = render_panel(report);

    // Panel-only when not a TTY (colour already off) or the terminal is too
    // narrow to fit art + gap + a reasonable panel width.
    const GAP: usize = 3;
    const PANEL_MIN: usize = 34;
    let show_art =
        crate::output::stdout_is_tty() && term_width() >= art::ART_WIDTH + GAP + PANEL_MIN;

    if !show_art {
        for line in &panel {
            writeln!(out, "{line}")?;
        }
        return Ok(());
    }

    let art = art::bookshelf();
    let blank_art = " ".repeat(art::ART_WIDTH);
    let gap = " ".repeat(GAP);
    let rows = art.len().max(panel.len());
    for i in 0..rows {
        let left = art.get(i).map(String::as_str).unwrap_or(&blank_art);
        let right = panel.get(i).map(String::as_str).unwrap_or("");
        // Trim trailing whitespace when the right column is empty.
        if right.is_empty() {
            writeln!(out, "{}", left.trim_end())?;
        } else {
            writeln!(out, "{left}{gap}{right}")?;
        }
    }
    Ok(())
}

/// The right-hand info panel, as colored lines. Colour auto-disables in
/// non-TTY contexts (so this same function yields the plain rendering).
fn render_panel(report: &StatusReport) -> Vec<String> {
    use crate::presentation::colour;

    let glyph_ok = if colour::is_enabled() {
        format!("{} ok", colour::success("✓"))
    } else {
        "[ok]".to_owned()
    };
    let glyph_fail = if colour::is_enabled() {
        format!("{} fail", colour::error("✗"))
    } else {
        "[fail]".to_owned()
    };
    let model_glyph = |state: &str| -> String {
        if state == "ok" {
            glyph_ok.clone()
        } else {
            format!("{glyph_fail} ({state})")
        }
    };
    // Pad the key to a fixed column then colour it (padding inside the span
    // is invisible; this keeps alignment correct despite ANSI codes).
    let key = |k: &str| colour::label(&format!("{k:<12}"));

    let models = format!(
        "embedder {} / reranker {} / summariser {}",
        model_glyph(&report.embedder.state),
        model_glyph(&report.reranker.state),
        model_glyph(&report.summariser.state),
    );

    let index_line = if report.index.present {
        let schema = report
            .index
            .schema_version
            .map(|v| format!("schema v{v}"))
            .unwrap_or_else(|| "schema ?".to_owned());
        let integ = if report.index.integrity_ok {
            glyph_ok.clone()
        } else {
            glyph_fail.clone()
        };
        format!(
            "{} · {} · {} integrity",
            human_size(report.index.size_bytes),
            schema,
            integ
        )
    } else {
        "not yet bootstrapped".to_owned()
    };

    let reindexed = match report.reindexed_at {
        Some(t) => {
            let now = time::OffsetDateTime::now_utc().unix_timestamp();
            relative_time(t, now)
        }
        None => "never".to_owned(),
    };

    let overall = match report.overall {
        OverallHealth::Ok => format!("{} healthy", colour::success("✓")),
        OverallHealth::Degraded => format!("{} degraded", colour::warning("⚠")),
        OverallHealth::Unhealthy => format!("{} unhealthy", colour::error("✗")),
    };

    let mut lines = Vec::new();
    lines.push(colour::bold(&format!("Tome v{}", report.tome)));
    lines.push(String::new());

    lines.push(colour::dim("Global"));
    lines.push(format!("{} {}", key("Models:"), models));
    lines.push(format!(
        "{} {} on disk",
        key(""),
        human_size(report.models_on_disk_bytes)
    ));
    lines.push(format!(
        "{} {}",
        key("Workspaces:"),
        report.workspaces_total
    ));
    lines.push(format!("{} {}", key("Index:"), index_line));
    lines.push(format!(
        "{} {}",
        key("Drift:"),
        drift_description(&report.drift)
    ));
    lines.push(String::new());

    lines.push(colour::dim("Workspace"));
    lines.push(format!(
        "{} {} [{}]",
        key("Current:"),
        report.current_workspace,
        report.current_scope
    ));
    lines.push(format!(
        "{} {} skills, {} commands, {} agents",
        key("Entries:"),
        report.entries.skills,
        report.entries.commands,
        report.entries.agents
    ));
    lines.push(format!(
        "{} {} enrolled",
        key("Catalogs:"),
        report.catalogs_enrolled
    ));
    lines.push(format!("{} {}", key("Reindexed:"), reindexed));

    // Phase 11 / US5 (T065): per-harness MCP integration state, when a
    // project/scope resolved any harnesses.
    if !report.harness_mcp.is_empty() {
        let states = report
            .harness_mcp
            .iter()
            .map(|h| format!("{} {}", h.harness, mcp_state_glyph(&h.state)))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("{} {}", key("MCP:"), states));
    }
    lines.push(String::new());

    lines.push(format!("{} {}", key("Overall:"), overall));
    lines
}

/// Render a per-harness MCP-integration state (`ok`/`manual`/`unverified`/
/// `drift`/…) into a short colored glyph for the status panel.
fn mcp_state_glyph(state: &str) -> String {
    use crate::presentation::colour;
    match state {
        "ok" => {
            if colour::is_enabled() {
                format!("{} ok", colour::success("✓"))
            } else {
                "[ok]".to_owned()
            }
        }
        "manual" | "unverified" => {
            if colour::is_enabled() {
                format!("{} {state}", colour::warning("⚠"))
            } else {
                format!("[{state}]")
            }
        }
        other => {
            if colour::is_enabled() {
                format!("{} {other}", colour::error("✗"))
            } else {
                format!("[{other}]")
            }
        }
    }
}

fn drift_description(drift: &DriftStatus) -> String {
    match drift {
        DriftStatus::None => "none".to_owned(),
        DriftStatus::EmbedderNameDrift { stored, configured } => format!(
            "embedder name drift (stored: {stored}, configured: {configured}) — run `tome reindex --force`"
        ),
        DriftStatus::EmbedderVersionDrift { stored, configured } => format!(
            "embedder version drift (stored: {stored}, configured: {configured}) — run `tome reindex --force`"
        ),
        DriftStatus::RerankerDrift { stored, configured } => format!(
            "reranker drift (stored: {stored}, configured: {configured}) — queries still serve; consider `tome reindex --force` for consistency"
        ),
        DriftStatus::SummariserDrift { stored, configured } => format!(
            "summariser drift (stored: {stored}, configured: {configured}) — cached summaries regenerate on next enable/disable"
        ),
    }
}

/// Humanize the gap between `then` and `now` (both unix seconds). Future
/// timestamps (clock skew) clamp to "just now".
fn relative_time(then: i64, now: i64) -> String {
    let d = (now - then).max(0);
    let plural = |n: i64| if n == 1 { "" } else { "s" };
    if d < 60 {
        "just now".to_owned()
    } else if d < 3600 {
        let m = d / 60;
        format!("{m} minute{} ago", plural(m))
    } else if d < 86400 {
        let h = d / 3600;
        format!("{h} hour{} ago", plural(h))
    } else {
        let days = d / 86400;
        format!("{days} day{} ago", plural(days))
    }
}

/// Best-effort terminal width. No terminal-size dependency by design: read
/// `$COLUMNS`, else assume a standard 80-column terminal.
fn term_width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|w| *w > 0)
        .unwrap_or(80)
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

// ---- DB-derived stats (gather_db_stats) ------------------------------------

#[derive(Default)]
struct DbStats {
    workspaces_total: u32,
    entries: EntryCounts,
    catalogs_enrolled: u32,
    reindexed_at: Option<i64>,
}

/// All index-DB-derived stats in a single read-only open. Returns defaults
/// (zeros / None) when the index DB does not exist yet. A query failure on
/// any single stat degrades that stat to 0 / None rather than aborting the
/// report (the report must always render). Read-only; never takes the lock.
fn gather_db_stats(paths: &Paths, scope: &Scope) -> Result<DbStats, TomeError> {
    if !paths.index_db.is_file() {
        return Ok(DbStats::default());
    }
    let conn = index::open_read_only(&paths.index_db)?;
    let ws = scope.name().as_str();

    let workspaces_total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspaces WHERE name != ?1",
            rusqlite::params![index::GLOBAL_WORKSPACE],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let mut entries = EntryCounts::default();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT s.kind, COUNT(*) FROM skills s
         JOIN workspace_skills ws ON ws.skill_id = s.id
         JOIN workspaces       w  ON w.id = ws.workspace_id
         WHERE w.name = ?1
         GROUP BY s.kind",
    ) && let Ok(rows) = stmt.query_map(rusqlite::params![ws], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    }) {
        for (kind, n) in rows.flatten() {
            let n = u32::try_from(n).unwrap_or(u32::MAX);
            match kind.as_str() {
                "skill" => entries.skills = n,
                "command" => entries.commands = n,
                "agent" => entries.agents = n,
                _ => {}
            }
        }
    }

    let catalogs_enrolled: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_catalogs wc
             JOIN workspaces w ON w.id = wc.workspace_id
             WHERE w.name = ?1",
            rusqlite::params![ws],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // `indexed_at` is stored as RFC 3339 TEXT (lexicographically sortable);
    // MAX() over text gives the latest timestamp string. Parse it to unix
    // seconds for the report field; degrade to None on any failure.
    let reindexed_at: Option<i64> = conn
        .query_row(
            "SELECT MAX(s.indexed_at) FROM skills s
             JOIN workspace_skills ws ON ws.skill_id = s.id
             JOIN workspaces       w  ON w.id = ws.workspace_id
             WHERE w.name = ?1",
            rusqlite::params![ws],
            |r| r.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()
        .and_then(|s| {
            time::OffsetDateTime::parse(&s, &time::format_description::well_known::Rfc3339)
                .ok()
                .map(|dt| dt.unix_timestamp())
        });

    Ok(DbStats {
        workspaces_total: u32::try_from(workspaces_total).unwrap_or(u32::MAX),
        entries,
        catalogs_enrolled: u32::try_from(catalogs_enrolled).unwrap_or(u32::MAX),
        reindexed_at,
    })
}

/// Sum the on-disk size of each model's directory. Missing dirs contribute 0.
fn models_on_disk(paths: &Paths, entries: &[&ModelEntry]) -> u64 {
    entries
        .iter()
        .filter_map(|e| paths.model_path(e.name).ok())
        .map(|dir| dir_size(&dir))
        .sum()
}

fn dir_size(dir: &std::path::Path) -> u64 {
    let mut total = 0u64;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            match entry.metadata() {
                Ok(md) if md.is_file() => total += md.len(),
                Ok(md) if md.is_dir() => total += dir_size(&entry.path()),
                _ => {}
            }
        }
    }
    total
}

// ---- --version handling ----------------------------------------------------

/// Print the extended `--version` output. Invoked by a pre-parse hook in
/// `main.rs` so it can short-circuit before clap dispatch. Returns `()` —
/// the caller exits with code 0 directly.
///
/// When `json` is true, emits the structured form per
/// `contracts/version-output.md`. Otherwise emits the three-line plain text.
pub fn print_version(json: bool) {
    let embedder = embedder_entry();
    let reranker = reranker_entry();
    let tome = env!("CARGO_PKG_VERSION");
    if json {
        #[derive(Serialize)]
        struct VersionRecord<'a> {
            tome: &'a str,
            embedder: ModelSerial<'a>,
            reranker: ModelSerial<'a>,
        }
        #[derive(Serialize)]
        struct ModelSerial<'a> {
            name: &'a str,
            version: &'a str,
        }
        let rec = VersionRecord {
            tome,
            embedder: ModelSerial {
                name: embedder.name,
                version: embedder.version,
            },
            reranker: ModelSerial {
                name: reranker.name,
                version: reranker.version,
            },
        };
        let body = serde_json::to_string(&rec).unwrap_or_else(|_| "{}".to_owned());
        println!("{body}");
    } else {
        println!("tome {tome}");
        println!("embedder: {} {}", embedder.name, embedder.version);
        println!("reranker: {} {}", reranker.name, reranker.version);
    }
}

#[cfg(test)]
mod relative_time_tests {
    use super::relative_time;

    #[test]
    fn formats_buckets() {
        assert_eq!(relative_time(1000, 1000), "just now");
        assert_eq!(relative_time(1000, 1030), "just now"); // < 60s
        assert_eq!(relative_time(1000, 1000 + 60), "1 minute ago");
        assert_eq!(relative_time(1000, 1000 + 600), "10 minutes ago");
        assert_eq!(relative_time(1000, 1000 + 3600), "1 hour ago");
        assert_eq!(relative_time(1000, 1000 + 2 * 86400), "2 days ago");
        assert_eq!(relative_time(1000, 500), "just now"); // clock skew (future) clamps
    }
}

#[cfg(test)]
mod harness_mcp_status_tests {
    use super::*;

    fn base_report(harness_mcp: Vec<HarnessMcpStatus>) -> StatusReport {
        StatusReport {
            tome: "0".to_string(),
            embedder: ModelHealth {
                name: "e".to_string(),
                version: "1".to_string(),
                state: "ok".to_string(),
            },
            reranker: ModelHealth {
                name: "r".to_string(),
                version: "1".to_string(),
                state: "ok".to_string(),
            },
            summariser: ModelHealth {
                name: "s".to_string(),
                version: "1".to_string(),
                state: "ok".to_string(),
            },
            index: IndexHealth {
                present: false,
                schema_version: None,
                plugins_enabled: 0,
                skills_indexed: 0,
                size_bytes: 0,
                integrity_ok: false,
            },
            drift: DriftStatus::None,
            overall: OverallHealth::Ok,
            workspaces_total: 0,
            current_workspace: "global".to_string(),
            current_scope: "global".to_string(),
            entries: EntryCounts::default(),
            catalogs_enrolled: 0,
            reindexed_at: None,
            models_on_disk_bytes: 0,
            harness_mcp,
        }
    }

    /// T065: `harness_mcp` is `skip_serializing_if`-gated — an EMPTY Vec omits
    /// the key, so the pre-Phase-11 `tome status --json` byte pin is unchanged.
    #[test]
    fn empty_harness_mcp_is_omitted_from_json() {
        let json = serde_json::to_string(&base_report(Vec::new())).unwrap();
        assert!(
            !json.contains("harness_mcp"),
            "empty harness_mcp must be omitted; got: {json}",
        );
        // It is also the LAST field, so a populated value appends last.
        assert!(json.ends_with("\"models_on_disk_bytes\":0}"));
    }

    /// T065: a populated `harness_mcp` appends LAST, carrying the
    /// ok/manual/unverified/drift vocabulary.
    #[test]
    fn populated_harness_mcp_appends_last() {
        let report = base_report(vec![
            HarnessMcpStatus {
                harness: "crush".to_string(),
                state: "ok".to_string(),
            },
            HarnessMcpStatus {
                harness: "jetbrains-ai".to_string(),
                state: "manual".to_string(),
            },
            HarnessMcpStatus {
                harness: "pi".to_string(),
                state: "unverified".to_string(),
            },
        ]);
        let json = serde_json::to_string(&report).unwrap();
        assert!(
            json.ends_with(
                "\"harness_mcp\":[{\"harness\":\"crush\",\"state\":\"ok\"},\
                 {\"harness\":\"jetbrains-ai\",\"state\":\"manual\"},\
                 {\"harness\":\"pi\",\"state\":\"unverified\"}]}"
            ),
            "harness_mcp must append last with the state vocabulary; got: {json}",
        );
    }

    /// T065: the panel glyphs distinguish ok / manual / unverified / failure.
    #[test]
    fn mcp_state_glyph_buckets() {
        // Colour is off in the test process (no TTY), so plain forms render.
        assert_eq!(mcp_state_glyph("ok"), "[ok]");
        assert_eq!(mcp_state_glyph("manual"), "[manual]");
        assert_eq!(mcp_state_glyph("unverified"), "[unverified]");
        assert_eq!(mcp_state_glyph("drift"), "[drift]");
        assert_eq!(mcp_state_glyph("broken"), "[broken]");
    }
}
