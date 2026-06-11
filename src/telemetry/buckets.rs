//! Bucket enums for the anonymous telemetry stream (FR-034).
//!
//! WHY buckets at all: the anonymous stream must never let a precise count or
//! latency off the box — a raw `skills: 137` or `latency_ms: 412` is a
//! fingerprint. Every quantity is collapsed to a coarse, closed enum *before* it
//! reaches an event field, so the type system makes "a raw number leaked" an
//! unrepresentable state. The wire tokens are pinned with explicit
//! `#[serde(rename = ...)]` (not a `rename_all` scheme) because the collector
//! contract and the byte-stable `TELEMETRY.md` pin depend on these exact strings.

use serde::Serialize;
use std::time::Duration;

/// Coarse count bucket for corpus/workspace/catalog/entry cardinalities.
///
/// Half-open boundaries: `0`, `1..=4`, `5..=19`, `20..=99`, `100+`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CountBucket {
    #[serde(rename = "0")]
    Zero,
    #[serde(rename = "1-4")]
    OneToFour,
    #[serde(rename = "5-19")]
    FiveToNineteen,
    #[serde(rename = "20-99")]
    TwentyToNinetyNine,
    #[serde(rename = "100+")]
    HundredPlus,
}

impl From<u64> for CountBucket {
    fn from(n: u64) -> Self {
        match n {
            0 => CountBucket::Zero,
            1..=4 => CountBucket::OneToFour,
            5..=19 => CountBucket::FiveToNineteen,
            20..=99 => CountBucket::TwentyToNinetyNine,
            _ => CountBucket::HundredPlus,
        }
    }
}

impl From<usize> for CountBucket {
    fn from(n: usize) -> Self {
        // Route through the `u64` impl so the single set of boundaries is the
        // one source of truth (avoids the two impls drifting apart).
        CountBucket::from(n as u64)
    }
}

/// Coarse latency bucket (search round-trip), millisecond boundaries.
///
/// Half-open: `<50`, `[50,200)`, `[200,500)`, `[500,1000)`, `>=1000`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LatencyBucket {
    #[serde(rename = "<50ms")]
    Under50,
    #[serde(rename = "50-200ms")]
    From50To200,
    #[serde(rename = "200-500ms")]
    From200To500,
    #[serde(rename = "500ms-1s")]
    From500To1000,
    #[serde(rename = "1s+")]
    Over1s,
}

impl From<Duration> for LatencyBucket {
    fn from(d: Duration) -> Self {
        match d.as_millis() {
            // `as_millis()` is `u128`; the open-ended arms keep us safe past
            // `u64::MAX` without a cast that could wrap.
            ms if ms < 50 => LatencyBucket::Under50,
            ms if ms < 200 => LatencyBucket::From50To200,
            ms if ms < 500 => LatencyBucket::From200To500,
            ms if ms < 1000 => LatencyBucket::From500To1000,
            _ => LatencyBucket::Over1s,
        }
    }
}

/// Result-rank bucket for "which position did the selected entry sit at".
///
/// `None` doubles as "no preceding search this session" (the selection had no
/// rank to report), which is why it is a first-class variant, not an `Option`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RankBucket {
    #[serde(rename = "1")]
    One,
    #[serde(rename = "2")]
    Two,
    #[serde(rename = "3")]
    Three,
    #[serde(rename = "4")]
    Four,
    #[serde(rename = "5")]
    Five,
    #[serde(rename = "6-10")]
    SixToTen,
    #[serde(rename = "11+")]
    ElevenPlus,
    #[serde(rename = "none")]
    None,
}

impl RankBucket {
    /// Map a 1-indexed result rank into its bucket.
    ///
    /// `0` is defensive — a 1-indexed rank should never be `0`, so we treat it
    /// as "no rank" (`None`) rather than inventing a position.
    pub fn from_rank(rank: u32) -> RankBucket {
        match rank {
            0 => RankBucket::None,
            1 => RankBucket::One,
            2 => RankBucket::Two,
            3 => RankBucket::Three,
            4 => RankBucket::Four,
            5 => RankBucket::Five,
            6..=10 => RankBucket::SixToTen,
            _ => RankBucket::ElevenPlus,
        }
    }
}

/// Coarse one-shot load-duration bucket (embedder load / index ready),
/// millisecond boundaries.
///
/// Half-open: `<100`, `[100,300)`, `[300,1000)`, `>=1000`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LoadBucket {
    #[serde(rename = "<100ms")]
    Under100,
    #[serde(rename = "100-300ms")]
    From100To300,
    #[serde(rename = "300-1000ms")]
    From300To1000,
    #[serde(rename = "1s+")]
    Over1s,
}

impl From<Duration> for LoadBucket {
    fn from(d: Duration) -> Self {
        match d.as_millis() {
            ms if ms < 100 => LoadBucket::Under100,
            ms if ms < 300 => LoadBucket::From100To300,
            ms if ms < 1000 => LoadBucket::From300To1000,
            _ => LoadBucket::Over1s,
        }
    }
}

/// Coarse doctor-findings bucket. Half-open: `0`, `1..=4`, `5+`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FindingsBucket {
    #[serde(rename = "0")]
    Zero,
    #[serde(rename = "1-4")]
    OneToFour,
    #[serde(rename = "5+")]
    FivePlus,
}

impl From<u64> for FindingsBucket {
    fn from(n: u64) -> Self {
        match n {
            0 => FindingsBucket::Zero,
            1..=4 => FindingsBucket::OneToFour,
            _ => FindingsBucket::FivePlus,
        }
    }
}

impl From<usize> for FindingsBucket {
    fn from(n: usize) -> Self {
        FindingsBucket::from(n as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token<T: Serialize>(v: &T) -> String {
        // Serialized form is a JSON string `"..."`; strip the quotes so the
        // assertions read against the bare wire token.
        let s = serde_json::to_string(v).unwrap();
        s.trim_matches('"').to_string()
    }

    #[test]
    fn count_bucket_tokens_pinned() {
        assert_eq!(token(&CountBucket::Zero), "0");
        assert_eq!(token(&CountBucket::OneToFour), "1-4");
        assert_eq!(token(&CountBucket::FiveToNineteen), "5-19");
        assert_eq!(token(&CountBucket::TwentyToNinetyNine), "20-99");
        assert_eq!(token(&CountBucket::HundredPlus), "100+");
    }

    #[test]
    fn count_bucket_boundaries() {
        assert_eq!(CountBucket::from(0u64), CountBucket::Zero);
        assert_eq!(CountBucket::from(1u64), CountBucket::OneToFour);
        assert_eq!(CountBucket::from(4u64), CountBucket::OneToFour);
        assert_eq!(CountBucket::from(5u64), CountBucket::FiveToNineteen);
        assert_eq!(CountBucket::from(19u64), CountBucket::FiveToNineteen);
        assert_eq!(CountBucket::from(20u64), CountBucket::TwentyToNinetyNine);
        assert_eq!(CountBucket::from(99u64), CountBucket::TwentyToNinetyNine);
        assert_eq!(CountBucket::from(100u64), CountBucket::HundredPlus);
        // usize impl agrees with u64 impl.
        assert_eq!(CountBucket::from(100usize), CountBucket::HundredPlus);
    }

    #[test]
    fn latency_bucket_tokens_pinned() {
        assert_eq!(token(&LatencyBucket::Under50), "<50ms");
        assert_eq!(token(&LatencyBucket::From50To200), "50-200ms");
        assert_eq!(token(&LatencyBucket::From200To500), "200-500ms");
        assert_eq!(token(&LatencyBucket::From500To1000), "500ms-1s");
        assert_eq!(token(&LatencyBucket::Over1s), "1s+");
    }

    #[test]
    fn latency_bucket_boundaries() {
        assert_eq!(
            LatencyBucket::from(Duration::from_millis(49)),
            LatencyBucket::Under50
        );
        assert_eq!(
            LatencyBucket::from(Duration::from_millis(50)),
            LatencyBucket::From50To200
        );
        assert_eq!(
            LatencyBucket::from(Duration::from_millis(199)),
            LatencyBucket::From50To200
        );
        assert_eq!(
            LatencyBucket::from(Duration::from_millis(200)),
            LatencyBucket::From200To500
        );
        assert_eq!(
            LatencyBucket::from(Duration::from_millis(499)),
            LatencyBucket::From200To500
        );
        assert_eq!(
            LatencyBucket::from(Duration::from_millis(500)),
            LatencyBucket::From500To1000
        );
        assert_eq!(
            LatencyBucket::from(Duration::from_millis(999)),
            LatencyBucket::From500To1000
        );
        assert_eq!(
            LatencyBucket::from(Duration::from_millis(1000)),
            LatencyBucket::Over1s
        );
    }

    #[test]
    fn rank_bucket_tokens_pinned() {
        assert_eq!(token(&RankBucket::One), "1");
        assert_eq!(token(&RankBucket::Two), "2");
        assert_eq!(token(&RankBucket::Three), "3");
        assert_eq!(token(&RankBucket::Four), "4");
        assert_eq!(token(&RankBucket::Five), "5");
        assert_eq!(token(&RankBucket::SixToTen), "6-10");
        assert_eq!(token(&RankBucket::ElevenPlus), "11+");
        assert_eq!(token(&RankBucket::None), "none");
    }

    #[test]
    fn rank_bucket_from_rank() {
        assert_eq!(RankBucket::from_rank(0), RankBucket::None);
        assert_eq!(RankBucket::from_rank(1), RankBucket::One);
        assert_eq!(RankBucket::from_rank(5), RankBucket::Five);
        assert_eq!(RankBucket::from_rank(6), RankBucket::SixToTen);
        assert_eq!(RankBucket::from_rank(10), RankBucket::SixToTen);
        assert_eq!(RankBucket::from_rank(11), RankBucket::ElevenPlus);
        assert_eq!(RankBucket::from_rank(9999), RankBucket::ElevenPlus);
    }

    #[test]
    fn load_bucket_tokens_pinned() {
        assert_eq!(token(&LoadBucket::Under100), "<100ms");
        assert_eq!(token(&LoadBucket::From100To300), "100-300ms");
        assert_eq!(token(&LoadBucket::From300To1000), "300-1000ms");
        assert_eq!(token(&LoadBucket::Over1s), "1s+");
    }

    #[test]
    fn load_bucket_boundaries() {
        assert_eq!(
            LoadBucket::from(Duration::from_millis(99)),
            LoadBucket::Under100
        );
        assert_eq!(
            LoadBucket::from(Duration::from_millis(100)),
            LoadBucket::From100To300
        );
        assert_eq!(
            LoadBucket::from(Duration::from_millis(299)),
            LoadBucket::From100To300
        );
        assert_eq!(
            LoadBucket::from(Duration::from_millis(300)),
            LoadBucket::From300To1000
        );
        assert_eq!(
            LoadBucket::from(Duration::from_millis(999)),
            LoadBucket::From300To1000
        );
        assert_eq!(
            LoadBucket::from(Duration::from_millis(1000)),
            LoadBucket::Over1s
        );
    }

    #[test]
    fn findings_bucket_tokens_pinned() {
        assert_eq!(token(&FindingsBucket::Zero), "0");
        assert_eq!(token(&FindingsBucket::OneToFour), "1-4");
        assert_eq!(token(&FindingsBucket::FivePlus), "5+");
    }

    #[test]
    fn findings_bucket_boundaries() {
        assert_eq!(FindingsBucket::from(0u64), FindingsBucket::Zero);
        assert_eq!(FindingsBucket::from(1u64), FindingsBucket::OneToFour);
        assert_eq!(FindingsBucket::from(4u64), FindingsBucket::OneToFour);
        assert_eq!(FindingsBucket::from(5u64), FindingsBucket::FivePlus);
        assert_eq!(FindingsBucket::from(5usize), FindingsBucket::FivePlus);
    }
}
