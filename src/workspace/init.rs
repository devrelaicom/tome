//! `tome workspace init <name>` — create a workspace in the central
//! registry plus a populated `<root>/workspaces/<name>/` directory.
//!
//! Phase 4 / US2.a-1 replaces Phase 3's path-based init (which created a
//! per-project `.tome/` marker). The Phase 4 surface is name-keyed and
//! interacts with the central DB:
//!
//! 1. Validate the name via [`WorkspaceName::parse`] (refuses the
//!    reserved `global` name — exit 15).
//! 2. Acquire the central advisory lockfile.
//! 3. Open the central index (the privileged `global` workspace is
//!    seeded on first bootstrap; subsequent opens are idempotent).
//! 4. Refuse if a `workspaces.name = <name>` row already exists — exit 14.
//! 5. Inside one DB transaction, INSERT the `workspaces` row with
//!    `created_at = last_used_at = now`. If `--inherit-global`, copy the
//!    global workspace's `workspace_catalogs` rows over to the new
//!    workspace's `workspace_id` in the same transaction. Commit.
//! 6. Outside the transaction (still holding the lock), land
//!    `<root>/workspaces/<name>/` atomically via
//!    [`crate::util::atomic_dir::land_directory`]:
//!    - Write `settings.toml` with `name = "<name>"` and (when
//!      `--inherit-global` and global had catalogs) a `[[catalogs]]`
//!      array mirroring the just-inserted junction rows.
//!    - Write an empty `RULES.md` placeholder. The body is filled by
//!      US2.a-2's `regen-summary`; US2.a-1 ships an empty stub.
//!
//! ## Atomicity ordering
//!
//! The DB INSERT runs BEFORE the directory landing. If the directory
//! landing fails after the row is committed, an orphan DB row remains —
//! pointing at a directory that doesn't exist. Doctor's workspace
//! subsystem (US5) surfaces this; the user can re-run `tome workspace
//! init <name>` after `tome workspace remove --force <name>` (US2.b) to
//! recover. The alternative ordering (directory first, INSERT second)
//! leaves an orphaned directory if the INSERT fails — a worse outcome
//! because the DB is the source of truth.
//!
//! The atomic-directory landing itself never leaves a partial: if
//! `populate` returns an error, `tempfile::TempDir::drop` cleans the
//! staging directory.
//!
//! ## SIGINT mid-init
//!
//! If interrupted between the DB commit and the atomic-rename:
//! - The DB row exists.
//! - The staging directory (`.tome.tmp.*`) lingers — `doctor --fix` (US5)
//!   sweeps the prefix.
//! - The target directory `<root>/workspaces/<name>/` does not exist yet.
//!
//! Recovery: re-run `init` after `remove --force` (US2.b), or wait for
//! `doctor --fix`.

use std::path::PathBuf;

use serde::Serialize;
use time::OffsetDateTime;

use crate::commands::plugin::registry_seeds;
use crate::error::TomeError;
use crate::index::{self, OpenOptions, acquire_lock, workspace_catalogs};
use crate::paths::Paths;
use crate::util::atomic_dir;
use crate::workspace::WorkspaceName;

/// Outcome of [`init`]. Serialised by the CLI's `--json` mode.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct InitOutcome {
    /// The validated name of the freshly-created workspace.
    pub name: WorkspaceName,
    /// Absolute on-disk path of the landed workspace directory.
    pub workspace_dir: PathBuf,
    /// Number of catalogs seeded from the global workspace. Zero unless
    /// `--inherit-global` was set AND global had at least one enrolled
    /// catalog.
    pub inherited_catalogs: u32,
}

/// Create a new workspace.
///
/// See module-level docs for the algorithm + atomicity ordering. Returns
/// the populated [`InitOutcome`] on success; surfaces:
/// - `WorkspaceNameInvalid` (15) on a reserved name.
/// - `WorkspaceAlreadyExists` (14) on a duplicate name.
/// - `Io` (7) on filesystem failures inside the atomic landing.
/// - `IndexIntegrityCheckFailure` (51) on unexpected SQL errors.
pub fn init(
    name: WorkspaceName,
    inherit_global: bool,
    paths: &Paths,
) -> Result<InitOutcome, TomeError> {
    if name.is_reserved() {
        return Err(TomeError::WorkspaceNameInvalid {
            name: name.as_str().to_owned(),
            reason: "`global` is the privileged seeded workspace; it cannot be re-created via \
                     `tome workspace init`"
                .to_owned(),
        });
    }

    // Make sure the parent of index.db exists; lock acquisition will
    // create the lockfile itself, but the surrounding directory must
    // already be present.
    if let Some(parent) = paths.index_lock.parent()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent).map_err(TomeError::Io)?;
    }

    let lock = acquire_lock(&paths.index_lock)?;

    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
    let mut conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: embedder_seed,
            reranker: reranker_seed,
            summariser: summariser_seed,
        },
    )?;

    // Phase 4 / FR-400: refuse if the name already exists. The check
    // runs under the lock so the subsequent INSERT can't race with
    // another concurrent `init`.
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = ?1",
            rusqlite::params![name.as_str()],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("lookup existing workspace: {e}"))
        })?;
    if existing.is_some() {
        return Err(TomeError::WorkspaceAlreadyExists {
            name: name.as_str().to_owned(),
        });
    }

    // Collect global's catalogs FIRST (still under the lock; no INSERT
    // yet). The `--inherit-global` path mirrors these into both the
    // junction table AND the on-disk settings.toml. If global has no
    // enrolments, the flag is a documented no-op.
    let inherited: Vec<workspace_catalogs::CatalogEnrolment> = if inherit_global {
        workspace_catalogs::list_for_workspace(&conn, WorkspaceName::GLOBAL)
            .unwrap_or_else(|_| Vec::new())
    } else {
        Vec::new()
    };
    let inherited_count = u32::try_from(inherited.len()).unwrap_or(u32::MAX);

    let now_unix = OffsetDateTime::now_utc().unix_timestamp();
    let new_workspace_id: i64 = {
        let tx = conn.transaction().map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("begin init transaction: {e}"))
        })?;
        tx.execute(
            "INSERT INTO workspaces (name, created_at, last_used_at)
             VALUES (?1, ?2, ?2)",
            rusqlite::params![name.as_str(), now_unix],
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("insert workspaces row: {e}"))
        })?;
        let id = tx.last_insert_rowid();
        for entry in &inherited {
            tx.execute(
                "INSERT INTO workspace_catalogs (workspace_id, catalog_name, url, pinned_ref)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    id,
                    entry.catalog_name.as_str(),
                    entry.url.as_str(),
                    entry.pinned_ref.as_str(),
                ],
            )
            .map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!(
                    "copy global catalog `{}` into new workspace: {e}",
                    entry.catalog_name
                ))
            })?;
        }
        tx.commit().map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("commit init transaction: {e}"))
        })?;
        id
    };

    // Drop the DB handle BEFORE the directory landing so a WAL
    // checkpoint completes inside the lock window. The lock itself is
    // released at function return.
    drop(conn);

    // Atomically land the workspace directory. Same-FS rename guarantees
    // no observer sees a partial. On failure, the staging dir is
    // auto-cleaned via TempDir::drop; the DB row above stays committed
    // (orphan; recoverable via remove + re-init, or doctor --fix).
    let workspace_dir = paths.workspace_dir(&name);
    let inherited_for_populate = inherited.clone();
    let name_for_populate = name.clone();
    atomic_dir::land_directory(&workspace_dir, 0o700, move |staged| {
        let settings_body =
            render_settings_toml(name_for_populate.as_str(), &inherited_for_populate);
        std::fs::write(staged.join("settings.toml"), settings_body.as_bytes())
            .map_err(TomeError::Io)?;
        // US2.a-2 owns the real RULES.md body (summariser output). Until
        // then, ship an empty placeholder so the file exists for the
        // `workspace use` sync pickup.
        std::fs::write(staged.join("RULES.md"), b"").map_err(TomeError::Io)?;
        Ok(())
    })?;

    // Drop the lock at end-of-scope.
    drop(lock);
    // `new_workspace_id` is informational — useful for debugging.
    let _ = new_workspace_id;

    Ok(InitOutcome {
        name,
        workspace_dir,
        inherited_catalogs: inherited_count,
    })
}

/// Render the initial `<root>/workspaces/<name>/settings.toml`. The
/// shape is the Phase 4 [`crate::settings::WorkspaceSettings`] but
/// emitted hand-rolled to keep the formatting human-friendly (TOML
/// arrays of tables, no `summaries` section until US2.a-2 fills it).
fn render_settings_toml(name: &str, catalogs: &[workspace_catalogs::CatalogEnrolment]) -> String {
    let mut out = String::new();
    out.push_str(&format!("name = \"{}\"\n", escape_toml_basic(name)));
    for entry in catalogs {
        out.push_str("\n[[catalogs]]\n");
        out.push_str(&format!(
            "name = \"{}\"\n",
            escape_toml_basic(&entry.catalog_name)
        ));
        out.push_str(&format!("url = \"{}\"\n", escape_toml_basic(&entry.url)));
        out.push_str(&format!(
            "ref = \"{}\"\n",
            escape_toml_basic(&entry.pinned_ref)
        ));
    }
    out
}

/// Minimal TOML basic-string escape: backslash + double-quote are the
/// only metacharacters that show up in workspace names / catalog
/// metadata in practice. The `WorkspaceName` newtype already restricts
/// the name's charset; catalog URLs may contain neither character.
fn escape_toml_basic(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_settings_with_no_catalogs() {
        let body = render_settings_toml("ws", &[]);
        assert_eq!(body, "name = \"ws\"\n");
    }

    #[test]
    fn render_settings_with_two_catalogs() {
        let entries = vec![
            workspace_catalogs::CatalogEnrolment {
                workspace_name: "global".into(),
                catalog_name: "a".into(),
                url: "https://example.com/a".into(),
                pinned_ref: "main".into(),
            },
            workspace_catalogs::CatalogEnrolment {
                workspace_name: "global".into(),
                catalog_name: "b".into(),
                url: "https://example.com/b".into(),
                pinned_ref: "v1".into(),
            },
        ];
        let body = render_settings_toml("test-ws", &entries);
        let expected = "name = \"test-ws\"\n\n\
[[catalogs]]\n\
name = \"a\"\n\
url = \"https://example.com/a\"\n\
ref = \"main\"\n\n\
[[catalogs]]\n\
name = \"b\"\n\
url = \"https://example.com/b\"\n\
ref = \"v1\"\n";
        assert_eq!(body, expected);
    }
}
