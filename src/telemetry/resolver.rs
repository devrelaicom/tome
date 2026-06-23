//! Emit-time catalog-attribution resolution (Phase 10 / US4).
//!
//! The SSOT every attributed emit site routes through: given a resolved scope
//! and a catalog NAME, decide whether the action's catalog resolves — **at emit
//! time** — to an allowlisted SOURCE (FR-052). The NAME is never the gate: a
//! local catalog named `midnight` whose enrolled URL is not the Midnight source
//! yields `None` (anonymous only). Only the canonicalized enrolled URL,
//! compared against the compiled-in [`allowlist::ATTRIBUTED_TELEMETRY_CATALOGS`],
//! decides attribution.
//!
//! Best-effort + infallible, like the rest of the silent telemetry path: any
//! error — no `$HOME`, missing DB, no enrolment, query failure — collapses to
//! `None` (the action is still recorded anonymously by the unchanged anonymous
//! emit at the same site). It opens the central index READ-ONLY and NEVER takes
//! the advisory lock (NFR-009; reuses [`crate::index::open_read_only`], the same
//! read path `heartbeat` uses).

use crate::telemetry::allowlist;
use crate::workspace::ResolvedScope;

/// Resolve a `(scope, catalog_name)` to its allowlist short id, if the catalog's
/// enrolled SOURCE is allowlisted (FR-052).
///
/// Reads the catalog's `url` from `workspace_catalogs` for the resolved scope's
/// workspace, then [`allowlist::match_source`]s it. `Some(short_id)` ⇒ the emit
/// site should ALSO enqueue the attributed event; `None` ⇒ anonymous only.
///
/// This is the place attribution is decided; it does NOT emit anything itself —
/// the caller pairs the returned id with the typed attributed event so the
/// `catalog.<id>.<suffix>` construction stays at the site that knows the
/// artefact names.
pub fn resolve_attribution(scope: &ResolvedScope, catalog_name: &str) -> Option<&'static str> {
    // Open the central index read-only — NO advisory lock (NFR-009). A missing
    // DB (fresh install) surfaces as an `Err` here, folded to `None`: an
    // un-enrolled catalog is simply not attributed.
    let paths = crate::paths::Paths::resolve().ok()?;
    let conn = match crate::index::open_read_only(&paths.index_db) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(target: "telemetry", error = %e, "attribution: index unavailable");
            return None;
        }
    };
    resolve_attribution_with(&conn, scope, catalog_name)
}

/// Connection-injectable [`resolve_attribution`] (the shared body). Doc-hidden —
/// exposed so tests can target a staged `TempDir`-rooted index without resolving
/// the real `$HOME`. The read is best-effort: any look-up failure ⇒ `None`.
#[doc(hidden)]
pub fn resolve_attribution_with(
    conn: &rusqlite::Connection,
    scope: &ResolvedScope,
    catalog_name: &str,
) -> Option<&'static str> {
    let workspace_name = scope.scope.name().as_str();
    // Look up the enrolment row for THIS workspace's catalog. Any error (no such
    // workspace, query failure) or a missing enrolment ⇒ no attribution.
    let enrolment = match crate::index::workspace_catalogs::find(conn, workspace_name, catalog_name)
    {
        Ok(Some(e)) => e,
        Ok(None) => return None,
        Err(e) => {
            tracing::debug!(target: "telemetry", error = %e, "attribution: enrolment lookup failed");
            return None;
        }
    };
    // The SOURCE — not the name — is the gate (FR-052). Canonicalize + match
    // the enrolled URL against the compiled-in allowlist.
    allowlist::match_source(&enrolment.url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{self, OpenOptions, workspace_catalogs};
    use crate::workspace::{ResolvedScope, WorkspaceName};
    use tempfile::TempDir;

    /// The canonical Midnight catalog source — exactly what the allowlist const
    /// canonicalizes to. An enrolment at this URL MUST attribute to `"midnight"`.
    const MIDNIGHT_SOURCE: &str = "https://github.com/devrelaicom/midnight-expert-tome";

    fn registry_seeds() -> (index::MetaSeed, index::MetaSeed, index::MetaSeed) {
        let seed = |n: &str| index::MetaSeed {
            name: n.to_owned(),
            version: "1".to_owned(),
        };
        (seed("e"), seed("r"), seed("s"))
    }

    /// Open a fresh central index in `dir`, returning the live connection. The
    /// `global` workspace is seeded at bootstrap, so enrolments can target it.
    fn open_seeded_index(dir: &TempDir) -> rusqlite::Connection {
        let paths = crate::paths::Paths::from_root(dir.path().to_path_buf());
        let (embedder, reranker, summariser) = registry_seeds();
        index::open(
            &paths.index_db,
            &OpenOptions {
                embedder,
                reranker,
                summariser,
                profile: None,
            },
        )
        .expect("open seeded index")
    }

    fn global_scope() -> ResolvedScope {
        ResolvedScope::global_fallback()
    }

    fn enrol(conn: &rusqlite::Connection, name: &str, url: &str) {
        workspace_catalogs::insert(conn, WorkspaceName::global().as_str(), name, url, "main")
            .expect("enrol catalog");
    }

    #[test]
    fn allowlisted_source_attributes_to_midnight() {
        let dir = TempDir::new().unwrap();
        let conn = open_seeded_index(&dir);
        // The catalog NAME here is deliberately NOT "midnight" — attribution is
        // by SOURCE, so an arbitrary local alias still attributes.
        enrol(&conn, "my-midnight-alias", MIDNIGHT_SOURCE);

        assert_eq!(
            resolve_attribution_with(&conn, &global_scope(), "my-midnight-alias"),
            Some("midnight"),
            "an enrolment at the Midnight source attributes regardless of its alias",
        );
    }

    #[test]
    fn non_allowlisted_source_is_not_attributed() {
        let dir = TempDir::new().unwrap();
        let conn = open_seeded_index(&dir);
        enrol(
            &conn,
            "other",
            "https://github.com/someone/unrelated-catalog",
        );

        assert_eq!(
            resolve_attribution_with(&conn, &global_scope(), "other"),
            None,
            "a catalog whose source is not on the allowlist ⇒ no attribution",
        );
    }

    #[test]
    fn name_collision_with_non_allowlisted_source_is_not_attributed() {
        // The defining FR-052 case: a DIFFERENT catalog NAMED like Midnight but
        // whose SOURCE is not the Midnight repo ⇒ anonymous only. The name is
        // never the gate.
        let dir = TempDir::new().unwrap();
        let conn = open_seeded_index(&dir);
        enrol(
            &conn,
            "midnight",
            "https://github.com/someone/midnight-expert-tome",
        );

        assert_eq!(
            resolve_attribution_with(&conn, &global_scope(), "midnight"),
            None,
            "a name collision with a non-allowlisted source must NOT attribute (the source is the gate)",
        );
    }

    #[test]
    fn missing_enrolment_is_not_attributed() {
        let dir = TempDir::new().unwrap();
        let conn = open_seeded_index(&dir);
        // No enrolment inserted for this name.
        assert_eq!(
            resolve_attribution_with(&conn, &global_scope(), "nonexistent"),
            None,
            "no enrolment ⇒ no attribution (best-effort None, never an error)",
        );
    }
}
