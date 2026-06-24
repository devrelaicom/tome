//! The first-run remote-provider notice (Phase 12 / FR-023).
//!
//! The FIRST time a given remote provider is used, Tome prints a one-line
//! stderr notice that text (skill content / queries / descriptions) is sent
//! off-box to the configured provider. "Seen" is persisted in a `~/.tome/`
//! sidecar file (`<root>/provider_notice_seen`), one provider registry name per
//! line — NOT in `config.toml`. This models the telemetry first-run-notice
//! precedent ([`crate::telemetry::notice`]): the "seen" set is local
//! bookkeeping, not user-edited config.
//!
//! Best-effort: any IO error (unreadable/unwritable sidecar) is swallowed and
//! the notice is simply skipped — a notice failure must never break the
//! foreground command, and an over-suppressed or duplicated notice is harmless.
//!
//! CLI/stderr only: the notice goes to stderr so it never pollutes `--json`
//! stdout, and (like the telemetry notice) a server surface has no human to
//! read it. The MCP server constructs its summariser from the SAME config but
//! never reaches a foreground-visible stderr; printing here is harmless (it
//! lands in the MCP server's own stderr/log).

use std::io::Write;

use crate::paths::Paths;

/// Notify (once per provider name) that a remote provider is in use.
///
/// On the FIRST use of `provider_name` this run-or-ever: append the name to the
/// sidecar "seen" file and print the stderr notice. On a subsequent use (name
/// already recorded) it does nothing.
///
/// Best-effort and infallible: a sidecar read/write failure degrades to "skip
/// the notice" — never propagates.
pub fn notify_remote_use(paths: &Paths, provider_name: &str) {
    if has_seen(paths, provider_name) {
        return;
    }
    // Record BEFORE printing so a concurrent second caller in the same run is
    // less likely to double-print; if the record fails we still print once
    // (better to inform than to suppress on a write error).
    let recorded = record_seen(paths, provider_name);
    print_notice(provider_name);
    if !recorded {
        tracing::debug!(
            provider = provider_name,
            "provider first-run notice printed but the seen-marker write failed; \
             it may print again next run"
        );
    }
}

/// Whether `provider_name` is already recorded in the sidecar "seen" file. A
/// missing/unreadable file ⇒ "not seen" (we'll attempt to print + record).
fn has_seen(paths: &Paths, provider_name: &str) -> bool {
    let path = paths.provider_notice_seen();
    match std::fs::read_to_string(&path) {
        Ok(contents) => contents.lines().any(|line| line.trim() == provider_name),
        Err(_) => false,
    }
}

/// Append `provider_name` as a new line to the sidecar "seen" file, creating it
/// (and `~/.tome/`) if needed. Returns whether the record succeeded.
fn record_seen(paths: &Paths, provider_name: &str) -> bool {
    let path = paths.provider_notice_seen();
    if let Some(parent) = path.parent()
        && !parent.exists()
        && std::fs::create_dir_all(parent).is_err()
    {
        return false;
    }
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(mut f) => writeln!(f, "{provider_name}").is_ok(),
        Err(_) => false,
    }
}

/// Print the one-line stderr notice. Plain text (no color); best-effort.
fn print_notice(provider_name: &str) {
    eprintln!(
        "Tome is now using the remote provider `{provider_name}` for this capability. \
         Text (skill content, queries, and descriptions) will be sent off-box to that \
         provider. Configure or remove it in ~/.tome/config.toml."
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn paths_in(dir: &TempDir) -> Paths {
        Paths::from_root(dir.path().to_path_buf())
    }

    #[test]
    fn first_use_records_the_provider_name() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        assert!(!has_seen(&paths, "myprov"));

        notify_remote_use(&paths, "myprov");
        assert!(
            has_seen(&paths, "myprov"),
            "first use must record the provider as seen"
        );
        // The sidecar holds exactly the one name.
        let contents = std::fs::read_to_string(paths.provider_notice_seen()).unwrap();
        let lines: Vec<&str> = contents.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines, vec!["myprov"]);
    }

    #[test]
    fn second_use_does_not_double_record() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);

        notify_remote_use(&paths, "myprov");
        notify_remote_use(&paths, "myprov");

        let contents = std::fs::read_to_string(paths.provider_notice_seen()).unwrap();
        let count = contents.lines().filter(|l| l.trim() == "myprov").count();
        assert_eq!(count, 1, "a second use must not append a duplicate line");
    }

    #[test]
    fn distinct_providers_each_recorded_once() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);

        notify_remote_use(&paths, "alpha");
        notify_remote_use(&paths, "beta");
        notify_remote_use(&paths, "alpha");

        assert!(has_seen(&paths, "alpha"));
        assert!(has_seen(&paths, "beta"));
        let contents = std::fs::read_to_string(paths.provider_notice_seen()).unwrap();
        assert_eq!(contents.lines().filter(|l| l.trim() == "alpha").count(), 1);
        assert_eq!(contents.lines().filter(|l| l.trim() == "beta").count(), 1);
    }
}
