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

/// The exact acceptance test the strict validator applies to a `release_date`:
/// it parses either as a bare `YYYY-MM-DD` (with a synthetic midnight-UTC time)
/// or as a full RFC 3339 timestamp. This is the single source of truth for
/// "is this date acceptable" — both [`validate_snapshot`] and the lenient
/// [`sanitize_snapshot`] pass call it, so the two can never drift.
fn release_date_is_valid(raw: &str) -> bool {
    use time::format_description::well_known::Rfc3339;
    OffsetDateTime::parse(&format!("{raw}T00:00:00Z"), &Rfc3339).is_ok()
        || OffsetDateTime::parse(raw, &Rfc3339).is_ok()
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
            if !release_date_is_valid(&m.release_date) {
                return Err(format!(
                    "model `{}` has unparsable release_date `{}`",
                    m.id, m.release_date
                ));
            }
        }
    }
    Ok(())
}

/// Why a single model entry was dropped during a lenient sanitize pass.
/// The set of distinct variants present is the "error type diversity" the
/// guardrail counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SkipReason {
    /// `release_date` was empty/absent upstream.
    EmptyDate,
    /// `release_date` had the wrong shape entirely (not `YYYY-MM-DD`).
    MalformedDate,
    /// `release_date` was well-shaped (`YYYY-MM-DD`) but not a real calendar
    /// date, e.g. `2026-13-01` or `2025-25-11`.
    OutOfRangeDate,
    /// The model `id` was empty after trimming.
    EmptyId,
}

impl SkipReason {
    /// Short human label for logs / the PR body.
    pub fn describe(self) -> &'static str {
        match self {
            SkipReason::EmptyDate => "empty release_date",
            SkipReason::MalformedDate => "malformed release_date",
            SkipReason::OutOfRangeDate => "out-of-range release_date",
            SkipReason::EmptyId => "empty id",
        }
    }
}

/// A single model entry dropped by the lenient sanitize pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedEntry {
    pub vendor: String,
    pub id: String,
    pub release_date: String,
    pub reason: SkipReason,
}

/// What the lenient sanitize pass dropped (possibly empty).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SanitizeReport {
    pub skipped: Vec<SkippedEntry>,
}

/// Fall back (keep vendored file, raise issue) if total skips reach this,
/// regardless of how they are distributed. Deliberately > the multi-family
/// limit so a single vendor's consistent quirk does not kill the whole refresh.
pub const SKIP_TOTAL_LIMIT: usize = 10; // X
/// Fall back if skips reach this AND span 2 or more families (vendors).
pub const SKIP_MULTI_FAMILY_LIMIT: usize = 3; // Y  (X > Y)
/// Fall back if the skips span this many DISTINCT error types.
pub const SKIP_ERROR_TYPE_LIMIT: usize = 3; // Z

/// Escape an untrusted upstream string (`vendor`/`id`/`release_date`) for one
/// Markdown table cell: a stray `|` splits the cell and a newline splits the
/// row. Cosmetic hardening for the human-reviewed PR body — this report's whole
/// job is to render malformed upstream values, so it should render them cleanly.
fn escape_md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\r', '\n'], " ")
}

impl SanitizeReport {
    pub fn is_empty(&self) -> bool {
        self.skipped.is_empty()
    }

    /// Distinct families (vendors) that had a dropped entry.
    fn families(&self) -> std::collections::BTreeSet<&str> {
        self.skipped.iter().map(|s| s.vendor.as_str()).collect()
    }

    /// Distinct error types present.
    fn reason_kinds(&self) -> std::collections::BTreeSet<SkipReason> {
        self.skipped.iter().map(|s| s.reason).collect()
    }

    /// `Some(reason)` when the skips look systemic and the refresh must fall
    /// back to keeping the vendored file + raising an issue. `None` when the
    /// skips are tolerable and the refresh may proceed.
    pub fn guardrail_breach(&self) -> Option<String> {
        // No skips can never be systemic — and a clean refresh (zero skips) is
        // the common weekly case, so short-circuit before the set allocations.
        if self.skipped.is_empty() {
            return None;
        }
        let total = self.skipped.len();
        let families = self.families();
        let kinds = self.reason_kinds();
        if total >= SKIP_TOTAL_LIMIT {
            return Some(format!(
                "skipped {total} entries (>= total limit {SKIP_TOTAL_LIMIT})"
            ));
        }
        if total >= SKIP_MULTI_FAMILY_LIMIT && families.len() >= 2 {
            let names: Vec<&str> = families.iter().copied().collect();
            return Some(format!(
                "skipped {total} entries across {} families ({}) (>= multi-family limit {SKIP_MULTI_FAMILY_LIMIT})",
                families.len(),
                names.join(", ")
            ));
        }
        if kinds.len() >= SKIP_ERROR_TYPE_LIMIT {
            return Some(format!(
                "skipped entries span {} distinct error types (>= type limit {SKIP_ERROR_TYPE_LIMIT})",
                kinds.len()
            ));
        }
        None
    }

    /// Markdown summary for the PR body (call only when non-empty).
    pub fn to_markdown(&self) -> String {
        let mut out = format!(
            "Skipped {} upstream {} during refresh (within guardrail limits):\n\n",
            self.skipped.len(),
            if self.skipped.len() == 1 {
                "entry"
            } else {
                "entries"
            }
        );
        out.push_str("| Family | Model | release_date | Reason |\n");
        out.push_str("| --- | --- | --- | --- |\n");
        for s in &self.skipped {
            // release_date may be empty; render a placeholder so the cell isn't
            // blank. Every upstream-sourced cell is escaped so a stray `|` or
            // newline in a malformed value can't break the table.
            let rd = if s.release_date.is_empty() {
                "(empty)".to_owned()
            } else {
                escape_md_cell(&s.release_date)
            };
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                escape_md_cell(&s.vendor),
                escape_md_cell(&s.id),
                rd,
                s.reason.describe()
            ));
        }
        out
    }

    /// One line per skipped entry, for stderr / tracing logs.
    pub fn log_lines(&self) -> Vec<String> {
        self.skipped
            .iter()
            .map(|s| {
                format!(
                    "skipped {}/{} (release_date {:?}): {}",
                    s.vendor,
                    s.id,
                    s.release_date,
                    s.reason.describe()
                )
            })
            .collect()
    }
}

/// True when `raw` has the full `YYYY-MM-DD` shape (10 chars, digit groups
/// separated by `-`). Used to distinguish a well-shaped-but-out-of-range date
/// from a wholly-malformed one when classifying a dropped entry.
fn is_full_date_shape(raw: &str) -> bool {
    let b = raw.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[0..4].iter().all(u8::is_ascii_digit)
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[8..10].iter().all(u8::is_ascii_digit)
}

/// Classify a dropped entry. Precondition: the entry IS invalid (empty id, or
/// `!release_date_is_valid`).
fn classify_skip(id: &str, release_date: &str) -> SkipReason {
    if id.trim().is_empty() {
        return SkipReason::EmptyId;
    }
    if release_date.trim().is_empty() {
        return SkipReason::EmptyDate;
    }
    if is_full_date_shape(release_date) {
        SkipReason::OutOfRangeDate
    } else {
        SkipReason::MalformedDate
    }
}

/// Drop per-entry-invalid models (empty id or unparsable release_date) in place,
/// returning what was dropped. Does NOT enforce structural rules (count /
/// required vendors) — that stays in [`validate_snapshot`], applied to survivors.
pub fn sanitize_snapshot(snapshot: &mut RegistrySnapshot) -> SanitizeReport {
    let mut skipped = Vec::new();
    for (vendor, provider) in &mut snapshot.providers {
        provider.models.retain(|m| {
            let invalid = m.id.trim().is_empty() || !release_date_is_valid(&m.release_date);
            if invalid {
                skipped.push(SkippedEntry {
                    vendor: vendor.clone(),
                    id: m.id.clone(),
                    release_date: m.release_date.clone(),
                    reason: classify_skip(&m.id, &m.release_date),
                });
            }
            !invalid
        });
    }
    SanitizeReport { skipped }
}

/// Parse raw `models.dev` bytes, drop per-entry-invalid models (lenient),
/// enforce the systemic-breakage guardrails, then structurally validate the
/// survivors. On success returns the clean snapshot plus the (possibly empty)
/// report of what was dropped. On a tripped guardrail or a structural failure,
/// returns `Err(reason)` and the caller must write NOTHING (keep the existing
/// vendored/override file). The `Err` string is prefixed to preserve the
/// existing `failed:<reason>` categories the CI issue relies on
/// (`trim:` / `validate:`), plus a new `guardrail:` category.
pub fn refresh_from_bytes(
    bytes: &[u8],
    fetched_at: &str,
) -> Result<(RegistrySnapshot, SanitizeReport), String> {
    let mut snapshot = parse_raw_api(bytes, fetched_at).map_err(|e| format!("trim: {e}"))?;
    let report = sanitize_snapshot(&mut snapshot);
    if let Some(reason) = report.guardrail_breach() {
        return Err(format!("guardrail: {reason}"));
    }
    validate_snapshot(&snapshot).map_err(|e| format!("validate: {e}"))?;
    Ok((snapshot, report))
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

    // 2. Parse, lenient-sanitize, guardrail, and structurally validate before
    //    touching any file on disk. A tolerable set of bad upstream entries is
    //    dropped from `snapshot` and recorded in `report`; a systemic breakage
    //    (or a structural failure) errors out and leaves any existing override
    //    untouched.
    let (snapshot, report) = refresh_from_bytes(&bytes, fetched_at).map_err(|msg| {
        crate::error::TomeError::Io(std::io::Error::other(format!(
            "model registry refresh: {msg}"
        )))
    })?;

    // A user running `tome models update --include-registry` should see exactly
    // which entries were dropped.
    for line in report.log_lines() {
        tracing::warn!("model registry refresh: {line}");
    }

    // 3. Serialise the sanitized snapshot.
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
    use std::collections::BTreeSet;

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

    // ---- lenient per-entry sanitize + guardrails (#489) ----

    /// Build a snapshot from an explicit list of `(vendor, id, name, rd)` rows,
    /// grouped into providers by vendor. Bypasses `parse_raw_api`'s sorting +
    /// normalisation so an invalid `release_date` reaches `sanitize_snapshot`
    /// verbatim.
    fn snap_multi(rows: Vec<(&str, &str, &str, &str)>) -> RegistrySnapshot {
        let mut providers: BTreeMap<String, Provider> = BTreeMap::new();
        for (vendor, id, name, rd) in rows {
            providers
                .entry(vendor.to_owned())
                .or_insert_with(|| Provider { models: Vec::new() })
                .models
                .push(Model {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    release_date: rd.to_owned(),
                });
        }
        RegistrySnapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            source: "test".to_owned(),
            fetched_at: "2026-06-20T00:00:00Z".to_owned(),
            providers,
        }
    }

    /// Build a `SanitizeReport` from `(vendor, reason)` pairs for guardrail
    /// tests (ids/dates are irrelevant to the guardrail arithmetic).
    fn report_of(entries: Vec<(&str, SkipReason)>) -> SanitizeReport {
        SanitizeReport {
            skipped: entries
                .into_iter()
                .enumerate()
                .map(|(i, (vendor, reason))| SkippedEntry {
                    vendor: vendor.to_owned(),
                    id: format!("m{i}"),
                    release_date: "2025-25-11".to_owned(),
                    reason,
                })
                .collect(),
        }
    }

    #[test]
    fn classify_skip_covers_each_reason() {
        // Empty date → EmptyDate.
        assert_eq!(classify_skip("qwen3", ""), SkipReason::EmptyDate);
        assert_eq!(classify_skip("qwen3", "   "), SkipReason::EmptyDate);
        // Wrong shape entirely → MalformedDate.
        assert_eq!(classify_skip("qwen3", "garbage"), SkipReason::MalformedDate);
        assert_eq!(classify_skip("qwen3", "2026"), SkipReason::MalformedDate);
        assert_eq!(classify_skip("qwen3", "2026-1"), SkipReason::MalformedDate);
        // Well-shaped YYYY-MM-DD but not a real calendar date → OutOfRangeDate.
        assert_eq!(
            classify_skip("qwen3", "2025-25-11"),
            SkipReason::OutOfRangeDate
        );
        assert_eq!(
            classify_skip("qwen3", "2026-13-01"),
            SkipReason::OutOfRangeDate
        );
        // Empty id wins even with a valid date.
        assert_eq!(classify_skip("", "2026-01-15"), SkipReason::EmptyId);
        assert_eq!(classify_skip("   ", "2026-01-15"), SkipReason::EmptyId);
    }

    #[test]
    fn release_date_is_valid_accepts_real_dates_rejects_bad() {
        assert!(release_date_is_valid("2026-01-15"));
        assert!(release_date_is_valid("2026-01-01")); // normalised month
        assert!(!release_date_is_valid(""));
        assert!(!release_date_is_valid("2025-25-11"));
        assert!(!release_date_is_valid("garbage"));
    }

    #[test]
    fn sanitize_snapshot_drops_only_invalid_and_records_them() {
        let mut s = snap_multi(vec![
            // valid survivors
            (
                "anthropic",
                "claude-opus-4-5",
                "Claude Opus 4.5",
                "2026-03-01",
            ),
            (
                "anthropic",
                "claude-sonnet-4-5",
                "Claude Sonnet 4.5",
                "2026-02-01",
            ),
            ("qwen", "qwen3-32b", "Qwen3 32B", "2025-05-01"),
            // invalid: out-of-range date (month/day swap, #489)
            (
                "qwen",
                "qwen3-embedding-8b",
                "Qwen3 Embedding 8B",
                "2025-25-11",
            ),
            // invalid: empty date
            ("moonshotai", "kimi-empty", "Kimi Empty", ""),
            // invalid: empty id (valid date, still dropped)
            ("moonshotai", "", "No Id", "2026-01-15"),
            // invalid: malformed date
            ("google", "gemini-bad", "Gemini Bad", "not-a-date"),
        ]);

        let report = sanitize_snapshot(&mut s);

        // Survivors are exactly the valid ones.
        let survivors: BTreeSet<(&str, &str)> = s
            .providers
            .iter()
            .flat_map(|(v, p)| p.models.iter().map(move |m| (v.as_str(), m.id.as_str())))
            .collect();
        let expected: BTreeSet<(&str, &str)> = [
            ("anthropic", "claude-opus-4-5"),
            ("anthropic", "claude-sonnet-4-5"),
            ("qwen", "qwen3-32b"),
        ]
        .into_iter()
        .collect();
        assert_eq!(survivors, expected);

        // The report lists the dropped ones with correct vendor/id/reason.
        let dropped: BTreeSet<(&str, &str, SkipReason)> = report
            .skipped
            .iter()
            .map(|e| (e.vendor.as_str(), e.id.as_str(), e.reason))
            .collect();
        let expected_dropped: BTreeSet<(&str, &str, SkipReason)> = [
            ("qwen", "qwen3-embedding-8b", SkipReason::OutOfRangeDate),
            ("moonshotai", "kimi-empty", SkipReason::EmptyDate),
            ("moonshotai", "", SkipReason::EmptyId),
            ("google", "gemini-bad", SkipReason::MalformedDate),
        ]
        .into_iter()
        .collect();
        assert_eq!(dropped, expected_dropped);
        assert!(!report.is_empty());
    }

    #[test]
    fn guardrail_nine_same_family_same_reason_is_tolerated() {
        // The core #489 property: a single vendor's consistent quirk (even a
        // lot of it) must NOT kill the whole refresh.
        let report =
            report_of(std::iter::repeat_n(("qwen", SkipReason::OutOfRangeDate), 9).collect());
        assert_eq!(report.guardrail_breach(), None);
    }

    #[test]
    fn guardrail_ten_same_family_trips_total_limit() {
        let report =
            report_of(std::iter::repeat_n(("qwen", SkipReason::OutOfRangeDate), 10).collect());
        let breach = report.guardrail_breach().expect("10 skips trips the total");
        assert!(breach.contains("total limit"), "got: {breach}");
    }

    #[test]
    fn guardrail_three_across_two_families_trips_multi_family() {
        let report = report_of(vec![
            ("qwen", SkipReason::OutOfRangeDate),
            ("qwen", SkipReason::OutOfRangeDate),
            ("moonshotai", SkipReason::OutOfRangeDate),
        ]);
        let breach = report
            .guardrail_breach()
            .expect("3 across 2 families trips multi-family");
        assert!(breach.contains("multi-family"), "got: {breach}");
    }

    #[test]
    fn guardrail_two_across_two_families_is_tolerated() {
        // Below Y (3) and only 1 distinct reason kind, so neither the
        // multi-family nor the type rule fires.
        let report = report_of(vec![
            ("qwen", SkipReason::OutOfRangeDate),
            ("moonshotai", SkipReason::OutOfRangeDate),
        ]);
        assert_eq!(report.guardrail_breach(), None);
    }

    #[test]
    fn guardrail_three_distinct_reason_kinds_trips_type_limit() {
        // Only 2 entries in one family (below the total and multi-family
        // rules), but 3 distinct reason kinds → type-diversity breach.
        let report = SanitizeReport {
            skipped: vec![
                SkippedEntry {
                    vendor: "qwen".to_owned(),
                    id: "a".to_owned(),
                    release_date: "".to_owned(),
                    reason: SkipReason::EmptyDate,
                },
                SkippedEntry {
                    vendor: "qwen".to_owned(),
                    id: "b".to_owned(),
                    release_date: "garbage".to_owned(),
                    reason: SkipReason::MalformedDate,
                },
                SkippedEntry {
                    vendor: "qwen".to_owned(),
                    id: "c".to_owned(),
                    release_date: "2025-25-11".to_owned(),
                    reason: SkipReason::OutOfRangeDate,
                },
            ],
        };
        // Guard the test's own premise: single family, below the total limit,
        // so only the type rule can be the cause of a breach.
        assert!(report.families().len() < 2 || report.skipped.len() < SKIP_MULTI_FAMILY_LIMIT);
        assert!(report.skipped.len() < SKIP_TOTAL_LIMIT);
        let breach = report
            .guardrail_breach()
            .expect("3 distinct reason kinds trips the type limit");
        assert!(breach.contains("error types"), "got: {breach}");
    }

    /// Load the all-valid fixture into a JSON value and add a `qwen` provider
    /// whose single model has an out-of-range `release_date`, returning the
    /// re-serialized raw-api bytes.
    fn api_bytes_with_extra_qwen(bad_date: &str) -> Vec<u8> {
        let mut v: serde_json::Value =
            serde_json::from_slice(test_api_bytes()).expect("fixture is valid json");
        let obj = v.as_object_mut().expect("api.json is an object");
        obj.insert(
            "qwen".to_owned(),
            serde_json::json!({
                "models": {
                    "qwen3-embedding-8b": {
                        "name": "Qwen3 Embedding 8B",
                        "release_date": bad_date,
                    }
                }
            }),
        );
        serde_json::to_vec(&v).expect("re-serialize")
    }

    #[test]
    fn refresh_from_bytes_tolerates_one_bad_entry() {
        // #489 regression: one out-of-range upstream date no longer fails the
        // whole refresh — the entry is dropped and the survivors validate.
        let bytes = api_bytes_with_extra_qwen("2025-25-11");
        let (snapshot, report) =
            refresh_from_bytes(&bytes, "2026-07-07T00:00:00Z").expect("tolerable skip → Ok");

        // Exactly one skip: the qwen model, out-of-range date.
        assert_eq!(report.skipped.len(), 1);
        let e = &report.skipped[0];
        assert_eq!(e.vendor, "qwen");
        assert_eq!(e.id, "qwen3-embedding-8b");
        assert_eq!(e.reason, SkipReason::OutOfRangeDate);

        // The bad model does not survive; the qwen provider carries none of it.
        let qwen_ids: Vec<&str> = snapshot
            .providers
            .get("qwen")
            .map(|p| p.models.iter().map(|m| m.id.as_str()).collect())
            .unwrap_or_default();
        assert!(!qwen_ids.contains(&"qwen3-embedding-8b"));

        // The surviving snapshot is structurally valid.
        assert!(validate_snapshot(&snapshot).is_ok());
    }

    #[test]
    fn refresh_from_bytes_falls_back_on_systemic_breakage() {
        // Inject 3 bad entries across two extra vendors (qwen + moonshotai) so
        // the multi-family guardrail trips; the refresh must error out.
        let mut v: serde_json::Value =
            serde_json::from_slice(test_api_bytes()).expect("fixture is valid json");
        let obj = v.as_object_mut().unwrap();
        obj.insert(
            "qwen".to_owned(),
            serde_json::json!({
                "models": {
                    "qwen-a": { "name": "A", "release_date": "2025-25-11" },
                    "qwen-b": { "name": "B", "release_date": "2025-25-12" }
                }
            }),
        );
        obj.insert(
            "moonshotai".to_owned(),
            serde_json::json!({
                "models": {
                    "kimi-c": { "name": "C", "release_date": "2026-13-01" }
                }
            }),
        );
        let bytes = serde_json::to_vec(&v).unwrap();

        let err = refresh_from_bytes(&bytes, "2026-07-07T00:00:00Z")
            .expect_err("systemic breakage → Err");
        assert!(err.starts_with("guardrail:"), "got: {err}");
    }

    #[test]
    fn to_markdown_renders_header_row_and_empty_placeholder() {
        let report = SanitizeReport {
            skipped: vec![SkippedEntry {
                vendor: "moonshotai".to_owned(),
                id: "kimi-empty".to_owned(),
                release_date: "".to_owned(),
                reason: SkipReason::EmptyDate,
            }],
        };
        let md = report.to_markdown();
        assert!(md.contains("| Family | Model | release_date | Reason |"));
        assert!(md.contains("| --- | --- | --- | --- |"));
        // Empty release_date renders the placeholder, not a blank cell.
        assert!(
            md.contains("| moonshotai | kimi-empty | (empty) | empty release_date |"),
            "got:\n{md}"
        );
        // Singular "entry" for a 1-skip report.
        assert!(md.contains("Skipped 1 upstream entry"), "got:\n{md}");
    }

    #[test]
    fn to_markdown_escapes_pipe_and_newline_in_cells() {
        // The report exists to render malformed upstream values; a `|` or
        // newline in a bad id/date must not split the table cell/row.
        let report = SanitizeReport {
            skipped: vec![SkippedEntry {
                vendor: "qwen".to_owned(),
                id: "weird|id".to_owned(),
                release_date: "2026\n01".to_owned(),
                reason: SkipReason::MalformedDate,
            }],
        };
        let md = report.to_markdown();
        assert!(
            md.contains(r"weird\|id"),
            "pipe must be escaped, got:\n{md}"
        );
        assert!(
            md.contains("2026 01"),
            "newline flattened to space, got:\n{md}"
        );
        assert!(
            !md.contains("2026\n01"),
            "a raw newline must not survive into a cell, got:\n{md}"
        );
    }

    #[test]
    fn refresh_from_bytes_structurally_rejects_when_sanitize_empties_required_vendor() {
        // Sanitize drops anthropic's only (bad-date) model, emptying the
        // required vendor. That is a single skip — below every guardrail — so
        // the failure must come from the STRUCTURAL validator (`validate:`),
        // NOT the guardrail. This pins the sanitize → guardrail → validate
        // ordering: a degenerate survivor set is still caught downstream.
        let mut v: serde_json::Value =
            serde_json::from_slice(test_api_bytes()).expect("fixture is valid json");
        let obj = v.as_object_mut().unwrap();
        obj.insert(
            "anthropic".to_owned(),
            serde_json::json!({
                "models": {
                    "claude-bad": { "name": "Bad", "release_date": "2025-25-11" }
                }
            }),
        );
        let bytes = serde_json::to_vec(&v).unwrap();
        let err = refresh_from_bytes(&bytes, "2026-07-07T00:00:00Z")
            .expect_err("required vendor emptied → structural Err");
        assert!(err.starts_with("validate:"), "got: {err}");
    }
}
