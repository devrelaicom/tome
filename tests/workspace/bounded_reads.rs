//! FR-006 (F-PLUGIN-MANIFEST-DOS): every read of a third-party file is
//! bounded by its per-class cap, so an oversized file yields a bounded,
//! named, per-class error instead of an unbounded allocation / OOM.
//!
//! Contract: `specs/007-phase-7-beta-release/contracts/robustness-trust.md`
//! §FR-006. The per-class cap for plugin manifests, `tome-catalog.toml`,
//! frontmatter, and the plugin-local `.mcp.json` is
//! [`tome::util::PLUGIN_MANIFEST_MAX`] (256 KiB).
//!
//! The discriminating construction at every site is an **oversized but
//! otherwise-valid** file: valid JSON / valid TOML, padded past the cap.
//! Before the fix each site reads the whole file (the DoS) and then parses
//! it successfully — so the unbounded read is silently accepted. After the
//! fix the metadata-length check refuses the file before the read, and the
//! over-cap result maps to the site's existing per-class error.
//!
//! Were the oversized fixtures *invalid* (garbage bytes), these tests would
//! pass today for the wrong reason (a parse failure rather than a bounded
//! read), so the validity of the body is load-bearing.

use std::path::Path;

use crate::common::{Fixture, ToolEnv, cache_dir_for, fabricate_all_registry_models, paths_for};
use tempfile::TempDir;
use tome::doctor::{self, CatalogCacheState};
use tome::error::TomeError;
use tome::util::PLUGIN_MANIFEST_MAX;
use tome::workspace::{ResolvedScope, ScopeSource};

/// A byte count comfortably past the 256 KiB `PLUGIN_MANIFEST_MAX` cap.
/// Large enough to exceed the cap, small enough to keep the suite fast —
/// the point is "past the cap", not "gigabytes".
const OVER_CAP: usize = PLUGIN_MANIFEST_MAX as usize + 4096;

fn global_scope() -> ResolvedScope {
    ResolvedScope {
        scope: tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
    }
}

/// Build a syntactically-valid `plugin.json` whose serialised size exceeds
/// the cap. The padding rides inside the (ignored under the lenient parse)
/// `description` field so the JSON stays well-formed and `name` is present.
fn oversized_valid_plugin_json() -> String {
    let pad = "A".repeat(OVER_CAP);
    let body = format!(r#"{{"name":"plugin-alpha","description":"{pad}"}}"#);
    assert!(body.len() > PLUGIN_MANIFEST_MAX as usize);
    body
}

/// Build a syntactically-valid `tome-catalog.toml` whose size exceeds the
/// cap. The padding rides in TOML comment lines so the document parses and
/// validates exactly like the small `GOOD` manifest would.
fn oversized_valid_catalog_toml() -> String {
    let mut body = String::with_capacity(OVER_CAP + 256);
    body.push_str(
        "name = \"oversized\"\n\
         description = \"valid but huge\"\n\
         version = \"0.1.0\"\n\
         [owner]\n\
         name = \"n\"\n\
         email = \"n@e.co\"\n",
    );
    // Comment lines are valid TOML and ignored by the parser — they let us
    // grow the file past the cap without making it invalid.
    while body.len() <= OVER_CAP {
        body.push_str("# padding padding padding padding padding padding padding\n");
    }
    assert!(body.len() > PLUGIN_MANIFEST_MAX as usize);
    body
}

/// Build a syntactically-valid `.mcp.json` advertising two servers, padded
/// past the cap inside an ignored string field so the JSON stays parseable.
fn oversized_valid_mcp_json() -> String {
    let pad = "A".repeat(OVER_CAP);
    let body = format!(r#"{{"_pad":"{pad}","mcpServers":{{"one":{{}},"two":{{}}}}}}"#);
    assert!(body.len() > PLUGIN_MANIFEST_MAX as usize);
    body
}

// ---------------------------------------------------------------------------
// Site 1 — `plugin/manifest.rs`: `parse_plugin_manifest` of `plugin.json`.
// Over-cap MUST surface the existing `PluginManifestParseError` (exit 22),
// never an unbounded read of the whole file.
// ---------------------------------------------------------------------------

#[test]
fn plugin_manifest_over_cap_is_bounded_parse_error() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("plugin.json");
    std::fs::write(&path, oversized_valid_plugin_json()).unwrap();

    let err = tome::plugin::manifest::parse_plugin_manifest(&path)
        .expect_err("oversized plugin.json must be refused, not slurped whole and parsed");

    match err {
        TomeError::PluginManifestParseError { file, .. } => {
            assert_eq!(file, path, "error must name the offending file");
        }
        other => panic!("expected PluginManifestParseError, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Site 2 — `catalog/manifest.rs`: `read_catalog_manifest` (the `.ok()?`
// site). An oversized manifest must behave as "invalid / unreadable"
// (`None`), NOT be silently accepted as a valid manifest.
// ---------------------------------------------------------------------------

#[test]
fn catalog_manifest_over_cap_reads_as_invalid_none() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let catalog_path = tmp.path();
    std::fs::write(
        catalog_path.join("tome-catalog.toml"),
        oversized_valid_catalog_toml(),
    )
    .unwrap();

    let parsed = tome::catalog::manifest::read_catalog_manifest(catalog_path);
    assert!(
        parsed.is_none(),
        "an oversized tome-catalog.toml must surface as invalid/unreadable (None), \
         not be slurped whole and accepted as a valid manifest"
    );
}

// ---------------------------------------------------------------------------
// Site 3 — `plugin/components.rs`: `count_mcp_servers` reads `.mcp.json`.
// Over-cap MUST yield the I/O-tolerant 0 (the same outcome as an unreadable
// file), never an unbounded read of the whole file.
// ---------------------------------------------------------------------------

#[test]
fn plugin_components_mcp_json_over_cap_is_bounded_zero() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let plugin_dir = tmp.path().join("plugin");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join(".mcp.json"), oversized_valid_mcp_json()).unwrap();

    let counts = tome::plugin::components::count_components(&plugin_dir);
    assert_eq!(
        counts.mcp_servers, 0,
        "an oversized .mcp.json must be refused (0 count), not slurped whole and counted"
    );
}

// ---------------------------------------------------------------------------
// Site 4 — `doctor/checks.rs`: `classify_clone` reads the cached
// `tome-catalog.toml`. An oversized manifest must classify as
// `ManifestInvalid` (a reported problem), NOT `Ok`.
// ---------------------------------------------------------------------------

#[test]
fn doctor_catalog_cache_over_cap_is_manifest_invalid() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = TempDir::new().unwrap();

    let fix = Fixture::build_sample();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();

    let cache_dir = cache_dir_for(&env, &fix.url);
    // Keep `.git/` so the cache is not classified `NotARepo`; replace the
    // manifest with an oversized-but-valid one so the ONLY way it could be
    // classified `Ok` is by reading the whole file and parsing it.
    std::fs::write(
        cache_dir.join("tome-catalog.toml"),
        oversized_valid_catalog_toml(),
    )
    .unwrap();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert_eq!(
        report.catalogs[0].state,
        CatalogCacheState::ManifestInvalid,
        "an oversized cached tome-catalog.toml must be reported as a problem, not OK"
    );
}

// ---------------------------------------------------------------------------
// Site 5 — command surface (`catalog show`): the CLI binary path that reads
// the cached `tome-catalog.toml`. Over-cap MUST fail with a non-zero exit
// (the existing `Io`/`ManifestInvalid` error), never read the whole file.
// ---------------------------------------------------------------------------

#[test]
fn catalog_show_over_cap_manifest_fails_bounded() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let fix = Fixture::build_sample();
    let added = env
        .cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();
    assert!(added.status.success(), "catalog add should succeed");

    // The sample fixture's catalog name (as recorded by `catalog add`).
    let cache_dir = cache_dir_for(&env, &fix.url);
    let catalog_name = read_catalog_name(&cache_dir);

    // Replace the manifest with an oversized-but-valid one. `catalog show`
    // would happily render it today (the unbounded read succeeds + parses);
    // bounded, it refuses the file.
    std::fs::write(
        cache_dir.join("tome-catalog.toml"),
        oversized_valid_catalog_toml(),
    )
    .unwrap();

    let shown = env
        .cmd()
        .args(["catalog", "show", &catalog_name])
        .output()
        .unwrap();
    assert!(
        !shown.status.success(),
        "`catalog show` of an oversized manifest must fail (bounded read), \
         not slurp the whole file and render it; stdout={:?}",
        String::from_utf8_lossy(&shown.stdout)
    );
}

/// Read the catalog's declared name from the freshly-cloned cache so the
/// `catalog show <name>` invocation matches the enrolment.
fn read_catalog_name(cache_dir: &Path) -> String {
    let bytes = std::fs::read(cache_dir.join("tome-catalog.toml")).unwrap();
    let manifest = tome::catalog::manifest::CatalogManifest::parse_and_validate(
        &cache_dir.join("tome-catalog.toml"),
        cache_dir,
        &bytes,
    )
    .expect("freshly-cloned manifest parses");
    manifest.name
}
