//! Phase 10 / US5 (FR-064): the read-only `tome doctor` telemetry subsystem
//! projection.
//!
//! This is a pure projection over the on-disk telemetry state the writers
//! produced — the doctor-as-projection precedent (P6/US5). It performs NO
//! writes, NO mint, and NO directory creation (FR-124): every field routes
//! through an existing telemetry reader, so doctor and `tome telemetry status`
//! read the SAME state and cannot diverge.
//!
//! Read-only proof, field by field:
//! - enabled/source: [`config::resolve_enabled_with_source`] (a malformed
//!   config is reported as `config_error`, NOT propagated — doctor never
//!   crashes, FR-561);
//! - install id: `std::fs::metadata` of `telemetry/id` (mode + mtime only —
//!   never opens for write, never mints);
//! - queue: [`queue::count_pending`] / [`queue::classify_lines`] — the SAME
//!   bounded, fail-closed reads `inspect`/`status` use; they never mutate;
//! - last flush: a bounded read of the `last-flush` stamp;
//! - endpoint: [`transport::resolve_endpoint`] (already credential-scrubbed);
//! - allowlist: the compiled-in [`allowlist::ATTRIBUTED_TELEMETRY_CATALOGS`].
//!
//! `--fix` gains NOTHING here (FR-065): disabling is a user action and a corrupt
//! queue self-heals on the next drain, so there is no repair function — the
//! command layer simply re-assembles this read-only section.

use serde::Deserialize;
use time::OffsetDateTime;

use crate::doctor::report::{
    TelemetryAllowlistEntry, TelemetryFlushReport, TelemetryIdReport, TelemetryQueueReport,
    TelemetrySection,
};
use crate::paths::Paths;
use crate::telemetry::{allowlist, config, queue, transport};

/// The `last-flush` stamp shape — written by the flusher as
/// `{"timestamp":"<rfc3339>","last_status":<u16|null>}`. Local mirror of the
/// `commands::telemetry` reader's shape (kept private to each read-only surface;
/// neither writes).
#[derive(Debug, Deserialize)]
struct LastFlushStamp {
    timestamp: String,
    #[serde(rename = "last_status", default)]
    status: Option<u16>,
}

/// Assemble the read-only telemetry section for the doctor report.
///
/// Infallible by construction: a malformed config is captured as
/// `config_error` (with the enabled-state defaulting to the opt-out default-on,
/// matching what a fresh install reports) rather than bubbling — doctor must
/// keep rendering every other subsystem (FR-561). Every other read degrades to
/// a benign absent/zero value.
pub fn assemble(paths: &Paths) -> TelemetrySection {
    // enabled + source. A present-but-malformed config surfaces (exit 91) on the
    // FOREGROUND CLI; here we degrade it to a reported `config_error` so the
    // read-only doctor pass never crashes. The default enabled value mirrors the
    // opt-out default (on) — the user sees both the error AND the effective state.
    // NOTE: `e.to_string()` can echo a snippet of the offending config CONTENT
    // (the toml parse error's `detail`). Safe today because `telemetry/config.toml`
    // is Tome-owned and its only field is `enabled: bool` (no credential-shaped
    // value can land there), and the path component is already scrubbed. If this
    // config ever gains a free-text field, scrub the surfaced detail here.
    let (enabled, source, config_error) = match config::resolve_enabled_with_source(paths) {
        Ok((enabled, source)) => (enabled, source, None),
        Err(e) => (true, config::Source::Default, Some(e.to_string())),
    };

    TelemetrySection {
        enabled,
        source,
        config_error,
        install_id: install_id_report(paths),
        queue: queue_report(paths),
        last_flush: last_flush_report(paths),
        endpoint: transport::resolve_endpoint(),
        allowlist: allowlist_report(),
    }
}

/// The install-id file's path (scrubbed), existence, mode, and age. Reads only
/// `std::fs::metadata` — never opens, never mints.
fn install_id_report(paths: &Paths) -> TelemetryIdReport {
    let id_path = paths.telemetry_id();
    let path = scrubbed_path(&id_path.to_string_lossy());

    match std::fs::metadata(&id_path) {
        Ok(meta) => TelemetryIdReport {
            path,
            present: true,
            mode: file_mode(&meta),
            age_seconds: meta
                .modified()
                .ok()
                .map(OffsetDateTime::from)
                .and_then(age_seconds_from),
        },
        // Absent (or unreadable metadata): doctor reports "not present"; it never
        // mints to make it exist.
        Err(_) => TelemetryIdReport {
            path,
            present: false,
            mode: None,
            age_seconds: None,
        },
    }
}

/// The Unix mode bits (`& 0o777`) of a file; `None` on non-Unix.
#[cfg(unix)]
fn file_mode(meta: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn file_mode(_meta: &std::fs::Metadata) -> Option<u32> {
    None
}

/// Pending depth, corrupt-line count, and the oldest event's age. All three
/// reads are read-only (`count_pending` / `classify_lines` never mutate).
fn queue_report(paths: &Paths) -> TelemetryQueueReport {
    let pending = queue::count_pending(paths) as u64;

    // `classify_lines` returns the parsed values (oldest first) + a corrupt
    // count. Degrade any read error to `(empty, 0)` — a read-only report never
    // fails on the queue.
    let (events, corrupt) = queue::classify_lines(paths).unwrap_or_default();

    // FIFO: the first parsable event is the oldest. Its envelope `timestamp` is
    // the RFC3339-millis string the enqueue path stamped; age it against now.
    let oldest_age_seconds = events
        .iter()
        .find_map(|v| v.get("timestamp").and_then(serde_json::Value::as_str))
        .and_then(parse_rfc3339)
        .and_then(age_seconds_from);

    TelemetryQueueReport {
        pending,
        corrupt,
        oldest_age_seconds,
    }
}

/// The `last-flush` stamp (time + HTTP status), when present. Bounded,
/// read-only; an absent/unreadable/unparsable stamp degrades to `None`.
fn last_flush_report(paths: &Paths) -> Option<TelemetryFlushReport> {
    let body = crate::util::bounded_read_to_string(
        &paths.telemetry_last_flush(),
        crate::util::TOME_CONFIG_MAX,
    )
    .ok()?;
    let stamp = serde_json::from_str::<LastFlushStamp>(&body).ok()?;
    Some(TelemetryFlushReport {
        timestamp: stamp.timestamp,
        status: stamp.status,
    })
}

/// The compiled-in allowlist, projected to `(short_id, canonical_source)` rows.
fn allowlist_report() -> Vec<TelemetryAllowlistEntry> {
    allowlist::ATTRIBUTED_TELEMETRY_CATALOGS
        .iter()
        .map(|(short_id, source)| TelemetryAllowlistEntry {
            short_id: (*short_id).to_owned(),
            // Canonicalize the const side (idempotent — it is already canonical)
            // so the reported value equals what `match_source` compares against.
            canonical_source: allowlist::canonicalize(source)
                .unwrap_or_else(|| (*source).to_owned()),
        })
        .collect()
}

/// Parse the envelope's RFC3339-millis timestamp (`YYYY-MM-DDTHH:MM:SS.mmmZ`).
/// Returns `None` on any parse failure (a corrupt timestamp is data, not a
/// fault — the read-only report simply omits the oldest-age field).
fn parse_rfc3339(s: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
}

/// Whole-seconds age (now − `instant`), clamped at 0 (a clock skew that puts the
/// instant in the future reports age 0 rather than a negative/huge number).
fn age_seconds_from(instant: OffsetDateTime) -> Option<u64> {
    let delta = OffsetDateTime::now_utc() - instant;
    let secs = delta.whole_seconds();
    Some(secs.max(0) as u64)
}

/// Scrub a path for the report. A filesystem path can't carry URL credentials,
/// but routing it through the shared scrubber keeps "every telemetry-facing
/// string is scrubbed" true by construction (mirrors `config::scrubbed_path`).
fn scrubbed_path(path: &str) -> String {
    let scrubbed = crate::catalog::git::scrub_credentials(path.as_bytes());
    String::from_utf8_lossy(&scrubbed).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn paths_in(dir: &TempDir) -> Paths {
        Paths::from_root(dir.path().to_path_buf())
    }

    /// A fresh (no telemetry files) install: section reports default-on, no id,
    /// empty queue, no flush, and the one allowlist entry. Critically, assembling
    /// the report MINTS NOTHING — the telemetry dir/id/queue stay absent.
    #[test]
    fn assemble_on_fresh_install_is_read_only_and_default_on() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        // Force the env-independent default path: clear any ambient override.
        let section = with_telemetry_env_cleared(|| super::assemble(&paths));

        assert!(section.config_error.is_none());
        assert!(!section.install_id.present, "doctor must not mint the id");
        assert_eq!(section.queue.pending, 0);
        assert_eq!(section.queue.corrupt, 0);
        assert!(section.queue.oldest_age_seconds.is_none());
        assert!(section.last_flush.is_none());
        assert_eq!(section.allowlist.len(), 1);
        assert_eq!(section.allowlist[0].short_id, "midnight");
        assert_eq!(
            section.allowlist[0].canonical_source,
            "github.com/devrelaicom/midnight-expert-tome"
        );

        // No telemetry files were created by the read.
        assert!(!paths.telemetry_id().exists());
        assert!(!paths.telemetry_queue().exists());
        assert!(!paths.telemetry_dir().exists());
    }

    #[cfg(unix)]
    #[test]
    fn assemble_reports_id_mode_and_queue_depth() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        // Mint an id and seed two queue lines via the real telemetry writers.
        crate::telemetry::identity::ensure_install_id(&paths).unwrap();
        crate::telemetry::queue::append(&paths, "{\"timestamp\":\"2020-01-01T00:00:00.000Z\"}")
            .unwrap();
        crate::telemetry::queue::append(&paths, "{\"timestamp\":\"2020-01-02T00:00:00.000Z\"}")
            .unwrap();

        let section = with_telemetry_env_cleared(|| super::assemble(&paths));
        assert!(section.install_id.present);
        assert_eq!(section.install_id.mode, Some(0o600));
        assert!(section.install_id.age_seconds.is_some());
        assert_eq!(section.queue.pending, 2);
        // The oldest event (2020) is far in the past → a large positive age.
        assert!(section.queue.oldest_age_seconds.unwrap() > 0);
    }

    #[test]
    fn assemble_reports_last_flush_stamp() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        std::fs::create_dir_all(paths.telemetry_dir()).unwrap();
        std::fs::write(
            paths.telemetry_last_flush(),
            b"{\"timestamp\":\"2026-06-11T14:11:45.123Z\",\"last_status\":200}",
        )
        .unwrap();

        let section = with_telemetry_env_cleared(|| super::assemble(&paths));
        let flush = section.last_flush.expect("stamp present");
        assert_eq!(flush.timestamp, "2026-06-11T14:11:45.123Z");
        assert_eq!(flush.status, Some(200));
    }

    /// Run `f` with `TOME_TELEMETRY` cleared so the resolver takes the
    /// config/default path deterministically (and not a CI runner's ambient
    /// `CI=true`). Restores the prior value. Serialised on a process-wide mutex
    /// because env is global.
    fn with_telemetry_env_cleared<T>(f: impl FnOnce() -> T) -> T {
        use std::sync::Mutex;
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());

        let saved: Vec<(&str, Option<String>)> = [
            "TOME_TELEMETRY",
            "CI",
            "GITHUB_ACTIONS",
            "GITLAB_CI",
            "CIRCLECI",
            "BUILDKITE",
            "JENKINS_URL",
            "TF_BUILD",
            "TEAMCITY_VERSION",
        ]
        .iter()
        .map(|k| (*k, std::env::var(*k).ok()))
        .collect();
        for (k, _) in &saved {
            // SAFETY: serialised by ENV_LOCK; this test crate is the only mutator.
            unsafe { std::env::remove_var(k) };
        }

        let out = f();

        for (k, v) in saved {
            match v {
                Some(v) => unsafe { std::env::set_var(k, v) },
                None => unsafe { std::env::remove_var(k) },
            }
        }
        out
    }
}
