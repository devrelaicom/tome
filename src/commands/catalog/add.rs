//! `tome catalog add` — clone, parse, register. See
//! `specs/001-phase-1-foundations/contracts/catalog-add.md` and the Phase
//! 4 FR-362 flow.
//!
//! Phase 4 / F11b: catalog enrolment moves to the `workspace_catalogs`
//! junction table. The Phase 1 `config.toml` registry is no longer
//! written. Per-URL metadata (`last_synced`, `path`, `plugin_count`)
//! is derived at emit time from the filesystem.

use std::io::Write;
use std::path::Path;

use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::catalog::git::{Git, scrub_to_string};
use crate::catalog::manifest::CatalogManifest;
use crate::cli::CatalogAddArgs;
use crate::commands::plugin::registry_seeds;
use crate::error::TomeError;
use crate::index::{self, OpenOptions, acquire_lock, workspace_catalogs};
use crate::output::Mode;
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

use super::source;

pub fn run(args: CatalogAddArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    let workspace_name = scope.scope.name().as_str().to_owned();
    let url = source::resolve(&args.source)?;

    // Scrub credentials at the boundary. Phase 1 stored the user's typed
    // URL into config.toml; F11b stores it into workspace_catalogs.url.
    // Same scrubbing discipline either way.
    let scrubbed_url = scrub_to_string(url.as_bytes());

    // F-CACHE-KEY: key the cache dir + reuse refcount by the SCRUBBED URL,
    // because that is the URL we STORE and every reader (show / update /
    // remove + the reuse path below) resolves by. The raw `url` is kept
    // only for `clone_shallow`, which needs the credentials/SSH form for
    // auth. Keying the cache by the raw URL here would land the clone under
    // a different content-address than readers compute (for any source
    // where scrubbing changes the URL — SSH, `user:token@`), orphaning the
    // clone on disk and breaking reuse/refcount.
    let cache_dir = paths.cache_dir_for(&scrubbed_url);

    let clone_ref = args.ref_.as_deref();
    let pinned_ref = clone_ref.unwrap_or("main").to_owned();

    // Open the central DB up front so the workspace-existence check
    // surfaces (`WorkspaceNotFound`, exit 13) before we touch git or the
    // filesystem. Phase 4's lookup_workspace_id errors with that variant
    // for missing rows.
    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: embedder_seed,
            reranker: reranker_seed,
            summariser: summariser_seed,
        },
    )?;

    // Acquire the advisory lock across the clone/reuse decision + INSERT
    // (FR-366). The reuse path's `refcount_by_url > 0` check is the
    // safety belt against silently adopting an orphan cache dir.
    let lock = acquire_lock(&paths.index_lock)?;

    let result = (|| -> Result<(CatalogManifest, bool), TomeError> {
        // Reuse the existing clone only when at least one other workspace
        // already references this URL via the junction table. The Phase 3
        // `cache_dir.exists()` shortcut is not sufficient on its own —
        // it would silently adopt an orphan directory.
        let refs = workspace_catalogs::refcount_by_url(&conn, &scrubbed_url)?;
        let reuse_existing = cache_dir.exists() && refs > 0;

        let (manifest, _tempdir_guard) = if reuse_existing {
            let manifest_path = cache_dir.join("tome-catalog.toml");
            // Third-party manifest: cap at PLUGIN_MANIFEST_MAX (FR-006,
            // F-PLUGIN-MANIFEST-DOS). Preserve the NotFound → "no manifest"
            // mapping; an over-cap file falls to the `_` arm and surfaces as
            // exit-7 `Io`, never an unbounded read.
            let manifest_bytes =
                crate::util::bounded_read(&manifest_path, crate::util::PLUGIN_MANIFEST_MAX)
                    .map_err(|e| match &e {
                        TomeError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
                            TomeError::ManifestInvalid(crate::error::ManifestInvalid::TomlParse {
                                file: manifest_path.clone(),
                                message: "cached catalog has no tome-catalog.toml".to_string(),
                            })
                        }
                        _ => e,
                    })?;
            let manifest =
                CatalogManifest::parse_and_validate(&manifest_path, &cache_dir, &manifest_bytes)
                    .map_err(TomeError::ManifestInvalid)?;
            (manifest, None)
        } else {
            // Clone fresh. If the cache dir exists without a refcount,
            // it's orphaned (e.g. a crashed previous add); remove it so
            // the atomic rename can land cleanly.
            if cache_dir.exists() {
                std::fs::remove_dir_all(&cache_dir).map_err(TomeError::Io)?;
            }
            std::fs::create_dir_all(&paths.catalogs_dir).map_err(TomeError::Io)?;
            let tempdir = tempfile::Builder::new()
                .prefix(".tome-incoming-")
                .tempdir_in(&paths.catalogs_dir)
                .map_err(TomeError::Io)?;
            let clone_dest = tempdir.path().join("repo");

            let display_source = scrub_to_string(args.source.as_bytes());
            let git = Git::new(&display_source);
            // Clone with the RAW url — it carries the credentials / SSH form
            // git needs for auth. Only the cache key + stored URL are scrubbed.
            git.clone_shallow(&url, &clone_dest, clone_ref)?;

            let manifest_path = clone_dest.join("tome-catalog.toml");
            // Third-party manifest: cap at PLUGIN_MANIFEST_MAX (FR-006,
            // F-PLUGIN-MANIFEST-DOS). Preserve the NotFound → "no manifest"
            // mapping; an over-cap file falls to the `_` arm and surfaces as
            // exit-7 `Io`, never an unbounded read.
            let manifest_bytes =
                crate::util::bounded_read(&manifest_path, crate::util::PLUGIN_MANIFEST_MAX)
                    .map_err(|e| match &e {
                        TomeError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
                            TomeError::ManifestInvalid(crate::error::ManifestInvalid::TomlParse {
                                file: manifest_path.clone(),
                                message: "no tome-catalog.toml at the catalog root".to_string(),
                            })
                        }
                        _ => e,
                    })?;
            let manifest =
                CatalogManifest::parse_and_validate(&manifest_path, &clone_dest, &manifest_bytes)
                    .map_err(TomeError::ManifestInvalid)?;

            persist_clone(&clone_dest, &cache_dir)?;
            (manifest, Some(tempdir))
        };

        let display_name = args.name.clone().unwrap_or_else(|| manifest.name.clone());

        // INSERT. Per-workspace uniqueness on `(workspace, catalog_name)`
        // surfaces `CatalogAlreadyExists` (4) on a duplicate. We don't
        // pre-check via `find` first — letting the INSERT race produce
        // the error keeps the lock-hold window minimal and the error
        // path on the same code path.
        let insert_result = workspace_catalogs::insert(
            &conn,
            &workspace_name,
            &display_name,
            &scrubbed_url,
            &pinned_ref,
        );
        if let Err(e) = insert_result {
            // Rollback the cache directory ONLY if we cloned it. A
            // reused clone belongs to another workspace's enrolment;
            // deleting it would yank the rug out from under them.
            if !reuse_existing {
                let _ = std::fs::remove_dir_all(&cache_dir);
            }
            return Err(e);
        }

        // Construct an emit record local to this function — the Phase 1
        // CatalogEntry struct is on its way out, so we don't reach for it
        // here. The fields mirror what the JSON wire shape needs.
        let _ = display_name; // captured below via clone via emit() args
        Ok((manifest, reuse_existing))
    })();

    drop(lock);

    let (manifest, _reused) = result?;
    let display_name = args.name.clone().unwrap_or_else(|| manifest.name.clone());

    let last_synced = clone_mtime(&cache_dir);
    let emit_record = EmittedCatalog {
        name: display_name,
        url: scrubbed_url,
        pinned_ref,
        plugin_count: manifest.plugins.len(),
        cache_path: cache_dir,
        last_synced,
    };
    emit(mode, &emit_record)?;

    // best-effort: `file://` is the resolved shape for a local path source;
    // every other shape (https/ssh/git/owner-repo→github) is a remote clone.
    let source_type = if emit_record.url.starts_with("file://") {
        crate::telemetry::event::SourceType::Local
    } else {
        crate::telemetry::event::SourceType::Git
    };
    crate::telemetry::enqueue(crate::telemetry::event::CatalogActionEvent {
        action: crate::telemetry::event::CatalogAction::Added,
        source_type,
    });

    Ok(())
}

fn persist_clone(staged: &Path, final_dir: &Path) -> Result<(), TomeError> {
    if let Some(parent) = final_dir.parent() {
        std::fs::create_dir_all(parent).map_err(TomeError::Io)?;
    }
    std::fs::rename(staged, final_dir).map_err(TomeError::Io)?;
    Ok(())
}

/// Read the clone directory's mtime as RFC 3339. Returns `None` if the
/// directory is absent or its modified-time can't be read (rare; we
/// just-created it, but the kernel can still surprise us).
pub(crate) fn clone_mtime(cache_dir: &Path) -> Option<OffsetDateTime> {
    let meta = std::fs::metadata(cache_dir).ok()?;
    let systime = meta.modified().ok()?;
    Some(OffsetDateTime::from(systime))
}

/// Local emit record. Phase 1's `CatalogEntry` is deprecated; building a
/// purpose-shaped struct here keeps the JSON wire shape unchanged
/// without forcing the broader migration.
struct EmittedCatalog {
    name: String,
    url: String,
    pinned_ref: String,
    plugin_count: usize,
    cache_path: std::path::PathBuf,
    last_synced: Option<OffsetDateTime>,
}

#[derive(Serialize)]
struct AddedEnvelope<'a> {
    added: AddedRecord<'a>,
}

#[derive(Serialize)]
struct AddedRecord<'a> {
    name: &'a str,
    url: &'a str,
    #[serde(rename = "ref")]
    ref_: &'a str,
    plugin_count: usize,
    #[serde(with = "time::serde::rfc3339::option")]
    last_synced: Option<OffsetDateTime>,
}

fn emit(mode: Mode, rec: &EmittedCatalog) -> Result<(), TomeError> {
    match mode {
        Mode::Human => {
            let mut out = std::io::stdout().lock();
            writeln!(
                out,
                "Added catalog `{}` from {} (ref: {}, plugins: {}).",
                rec.name, rec.url, rec.pinned_ref, rec.plugin_count
            )?;
            let _ = rec.cache_path; // human form omits the path
            if let Some(ts) = rec.last_synced
                && let Ok(formatted) = ts.format(&Rfc3339)
            {
                writeln!(out, "  Cached at: {}", formatted)?;
            }
        }
        Mode::Json => {
            let env = AddedEnvelope {
                added: AddedRecord {
                    name: &rec.name,
                    url: &rec.url,
                    ref_: &rec.pinned_ref,
                    plugin_count: rec.plugin_count,
                    last_synced: rec.last_synced,
                },
            };
            crate::output::write_json(&env)?;
        }
    }
    Ok(())
}
