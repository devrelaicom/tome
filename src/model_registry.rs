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
use std::io::Write as _;

use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

// Build-generated: `pub static MODEL_REGISTRY_SNAPSHOT: &[u8]`
include!(concat!(env!("OUT_DIR"), "/model_registry_snapshot.rs"));

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

    /// Parse the build-embedded snapshot. `build.rs` gates the asset, so a
    /// parse failure here is a build/packaging bug, not a runtime condition.
    pub fn baked() -> ModelRegistry {
        let snapshot = parse_snapshot(MODEL_REGISTRY_SNAPSHOT)
            .expect("baked model registry snapshot must parse (gated by build.rs)");
        // A parse-ok-but-semantically-invalid baked asset (e.g. too few models,
        // missing required vendor) is also a packaging bug — fail loudly at
        // startup rather than silently serving a degenerate registry.
        validate_snapshot(&snapshot)
            .expect("baked model registry snapshot must be valid (gated by build.rs)");
        ModelRegistry {
            source: RegistrySource::Baked,
            snapshot,
        }
    }

    /// Override-if-valid, else baked. A present-but-corrupt override falls back
    /// to baked (never errors a sync); `doctor` surfaces the corruption via
    /// [`override_health`].
    pub fn load(paths: &crate::paths::Paths) -> ModelRegistry {
        let path = paths.model_registry_cache_path();
        if let Ok(bytes) = std::fs::read(&path)
            && let Ok(snapshot) = parse_snapshot(&bytes)
            && validate_snapshot(&snapshot).is_ok()
        {
            return ModelRegistry {
                source: RegistrySource::Override,
                snapshot,
            };
        }
        ModelRegistry::baked()
    }
}

/// Read-only health of the model-registry override file (for `doctor`).
#[derive(Debug, Clone)]
pub enum OverrideHealth {
    /// No override file at `~/.tome/cache/model-registry.json`.
    Absent,
    /// Override present and passes `validate_snapshot`.
    Valid(RegistryInfo),
    /// Override present but fails to parse or validate.
    Corrupt,
}

/// Read-only health check on the override file (for `doctor`). Never writes.
///
/// A genuinely-missing file is [`OverrideHealth::Absent`]. A file that exists
/// but cannot be read (permissions, an I/O error) is NOT the same as "no
/// override" — it is reported as [`OverrideHealth::Corrupt`] (with a `warn!`),
/// so `doctor` surfaces it instead of treating it as a clean default.
pub fn override_health(paths: &crate::paths::Paths) -> OverrideHealth {
    let path = paths.model_registry_cache_path();
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return OverrideHealth::Absent,
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "model-registry override present but unreadable; treating as corrupt"
            );
            return OverrideHealth::Corrupt;
        }
    };
    match parse_snapshot(&bytes) {
        Ok(s) if validate_snapshot(&s).is_ok() => {
            let reg = ModelRegistry {
                source: RegistrySource::Override,
                snapshot: s,
            };
            OverrideHealth::Valid(reg.info())
        }
        _ => OverrideHealth::Corrupt,
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

/// models.dev sometimes publishes a month-precision `release_date`
/// (`YYYY-MM`, e.g. kimi-k2.5's `2026-01`), which the strict full-date
/// validator would reject and fail the whole refresh (#487/#455). Normalise
/// exactly that shape to the first of the month so every stored date is a
/// full `YYYY-MM-DD`; anything else passes through verbatim for
/// `validate_snapshot` to judge (an out-of-range month like `2026-13`
/// becomes `2026-13-01` and still fails loudly there).
fn normalize_release_date(raw: String) -> String {
    let b = raw.as_bytes();
    if b.len() == 7
        && b[4] == b'-'
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[5..].iter().all(u8::is_ascii_digit)
    {
        return format!("{raw}-01");
    }
    raw
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
                release_date: normalize_release_date(m.release_date),
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

/// The canonical URL for the upstream model registry.
pub(crate) const MODELS_DEV_API_URL: &str = "https://models.dev/api.json";

/// Fetch (via the injected `fetch`), trim, validate, and atomically write the
/// override. Fail-loud: any error returns `TomeError::Io` with a scrubbed
/// message and leaves any existing override untouched.
///
/// The injected `fetch` closure takes the URL string and returns the raw bytes;
/// in production that is a `reqwest::blocking::get` call; in tests it is a
/// closure that returns fixture bytes.
pub fn refresh_override(
    paths: &crate::paths::Paths,
    fetched_at: &str,
    fetch: impl FnOnce(&str) -> Result<Vec<u8>, crate::error::TomeError>,
) -> Result<RegistryInfo, crate::error::TomeError> {
    // 1. Fetch — propagate the fetcher's error directly (already a TomeError::Io).
    let bytes = fetch(MODELS_DEV_API_URL)?;

    // 2. Parse + validate before touching any file on disk.
    let snapshot = parse_raw_api(&bytes, fetched_at)
        .and_then(|s| validate_snapshot(&s).map(|()| s))
        .map_err(|msg| {
            crate::error::TomeError::Io(std::io::Error::other(format!(
                "model registry refresh: {msg}"
            )))
        })?;

    // 3. Serialise the validated snapshot.
    let json = serde_json::to_vec_pretty(&snapshot).map_err(|e| {
        crate::error::TomeError::Io(std::io::Error::other(format!(
            "model registry serialise: {e}"
        )))
    })?;

    // 4. Atomically write via temp-file + rename (same-FS, POSIX-atomic).
    let path = paths.model_registry_cache_path();
    let parent = path.parent().ok_or_else(|| {
        crate::error::TomeError::Io(std::io::Error::other(
            "model_registry_cache_path has no parent directory",
        ))
    })?;
    std::fs::create_dir_all(parent).map_err(crate::error::TomeError::Io)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(crate::error::TomeError::Io)?;
    tmp.write_all(&json).map_err(crate::error::TomeError::Io)?;
    tmp.persist(&path).map_err(|e| {
        crate::error::TomeError::Io(std::io::Error::other(format!(
            "model registry write (persist): {e}"
        )))
    })?;

    Ok(ModelRegistry {
        source: RegistrySource::Override,
        snapshot,
    }
    .info())
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

    /// Return a `RegistrySnapshot` that passes `validate_snapshot` (≥ 50 models)
    /// for use in override-precedence tests. Derived from the same fixture used
    /// by the Task-1 parse/validate tests.
    fn test_registry_snapshot_for_override() -> RegistrySnapshot {
        parse_raw_api(
            include_bytes!("../tests/fixtures/model_registry/api_min.json"),
            "2026-06-20T00:00:00Z",
        )
        .expect("fixture parses")
    }

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
    fn parse_normalizes_month_precision_release_date() {
        // #487/#455: kimi-k2.5 shipped a month-precision `2026-01` upstream
        // and the refresh failed at validation. Parse now lands it as the
        // first of the month; every other shape passes through verbatim.
        let raw = br#"{
            "moonshotai": { "models": {
                "kimi-k2.5":  { "name": "Kimi K2.5",  "release_date": "2026-01" },
                "kimi-full":  { "name": "Kimi Full",  "release_date": "2026-01-15" },
                "kimi-year":  { "name": "Kimi Year",  "release_date": "2026" },
                "kimi-short": { "name": "Kimi Short", "release_date": "2026-1" }
            } }
        }"#;
        let snap = parse_raw_api(raw, "2026-07-07T00:00:00Z").unwrap();
        let dates: BTreeMap<&str, &str> = snap.providers["moonshotai"]
            .models
            .iter()
            .map(|m| (m.id.as_str(), m.release_date.as_str()))
            .collect();
        assert_eq!(dates["kimi-k2.5"], "2026-01-01", "YYYY-MM gains day 01");
        assert_eq!(dates["kimi-full"], "2026-01-15", "full date untouched");
        assert_eq!(dates["kimi-year"], "2026", "bare year passes through");
        assert_eq!(
            dates["kimi-short"], "2026-1",
            "1-digit month passes through"
        );
    }

    #[test]
    fn month_precision_date_validates_but_out_of_range_month_still_fails() {
        let ok = parse_raw_api(test_api_bytes(), "2026-06-20T00:00:00Z").unwrap();

        // A normalized month-precision date is a real date → accepted.
        let mut month_only = ok.clone();
        month_only
            .providers
            .get_mut("anthropic")
            .unwrap()
            .models
            .first_mut()
            .unwrap()
            .release_date = normalize_release_date("2026-01".to_owned());
        assert!(validate_snapshot(&month_only).is_ok());

        // Normalisation is shape-only: `2026-13` → `2026-13-01` is not a
        // real date, so the validator still rejects it loudly.
        let mut bad_month = ok;
        bad_month
            .providers
            .get_mut("anthropic")
            .unwrap()
            .models
            .first_mut()
            .unwrap()
            .release_date = normalize_release_date("2026-13".to_owned());
        assert!(validate_snapshot(&bad_month).is_err());
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

    #[test]
    fn baked_parses_and_reports_baked_source() {
        let reg = ModelRegistry::baked();
        let info = reg.info();
        assert!(matches!(info.source, RegistrySource::Baked));
        assert!(info.model_count >= MIN_MODEL_COUNT);
    }

    #[test]
    fn override_takes_precedence_then_falls_back_when_corrupt() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".tome");
        let paths = crate::paths::Paths::from_root(root);
        let cache = paths.model_registry_cache_path();
        std::fs::create_dir_all(cache.parent().unwrap()).unwrap();

        // Valid override → Override source.
        let valid_snap = test_registry_snapshot_for_override();
        let valid = serde_json::to_vec(&valid_snap).unwrap();
        std::fs::write(&cache, &valid).unwrap();
        assert!(
            matches!(
                ModelRegistry::load(&paths).info().source,
                RegistrySource::Override
            ),
            "expected Override source when valid override is present"
        );
        assert!(matches!(override_health(&paths), OverrideHealth::Valid(_)));

        // Corrupt override → fall back to Baked.
        std::fs::write(&cache, b"{ not json").unwrap();
        assert!(
            matches!(
                ModelRegistry::load(&paths).info().source,
                RegistrySource::Baked
            ),
            "expected Baked source when override is corrupt"
        );
        assert!(matches!(override_health(&paths), OverrideHealth::Corrupt));

        // Absent (remove) → Absent health + Baked source.
        std::fs::remove_file(&cache).unwrap();
        assert!(
            matches!(
                ModelRegistry::load(&paths).info().source,
                RegistrySource::Baked
            ),
            "expected Baked source when override is absent"
        );
        assert!(matches!(override_health(&paths), OverrideHealth::Absent));
    }

    #[test]
    fn refresh_override_writes_on_valid_and_fails_loud_on_invalid() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = crate::paths::Paths::from_root(tmp.path().join(".tome"));

        // Valid fetch → override written, source becomes Override.
        let info = refresh_override(&paths, "2026-06-26T00:00:00Z", |_url| {
            Ok(include_bytes!("../tests/fixtures/model_registry/api_min.json").to_vec())
        })
        .expect("valid fetch writes override");
        assert!(matches!(info.source, RegistrySource::Override));
        assert!(matches!(override_health(&paths), OverrideHealth::Valid(_)));

        // Invalid fetch → Err, override unchanged (still the valid one).
        let err = refresh_override(&paths, "2026-06-26T00:00:00Z", |_url| Ok(b"{bad".to_vec()))
            .unwrap_err();
        assert_eq!(err.exit_code(), 7); // TomeError::Io
        assert!(matches!(override_health(&paths), OverrideHealth::Valid(_)));
    }

    /// An override file that exists but cannot be read must report `Corrupt`,
    /// not `Absent` — an unreadable-but-present file is a real problem `doctor`
    /// should surface. Unix-only (mode bits); self-skips when the file stays
    /// readable anyway (e.g. running as root, or a filesystem that ignores the
    /// mode), so it is never flaky.
    #[cfg(unix)]
    #[test]
    fn override_health_unreadable_present_file_is_corrupt() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".tome");
        let paths = crate::paths::Paths::from_root(root);
        let cache = paths.model_registry_cache_path();
        std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
        std::fs::write(&cache, b"anything").unwrap();
        std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o000)).unwrap();

        // If the platform/user still lets us read it (root, or a mode-ignoring
        // FS), the precondition isn't met — skip rather than assert falsely.
        if std::fs::read(&cache).is_ok() {
            // Restore so the tempdir cleanup can remove the file.
            let _ = std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o644));
            return;
        }

        assert!(
            matches!(override_health(&paths), OverrideHealth::Corrupt),
            "an unreadable present override must report Corrupt, not Absent"
        );

        // Restore permissions so TempDir cleanup succeeds.
        std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
}
