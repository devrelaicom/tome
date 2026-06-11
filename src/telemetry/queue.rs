//! The append-only local JSONL queue (`telemetry/queue.jsonl`).
//!
//! The defining invariant of the whole feature lives here: a foreground enqueue
//! does **exactly one append** — no network, no contended lock, no wait
//! (NFR-001). [`append`] re-opens an `O_APPEND` fd per call and issues a SINGLE
//! `write` of one `\n`-terminated line; on a local FS that one syscall is atomic
//! (no interleave), and any non-local torn line is absorbed by the flusher's
//! self-heal (it drops unparsable lines on drain). Correctness — never crashing
//! the caller — holds everywhere.
//!
//! Everything in this module is sync and best-effort: a write/read failure is a
//! dropped event, never a propagated crash on the silent path. The fallible
//! signatures exist so the *foreground* `tome telemetry` surfaces (status,
//! inspect, flush) can still report loudly; the silent enqueue path collapses
//! any `Err` to a `debug!` + return at its call site.

use std::io::Write;

use crate::error::TomeError;
use crate::paths::Paths;

/// Hard per-line cap, INCLUDING the trailing `\n` (FR-036, research §R-5). The
/// single-`write` atomic-interleave guarantee is bounded to lines ≤ this on a
/// local FS, so a line at/over the cap is dropped rather than split (splitting
/// would break the one-event-one-line invariant the flusher relies on).
const MAX_LINE_BYTES: usize = 4096;

/// Soft queue size cap (FR-038). Past this, foreground enqueues drop silently
/// (`telemetry_queue_overflow`); FIFO eviction of the oldest events happens
/// later under the flush lock at the next drain (US3), never on this hot path.
///
/// `pub` so the flusher (US3) can reuse the SAME cap for its drain-time FIFO
/// eviction (FR-038/038a) — the append-path cap and the drain-path eviction
/// threshold MUST be the one number, not two that "agree today".
pub const MAX_QUEUE_BYTES: u64 = 1_048_576; // 1 MiB

/// Generous read cap for whole-queue reads: the queue is bounded to ~1 MiB by
/// [`append`], but a pre-existing/over-grown file (e.g. an interrupted earlier
/// run) should still read what fits rather than error. 2 MiB gives slack over
/// the 1 MiB soft cap; a true read failure (missing-except-handled, unreadable)
/// surfaces as `Io`.
const QUEUE_READ_CAP: u64 = 2 * 1_048_576; // 2 MiB

/// Ensure `telemetry/` exists (0700, owner-only). Idempotent. Mirrors
/// `identity::ensure_dir` / `lock::open_lock_file`'s dir landing so the tree's
/// mode is consistent regardless of which subsystem creates it first.
fn ensure_dir(paths: &Paths) -> Result<(), TomeError> {
    let dir = paths.telemetry_dir();
    std::fs::create_dir_all(&dir).map_err(TomeError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Best-effort: the per-file 0600 modes are the real guarantee.
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

/// Append one event line to the queue with a single `O_APPEND` `write`.
///
/// `line` is the JSON body WITHOUT a trailing newline (as produced by
/// [`event::to_line`](crate::telemetry::event::to_line)); this fn adds the `\n`.
///
/// Drops (returning `Ok(())`, never an error — a dropped event is not a failure):
/// - **oversize**: `line.len() + 1 > 4096` ⇒ dropped, NEVER split (FR-036, R-5);
/// - **overflow**: `current_queue_len + line_bytes > 1 MiB` ⇒ dropped silently,
///   FIFO eviction deferred to the next flush-lock drain (FR-038, R-8).
///
/// The fd is re-opened per call by design (R-5): a cached fd would keep pointing
/// at the now-unlinked inode after the flusher's rewrite-rename, silently losing
/// every subsequent append. Re-opening is one `open` per (rare) event — cheap.
pub fn append(paths: &Paths, line: &str) -> Result<(), TomeError> {
    // Pre-size the wire buffer: the line plus its single trailing newline. We
    // build the full buffer first so the subsequent `write` is one syscall.
    let wire_len = line.len() + 1;
    if wire_len > MAX_LINE_BYTES {
        // Over the per-line cap: drop, never split. Splitting would emit two
        // half-lines the flusher would later drop as unparsable anyway.
        tracing::debug!(
            target: "telemetry",
            len = wire_len,
            "telemetry_queue_line_oversize"
        );
        return Ok(());
    }

    ensure_dir(paths)?;
    let queue = paths.telemetry_queue();

    // Size cap (FR-038): stat first; if appending this line would cross 1 MiB,
    // drop silently. A missing queue is size 0 (the first append). FIFO eviction
    // of the oldest lines is the flusher's job under the flush lock — not here,
    // where we must stay single-append and lock-free.
    let current_len = match std::fs::metadata(&queue) {
        Ok(m) => m.len(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(e) => return Err(TomeError::Io(e)),
    };
    if current_len + wire_len as u64 > MAX_QUEUE_BYTES {
        tracing::debug!(
            target: "telemetry",
            current_len,
            line_bytes = wire_len,
            "telemetry_queue_overflow"
        );
        return Ok(());
    }

    // Read/write containment parity (S-M1): every other telemetry write sink
    // (`write_atomic` on the id / last-version, the id read in `ensure_install_id`)
    // refuses a symlinked final component; the append sink must too, or a hostile
    // `telemetry/queue.jsonl` symlink/FIFO could redirect this `O_APPEND` write —
    // and the install UUID it carries — out of tree. On refusal we DROP the event
    // (`Ok(())`, never block/propagate): this is the best-effort silent path, so a
    // poisoned queue path must not crash or stall the user's foreground command.
    if let Err(e) = crate::util::refuse_symlinked_component(&queue) {
        tracing::debug!(
            target: "telemetry",
            error = %e,
            "telemetry_queue_unsafe_path"
        );
        return Ok(());
    }

    // Open append-only. `.mode(0o600)` only takes effect when this `create`s the
    // file; appending to an existing queue keeps its mode (already 0600 from the
    // creating append), so no defensive re-chmod is needed.
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&queue).map_err(TomeError::Io)?;

    // Build the exact bytes (line + '\n') and issue ONE `write`. On an O_APPEND
    // fd the kernel makes the seek-to-end + write a single atomic step, so a
    // sub-4096-byte write to a local regular file cannot interleave with another
    // appender's line. We deliberately avoid buffered `writeln!` (which can split
    // into multiple writes and defeats the atomicity guarantee).
    let mut buf = Vec::with_capacity(wire_len);
    buf.extend_from_slice(line.as_bytes());
    buf.push(b'\n');

    let n = file.write(&buf).map_err(TomeError::Io)?;
    if n != buf.len() {
        // A short write would leave a torn line. For a <4096-byte write to a
        // local regular file this does not happen, but if it ever did we drop
        // (the flusher self-heals torn lines on drain) rather than retry — a
        // retry on an O_APPEND fd would duplicate the prefix.
        tracing::debug!(
            target: "telemetry",
            wrote = n,
            expected = buf.len(),
            "telemetry_queue_short_write"
        );
    }
    Ok(())
}

/// Read the queue's non-empty lines (oldest first). Read-only — NEVER mutates
/// the file (inspect/status rely on this). A missing queue reads as empty.
///
/// Bounded by [`QUEUE_READ_CAP`]; a true read failure surfaces as `Io`. The
/// trailing empty fragment after the final `\n` is dropped.
pub fn read_lines(paths: &Paths) -> Result<Vec<String>, TomeError> {
    let queue = paths.telemetry_queue();
    let body = match crate::util::bounded_read_to_string(&queue, QUEUE_READ_CAP) {
        Ok(b) => b,
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect())
}

/// Count the pending (non-blank) lines in the queue. The SSOT line-count used by
/// `status` and any other read-only reporter. Degrades to `0` on ANY error
/// (missing/unreadable/over-cap) so a read-only report never fails on the count.
pub fn count_pending(paths: &Paths) -> usize {
    read_lines(paths).map(|v| v.len()).unwrap_or(0)
}

/// Parse each queued line as JSON, partitioning into the parsed values and a
/// count of unparsable lines. Read-only.
///
/// `inspect` (US2) uses the corrupt count to decide whether to surface
/// [`TelemetryQueueCorrupt`](crate::error::TomeError::TelemetryQueueCorrupt)
/// (exit 92); the flusher (US3) uses the same partition to self-heal (drop the
/// unparsable lines on drain).
pub fn classify_lines(paths: &Paths) -> Result<(Vec<serde_json::Value>, usize), TomeError> {
    let lines = read_lines(paths)?;
    let mut values = Vec::with_capacity(lines.len());
    let mut corrupt = 0usize;
    for line in &lines {
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(v) => values.push(v),
            // A non-JSON line is a torn/corrupt entry — count it (the caller
            // reports or self-heals). We do not propagate: a corrupt line is data,
            // not an I/O fault.
            Err(_) => corrupt += 1,
        }
    }
    Ok((values, corrupt))
}

/// Atomically replace the queue with exactly `lines` (each terminated by `\n`).
///
/// The primitive the flusher (US3) uses for FIFO eviction and post-2xx removal:
/// it builds the surviving lines and rewrites in one atomic temp-file + rename
/// (0600, symlink-refusing) via [`write_atomic`](crate::catalog::store::write_atomic),
/// so a crash mid-rewrite leaves either the old or the new queue, never a torn
/// one. Callers MUST hold `flush.lock` (this fn does not — it is the primitive).
pub fn rewrite(paths: &Paths, lines: &[String]) -> Result<(), TomeError> {
    ensure_dir(paths)?;
    // Pre-size: sum of line lengths + one '\n' each.
    let cap = lines.iter().map(|l| l.len() + 1).sum();
    let mut body = String::with_capacity(cap);
    for line in lines {
        body.push_str(line);
        body.push('\n');
    }
    crate::catalog::store::write_atomic(&paths.telemetry_queue(), body.as_bytes())
}

/// Re-assert `0600` on the queue after an atomic *replace*: `write_atomic`
/// preserves the prior file's mode, so a queue whose mode was somehow loosened
/// would inherit the loose mode across a [`rewrite`]. Best-effort, Unix-only.
/// (Exposed for the flusher's rewrite path; harmless on the append path, which
/// creates with `0o600` directly.)
#[cfg(unix)]
pub fn reassert_queue_0600(paths: &Paths) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(
        paths.telemetry_queue(),
        std::fs::Permissions::from_mode(0o600),
    );
}

/// No-op on non-Unix.
#[cfg(not(unix))]
pub fn reassert_queue_0600(_paths: &Paths) {}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn paths_in(dir: &TempDir) -> Paths {
        Paths::from_root(dir.path().to_path_buf())
    }

    #[test]
    fn append_writes_single_newline_terminated_line() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        append(&paths, "{\"a\":1}").unwrap();
        let body = std::fs::read_to_string(paths.telemetry_queue()).unwrap();
        assert_eq!(body, "{\"a\":1}\n");
    }

    #[cfg(unix)]
    #[test]
    fn append_creates_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        append(&paths, "{}").unwrap();
        let mode = std::fs::metadata(paths.telemetry_queue())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn oversize_line_is_dropped_file_unchanged() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        // Seed one good line so we can prove the file is unchanged after the drop.
        append(&paths, "{\"ok\":true}").unwrap();
        let before = std::fs::read_to_string(paths.telemetry_queue()).unwrap();

        // A line whose len + 1 (newline) is exactly at the cap is dropped (4096
        // would need a 4095-byte line + '\n'; use 4096 chars so wire_len = 4097).
        let huge = "x".repeat(MAX_LINE_BYTES);
        append(&paths, &huge).unwrap();

        let after = std::fs::read_to_string(paths.telemetry_queue()).unwrap();
        assert_eq!(before, after, "oversize line must not touch the queue");
    }

    #[test]
    fn line_exactly_at_cap_minus_newline_is_kept() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        // 4095 bytes + '\n' == 4096 == MAX_LINE_BYTES ⇒ kept (boundary is `>`).
        let line = "y".repeat(MAX_LINE_BYTES - 1);
        append(&paths, &line).unwrap();
        let lines = read_lines(&paths).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), MAX_LINE_BYTES - 1);
    }

    #[test]
    fn appends_accumulate_in_fifo_order() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        append(&paths, "{\"n\":1}").unwrap();
        append(&paths, "{\"n\":2}").unwrap();
        append(&paths, "{\"n\":3}").unwrap();
        let lines = read_lines(&paths).unwrap();
        assert_eq!(lines, vec!["{\"n\":1}", "{\"n\":2}", "{\"n\":3}"]);
    }

    #[test]
    fn over_one_mib_append_is_dropped() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        // Pre-fill the queue to AT/OVER 1 MiB so any further append must be
        // dropped. Each filler line is ~2 KiB; 600 of them ≈ 1.2 MiB > 1 MiB.
        let line = format!("{{\"x\":\"{}\"}}", "z".repeat(2000));
        let filler: Vec<String> = std::iter::repeat_n(line, 600).collect();
        rewrite(&paths, &filler).unwrap();
        let len_before = std::fs::metadata(paths.telemetry_queue()).unwrap().len();
        assert!(
            len_before > MAX_QUEUE_BYTES,
            "fixture must exceed the 1 MiB cap (was {len_before})"
        );

        let count_before = count_pending(&paths);
        // The queue is already over the cap ⇒ this append is silently dropped.
        append(&paths, "{\"overflow\":true}").unwrap();
        assert_eq!(
            count_pending(&paths),
            count_before,
            "an over-cap append adds no line"
        );
    }

    #[test]
    fn read_lines_and_count_round_trip() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        assert_eq!(count_pending(&paths), 0, "missing queue is 0 pending");
        append(&paths, "{\"a\":1}").unwrap();
        append(&paths, "{\"b\":2}").unwrap();
        assert_eq!(count_pending(&paths), 2);
        assert_eq!(read_lines(&paths).unwrap(), vec!["{\"a\":1}", "{\"b\":2}"]);
    }

    #[test]
    fn read_lines_drops_blank_fragments() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        // A queue with blank lines / trailing newline noise.
        std::fs::create_dir_all(paths.telemetry_dir()).unwrap();
        std::fs::write(paths.telemetry_queue(), "{\"a\":1}\n\n  \n{\"b\":2}\n").unwrap();
        assert_eq!(read_lines(&paths).unwrap(), vec!["{\"a\":1}", "{\"b\":2}"]);
        assert_eq!(count_pending(&paths), 2);
    }

    #[test]
    fn rewrite_replaces_contents_atomically() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        append(&paths, "{\"old\":1}").unwrap();
        append(&paths, "{\"old\":2}").unwrap();
        rewrite(&paths, &["{\"new\":1}".to_string()]).unwrap();
        assert_eq!(read_lines(&paths).unwrap(), vec!["{\"new\":1}"]);
        // Exact bytes: one line + newline, nothing left over.
        let body = std::fs::read_to_string(paths.telemetry_queue()).unwrap();
        assert_eq!(body, "{\"new\":1}\n");
    }

    #[cfg(unix)]
    #[test]
    fn rewrite_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        rewrite(&paths, &["{\"a\":1}".to_string()]).unwrap();
        let mode = std::fs::metadata(paths.telemetry_queue())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn rewrite_empty_truncates_to_empty() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        append(&paths, "{\"a\":1}").unwrap();
        rewrite(&paths, &[]).unwrap();
        assert_eq!(count_pending(&paths), 0);
        assert_eq!(
            std::fs::read_to_string(paths.telemetry_queue()).unwrap(),
            ""
        );
    }

    #[cfg(unix)]
    #[test]
    fn append_drops_when_queue_is_a_symlink() {
        // S-M1: a hostile `telemetry/queue.jsonl` that is a SYMLINK must be
        // refused — the append drops (returns Ok, writes nothing) and the link's
        // target is left untouched, so the install UUID can't be redirected
        // out of tree.
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        std::fs::create_dir_all(paths.telemetry_dir()).unwrap();

        // The link target lives OUTSIDE the telemetry dir; if the guard failed,
        // the append would write through the link and clobber it.
        let outside = dir.path().join("outside-target.txt");
        std::fs::write(&outside, b"untouched\n").unwrap();
        std::os::unix::fs::symlink(&outside, paths.telemetry_queue()).unwrap();

        // Append returns Ok (best-effort drop, never an error/block).
        append(&paths, "{\"redirect\":true}").unwrap();

        // The link target is byte-for-byte untouched — nothing was written through.
        assert_eq!(
            std::fs::read_to_string(&outside).unwrap(),
            "untouched\n",
            "a symlinked queue must NOT be followed (the target stays untouched)"
        );
        // The symlinked queue path was not turned into a real file with content.
        assert_eq!(
            std::fs::read_to_string(paths.telemetry_queue()).unwrap(),
            "untouched\n",
            "reading through the still-present symlink shows the untouched target"
        );
    }

    #[test]
    fn classify_lines_counts_corrupt() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        std::fs::create_dir_all(paths.telemetry_dir()).unwrap();
        // Two valid JSON lines and one deliberately-corrupt fragment.
        std::fs::write(
            paths.telemetry_queue(),
            "{\"a\":1}\nnot json at all\n{\"b\":2}\n",
        )
        .unwrap();
        let (values, corrupt) = classify_lines(&paths).unwrap();
        assert_eq!(values.len(), 2, "two parsable lines");
        assert_eq!(corrupt, 1, "one corrupt line");
    }

    #[test]
    fn classify_empty_queue_is_empty() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let (values, corrupt) = classify_lines(&paths).unwrap();
        assert!(values.is_empty());
        assert_eq!(corrupt, 0);
    }
}
