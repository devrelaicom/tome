//! `tome.heartbeat` — a once-per-UTC-day inventory snapshot (FR-039).
//!
//! The heartbeat is the anonymous stream's product-fitness pulse: it tells us how
//! large an install's corpus is (skills / commands / agents / workspaces /
//! catalogs, each a raw count the kernel buckets at read time) and which
//! harnesses are detected on disk (a sorted, comma-joined closed-vocabulary
//! string — the kernel rejects arrays). It carries no free-form string, so it
//! stays inside the anonymous-stream invariant by construction. It flattens the
//! kernel's environment snapshot, supplied by the caller from `handle.env()`.
//!
//! Two non-negotiable properties shape every line below:
//!
//! 1. **Best-effort** — this NEVER crashes or blocks the foreground command
//!    (NFR-001). Every count is gathered defensively and defaults to `0`/empty on
//!    ANY error; a missing/unopenable index is a *valid* heartbeat (a fresh
//!    install with no index still tells us "installed, not yet used").
//! 2. **Read-only, no advisory lock** (NFR-009) — we open the central index with
//!    [`crate::index::open_read_only`], which never touches `index.lock`.
//!
//! Cheap gate FIRST: the once-per-day check reads only a tiny date stamp and
//! returns before opening the index on all but (at most) one call per UTC day.

use std::path::Path;

use gauge_telemetry::env::EnvAttributes;

use crate::paths::Paths;
use crate::telemetry::emit;
use crate::telemetry::event::{Harness, Heartbeat};

/// Cap on the stored `last-heartbeat` date read. A `YYYY-MM-DD` stamp is 10
/// bytes; 64 is comfortable slack while still bounding a corrupt/over-grown file.
const LAST_HEARTBEAT_READ_CAP: u64 = 64;

/// Emit a `tome.heartbeat` if one is due today, then record today's date.
///
/// Wired as the LAST step of [`crate::telemetry::cli_startup`]: it runs once per
/// CLI start but the once-per-UTC-day gate keeps the actual emit (and the
/// expensive count-gathering) to at most once daily. Infallible and best-effort
/// throughout — every failure branch is a `debug!` + return. `env` is the kernel
/// environment snapshot from `handle.env()`, flattened onto the event.
pub fn maybe_emit_heartbeat(paths: &Paths, env: EnvAttributes) {
    // 1. Cheap gate FIRST. Read the stored `YYYY-MM-DD`; a missing/unreadable
    //    stamp is treated as "never sent" (None) so a fresh install fires today.
    let last = read_last_heartbeat(paths);
    let today = crate::telemetry::clock::today_utc_date(crate::telemetry::clock::now_utc());
    if !crate::telemetry::clock::heartbeat_due(last.as_deref(), &today) {
        // Already sent today — return BEFORE opening the index.
        return;
    }

    // 2. Gather the corpus counts + detected harnesses, each best-effort. A
    //    missing/unopenable index yields all-zero corpus counts (still a valid
    //    heartbeat — install-without-use is a signal we want).
    let counts = gather_corpus_counts(paths);
    let harnesses_detected = join_harnesses(&detect_harnesses(paths));

    // 3. Emit through the global handle. The enabled gate is the kernel's
    //    (the caller — `cli_startup` — already confirmed `h.is_enabled()`); a
    //    disabled handle is a pure no-op.
    emit(Heartbeat {
        skills: counts.skills,
        commands: counts.commands,
        agents: counts.agents,
        workspaces: counts.workspaces,
        catalogs: counts.catalogs,
        harnesses_detected,
        env,
    });

    // 4. Record today's date AFTER the emit. Ordering is deliberate and
    //    lossy-by-design (FR-039): a crash between emit and this write at worst
    //    re-emits one heartbeat on the next run.
    if let Err(e) = record_last_heartbeat(paths, &today) {
        tracing::debug!(target: "telemetry", error = %e, "heartbeat: recording last-heartbeat failed (best-effort)");
    }
}

/// Closed-vocabulary, sorted, comma-joined harness wire tokens (the kernel
/// rejects arrays, so the heartbeat carries a single flat string). Empty when no
/// harness is detected.
fn join_harnesses(detected: &[Harness]) -> String {
    let mut tokens: Vec<&'static str> = detected.iter().map(Harness::as_wire_token).collect();
    tokens.sort_unstable();
    tokens.dedup();
    tokens.join(",")
}

/// Read the stored `YYYY-MM-DD` last-heartbeat date, or `None` if absent /
/// unreadable. A bounded read so a corrupt/over-grown stamp can't blow memory.
fn read_last_heartbeat(paths: &Paths) -> Option<String> {
    let path = paths.telemetry_last_heartbeat();
    // Read/write containment parity — `record_last_heartbeat` writes via
    // `write_atomic` (symlink-refusing); refuse a symlinked component on the read
    // too. A hostile stamp is treated as absent (best-effort `None`).
    if crate::util::refuse_symlinked_component(&path).is_err() {
        return None;
    }
    match crate::util::bounded_read_to_string(&path, LAST_HEARTBEAT_READ_CAP) {
        Ok(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Err(_) => None,
    }
}

/// Atomically write today's `YYYY-MM-DD` to the `last-heartbeat` stamp (0600).
fn record_last_heartbeat(paths: &Paths, today: &str) -> Result<(), crate::error::TomeError> {
    crate::catalog::store::write_atomic(&paths.telemetry_last_heartbeat(), today.as_bytes())
}

/// The raw corpus counts carried by one heartbeat (the kernel buckets them).
struct CorpusCounts {
    skills: u32,
    commands: u32,
    agents: u32,
    workspaces: u32,
    catalogs: u32,
}

impl CorpusCounts {
    /// The all-zero snapshot used when the index can't be read (a fresh install
    /// with no DB is a valid, all-zero heartbeat).
    fn zeroed() -> Self {
        CorpusCounts {
            skills: 0,
            commands: 0,
            agents: 0,
            workspaces: 0,
            catalogs: 0,
        }
    }
}

/// Gather the corpus inventory from the central index, READ-ONLY and lock-free.
///
/// Opens the index with [`crate::index::open_read_only`] — which never takes the
/// advisory lock (NFR-009) and errors (rather than creating) on a missing file.
/// On ANY failure to open OR to query, the corpus counts default to `0`.
fn gather_corpus_counts(paths: &Paths) -> CorpusCounts {
    let conn = match crate::index::open_read_only(&paths.index_db) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(target: "telemetry", error = %e, "heartbeat: index unavailable, zero corpus counts");
            return CorpusCounts::zeroed();
        }
    };

    let (skills, commands, agents) = count_entries_by_kind(&conn);
    let workspaces = count_rows(&conn, "SELECT COUNT(*) FROM workspaces");
    let catalogs = count_rows(&conn, "SELECT COUNT(DISTINCT url) FROM workspace_catalogs");

    CorpusCounts {
        skills: clamp_u32(skills),
        commands: clamp_u32(commands),
        agents: clamp_u32(agents),
        workspaces: clamp_u32(workspaces),
        catalogs: clamp_u32(catalogs),
    }
}

/// Saturating cast for a count that the wire carries as a `u32`. A pathologically
/// huge corpus saturates at `u32::MAX` rather than wrapping.
fn clamp_u32(n: u64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Count `skills` rows grouped by `kind`, returning `(skills, commands, agents)`.
/// Best-effort: ANY error collapses to `(0, 0, 0)`.
fn count_entries_by_kind(conn: &rusqlite::Connection) -> (u64, u64, u64) {
    let run = || -> Result<(u64, u64, u64), rusqlite::Error> {
        let mut stmt = conn.prepare("SELECT kind, COUNT(*) FROM skills GROUP BY kind")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let (mut skills, mut commands, mut agents) = (0u64, 0u64, 0u64);
        for r in rows {
            let (kind, n) = r?;
            let n = u64::try_from(n).unwrap_or(0);
            match kind.as_str() {
                "skill" => skills = n,
                "command" => commands = n,
                "agent" => agents = n,
                _ => {}
            }
        }
        Ok((skills, commands, agents))
    };
    run().unwrap_or((0, 0, 0))
}

/// Run a `SELECT COUNT(...)` returning a single non-negative integer; degrade to
/// `0` on ANY error.
fn count_rows(conn: &rusqlite::Connection, sql: &str) -> u64 {
    conn.query_row(sql, [], |row| row.get::<_, i64>(0))
        .ok()
        .map(|n| u64::try_from(n).unwrap_or(0))
        .unwrap_or(0)
}

/// Every supported harness DETECTED on disk, mapped to the closed [`Harness`]
/// enum. Reuses the existing detection (`with_effective_modules` + `m.detect`)
/// that `tome meta` / `tome harness` use. `home` is derived from `paths`. An
/// unmappable harness name (e.g. a stub) is skipped.
fn detect_harnesses(paths: &Paths) -> Vec<Harness> {
    let home = match paths.root.parent() {
        Some(p) => p.to_path_buf(),
        None => return Vec::new(),
    };
    detect_harness_names(&home)
        .into_iter()
        .filter_map(|name| harness_from_name(&name))
        .collect::<Vec<_>>()
}

/// The names of every supported harness detected under `home` (existence-only
/// probe), via the shared effective-modules registry.
fn detect_harness_names(home: &Path) -> Vec<String> {
    crate::harness::with_effective_modules(|mods| {
        mods.iter()
            .filter(|m| m.detect(home))
            .map(|m| m.name().to_string())
            .collect()
    })
}

/// Map a harness module `name()` to the closed [`Harness`] enum. Returns `None`
/// for any name with no enum variant (delegates to the SSOT bridge).
fn harness_from_name(name: &str) -> Option<Harness> {
    crate::commands::harness::harness_name_to_enum(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_harnesses_sorts_dedups_and_comma_joins() {
        let detected = vec![Harness::Cursor, Harness::ClaudeCode, Harness::Cursor];
        assert_eq!(join_harnesses(&detected), "claude-code,cursor");
        // Empty input ⇒ empty string (no leading/trailing comma).
        assert_eq!(join_harnesses(&[]), "");
    }

    #[test]
    fn harness_name_mapping_skips_unknown() {
        assert_eq!(harness_from_name("claude-code"), Some(Harness::ClaudeCode));
        assert_eq!(harness_from_name("cursor"), Some(Harness::Cursor));
        assert_eq!(harness_from_name("codex"), Some(Harness::Codex));
        assert_eq!(harness_from_name("opencode"), Some(Harness::Opencode));
        assert_eq!(harness_from_name("gemini"), Some(Harness::GeminiCli));
        // An unmappable name (e.g. the test stub) is dropped.
        assert_eq!(harness_from_name("stub"), None);
        assert_eq!(harness_from_name("not-a-harness"), None);
    }

    #[test]
    fn clamp_u32_saturates() {
        assert_eq!(clamp_u32(5), 5);
        assert_eq!(clamp_u32(u64::MAX), u32::MAX);
    }
}
