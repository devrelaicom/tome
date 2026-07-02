//! `tome models download` — fetch the active profile's models if missing.
//! With `--all`, fetch every registered model. With `--force`, re-download
//! whether or not the on-disk manifest already records a complete install.
//!
//! The default target set is the ACTIVE profile's `{embedder, reranker,
//! summariser}` (the summariser is profile-independent). Scoping the default
//! to the active profile mirrors the enable path's `ensure_models_or_prompt`
//! (B2) so a small/medium/large install never pulls every tier's weights.
//!
//! Spec: `contracts/models-commands.md` §"`tome models download`", FR-021.

use std::io::Write;
use std::time::Instant;

use serde::Serialize;
use tracing::info;

use crate::cli::ModelsDownloadArgs;
use crate::embedding::download::download_model;
use crate::embedding::profile::{Profile, embedder_for, reranker_for};
use crate::embedding::registry::{MODEL_REGISTRY, ModelEntry};
use crate::error::TomeError;
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::presentation::{colour, progress};

use super::{ModelState, cheap_state, human_mb};

pub fn run(args: ModelsDownloadArgs, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    std::fs::create_dir_all(&paths.models_dir).map_err(TomeError::Io)?;

    let targets = resolve_targets(&paths, args.all, args.profile)?;

    let mut records: Vec<DownloadRecord> = Vec::new();

    for entry in targets {
        let (state, _manifest) = cheap_state(&paths, entry)?;
        let already_installed = matches!(state, ModelState::Ok);

        if already_installed && !args.force {
            // Skipped — the manifest + files are consistent. Report and move
            // on.
            if mode == Mode::Human {
                let mut out = std::io::stdout().lock();
                writeln!(
                    out,
                    "{} {} ({}) — {} {}",
                    colour::dim("·"),
                    entry.name,
                    entry.version,
                    human_mb(entry.size_bytes),
                    colour::dim("skipped"),
                )?;
            }
            records.push(DownloadRecord {
                name: entry.name.to_owned(),
                version: entry.version.to_owned(),
                kind: kind_str(entry.kind),
                action: "skipped",
                size_bytes: entry.size_bytes,
                sha256_verified: true,
                duration_ms: 0,
            });
            continue;
        };

        // Re-download or first install.
        let action_label: &'static str = if already_installed {
            "redownloaded"
        } else {
            "downloaded"
        };

        if mode == Mode::Human {
            let mut out = std::io::stdout().lock();
            writeln!(
                out,
                "{} ({}) — {}",
                entry.name,
                entry.version,
                human_mb(entry.size_bytes)
            )?;
        }

        // F6 added a byte-progress hook to `download_model`; US4.a
        // (T319) wires the determinate byte bar so big artefacts (the
        // ~400 MB Qwen summariser, the ~280 MB reranker) show real
        // progress + ETA + throughput. The bar still works for tiny
        // artefacts; `byte_bar(0, ...)` saturates rather than panicking
        // (covered by `presentation::progress::tests::bar_with_zero_total_does_not_panic`).
        let pb = progress::byte_bar(entry.size_bytes, format!("downloading {}", entry.name));
        let cb = |bytes_so_far: u64, _total: u64| {
            pb.set_position(bytes_so_far);
        };
        let started = Instant::now();
        let result = download_model(entry, &paths.models_dir, Some(&cb));
        pb.finish_and_clear();
        let elapsed = started.elapsed();

        // OUTCOME-bearing: emit `tome.model_download` per attempt with the REAL
        // outcome (Ok on success / Failed on error). `model_id` is the closed
        // `&'static str` registry id; `error_class` is the failure's category.
        match &result {
            Ok(_) => crate::telemetry::emit(crate::telemetry::event::ModelDownload {
                model_id: entry.name,
                outcome: crate::telemetry::event::Outcome::Ok,
                error_class: None,
            }),
            Err(e) => crate::telemetry::emit(crate::telemetry::event::ModelDownload {
                model_id: entry.name,
                outcome: crate::telemetry::event::Outcome::Failed,
                error_class: Some(e.category()),
            }),
        }

        result?;
        info!(model = entry.name, "model artefact installed");

        if mode == Mode::Human {
            let mut out = std::io::stdout().lock();
            writeln!(
                out,
                "  {} {} · {:.1}s",
                colour::success("✓"),
                action_label,
                elapsed.as_secs_f64(),
            )?;
        }

        let duration_ms = elapsed.as_millis().min(u128::from(u64::MAX)) as u64;
        records.push(DownloadRecord {
            name: entry.name.to_owned(),
            version: entry.version.to_owned(),
            kind: kind_str(entry.kind),
            action: action_label,
            size_bytes: entry.size_bytes,
            sha256_verified: true,
            duration_ms,
        });
    }

    if mode == Mode::Json {
        let envelope = DownloadEnvelope { models: records };
        output::write_json(&envelope)?;
    }

    Ok(())
}

/// The set of registry entries `download` should fetch. With `all`, every
/// `MODEL_REGISTRY` entry. With an explicit `profile`, that tier's
/// `{embedder, reranker, summariser}` — WITHOUT reading or writing the stored
/// active profile. Otherwise the ACTIVE profile's set (resolved from the index
/// `meta`, falling back to the default profile when no DB exists — exactly what
/// the bootstrap will stamp).
///
/// `--profile` never touches `meta.model_profile`: it is a read-only override
/// of the download TARGET, so pre-fetching another tier's weights leaves the
/// active profile (and therefore the embedder identity + index) unchanged.
fn resolve_targets(
    paths: &Paths,
    all: bool,
    explicit_profile: Option<Profile>,
) -> Result<Vec<&'static ModelEntry>, TomeError> {
    if all {
        return Ok(MODEL_REGISTRY.iter().collect());
    }

    let profile = match explicit_profile {
        // `--profile <tier>` — download that tier's set, reading NOTHING from
        // and writing NOTHING to the stored active profile.
        Some(p) => p,
        // No `--profile` — the active profile, exactly as before.
        None if paths.index_db.is_file() => {
            let conn = crate::index::open_read_only(&paths.index_db)?;
            crate::index::meta::active_profile(&conn)?
        }
        None => Profile::DEFAULT,
    };

    Ok(vec![
        embedder_for(profile),
        reranker_for(profile),
        crate::summarise::registry::summariser_entry(),
    ])
}

fn kind_str(kind: crate::embedding::registry::ModelKind) -> &'static str {
    use crate::embedding::registry::ModelKind;
    match kind {
        ModelKind::Embedder => "embedder",
        ModelKind::Reranker => "reranker",
        ModelKind::Summariser => "summariser",
    }
}

#[derive(Serialize)]
struct DownloadEnvelope {
    models: Vec<DownloadRecord>,
}

#[derive(Serialize)]
struct DownloadRecord {
    name: String,
    version: String,
    kind: &'static str,
    action: &'static str,
    size_bytes: u64,
    sha256_verified: bool,
    duration_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Names of the entries `resolve_targets` selects.
    fn names(targets: &[&'static ModelEntry]) -> Vec<&'static str> {
        targets.iter().map(|e| e.name).collect()
    }

    #[test]
    fn explicit_profile_targets_that_tier_without_reading_any_db() {
        // `--profile large` selects the LARGE tier's {embedder, reranker,
        // summariser} — resolved purely from the tier argument, reading NO
        // index DB (the path passed here has none). This is what makes
        // `--profile` a pure download-target override that never touches the
        // stored active profile.
        let dir = tempfile::TempDir::new().unwrap();
        let paths = Paths::from_root(dir.path().to_path_buf());
        assert!(!paths.index_db.is_file(), "precondition: no DB");

        let targets = resolve_targets(&paths, false, Some(Profile::Large)).unwrap();
        let got = names(&targets);
        assert!(got.contains(&"bge-large-en-v1.5"), "{got:?}");
        assert!(got.contains(&"bge-reranker-v2-m3"), "{got:?}");
        assert!(got.contains(&"qwen2.5-0.5b-instruct"), "{got:?}");
        assert_eq!(targets.len(), 3);
        // Crucially, no DB was created by resolving the explicit tier.
        assert!(
            !paths.index_db.is_file(),
            "resolving --profile must not create the index DB"
        );
    }

    #[test]
    fn explicit_profile_ignores_the_active_profile_when_set() {
        // A different explicit tier than the (defaulted) active profile still
        // selects the explicit tier — `--profile` overrides, and it does so
        // without persisting the change.
        let dir = tempfile::TempDir::new().unwrap();
        let paths = Paths::from_root(dir.path().to_path_buf());

        let small = resolve_targets(&paths, false, Some(Profile::Small)).unwrap();
        assert_eq!(
            names(&small),
            vec![
                "bge-small-en-v1.5",
                "bge-reranker-base",
                "qwen2.5-0.5b-instruct"
            ],
        );
    }

    #[test]
    fn explicit_profile_overrides_a_populated_db_recording_a_different_active_profile() {
        // The `Some(profile)` branch must select the EXPLICIT tier's targets
        // even when a POPULATED index DB records a DIFFERENT active profile.
        // Bootstrap a real DB stamped with `active_profile == Small`, then ask
        // `resolve_targets` for `--profile large`: the returned targets must be
        // the LARGE pair (the stored Small is irrelevant on the explicit branch).
        let dir = tempfile::TempDir::new().unwrap();
        let paths = Paths::from_root(dir.path().to_path_buf());
        std::fs::create_dir_all(paths.index_db.parent().unwrap()).unwrap();

        // Open (bootstrap) the DB with the active profile seeded to Small.
        let (embedder, reranker, summariser) = crate::commands::plugin::registry_seeds();
        let conn = crate::index::open(
            &paths.index_db,
            &crate::index::OpenOptions {
                embedder,
                reranker,
                summariser,
                profile: Some(Profile::Small),
            },
        )
        .unwrap();
        // Sanity: the DB really records Small as the active profile.
        assert_eq!(
            crate::index::meta::active_profile(&conn).unwrap(),
            Profile::Small,
            "precondition: the populated DB's active profile is Small",
        );
        drop(conn);
        assert!(
            paths.index_db.is_file(),
            "the DB now exists and is populated"
        );

        // The explicit `--profile large` wins for target selection.
        let targets = resolve_targets(&paths, false, Some(Profile::Large)).unwrap();
        assert_eq!(
            names(&targets),
            vec![
                "bge-large-en-v1.5",
                "bge-reranker-v2-m3",
                "qwen2.5-0.5b-instruct"
            ],
            "explicit --profile large must select the LARGE tier's targets, \
             not the Small tier the DB records",
        );
    }

    #[test]
    fn no_profile_no_db_falls_back_to_default_tier() {
        // Without `--profile` and without a DB, the default profile (Medium)
        // set is targeted — byte-identical to the pre-`--profile` behaviour.
        let dir = tempfile::TempDir::new().unwrap();
        let paths = Paths::from_root(dir.path().to_path_buf());

        let targets = resolve_targets(&paths, false, None).unwrap();
        let got = names(&targets);
        assert!(
            got.contains(&"bge-base-en-v1.5"),
            "medium embedder: {got:?}"
        );
        assert!(
            got.contains(&"bge-reranker-large"),
            "medium reranker: {got:?}"
        );
        assert!(
            got.contains(&"qwen2.5-0.5b-instruct"),
            "summariser: {got:?}"
        );
    }

    #[test]
    fn all_flag_spans_every_registry_entry() {
        // `--all` still targets the full registry regardless of the profile arg.
        let dir = tempfile::TempDir::new().unwrap();
        let paths = Paths::from_root(dir.path().to_path_buf());
        let targets = resolve_targets(&paths, true, None).unwrap();
        assert_eq!(targets.len(), MODEL_REGISTRY.len());
    }
}
