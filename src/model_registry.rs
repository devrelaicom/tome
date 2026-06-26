//! Harness-target model-ID registry (Phase 1 of native-agent expansion).
//!
//! A trimmed derivative of `models.dev/api.json` is vendored in git and
//! embedded at build time (see `build.rs`). At runtime, an override at
//! `~/.tome/cache/model-registry.json` takes precedence over the baked
//! snapshot. `resolve_tier` turns a tier alias (`opus`/`sonnet`/`haiku`)
//! into the newest non-preview same-vendor model id.
//!
//! Sync only; no network here (the CLI/CI fetchers live elsewhere and pass
//! bytes in).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

/// Our Tome-owned trimmed snapshot shape (strict).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrySnapshot {
    pub schema_version: u32,
    pub source: String,
    pub fetched_at: String,
    pub providers: BTreeMap<String, Provider>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub models: Vec<Model>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub release_date: String,
}

/// The schema_version our trimmed shape currently emits.
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// Minimum total model count a fetched registry must carry to be accepted
/// (guards against a truncated/degenerate fetch).
pub const MIN_MODEL_COUNT: usize = 50;

/// Vendors that MUST be present for a fetched registry to validate.
pub const REQUIRED_VENDORS: &[&str] = &["anthropic"];

/// Tokens that mark a model as non-GA and exclude it from tier resolution.
const NON_GA_TOKENS: &[&str] = &["preview", "beta", "exp"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrySource {
    Baked,
    Override,
}

#[derive(Debug, Clone)]
pub struct ModelRegistry {
    source: RegistrySource,
    snapshot: RegistrySnapshot,
}

#[derive(Debug, Clone)]
pub struct RegistryInfo {
    pub source: RegistrySource,
    pub fetched_at: String,
    pub model_count: usize,
}

fn is_non_ga(model: &Model) -> bool {
    let id = model.id.to_ascii_lowercase();
    let name = model.name.to_ascii_lowercase();
    NON_GA_TOKENS
        .iter()
        .any(|t| id.contains(t) || name.contains(t))
}

impl ModelRegistry {
    /// Resolve a tier alias to the bare model `id` of the newest non-GA
    /// same-vendor model whose id contains the tier token. Ties broken by
    /// `id` descending (deterministic). Returns the bare id; the caller
    /// namespaces (e.g. `anthropic/<id>` for OpenCode).
    pub fn resolve_tier(&self, vendor: &str, tier: &str) -> Option<String> {
        let provider = self.snapshot.providers.get(vendor)?;
        let tier_lc = tier.to_ascii_lowercase();
        provider
            .models
            .iter()
            .filter(|m| m.id.to_ascii_lowercase().contains(&tier_lc) && !is_non_ga(m))
            .max_by(|a, b| {
                a.release_date
                    .cmp(&b.release_date)
                    .then_with(|| a.id.cmp(&b.id))
            })
            .map(|m| m.id.clone())
    }

    pub fn info(&self) -> RegistryInfo {
        let model_count = self
            .snapshot
            .providers
            .values()
            .map(|p| p.models.len())
            .sum();
        RegistryInfo {
            source: self.source,
            fetched_at: self.snapshot.fetched_at.clone(),
            model_count,
        }
    }
}

/// Strict parse of our trimmed snapshot shape.
pub fn parse_snapshot(bytes: &[u8]) -> Result<RegistrySnapshot, String> {
    serde_json::from_slice(bytes).map_err(|e| format!("trimmed registry parse failed: {e}"))
}

// Lenient mirror of models.dev/api.json — third-party input, so only the
// fields we trim to are declared and everything else is ignored.
// not-strict
#[derive(Debug, Deserialize)]
struct RawApi {
    #[serde(flatten)]
    providers: BTreeMap<String, RawProvider>,
}
// not-strict
#[derive(Debug, Deserialize)]
struct RawProvider {
    #[serde(default)]
    models: BTreeMap<String, RawModel>,
}
// not-strict
#[derive(Debug, Deserialize)]
struct RawModel {
    #[serde(default)]
    name: String,
    #[serde(default)]
    release_date: String,
}

/// Lenient parse of `models.dev/api.json` → our trimmed snapshot.
/// `fetched_at` is supplied by the caller (no clock in this module).
pub fn parse_raw_api(bytes: &[u8], fetched_at: &str) -> Result<RegistrySnapshot, String> {
    let raw: RawApi =
        serde_json::from_slice(bytes).map_err(|e| format!("api.json parse failed: {e}"))?;
    let mut providers = BTreeMap::new();
    for (vendor, rp) in raw.providers {
        let mut models: Vec<Model> = rp
            .models
            .into_iter()
            .map(|(id, m)| Model {
                id,
                name: m.name,
                release_date: m.release_date,
            })
            .collect();
        models.sort_by(|a, b| a.id.cmp(&b.id));
        providers.insert(vendor, Provider { models });
    }
    Ok(RegistrySnapshot {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        source: "https://models.dev/api.json".to_owned(),
        fetched_at: fetched_at.to_owned(),
        providers,
    })
}

/// Validate a trimmed snapshot before it is allowed to overwrite a good file.
pub fn validate_snapshot(s: &RegistrySnapshot) -> Result<(), String> {
    if s.schema_version != SNAPSHOT_SCHEMA_VERSION {
        return Err(format!(
            "schema_version {} != expected {SNAPSHOT_SCHEMA_VERSION}",
            s.schema_version
        ));
    }
    for v in REQUIRED_VENDORS {
        let p = s
            .providers
            .get(*v)
            .ok_or_else(|| format!("required vendor `{v}` missing"))?;
        if p.models.is_empty() {
            return Err(format!("vendor `{v}` has no models"));
        }
    }
    let total: usize = s.providers.values().map(|p| p.models.len()).sum();
    if total < MIN_MODEL_COUNT {
        return Err(format!("model count {total} < floor {MIN_MODEL_COUNT}"));
    }
    for p in s.providers.values() {
        for m in &p.models {
            if m.id.trim().is_empty() {
                return Err("model with empty id".to_owned());
            }
            if OffsetDateTime::parse(
                &format!("{}T00:00:00Z", m.release_date),
                &time::format_description::well_known::Rfc3339,
            )
            .is_err()
                && OffsetDateTime::parse(
                    &m.release_date,
                    &time::format_description::well_known::Rfc3339,
                )
                .is_err()
            {
                return Err(format!(
                    "model `{}` has unparsable release_date `{}`",
                    m.id, m.release_date
                ));
            }
        }
    }
    Ok(())
}

/// True when the vendored file is old enough to refresh.
pub fn should_refresh(fetched_at: &str, now: OffsetDateTime, min_age: Duration) -> bool {
    let parsed = OffsetDateTime::parse(fetched_at, &time::format_description::well_known::Rfc3339);
    match parsed {
        Ok(ts) => now - ts >= min_age,
        // Unparsable timestamp → treat as stale (refresh).
        Err(_) => true,
    }
}

/// Construct a fixed, valid registry for tests (used by harness translation
/// tests so byte-stable output is independent of the vendored asset / CI).
#[doc(hidden)]
pub fn test_registry() -> ModelRegistry {
    let json = br#"{
        "schema_version": 1,
        "source": "test",
        "fetched_at": "2026-06-20T00:00:00Z",
        "providers": {
            "anthropic": { "models": [
                { "id": "claude-opus-4-5",   "name": "Claude Opus 4.5",   "release_date": "2026-03-01" },
                { "id": "claude-sonnet-4-5", "name": "Claude Sonnet 4.5", "release_date": "2026-02-01" },
                { "id": "claude-haiku-4-5",  "name": "Claude Haiku 4.5",  "release_date": "2026-01-01" }
            ] }
        }
    }"#;
    ModelRegistry {
        source: RegistrySource::Baked,
        snapshot: parse_snapshot(json).expect("test registry parses"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(models: Vec<(&str, &str, &str)>) -> RegistrySnapshot {
        let mut providers = BTreeMap::new();
        providers.insert(
            "anthropic".to_owned(),
            Provider {
                models: models
                    .into_iter()
                    .map(|(id, name, rd)| Model {
                        id: id.to_owned(),
                        name: name.to_owned(),
                        release_date: rd.to_owned(),
                    })
                    .collect(),
            },
        );
        RegistrySnapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            source: "test".to_owned(),
            fetched_at: "2026-06-20T00:00:00Z".to_owned(),
            providers,
        }
    }

    #[test]
    fn resolve_tier_picks_newest_non_preview_match() {
        let reg = ModelRegistry {
            source: RegistrySource::Baked,
            snapshot: snap(vec![
                ("claude-opus-4-1", "Claude Opus 4.1", "2025-08-05"),
                ("claude-opus-4-5", "Claude Opus 4.5", "2026-03-01"),
                (
                    "claude-opus-5-preview",
                    "Claude Opus 5 Preview",
                    "2026-06-01",
                ),
                ("claude-sonnet-4", "Claude Sonnet 4", "2025-05-01"),
            ]),
        };
        // newest opus that is not preview
        assert_eq!(
            reg.resolve_tier("anthropic", "opus").as_deref(),
            Some("claude-opus-4-5")
        );
        assert_eq!(
            reg.resolve_tier("anthropic", "sonnet").as_deref(),
            Some("claude-sonnet-4")
        );
        // no haiku in the set
        assert_eq!(reg.resolve_tier("anthropic", "haiku"), None);
        // unknown vendor
        assert_eq!(reg.resolve_tier("openai", "opus"), None);
    }

    #[test]
    fn resolve_tier_breaks_same_date_ties_by_id_descending() {
        // Two non-preview opus models sharing the SAME release_date: the
        // lexicographically-higher id wins (id-descending tie-break), so the
        // result is deterministic regardless of insertion order.
        let reg = ModelRegistry {
            source: RegistrySource::Baked,
            snapshot: snap(vec![
                ("claude-opus-4-1", "Claude Opus 4.1", "2026-03-01"),
                ("claude-opus-4-5", "Claude Opus 4.5", "2026-03-01"),
            ]),
        };
        assert_eq!(
            reg.resolve_tier("anthropic", "opus").as_deref(),
            Some("claude-opus-4-5")
        );
    }

    #[test]
    fn validate_rejects_low_model_count() {
        let ok = parse_raw_api(test_api_bytes(), "2026-06-20T00:00:00Z").unwrap();
        assert!(validate_snapshot(&ok).is_ok());

        let mut too_few = ok.clone();
        too_few
            .providers
            .get_mut("anthropic")
            .unwrap()
            .models
            .truncate(1);
        too_few.providers.remove("openai");
        too_few.providers.remove("google");
        assert!(validate_snapshot(&too_few).is_err());
    }

    #[test]
    fn validate_rejects_missing_required_vendor() {
        let ok = parse_raw_api(test_api_bytes(), "2026-06-20T00:00:00Z").unwrap();

        let mut no_anthropic = ok.clone();
        no_anthropic.providers.remove("anthropic");
        assert!(validate_snapshot(&no_anthropic).is_err());
    }

    #[test]
    fn validate_rejects_unparsable_release_date() {
        let mut bad = parse_raw_api(test_api_bytes(), "2026-06-20T00:00:00Z").unwrap();
        // Corrupt one model's release_date; the rest of the snapshot stays
        // valid (count + required vendor) so this isolates the date check.
        bad.providers
            .get_mut("anthropic")
            .unwrap()
            .models
            .first_mut()
            .unwrap()
            .release_date = "not-a-date".to_owned();
        assert!(validate_snapshot(&bad).is_err());
    }

    #[test]
    fn should_refresh_respects_min_age() {
        let now = OffsetDateTime::parse(
            "2026-06-26T00:00:00Z",
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap();
        let min = Duration::days(7);
        assert!(should_refresh("2026-06-01T00:00:00Z", now, min)); // 25d old
        assert!(!should_refresh("2026-06-24T00:00:00Z", now, min)); // 2d old
        assert!(should_refresh("garbage", now, min)); // unparsable → refresh
    }

    // 60 synthetic models across 3 vendors so MIN_MODEL_COUNT passes.
    fn test_api_bytes() -> &'static [u8] {
        // A static fixture file keeps the test readable; see fixtures note.
        include_bytes!("../tests/fixtures/model_registry/api_min.json")
    }
}
