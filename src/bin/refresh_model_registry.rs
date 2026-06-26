//! CI-only: refresh the vendored model registry.
//!
//! Fail-open — prints one of `refreshed` / `skipped` / `failed:<reason>` and
//! exits 0 always, so it can never block a release or a scheduled run.  The
//! calling workflow branches on the printed status to open a PR or file an
//! issue.
//!
//! # Placement note
//!
//! The plan originally called for wiring this step inside `release.yml`, but
//! that workflow is tag-triggered (`on: push: tags`).  A tag-triggered
//! checkout produces a detached HEAD, so a `git commit` / `git push` inside it
//! would need force-pushing a tag or a separate branch dance — fragile and
//! confusing.  Instead this bin is invoked by the standalone
//! `.github/workflows/model-registry-refresh.yml` (weekly schedule +
//! `workflow_dispatch`), which checks out a real branch and can open a PR for
//! human review before the change lands on `main`.

use std::path::Path;

use time::Duration;
use time::OffsetDateTime;

fn main() {
    let asset = Path::new("assets/model-registry/registry.json");
    println!("{}", run(asset).unwrap_or_else(|e| format!("failed:{e}")));
}

/// Scrub a message through the shared credential scrubber before it is printed
/// as `failed:<reason>` — the same SSOT the CLI's `fetch_models_dev` uses.
fn scrub(msg: &str) -> String {
    tome::catalog::git::scrub_to_string(msg.as_bytes())
}

fn run(asset: &Path) -> Result<String, String> {
    // Min-age gate: read the current vendored file and check its `fetched_at`.
    let current_bytes = std::fs::read(asset).map_err(|e| scrub(&format!("read current: {e}")))?;
    let current_snap = tome::model_registry::parse_snapshot(&current_bytes)
        .map_err(|e| scrub(&format!("current parse: {e}")))?;

    let now = OffsetDateTime::now_utc();
    if !tome::model_registry::should_refresh(&current_snap.fetched_at, now, Duration::days(7)) {
        return Ok("skipped".to_owned());
    }

    // Fetch from the upstream API.
    // The reqwest error chain can echo back a URL (and, in principle,
    // redirect/credential material); route it through the same credential
    // scrubber the CLI's `fetch_models_dev` uses so the "every fetch path is
    // scrubbed" invariant holds literally. The other `map_err`s below describe
    // our own JSON content, but are scrubbed too for uniformity.
    let bytes = reqwest::blocking::get("https://models.dev/api.json")
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.bytes())
        .map_err(|e| scrub(&format!("fetch: {e}")))?;

    // Format the timestamp once; `parse_raw_api` has no clock of its own.
    let fetched_at = now
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|e| scrub(&format!("format ts: {e}")))?;

    // Parse → validate BEFORE writing anything (validate-before-overwrite).
    let snapshot = tome::model_registry::parse_raw_api(&bytes, &fetched_at)
        .map_err(|e| scrub(&format!("trim: {e}")))?;
    tome::model_registry::validate_snapshot(&snapshot)
        .map_err(|e| scrub(&format!("validate: {e}")))?;

    // Only now write to disk — atomically, via temp-file-then-rename in the
    // same directory (POSIX-atomic, same-FS), per the constitution's
    // atomic-writes requirement for the registry/cache. A crash mid-write can
    // never leave the vendored asset truncated.
    let json =
        serde_json::to_vec_pretty(&snapshot).map_err(|e| scrub(&format!("serialise: {e}")))?;
    let tmp = asset.with_extension("json.tmp");
    std::fs::write(&tmp, &json).map_err(|e| scrub(&format!("write tmp: {e}")))?;
    std::fs::rename(&tmp, asset).map_err(|e| scrub(&format!("rename: {e}")))?;

    Ok("refreshed".to_owned())
}
