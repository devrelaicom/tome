//! Injectable wall-clock seam + the time-based gates telemetry needs
//! (the post-mint grace period and the once-per-UTC-day heartbeat gate).
//!
//! WHY a seam: both gates are clock-sensitive (a backward system clock, a
//! day boundary), and the spec requires deterministic tests for the
//! fail-safe-on-skew behaviour (research §R-7, FR-039/FR-040). Rather than
//! thread an `OffsetDateTime` through every call site, production code reads
//! [`now_utc`] and tests install a fixed instant via [`ClockGuard`] — the same
//! `#[doc(hidden)] pub static` + RAII-guard override idiom used by
//! `SUMMARISER_OVERRIDE` / `MIGRATIONS_OVERRIDE`.

use time::OffsetDateTime;

/// The 10-minute grace window after an install id is minted, during which the
/// flusher holds back delivery (research §R-7 / FR-040). Gives a fresh install
/// a chance to be deleted / opted-out before its first event leaves the box.
const GRACE: time::Duration = time::Duration::minutes(10);

thread_local! {
    /// Test-only fixed-clock override. When `Some`, [`now_utc`] returns it
    /// verbatim; production never sets it. Thread-local (not a global Mutex)
    /// because each test runs on its own thread under cargo's default model,
    /// so per-thread isolation is exactly the scope a [`ClockGuard`] wants.
    #[doc(hidden)]
    pub static CLOCK_OVERRIDE: std::cell::RefCell<Option<OffsetDateTime>> =
        const { std::cell::RefCell::new(None) };
}

/// The current UTC wall-clock instant — the test override if installed, else
/// the real `OffsetDateTime::now_utc()`.
pub fn now_utc() -> OffsetDateTime {
    CLOCK_OVERRIDE.with(|slot| slot.borrow().unwrap_or_else(OffsetDateTime::now_utc))
}

/// RAII guard installing a fixed [`now_utc`] for its lifetime; clears the slot
/// on drop (including on test panic). Doc-hidden — tests only.
#[doc(hidden)]
pub struct ClockGuard;

impl ClockGuard {
    /// Install `now` as the fixed clock for the current thread.
    pub fn install(now: OffsetDateTime) -> Self {
        CLOCK_OVERRIDE.with(|slot| {
            *slot.borrow_mut() = Some(now);
        });
        ClockGuard
    }
}

impl Drop for ClockGuard {
    fn drop(&mut self) {
        CLOCK_OVERRIDE.with(|slot| {
            *slot.borrow_mut() = None;
        });
    }
}

/// Whether the post-mint grace period is still active (delivery held back).
///
/// Returns `true` when `now < mint_time + 10min`. A *backward* clock
/// (`now < mint_time`) is treated as FAIL-SAFE — grace stays active so we never
/// send early; the first `now < mint_time` term covers that explicitly even
/// though the second term already implies it, to make the intent legible
/// (research §R-7, FR-040).
pub fn grace_period_active(mint_time: OffsetDateTime, now: OffsetDateTime) -> bool {
    now < mint_time || now < mint_time + GRACE
}

/// The UTC calendar date of `now` as `YYYY-MM-DD`.
///
/// Built from the date components (not a format-description string) to avoid a
/// formatting feature dependency for what is a fixed, trivial shape. The
/// heartbeat gate compares calendar dates rather than elapsed seconds so it is
/// robust to minor clock skew (research §R-7 / FR-039).
pub fn today_utc_date(now: OffsetDateTime) -> String {
    let date = now.date();
    // `month()` is a `time::Month`; `as u8` gives the 1-based ordinal.
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        date.month() as u8,
        date.day()
    )
}

/// Whether a heartbeat is due today: `true` unless we already recorded one for
/// `today`. Calendar-date equality (not a duration) so a backward/forward clock
/// nudge inside the same day doesn't double-fire (research §R-7 / FR-039).
pub fn heartbeat_due(last_recorded: Option<&str>, today: &str) -> bool {
    last_recorded != Some(today)
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::{Date, Month, Time};

    /// Build a fixed UTC `OffsetDateTime` without the `time/macros` feature
    /// (we deliberately don't add that feature for tests-only convenience).
    fn at(y: i32, m: Month, d: u8, hh: u8, mm: u8, ss: u8) -> OffsetDateTime {
        let date = Date::from_calendar_date(y, m, d).unwrap();
        let tod = Time::from_hms(hh, mm, ss).unwrap();
        date.with_time(tod).assume_utc()
    }

    #[test]
    fn now_utc_honours_the_override() {
        let fixed = at(2026, Month::June, 11, 14, 11, 45);
        let _g = ClockGuard::install(fixed);
        assert_eq!(now_utc(), fixed);
    }

    #[test]
    fn override_clears_on_drop() {
        let fixed = at(2000, Month::January, 1, 0, 0, 0);
        {
            let _g = ClockGuard::install(fixed);
            assert_eq!(now_utc(), fixed);
        }
        // After the guard drops, we get a real (much later) clock.
        assert!(now_utc() > fixed);
    }

    #[test]
    fn grace_active_inside_ten_minutes() {
        let mint = at(2026, Month::June, 11, 14, 0, 0);
        let now = at(2026, Month::June, 11, 14, 9, 59);
        assert!(grace_period_active(mint, now));
    }

    #[test]
    fn grace_inactive_after_ten_minutes() {
        let mint = at(2026, Month::June, 11, 14, 0, 0);
        let now = at(2026, Month::June, 11, 14, 10, 1);
        assert!(!grace_period_active(mint, now));
    }

    #[test]
    fn grace_active_on_backward_clock() {
        let mint = at(2026, Month::June, 11, 14, 0, 0);
        // Clock jumped backwards: never send early.
        let now = at(2026, Month::June, 11, 13, 0, 0);
        assert!(grace_period_active(mint, now));
    }

    #[test]
    fn today_utc_date_formats_yyyy_mm_dd() {
        let now = at(2026, Month::June, 11, 14, 11, 45);
        assert_eq!(today_utc_date(now), "2026-06-11");
        // Zero-padding for single-digit month/day.
        let early = at(2026, Month::January, 5, 0, 0, 0);
        assert_eq!(today_utc_date(early), "2026-01-05");
    }

    #[test]
    fn heartbeat_due_when_last_differs() {
        assert!(heartbeat_due(None, "2026-06-11"));
        assert!(heartbeat_due(Some("2026-06-10"), "2026-06-11"));
    }

    #[test]
    fn heartbeat_not_due_when_last_equals_today() {
        assert!(!heartbeat_due(Some("2026-06-11"), "2026-06-11"));
    }
}
