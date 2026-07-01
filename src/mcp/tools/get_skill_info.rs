//! `get_skill_info` MCP tool — middle-tier metadata + resource enumeration.
//!
//! Sits between [`search_skills`](super::search_skills) (small ranked list with
//! truncated descriptions) and [`get_skill`](super::get_skill) (full body). The
//! middle tier returns the full description + `when_to_use` guidance + a
//! capped enumeration of the entry's adjacent resources so the calling agent
//! can decide whether to fetch the full body.
//!
//! Contract: [`mcp-tools-p5.md` § `get_skill_info`](../../../specs/005-phase-5-commands-prompts/contracts/mcp-tools-p5.md).
//!
//! Phase 5 / US4.a (T303–T308).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use rmcp::ErrorData as McpError;
use rmcp::model::ErrorCode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{error, info};

use crate::error::TomeError;
use crate::index::skills;
use crate::mcp::state::McpState;
use crate::plugin::identity::EntryKind;

/// Resource enumeration cap. Top-level files and each subdirectory's
/// listing are clipped to this many entries; the overflow is collapsed
/// into the sentinel `"and N more"` string appended to the array.
const PER_DIRECTORY_CAP: usize = 5;

/// The tool description per `mcp-tools-p5.md` § `get_skill_info` lives on the
/// `#[tool]`-decorated method in `mcp::server` as a doc comment.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    /// Disambiguator when a plugin ships entries with the same name across
    /// kinds. Defaults to `skill` per FR-084.
    #[serde(default = "default_kind")]
    pub kind: EntryKind,
}

fn default_kind() -> EntryKind {
    EntryKind::Skill
}

/// Output shape mirrors `contracts/mcp-tools-p5.md` § Output (skill-kind).
///
/// `description` is the FULL frontmatter description (no truncation — that's
/// `search_skills`' job per FR-082). `resources` is `None` for command-kind
/// entries per FR-083.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SkillInfo {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    pub kind: EntryKind,
    /// Absolute path to the entry body on disk.
    pub path: String,
    /// Full frontmatter `description` (NOT truncated).
    pub description: String,
    /// Optional `when_to_use` guidance text.
    pub when_to_use: Option<String>,
    pub plugin_version: String,
    pub user_invocable: bool,
    /// Resource enumeration. Omitted entirely (via `skip_serializing_if`) for
    /// command-kind entries per FR-083.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceEnumeration>,
    /// #289: the MCP prompt name this entry is reachable under via
    /// `prompts/list` / `prompts/get` (`<plugin>__<entry>` form, post-override
    /// and post-collision-suffix). Present for any user-invocable entry; absent
    /// (`skip_serializing_if`) when the entry has no prompt — so the existing
    /// skill-kind / command-kind wire shapes stay byte-stable for non-invocable
    /// entries. Resolved from the live `PromptRegistry` (the SSOT). Appended
    /// LAST so the additive field never reorders the pinned fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_name: Option<String>,
}

/// Per-entry resource enumeration. `files` carries top-level files in the
/// entry's parent directory (excluding the entry body itself); `directories`
/// carries each immediate subdirectory keyed by name with the alphabetised
/// list of children. Both axes are capped at [`PER_DIRECTORY_CAP`] entries;
/// overflow collapses into the sentinel string `"and {N} more"` appended to
/// the array.
///
/// The `directories` map uses [`BTreeMap`] so JSON serialisation produces
/// alphabetical key order — the contract pins this for byte-stability.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ResourceEnumeration {
    pub files: Vec<String>,
    pub directories: BTreeMap<String, Vec<String>>,
}

/// Pipeline:
///
/// 1. Validate non-empty `catalog` / `plugin` / `name`.
/// 2. Look up `(catalog, plugin, kind, name)` in the index, requiring
///    `enabled = 1`. #295: a miss is classified via the shared
///    [`common::classify_not_found`](crate::mcp::tools::common::classify_not_found)
///    SSOT into the SAME three codes `get_skill` uses — `unknown_catalog` /
///    `unknown_plugin` / `unknown_skill` — so an agent never round-trips to
///    learn which layer was wrong (pre-#295 this collapsed to a single
///    `entry_not_found`).
/// 3. Resolve the row's relative `path` to an absolute body path via
///    [`skills::resolve_entry_body_path`].
/// 4. For skill-kind, walk the body's parent directory (one level deep) and
///    enumerate top-level files + immediate subdirectories per the resource
///    enumeration rules.
/// 5. Construct [`SkillInfo`] from the index row + walked resources.
pub async fn handle(state: Arc<McpState>, input: Input) -> Result<SkillInfo, McpError> {
    let started = Instant::now();

    if input.catalog.is_empty() || input.plugin.is_empty() || input.name.is_empty() {
        return Err(McpError::invalid_params(
            "catalog, plugin, and name must be non-empty",
            None,
        ));
    }

    let paths = state.paths.clone();
    let scope = state.scope.scope.clone();
    let catalog = input.catalog.clone();
    let plugin = input.plugin.clone();
    let name = input.name.clone();
    let kind = input.kind;

    let lookup = tokio::task::spawn_blocking(move || {
        lookup_entry(&paths, &scope, &catalog, &plugin, kind, &name)
    })
    .await
    .map_err(|e| internal(&input, started, format!("lookup join: {e}"), "internal"))?
    .map_err(|e| {
        // C-L1: best-effort MCP-surface `tome.error` (closed category only),
        // with this session's `calling_harness`. Never alters the returned
        // `McpError`. The other error arms below are non-`TomeError` lookup/walk
        // outcomes already shaped to the contract codes.
        crate::mcp::enqueue_tool_error(&state, e.category());
        internal(&input, started, e.to_string(), e.category().as_str())
    })?;

    let hit = match lookup {
        LookupOutcome::Found(hit) => hit,
        // #295: emit `get_skill`'s three-code surface — `unknown_catalog` /
        // `unknown_plugin` / `unknown_skill` — with byte-identical messages and
        // `data` shapes, so the two read-side tools report the same code (and
        // the same fields) for the same not-found case. Pre-#295 this collapsed
        // to a single `entry_not_found`, forcing an agent to round-trip to learn
        // whether the catalog, the plugin, or just the entry name was wrong.
        LookupOutcome::NotFound(crate::mcp::tools::common::NotFound::UnknownCatalog) => {
            return Err(emit_error(
                &input,
                started,
                "unknown_catalog",
                McpError::invalid_params(
                    format!(
                        "catalog `{}` is not enabled in the resolved scope",
                        input.catalog
                    ),
                    Some(json!({ "code": "unknown_catalog", "catalog": input.catalog })),
                ),
            ));
        }
        LookupOutcome::NotFound(crate::mcp::tools::common::NotFound::UnknownPlugin) => {
            return Err(emit_error(
                &input,
                started,
                "unknown_plugin",
                McpError::invalid_params(
                    format!(
                        "plugin `{}/{}` is not enabled in the resolved scope",
                        input.catalog, input.plugin
                    ),
                    Some(json!({
                        "code": "unknown_plugin",
                        "catalog": input.catalog,
                        "plugin": input.plugin,
                    })),
                ),
            ));
        }
        LookupOutcome::NotFound(crate::mcp::tools::common::NotFound::UnknownSkill) => {
            return Err(emit_error(
                &input,
                started,
                "unknown_skill",
                McpError::invalid_params(
                    format!(
                        "skill `{}/{}/{}` is not enabled in the resolved scope",
                        input.catalog, input.plugin, input.name,
                    ),
                    Some(json!({
                        "code": "unknown_skill",
                        "catalog": input.catalog,
                        "plugin": input.plugin,
                        "name": input.name,
                    })),
                ),
            ));
        }
    };

    let LookupHit {
        body_path,
        description,
        when_to_use,
        plugin_version,
        user_invocable,
    } = hit;

    // Per FR-083 the resource enumeration is skill-only — commands don't ship
    // with a sibling-files convention (they live at
    // `<plugin>/commands/<name>.md`, not in a per-entry directory).
    let resources = if matches!(input.kind, EntryKind::Skill) {
        let body_path_for_walk = body_path.clone();
        let walked = tokio::task::spawn_blocking(move || walk_resources(&body_path_for_walk))
            .await
            .map_err(|e| internal(&input, started, format!("walk join: {e}"), "internal"))?;
        match walked {
            Ok(r) => Some(r),
            Err(err) => {
                return Err(emit_error(
                    &input,
                    started,
                    "resource_enum_failed",
                    McpError::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("resource enumeration failed: {err}"),
                        Some(json!({
                            "code": "resource_enum_failed",
                            "path": body_path.display().to_string(),
                        })),
                    ),
                ));
            }
        }
    } else {
        None
    };

    info!(
        target: "tome::mcp::tools::get_skill_info",
        catalog = input.catalog,
        plugin = input.plugin,
        name = input.name,
        kind = input.kind.as_str(),
        result = "ok",
        elapsed_ms = started.elapsed().as_millis() as u64,
        "call",
    );

    // FR-027/FR-028: `tome.entry_info` for the middle-tier lookup, carrying the
    // `rank_bucket` of THIS entry from the preceding search this session (the
    // funnel join). `None` ⇒ no preceding search ranked it ⇒ `RankBucket::None`.
    // Best-effort enqueue (a sub-ms local append; never blocks, never flushes).
    crate::telemetry::emit(crate::telemetry::event::EntryInfo {
        rank: crate::mcp::rank_for(&state, &input.name),
        calling_harness: crate::mcp::calling_harness(&state),
    });

    // #289: resolve the entry's MCP prompt name from the live registry (the
    // SSOT). Present for any user-invocable entry (so a command is actionable
    // straight from the middle tier); `None` for a non-invocable entry — which
    // is consistent with the `user_invocable` flag also returned above. The
    // lookup is a sub-µs in-memory scan (no DB I/O), safe on the reactor.
    let prompt_name = state
        .prompt_registry
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .prompt_name_for(&input.catalog, &input.plugin, input.kind, &input.name)
        .map(str::to_owned);

    Ok(SkillInfo {
        catalog: input.catalog,
        plugin: input.plugin,
        name: input.name,
        kind: input.kind,
        path: body_path.display().to_string(),
        description,
        when_to_use,
        plugin_version,
        user_invocable,
        resources,
        prompt_name,
    })
}

struct LookupHit {
    body_path: PathBuf,
    description: String,
    when_to_use: Option<String>,
    plugin_version: String,
    user_invocable: bool,
}

enum LookupOutcome {
    Found(LookupHit),
    /// #295: no enabled entry matched — carries the shared classification so the
    /// handler can emit `get_skill`'s three-code surface (`unknown_catalog` /
    /// `unknown_plugin` / `unknown_skill`) instead of the pre-#295 collapsed
    /// `entry_not_found`.
    NotFound(crate::mcp::tools::common::NotFound),
}

fn lookup_entry(
    paths: &crate::paths::Paths,
    scope: &crate::workspace::Scope,
    catalog: &str,
    plugin: &str,
    kind: EntryKind,
    name: &str,
) -> Result<LookupOutcome, TomeError> {
    let conn = crate::index::db::open_read_only(&paths.index_db)?;
    let workspace_name = scope.name().as_str();
    match skills::find(&conn, workspace_name, catalog, plugin, kind, name)? {
        Some(row) if row.enabled => {
            let body_path = skills::resolve_entry_body_path(
                &conn,
                paths,
                workspace_name,
                catalog,
                plugin,
                &row.path,
            )?;
            Ok(LookupOutcome::Found(LookupHit {
                body_path,
                description: row.description,
                when_to_use: row.when_to_use,
                plugin_version: row.plugin_version,
                user_invocable: row.user_invocable,
            }))
        }
        // #295: the entry didn't resolve to an enabled row (absent, or present
        // only as a disabled row). Classify *which* layer was missing via the
        // shared `common::classify_not_found` SSOT — the same catalog/plugin
        // guards `get_skill` uses — so `get_skill_info` reports
        // `unknown_catalog` / `unknown_plugin` / `unknown_skill` with the same
        // precedence, sparing the agent a second round-trip. (Pre-#295 this
        // collapsed both "row absent" and "row present but disabled" onto a
        // single `entry_not_found` envelope.)
        Some(_) | None => Ok(LookupOutcome::NotFound(
            crate::mcp::tools::common::classify_not_found(&conn, workspace_name, catalog, plugin)?,
        )),
    }
}

/// Enumerate the entry's parent directory per
/// `contracts/mcp-tools-p5.md` § Resource enumeration rules:
///
/// - `files`: top-level files in the parent directory, excluding the entry
///   body itself, sorted alphabetically by basename. First [`PER_DIRECTORY_CAP`]
///   entries are returned as absolute paths; overflow yields a single
///   `"and N more"` sentinel appended to the array.
/// - `directories`: immediate subdirectories sorted alphabetically by name;
///   for each, the immediate children (NOT recursed) sorted alphabetically by
///   basename. Same cap + sentinel per subdirectory. [`BTreeMap`] guarantees
///   the JSON object's key order matches alphabetical iteration.
/// - Symlinks (file or dir) are skipped at every level — same hostile-catalog
///   defence as `get_skill::walk_dir` (FR-S-02).
///
/// US4.d C-1 (accepted-risk note): after the per-entry `file_type()` lstat
/// check the parent walk collects subdir `PathBuf`s into a Vec, then a
/// second loop calls `read_dir(&sub)` on each. There's a residual TOCTOU
/// window: between the lstat check and the second `read_dir`, a hostile
/// concurrent `rename(2)` could swap a real directory for a symlink, and
/// the second `read_dir` would follow it. Accepted per Phase 4's trust
/// model — the walked directory is inside a catalog clone the user has
/// EXPLICITLY enabled (trusted-on-enrol, not trusted-on-read). Hardening
/// to per-FD `openat`/`O_NOFOLLOW` would require `cap-std`; deferred to
/// v0.6+ if a real threat materialises.
fn walk_resources(body_path: &Path) -> std::io::Result<ResourceEnumeration> {
    let parent = body_path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "entry path `{}` has no parent directory",
                body_path.display()
            ),
        )
    })?;

    // Collect top-level files and immediate subdirs in one pass; sort each
    // axis after collection so the per-directory walks below stay
    // alphabetical with no extra sort.
    let mut files: Vec<PathBuf> = Vec::new();
    let mut subdirs: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(parent)? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        // Defence in depth: `file_type()` uses lstat (does NOT follow
        // symlinks), so a symlink shows up as `is_symlink() == true`. The
        // explicit skip mirrors `get_skill::walk_dir` — informational
        // enumeration must not surface attacker-chosen paths.
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            subdirs.push(path);
        } else if ft.is_file() && path != body_path {
            files.push(path);
        }
        // Other file types (sockets, fifos, …) are silently skipped — they
        // can't be useful resource references.
    }

    files.sort_by(|a, b| basename_cmp(a, b));
    subdirs.sort_by(|a, b| basename_cmp(a, b));

    let files_out = clip_and_sentinel(files.iter().map(|p| p.display().to_string()).collect());

    let mut directories: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for sub in subdirs {
        let name = sub
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| sub.display().to_string());
        let children = list_dir_children(&sub)?;
        directories.insert(name, children);
    }

    Ok(ResourceEnumeration {
        files: files_out,
        directories,
    })
}

/// List one subdirectory's immediate children (files only — recursion is
/// intentionally NOT performed; the contract pins one-level enumeration).
/// Returns the alphabetised + clipped + sentinel'd list of absolute paths.
fn list_dir_children(dir: &Path) -> std::io::Result<Vec<String>> {
    let mut children: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_symlink() {
            continue;
        }
        if ft.is_file() {
            children.push(path);
        }
        // Nested subdirectories beneath the first level are NOT enumerated.
    }
    children.sort_by(|a, b| basename_cmp(a, b));
    Ok(clip_and_sentinel(
        children.iter().map(|p| p.display().to_string()).collect(),
    ))
}

/// Apply the `"and N more"` sentinel rule per the contract: if `items` fits
/// inside [`PER_DIRECTORY_CAP`], return it unchanged; otherwise truncate to
/// the cap and append `"and {N} more"` where N = omitted count.
fn clip_and_sentinel(items: Vec<String>) -> Vec<String> {
    if items.len() <= PER_DIRECTORY_CAP {
        return items;
    }
    let omitted = items.len() - PER_DIRECTORY_CAP;
    let mut out: Vec<String> = items.into_iter().take(PER_DIRECTORY_CAP).collect();
    out.push(format!("and {omitted} more"));
    out
}

/// Compare two paths by basename so the sorts above produce the
/// alphabetical-by-name ordering the contract pins (full-path sorts would
/// be position-dependent under tempdirs).
fn basename_cmp(a: &Path, b: &Path) -> std::cmp::Ordering {
    let an = a.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    let bn = b.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    an.cmp(&bn)
}

/// Build the `internal_error` envelope plus an error log event. Mirrors
/// `mcp::tools::get_skill::internal` so both surfaces emit identically
/// shaped log records.
fn internal(input: &Input, started: Instant, msg: String, code: &str) -> McpError {
    let scrubbed = crate::catalog::git::scrub_to_string(msg.as_bytes());
    error!(
        target: "tome::mcp::tools::get_skill_info",
        catalog = input.catalog,
        plugin = input.plugin,
        name = input.name,
        kind = input.kind.as_str(),
        error_code = code,
        error_message = %scrubbed,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "tool error",
    );
    McpError::internal_error(msg, Some(json!({ "code": code })))
}

/// Log the contract-recognised error variants, then return the pre-built
/// `McpError` unchanged. Mirrors `mcp::tools::get_skill::emit_error`.
fn emit_error(input: &Input, started: Instant, code: &str, err: McpError) -> McpError {
    info!(
        target: "tome::mcp::tools::get_skill_info",
        catalog = input.catalog,
        plugin = input.plugin,
        name = input.name,
        kind = input.kind.as_str(),
        result = code,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "call",
    );
    err
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_under_cap_returns_unchanged() {
        let items: Vec<String> = (0..PER_DIRECTORY_CAP)
            .map(|i| format!("item-{i}"))
            .collect();
        let clipped = clip_and_sentinel(items.clone());
        assert_eq!(clipped, items);
    }

    #[test]
    fn clip_at_cap_returns_unchanged() {
        let items: Vec<String> = (0..PER_DIRECTORY_CAP)
            .map(|i| format!("item-{i}"))
            .collect();
        assert_eq!(items.len(), PER_DIRECTORY_CAP);
        let clipped = clip_and_sentinel(items.clone());
        assert_eq!(clipped, items, "exactly-at-cap must NOT add sentinel");
    }

    #[test]
    fn clip_over_cap_truncates_and_appends_sentinel() {
        let total = PER_DIRECTORY_CAP + 3;
        let items: Vec<String> = (0..total).map(|i| format!("item-{i:02}")).collect();
        let clipped = clip_and_sentinel(items);
        assert_eq!(clipped.len(), PER_DIRECTORY_CAP + 1);
        assert_eq!(clipped[PER_DIRECTORY_CAP], "and 3 more");
        // First PER_DIRECTORY_CAP entries are kept in order.
        for (idx, val) in clipped.iter().take(PER_DIRECTORY_CAP).enumerate() {
            assert_eq!(val, &format!("item-{idx:02}"));
        }
    }

    #[test]
    fn basename_cmp_orders_by_filename_not_full_path() {
        // `b/aaa` should sort before `a/zzz` when keyed by basename.
        let p1 = PathBuf::from("/tmp/b/aaa");
        let p2 = PathBuf::from("/tmp/a/zzz");
        assert_eq!(basename_cmp(&p1, &p2), std::cmp::Ordering::Less);
    }

    #[test]
    fn walk_resources_alphabetises_files_and_skips_entry() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();
        let entry = dir.join("SKILL.md");
        std::fs::write(&entry, "body").unwrap();
        std::fs::write(dir.join("zebra.txt"), "z").unwrap();
        std::fs::write(dir.join("apple.txt"), "a").unwrap();
        std::fs::write(dir.join("mango.txt"), "m").unwrap();

        let r = walk_resources(&entry).unwrap();
        assert_eq!(r.files.len(), 3);
        // Alphabetical by basename + the entry itself excluded.
        assert!(r.files[0].ends_with("apple.txt"));
        assert!(r.files[1].ends_with("mango.txt"));
        assert!(r.files[2].ends_with("zebra.txt"));
        assert!(r.directories.is_empty());
    }

    #[test]
    fn walk_resources_caps_top_level_files_with_sentinel() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();
        let entry = dir.join("SKILL.md");
        std::fs::write(&entry, "body").unwrap();
        for i in 0..7 {
            std::fs::write(dir.join(format!("file-{i:02}.txt")), "x").unwrap();
        }
        let r = walk_resources(&entry).unwrap();
        assert_eq!(r.files.len(), PER_DIRECTORY_CAP + 1);
        assert_eq!(r.files[PER_DIRECTORY_CAP], "and 2 more");
    }

    #[test]
    fn walk_resources_enumerates_subdirs_alphabetically_with_per_dir_cap() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();
        let entry = dir.join("SKILL.md");
        std::fs::write(&entry, "body").unwrap();

        let examples = dir.join("examples");
        std::fs::create_dir(&examples).unwrap();
        std::fs::write(examples.join("basic.ts"), "x").unwrap();
        std::fs::write(examples.join("advanced.ts"), "x").unwrap();

        let scripts = dir.join("scripts");
        std::fs::create_dir(&scripts).unwrap();
        for i in 0..7 {
            std::fs::write(scripts.join(format!("step-{i:02}.sh")), "x").unwrap();
        }

        let r = walk_resources(&entry).unwrap();
        // BTreeMap iteration is alphabetical: examples, scripts.
        let keys: Vec<&str> = r.directories.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["examples", "scripts"]);

        let examples_out = r.directories.get("examples").unwrap();
        assert_eq!(examples_out.len(), 2);
        assert!(examples_out[0].ends_with("advanced.ts"));
        assert!(examples_out[1].ends_with("basic.ts"));

        let scripts_out = r.directories.get("scripts").unwrap();
        assert_eq!(scripts_out.len(), PER_DIRECTORY_CAP + 1);
        assert_eq!(scripts_out[PER_DIRECTORY_CAP], "and 2 more");
    }

    #[cfg(unix)]
    #[test]
    fn walk_resources_skips_symlinks() {
        // Symlink target must live OUTSIDE the entry's parent directory,
        // otherwise the real target would also be enumerated as a normal
        // sibling and the assertion below would be ambiguous about which
        // path the symlink-skip actually elided.
        let entry_tmp = tempfile::TempDir::new().unwrap();
        let target_tmp = tempfile::TempDir::new().unwrap();
        let dir = entry_tmp.path();
        let entry = dir.join("SKILL.md");
        std::fs::write(&entry, "body").unwrap();
        std::fs::write(dir.join("real.txt"), "r").unwrap();

        let target = target_tmp.path().join("secret.txt");
        std::fs::write(&target, "secret").unwrap();
        std::os::unix::fs::symlink(&target, dir.join("link.txt")).unwrap();

        let r = walk_resources(&entry).unwrap();
        assert_eq!(
            r.files.len(),
            1,
            "symlink must be skipped, got {:?}",
            r.files
        );
        assert!(r.files[0].ends_with("real.txt"));
    }
}
