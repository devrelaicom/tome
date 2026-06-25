//! FR-110 startup pre-flight for `tome mcp`.
//!
//! Per [`contracts/mcp-server.md`](../../specs/003-phase-3-mcp-workspaces/contracts/mcp-server.md)
//! §"Behaviour" step 3, the server validates the resolved scope's index
//! and embedder before binding the stdio transport. Each failure exits
//! the process before the harness sees a handshake; a specific Phase 1/2
//! exit code wins over the generic `McpStartupFailed` (60) per
//! [`contracts/exit-codes-p3.md`] §"Specific-over-generic preference".
//!
//! The reranker is intentionally **not** loaded here. FR-109 defers
//! reranker initialisation until the first `search_skills` call so a
//! handful of `get_skill` invocations never pay the cost.

use crate::embedding::download;
use crate::embedding::registry::ModelEntry;
use crate::embedding::{Embedder, fastembed::FastembedEmbedder};
use crate::error::TomeError;
use crate::index::MetaSeed;
use crate::index::meta::{DriftStatus, ModelIdent, detect_drift};
use crate::index::{db, migrations, schema};
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

/// The pre-flight's output. Carried into the MCP server's state in US1.
pub struct EmbedderHandle {
    pub embedder: Box<dyn Embedder>,
    pub embedder_entry: &'static ModelEntry,
    pub reranker_entry: &'static ModelEntry,
    /// Phase 12 / US2: the ACTIVE embedder's drift identity. Remote
    /// (`"<provider>/<model>"`/`"external"`) when an `[embedding]` provider is
    /// configured, else the bundled `embedder_entry`'s `(name, version)`. The
    /// `search_skills` handler passes THIS (not `embedder_entry`) to
    /// `detect_drift`, so on a remote index the comparison is against the right
    /// stored identity and the server refuses to serve under a real embedder
    /// change rather than reporting spurious drift on every search.
    pub embedder_seed: MetaSeed,
}

/// Run the pre-flight against the resolved scope's index and the
/// installed embedder/reranker artefacts. Returns the loaded embedder.
///
/// Steps, in order (matching `contracts/mcp-server.md`):
///
/// 1. Locate embedder + reranker registry entries.
/// 2. Open the resolved scope's index DB read-only.
/// 3. Refuse newer-on-disk schema with `SchemaVersionTooNew` (exit 73).
/// 4. Compare embedder identity against the index `meta` rows; surface
///    drift as `EmbedderNameDrift` (41) / `EmbedderVersionDrift` (42).
/// 5. Verify embedder files exist and pass SHA-256.
/// 6. Eager-load the embedder.
pub fn run(_scope: &ResolvedScope, paths: &Paths) -> Result<EmbedderHandle, TomeError> {
    // B4: the embedder + reranker are resolved from the ACTIVE profile AFTER
    // the index conn opens (see below) — the registry now carries every
    // profile's models, so the old "first registry entry of this kind"
    // (`pick_kind`) would pick the small profile rather than the active one.

    // F2a: single central index DB; F11 reintroduces workspace-aware view.
    let db_path = paths.index_db.clone();
    if !db_path.is_file() {
        // FR-M-MCP-4 / exit-codes-p3.md §"Specific-over-generic
        // preference": an absent index DB is a Phase 2 integrity-class
        // failure (exit 35), not a generic Phase 3 MCP-startup residual
        // (exit 60). Surfacing the specific code lets harnesses
        // distinguish "user hasn't enabled any plugins yet" from "MCP
        // wiring itself failed".
        return Err(TomeError::IndexIntegrityCheckFailure(format!(
            "index database not found at {} — enable at least one plugin first",
            db_path.display()
        )));
    }

    // `open_read_only` already gates on schema-too-new — but it routes
    // through the legacy `SchemaTooNew` (52). The MCP contract names
    // exit 73 for this case, so we re-check explicitly and surface the
    // Phase 3 variant. open_read_only will then run, observe a matching
    // version, and proceed.
    let probe_conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| TomeError::McpStartupFailed {
        reason: format!("open index probe: {e}"),
    })?;
    if let Some(stored) = migrations::current_schema_version(&probe_conn)?
        && stored > schema::SCHEMA_VERSION
    {
        return Err(TomeError::SchemaVersionTooNew {
            on_disk: stored,
            expected: schema::SCHEMA_VERSION,
        });
    }
    drop(probe_conn);

    let conn = db::open_read_only(&db_path)?;

    // B4: resolve the ACTIVE profile's embedder + reranker from the index
    // `meta` (the conn is in hand now). Used for BOTH the loaded
    // `EmbedderHandle` and the `detect_drift` idents below.
    let embedder_entry = crate::index::meta::active_embedder(&conn)?;
    let reranker_entry = crate::index::meta::active_reranker(&conn)?;

    // Phase 12 / US2: the MCP server embeds remotely when `[embedding]` is
    // configured. Load config defensively-strict here: a malformed config is a
    // startup failure (the server cannot resolve which embedder to bind), so we
    // use the strict loader and surface the parse error. The remote-aware seed
    // is the ACTIVE embedder identity used for BOTH drift detection and the
    // handle the server carries.
    let cfg = crate::config::load(paths)?;
    let remote_embedding =
        crate::provider::resolve(&cfg, crate::provider::Capability::Embedding)?.is_some();
    let embedder_seed = crate::embedding::embedder_seed(&cfg, embedder_entry)?;

    // Drift detection. The reranker comparison still happens here for
    // observability, but reranker drift is *not* a startup failure —
    // FR-109 defers reranker loading until first use, so the running
    // server can survive reranker drift by re-downloading on demand. The
    // embedder ident reflects the ACTIVE (remote-or-bundled) identity so a
    // remote-embedder change is detected against the stored `meta` rows.
    let embedder_ident = ModelIdent {
        name: embedder_seed.name.clone(),
        version: embedder_seed.version.clone(),
    };
    let reranker_ident = ModelIdent {
        name: reranker_entry.name.into(),
        version: reranker_entry.version.into(),
    };
    // Phase 4 / F9: summariser identity is also recorded. Drift on the
    // summariser is not a startup failure (same rationale as reranker
    // drift — only summary regeneration depends on it).
    let summariser_entry = crate::summarise::registry::summariser_entry();
    let summariser_ident = ModelIdent {
        name: summariser_entry.name.into(),
        version: summariser_entry.version.into(),
    };
    match detect_drift(&conn, &embedder_ident, &reranker_ident, &summariser_ident)? {
        DriftStatus::EmbedderNameDrift { stored, configured } => {
            return Err(TomeError::EmbedderNameDrift { stored, configured });
        }
        DriftStatus::EmbedderVersionDrift { stored, configured } => {
            return Err(TomeError::EmbedderVersionDrift { stored, configured });
        }
        DriftStatus::RerankerDrift { .. }
        | DriftStatus::SummariserDrift { .. }
        | DriftStatus::None => {}
    }

    // Phase 12 / US2: on the remote path, seed the embedder's expected dimension
    // from the persisted `meta.embedder_dimension` so query-time validation (run
    // for every `search_skills`) asserts the dimension the index was built at.
    // Read while the conn is still in hand.
    let persisted_dim = if remote_embedding {
        crate::index::read_embedder_dimension(&conn)?
    } else {
        None
    };
    drop(conn);

    let embedder: Box<dyn Embedder> = if remote_embedding {
        // A remote embedder has no local model artefacts to verify; the
        // first-run notice + RemoteEmbedder construction route through the
        // shared `build_embedder`. The startup-frozen embedder validates every
        // search at query time (the in-memory embedder is built ONCE here, so a
        // later config change is NOT picked up until restart — by design).
        crate::embedding::build_embedder(&cfg, paths, embedder_entry, persisted_dim)?
    } else {
        // Bundled: verify artefacts (SHA-256 of the primary file) then load.
        // The contract demands SHA-256 verification rather than the cheap
        // "exists + size" check that `tome status` uses — the MCP server is a
        // long-running process, so paying the full hash once at startup is the
        // right trade-off.
        verify_embedder_artefacts(paths, embedder_entry)?;
        let model_dir = paths.model_path(embedder_entry.name)?;
        Box::new(FastembedEmbedder::load(embedder_entry, &model_dir)?)
    };

    Ok(EmbedderHandle {
        embedder,
        embedder_entry,
        reranker_entry,
        embedder_seed,
    })
}

fn verify_embedder_artefacts(paths: &Paths, entry: &ModelEntry) -> Result<(), TomeError> {
    let model_dir = paths.model_path(entry.name)?;

    // Every declared file must exist.
    for rel in entry.files {
        let p = model_dir.join(rel);
        if !p.is_file() {
            return Err(TomeError::ModelMissing {
                model: entry.name.into(),
            });
        }
    }

    // SHA-256 of the primary file must match the pinned digest.
    if let Some(primary) = entry.files.first() {
        let primary_path = model_dir.join(primary);
        let computed = download::sha256_file(&primary_path)?;
        if computed != entry.sha256 {
            return Err(TomeError::ModelChecksumMismatch {
                model: entry.name.into(),
                expected: entry.sha256.into(),
                got: computed,
            });
        }
    }

    Ok(())
}
