//! Human-friendly relative-time rendering shared across capability modules.
//!
//! `relative_time` was introduced by `tome status` (the `Reindexed:` panel
//! line). Issue #300 gives `tome workspace list` a relative `Last used`
//! column, so the helper is promoted here as the single source of truth
//! rather than duplicated at the second consumer (the SSOT-at-the-second-
//! consumer pattern). Both `status` and `workspace list` render through this
//! one function, so the bucketing can never drift between the two surfaces.
//!
//! Sync-only, like the rest of `src/util/`.

/// Humanize the gap between `then` and `now` (both unix seconds). Future
/// timestamps (clock skew) clamp to "just now".
///
/// Buckets: `< 60s` → "just now"; `< 1h` → "N minute(s) ago"; `< 1d` →
/// "N hour(s) ago"; otherwise "N day(s) ago". Singular/plural are handled.
pub fn relative_time(then: i64, now: i64) -> String {
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

#[cfg(test)]
mod tests {
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
