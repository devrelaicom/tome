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
use crate::embedding::registry::{MODEL_REGISTRY, ModelEntry, ModelKind};
use crate::embedding::{Embedder, fastembed::FastembedEmbedder};
use crate::error::TomeError;
use crate::index::meta::{DriftStatus, ModelIdent, detect_drift};
use crate::index::{db, migrations, schema};
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

/// The pre-flight's output. Carried into the MCP server's state in US1.
pub struct EmbedderHandle {
    pub embedder: Box<dyn Embedder>,
    pub embedder_entry: &'static ModelEntry,
    pub reranker_entry: &'static ModelEntry,
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
    let embedder_entry = pick_kind(ModelKind::Embedder)?;
    let reranker_entry = pick_kind(ModelKind::Reranker)?;

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

    // Drift detection. The reranker comparison still happens here for
    // observability, but reranker drift is *not* a startup failure —
    // FR-109 defers reranker loading until first use, so the running
    // server can survive reranker drift by re-downloading on demand.
    let embedder_ident = ModelIdent {
        name: embedder_entry.name.into(),
        version: embedder_entry.version.into(),
    };
    let reranker_ident = ModelIdent {
        name: reranker_entry.name.into(),
        version: reranker_entry.version.into(),
    };
    match detect_drift(&conn, &embedder_ident, &reranker_ident)? {
        DriftStatus::EmbedderNameDrift { stored, configured } => {
            return Err(TomeError::EmbedderNameDrift { stored, configured });
        }
        DriftStatus::EmbedderVersionDrift { stored, configured } => {
            return Err(TomeError::EmbedderVersionDrift { stored, configured });
        }
        DriftStatus::RerankerDrift { .. } | DriftStatus::None => {}
    }
    drop(conn);

    // Embedder artefacts on disk. The contract demands SHA-256
    // verification of the primary file rather than the cheap "exists +
    // size" check that `tome status` uses — the MCP server is a
    // long-running process, so paying the full hash once at startup is
    // the right trade-off.
    verify_embedder_artefacts(paths, embedder_entry)?;

    let model_dir = paths.model_path(embedder_entry.name)?;
    let embedder = FastembedEmbedder::load(embedder_entry, &model_dir)?;

    Ok(EmbedderHandle {
        embedder: Box::new(embedder),
        embedder_entry,
        reranker_entry,
    })
}

fn pick_kind(kind: ModelKind) -> Result<&'static ModelEntry, TomeError> {
    MODEL_REGISTRY
        .iter()
        .find(|m| m.kind == kind)
        .ok_or_else(|| TomeError::McpStartupFailed {
            reason: format!(
                "MODEL_REGISTRY missing a {} entry",
                match kind {
                    ModelKind::Embedder => "embedder",
                    ModelKind::Reranker => "reranker",
                }
            ),
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
