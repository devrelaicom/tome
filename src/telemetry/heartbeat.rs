//! `tome.heartbeat` — a once-per-UTC-day inventory snapshot (US2 / FR-039).
//!
//! The heartbeat is the anonymous stream's product-fitness pulse: it tells us how
//! large an install's corpus is (skills / commands / agents / workspaces /
//! catalogs, every count BUCKETED — never an exact integer) and which harnesses
//! are detected on disk. It carries no free-form string and no exact count, so it
//! stays inside the anonymous-stream invariant by construction.
//!
//! Two non-negotiable properties shape every line below:
//!
//! 1. **Best-effort** — this NEVER crashes or blocks the foreground command
//!    (NFR-001). Every count is gathered defensively and defaults to `0`/empty on
//!    ANY error; a missing/unopenable index is a *valid* heartbeat (a fresh
//!    install with no index still tells us "installed, not yet used").
//! 2. **Read-only, no advisory lock** (NFR-009) — we open the central index with
//!    [`index::open_read_only`], which never touches `index.lock`. SQLite's MVCC
//!    gives a read-only handle a consistent snapshot regardless of a concurrent
//!    writer, so the heartbeat can never race or stall the index.
//!
//! Cheap gate FIRST: the once-per-day check reads only a tiny date stamp and
//! returns before opening the index on all but (at most) one call per UTC day, so
//! the expensive count-gathering runs at most once daily.

use std::path::Path;

use crate::paths::Paths;
use crate::telemetry::buckets::CountBucket;
use crate::telemetry::event::{Harness, Heartbeat};
use crate::telemetry::{clock, enqueue_to};

/// Cap on the stored `last-heartbeat` date read. A `YYYY-MM-DD` stamp is 10
/// bytes; 64 is comfortable slack while still bounding a corrupt/over-grown file.
const LAST_HEARTBEAT_READ_CAP: u64 = 64;

/// Emit a `tome.heartbeat` if one is due today, then record today's date.
///
/// Wired as the LAST step of [`crate::telemetry::cli_startup`]: it runs once per
/// CLI start but the once-per-UTC-day gate keeps the actual emit (and the
/// expensive count-gathering) to at most once daily. Infallible and best-effort
/// throughout — every failure branch is a `debug!` + return.
pub fn maybe_emit_heartbeat(paths: &Paths) {
    // 1. Cheap gate FIRST. Read the stored `YYYY-MM-DD`; a missing/unreadable
    //    stamp is treated as "never sent" (None) so a fresh install fires today.
    //    Comparing calendar dates (not elapsed seconds) makes the gate robust to
    //    minor clock skew within a day (research §R-7 / FR-039).
    let last = read_last_heartbeat(paths);
    let today = clock::today_utc_date(clock::now_utc());
    if !clock::heartbeat_due(last.as_deref(), &today) {
        // Already sent today — return BEFORE opening the index. This is what
        // keeps the count-gathering to once per UTC day.
        return;
    }

    // 2. Gather the corpus counts + detected harnesses, each best-effort. A
    //    missing/unopenable index yields all-`Zero` corpus buckets (still a
    //    valid heartbeat — install-without-use is a signal we want).
    let counts = gather_corpus_counts(paths);
    let harnesses_detected = detect_harnesses(paths);

    // 3. Enqueue against THE SAME `paths` we gated on. Using `enqueue_to` (not
    //    the default-resolving `enqueue`) keeps the heartbeat self-consistent:
    //    it lands in one `Paths` — no default-`$HOME` divergence. NOTE:
    //    `enqueue_to` is the UN-gated primitive (it does NOT call `is_enabled()`);
    //    the enabled gate for this path is the caller's — `cli_startup` resolved
    //    `resolve_enabled` and returned early on a disabled install before ever
    //    reaching `maybe_emit_heartbeat`, so a disabled install enqueues nothing.
    enqueue_to(
        paths,
        Heartbeat {
            skills_bucket: counts.skills,
            commands_bucket: counts.commands,
            agents_bucket: counts.agents,
            workspaces_bucket: counts.workspaces,
            catalogs_bucket: counts.catalogs,
            harnesses_detected,
        },
    );

    // 4. Record today's date AFTER the enqueue. Ordering is deliberate and
    //    lossy-by-design (FR-039): a crash between enqueue and this write at
    //    worst re-emits one heartbeat on the next run — acceptable — whereas
    //    recording first could silently SKIP a day's heartbeat if the enqueue
    //    then failed. A write failure here is best-effort (`debug!` + continue).
    if let Err(e) = record_last_heartbeat(paths, &today) {
        tracing::debug!(target: "telemetry", error = %e, "heartbeat: recording last-heartbeat failed (best-effort)");
    }
}

/// Read the stored `YYYY-MM-DD` last-heartbeat date, or `None` if absent /
/// unreadable. A bounded read so a corrupt/over-grown stamp can't blow memory.
fn read_last_heartbeat(paths: &Paths) -> Option<String> {
    let path = paths.telemetry_last_heartbeat();
    // Sec-L1: read/write containment parity — `record_last_heartbeat` writes via
    // `write_atomic` (symlink-refusing); refuse a symlinked component on the read
    // too. A hostile stamp is treated as absent (best-effort `None`), matching the
    // "never sent" degrade below — never propagated.
    if crate::util::refuse_symlinked_component(&path).is_err() {
        return None;
    }
    match crate::util::bounded_read_to_string(&path, LAST_HEARTBEAT_READ_CAP) {
        // Trim trailing newline/whitespace so the comparison against
        // `today_utc_date` (which has none) is exact.
        Ok(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        // Missing or any read error ⇒ treat as "never sent". Best-effort: we
        // never propagate, so a transient read fault just re-emits today.
        Err(_) => None,
    }
}

/// Atomically write today's `YYYY-MM-DD` to the `last-heartbeat` stamp (0600).
///
/// Routes through [`crate::catalog::store::write_atomic`] (the shared atomic
/// 0600 writer, symlink-refusing, dir-creating) so the stamp's mode and atomicity
/// match every other Tome-owned telemetry file.
fn record_last_heartbeat(paths: &Paths, today: &str) -> Result<(), crate::error::TomeError> {
    crate::catalog::store::write_atomic(&paths.telemetry_last_heartbeat(), today.as_bytes())
}

/// The bucketed corpus counts carried by one heartbeat.
struct CorpusCounts {
    skills: CountBucket,
    commands: CountBucket,
    agents: CountBucket,
    workspaces: CountBucket,
    catalogs: CountBucket,
}

impl CorpusCounts {
    /// The all-`Zero` snapshot used when the index can't be read (a fresh
    /// install with no DB is a valid, all-zero heartbeat).
    fn zeroed() -> Self {
        CorpusCounts {
            skills: CountBucket::from(0u64),
            commands: CountBucket::from(0u64),
            agents: CountBucket::from(0u64),
            workspaces: CountBucket::from(0u64),
            catalogs: CountBucket::from(0u64),
        }
    }
}

/// Gather the corpus inventory from the central index, READ-ONLY and lock-free.
///
/// Opens the index with [`crate::index::open_read_only`] — which never takes the
/// advisory lock (NFR-009) and errors (rather than creating) on a missing file.
/// On ANY failure to open OR to query, the corpus buckets default to `Zero`: the
/// heartbeat still emits, reporting "install present, nothing indexed".
fn gather_corpus_counts(paths: &Paths) -> CorpusCounts {
    // A missing DB file is the common fresh-install case: `open_read_only`
    // surfaces it as an error, which we deliberately fold into the all-zero
    // snapshot rather than skipping the heartbeat.
    let conn = match crate::index::open_read_only(&paths.index_db) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(target: "telemetry", error = %e, "heartbeat: index unavailable, zero corpus buckets");
            return CorpusCounts::zeroed();
        }
    };

    // Per-kind entry counts over the WHOLE `skills` corpus (not scoped to one
    // workspace) — the install-wide inventory. `kind` is the schema-v3 column,
    // widened in v4 to admit `agent`. Defaults to all-zero on any query error.
    let (skills, commands, agents) = count_entries_by_kind(&conn);

    // Registered workspaces: every row of `workspaces` (this INCLUDES the
    // privileged `global` workspace, matching `workspace list`'s own count).
    let workspaces = count_rows(&conn, "SELECT COUNT(*) FROM workspaces");

    // Enrolled catalogs: distinct catalog URLs across every workspace. Counting
    // DISTINCT urls (not enrolment rows) matches the content-addressed
    // one-clone-per-url model — the install knows N distinct catalogs.
    let catalogs = count_rows(&conn, "SELECT COUNT(DISTINCT url) FROM workspace_catalogs");

    CorpusCounts {
        skills: CountBucket::from(skills),
        commands: CountBucket::from(commands),
        agents: CountBucket::from(agents),
        workspaces: CountBucket::from(workspaces),
        catalogs: CountBucket::from(catalogs),
    }
}

/// Count `skills` rows grouped by `kind`, returning `(skills, commands, agents)`.
/// Best-effort: ANY error (prepare/query/row) collapses to `(0, 0, 0)` — a
/// degraded count must never fail the heartbeat.
fn count_entries_by_kind(conn: &rusqlite::Connection) -> (u64, u64, u64) {
    let run = || -> Result<(u64, u64, u64), rusqlite::Error> {
        let mut stmt = conn.prepare("SELECT kind, COUNT(*) FROM skills GROUP BY kind")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let (mut skills, mut commands, mut agents) = (0u64, 0u64, 0u64);
        for r in rows {
            let (kind, n) = r?;
            // SQLite COUNT(*) is non-negative; clamp defensively to u64.
            let n = u64::try_from(n).unwrap_or(0);
            // Stringly-typed match here (NOT the canonical EntryKind dispatch
            // doctor uses): an unknown `kind` on this BEST-EFFORT telemetry path
            // must be ignored, never error — surfacing schema drift is the
            // integrity checker's job, not the heartbeat's.
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
/// `0` on ANY error. The SSOT for the single-scalar count queries above.
fn count_rows(conn: &rusqlite::Connection, sql: &str) -> u64 {
    conn.query_row(sql, [], |row| row.get::<_, i64>(0))
        .ok()
        .map(|n| u64::try_from(n).unwrap_or(0))
        .unwrap_or(0)
}

/// Every supported harness DETECTED on disk, mapped to the closed [`Harness`]
/// enum and SORTED for a deterministic wire shape.
///
/// Reuses the existing detection (`with_effective_modules` + `m.detect(home)`)
/// that `tome meta` / `tome harness` use. `home` is derived from `paths`
/// (`paths.root` is `<home>/.tome`, so its parent is `<home>`) rather than re-read
/// from `$HOME`, so the heartbeat detects against the SAME tree it was gated on —
/// consistent with `enqueue_to`'s path-injection. An unmappable harness name
/// (e.g. a stub or a not-yet-enumerated harness) is skipped.
fn detect_harnesses(paths: &Paths) -> Vec<Harness> {
    // `paths.root` == `<home>/.tome`; the harness modules probe `<home>/.claude`
    // etc., so detection needs the parent. On the unlikely absence of a parent
    // (a root with no parent), detect nothing rather than panic.
    let home = match paths.root.parent() {
        Some(p) => p.to_path_buf(),
        None => return Vec::new(),
    };
    let mut detected = detect_harness_names(&home)
        .into_iter()
        .filter_map(|name| harness_from_name(&name))
        .collect::<Vec<_>>();
    // Sort by the kebab wire token so the Vec is reproducible run-to-run.
    detected.sort_by_key(harness_wire_token);
    detected.dedup();
    detected
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
/// for any name with no enum variant (e.g. the test `stub` harness) so an
/// unmappable name is silently dropped rather than forced onto the wire.
///
/// PW7: delegates to the SSOT [`crate::commands::harness::harness_name_to_enum`]
/// rather than carrying a second parallel match — the two can never drift.
fn harness_from_name(name: &str) -> Option<Harness> {
    crate::commands::harness::harness_name_to_enum(name)
}

/// The kebab wire token for a [`Harness`], used as the deterministic sort key.
/// Pinned by hand (rather than via serde) so the sort order is legible and
/// independent of the serializer.
fn harness_wire_token(h: &Harness) -> &'static str {
    match h {
        Harness::ClaudeCode => "claude-code",
        Harness::Cursor => "cursor",
        Harness::Codex => "codex",
        Harness::Opencode => "opencode",
        Harness::GeminiCli => "gemini-cli",
        // Phase 11 — kebab wire tokens must match the serde `kebab-case`
        // rendering (the `harness_serialises_with_pinned_kebab_tokens` pin) so
        // this hand-written sort key stays in lockstep with the wire shape.
        Harness::CopilotCli => "copilot-cli",
        Harness::Copilot => "copilot",
        Harness::Devin => "devin",
        Harness::Cline => "cline",
        Harness::Junie => "junie",
        Harness::JetbrainsAi => "jetbrains-ai",
        Harness::Antigravity => "antigravity",
        Harness::Pi => "pi",
        Harness::Crush => "crush",
        Harness::Zed => "zed",
        Harness::Kiro => "kiro",
        Harness::Generic => "generic",
        Harness::GenericOp => "generic-op",
        Harness::Goose => "goose",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::clock::ClockGuard;
    use crate::telemetry::queue;
    use tempfile::TempDir;
    use time::{Date, Month, OffsetDateTime, Time};

    fn paths_in(dir: &TempDir) -> Paths {
        Paths::from_root(dir.path().to_path_buf())
    }

    /// Fixed UTC instant builder (no `time/macros` feature).
    fn at(y: i32, m: Month, d: u8) -> OffsetDateTime {
        Date::from_calendar_date(y, m, d)
            .unwrap()
            .with_time(Time::from_hms(12, 0, 0).unwrap())
            .assume_utc()
    }

    /// RAII guard forcing telemetry ENABLED via `TOME_TELEMETRY=1` for the test
    /// duration, restoring the prior value (and CI vars) on drop. The force-on
    /// env override beats CI auto-off, so the enqueue path actually writes even
    /// under a CI runner.
    struct ForceEnabled {
        prev: Option<std::ffi::OsString>,
    }
    impl ForceEnabled {
        fn on() -> Self {
            let prev = std::env::var_os("TOME_TELEMETRY");
            // SAFETY: tests in this module that touch this var are serialised by
            // the `ENV_MUTEX` below; production never sets it here.
            unsafe { std::env::set_var("TOME_TELEMETRY", "1") };
            ForceEnabled { prev }
        }
    }
    impl Drop for ForceEnabled {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => unsafe { std::env::set_var("TOME_TELEMETRY", v) },
                None => unsafe { std::env::remove_var("TOME_TELEMETRY") },
            }
        }
    }

    /// Serialise the env-mutating heartbeat tests (process-global `TOME_TELEMETRY`).
    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn not_due_when_last_equals_today_enqueues_nothing() {
        let _lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _enabled = ForceEnabled::on();
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);

        // Fix the clock so "today" is deterministic, and stamp last-heartbeat to
        // the SAME day ⇒ not due.
        let _clock = ClockGuard::install(at(2026, Month::June, 11));
        std::fs::create_dir_all(paths.telemetry_dir()).unwrap();
        std::fs::write(paths.telemetry_last_heartbeat(), "2026-06-11").unwrap();

        maybe_emit_heartbeat(&paths);

        // Nothing enqueued and the stamp is unchanged.
        assert_eq!(queue::count_pending(&paths), 0, "no heartbeat when not due");
        assert_eq!(
            std::fs::read_to_string(paths.telemetry_last_heartbeat()).unwrap(),
            "2026-06-11",
            "last-heartbeat untouched when not due"
        );
    }

    #[test]
    fn due_with_no_stamp_emits_heartbeat_and_records_today() {
        let _lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _enabled = ForceEnabled::on();
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);

        let _clock = ClockGuard::install(at(2026, Month::June, 11));
        assert!(
            !paths.telemetry_last_heartbeat().exists(),
            "no stamp before first heartbeat"
        );

        maybe_emit_heartbeat(&paths);

        // Exactly one heartbeat line landed.
        let lines = queue::read_lines(&paths).unwrap();
        assert_eq!(lines.len(), 1, "one heartbeat enqueued");
        let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(v["event_type"], "tome.heartbeat");
        // The bucket fields are present (and serialize as closed-enum tokens).
        for field in [
            "skills_bucket",
            "commands_bucket",
            "agents_bucket",
            "workspaces_bucket",
            "catalogs_bucket",
        ] {
            assert!(v.get(field).is_some(), "missing bucket field {field}: {v}");
        }
        assert!(
            v.get("harnesses_detected").is_some(),
            "missing harnesses_detected: {v}"
        );

        // And the stamp now records today.
        assert_eq!(
            std::fs::read_to_string(paths.telemetry_last_heartbeat())
                .unwrap()
                .trim(),
            "2026-06-11",
            "last-heartbeat recorded after emit"
        );
    }

    #[test]
    fn fresh_install_with_no_index_emits_zero_corpus_buckets() {
        let _lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _enabled = ForceEnabled::on();
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);

        // No index.db at all — a fresh install. Must NOT panic and must emit.
        assert!(!paths.index_db.exists(), "no index db in fixture");
        let _clock = ClockGuard::install(at(2026, Month::June, 11));

        maybe_emit_heartbeat(&paths);

        let lines = queue::read_lines(&paths).unwrap();
        assert_eq!(lines.len(), 1, "heartbeat still emits with no index");
        let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(v["event_type"], "tome.heartbeat");
        // All corpus buckets are the `Zero` token.
        for field in [
            "skills_bucket",
            "commands_bucket",
            "agents_bucket",
            "workspaces_bucket",
            "catalogs_bucket",
        ] {
            assert_eq!(v[field], "0", "expected zero bucket for {field}: {v}");
        }
    }

    #[test]
    fn gather_corpus_counts_defaults_to_zero_on_missing_index() {
        // Directly exercise the gather path: a missing index DB ⇒ all-zero.
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let counts = gather_corpus_counts(&paths);
        assert_eq!(counts.skills, CountBucket::from(0u64));
        assert_eq!(counts.commands, CountBucket::from(0u64));
        assert_eq!(counts.agents, CountBucket::from(0u64));
        assert_eq!(counts.workspaces, CountBucket::from(0u64));
        assert_eq!(counts.catalogs, CountBucket::from(0u64));
    }

    /// T-M3 / SC-003 — the de-dup-across-invocations property: many
    /// `maybe_emit_heartbeat` calls within ONE UTC day emit EXACTLY ONE
    /// `tome.heartbeat` (the once-per-day gate via the `last-heartbeat` stamp),
    /// then a SECOND heartbeat fires once the clock advances to the next day.
    /// This is the unit-level proof of "100 runs → 1 heartbeat / day".
    #[test]
    fn heartbeat_dedups_within_a_day_then_re_emits_next_day() {
        let _lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _enabled = ForceEnabled::on();
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);

        // Day 1: five back-to-back invocations under a FIXED clock. The first is
        // due (no stamp) and emits + records today; the next four see the stamp
        // == today and return before the index open ⇒ no further emit.
        {
            let _clock = ClockGuard::install(at(2026, Month::June, 11));
            for _ in 0..5 {
                maybe_emit_heartbeat(&paths);
            }
        }
        let after_day1 = queue::count_pending(&paths);
        assert_eq!(
            after_day1, 1,
            "exactly one heartbeat across five same-day invocations (SC-003)",
        );
        // And it IS a heartbeat (not some other event leaking in).
        let lines = queue::read_lines(&paths).unwrap();
        let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(v["event_type"], "tome.heartbeat");
        assert_eq!(
            std::fs::read_to_string(paths.telemetry_last_heartbeat())
                .unwrap()
                .trim(),
            "2026-06-11",
            "day-1 stamp recorded",
        );

        // Day 2: advance the clock one calendar day. The stamp (2026-06-11) now
        // differs from today (2026-06-12) ⇒ a SECOND heartbeat is due.
        {
            let _clock = ClockGuard::install(at(2026, Month::June, 12));
            for _ in 0..5 {
                maybe_emit_heartbeat(&paths);
            }
        }
        assert_eq!(
            queue::count_pending(&paths),
            2,
            "the day boundary releases exactly one more heartbeat (total 2)",
        );
        assert_eq!(
            std::fs::read_to_string(paths.telemetry_last_heartbeat())
                .unwrap()
                .trim(),
            "2026-06-12",
            "day-2 stamp recorded after the second emit",
        );
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
}
