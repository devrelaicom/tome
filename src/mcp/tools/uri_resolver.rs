//! URI resolution for `get_skill` — parse a loose URI into candidate
//! identities, resolve each against the index, collapse to one/many/none.

use std::path::{Path, PathBuf};

use crate::error::TomeError;
use crate::index::skills;
use crate::index::workspace_catalogs;
use crate::paths::Paths;
use crate::plugin::identity::EntryKind;

/// One candidate interpretation of a URI, to be resolved against the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Candidate {
    /// A filesystem path (absolute or relative fragment) to match against
    /// enabled entries' resolved body paths.
    Path(String),
    /// A fully-qualified `(catalog, plugin, name)`.
    Triple {
        catalog: String,
        plugin: String,
        name: String,
    },
    /// A `(plugin, name)` to resolve across all catalogs.
    PluginName { plugin: String, name: String },
    /// A bare entry name to resolve across the whole workspace.
    BareName(String),
}

/// True when `s` should be treated as a filesystem path rather than a
/// delimited name.
fn looks_like_path(s: &str) -> bool {
    s.contains('/')
        || s.contains(std::path::MAIN_SEPARATOR)
        || s.starts_with('.')
        || s.starts_with('~')
        || std::path::Path::new(s).is_absolute()
        || s.ends_with(".md")
}

/// Split `s` on `delim`; return `Triple` (3 parts) or `PluginName` (2 parts),
/// or `None` when the segment count or any segment is invalid.
fn segments_to_candidate(parts: &[&str]) -> Option<Candidate> {
    if parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    match parts.len() {
        3 => Some(Candidate::Triple {
            catalog: parts[0].to_owned(),
            plugin: parts[1].to_owned(),
            name: parts[2].to_owned(),
        }),
        2 => Some(Candidate::PluginName {
            plugin: parts[0].to_owned(),
            name: parts[1].to_owned(),
        }),
        _ => None,
    }
}

/// Parse a loose URI into candidate identities. Pure — performs no I/O. An
/// empty result means the URI is malformed/empty. See the module rules.
pub fn parse_uri(uri: &str) -> Vec<Candidate> {
    let uri = uri.trim();
    if uri.is_empty() {
        return Vec::new();
    }

    if looks_like_path(uri) {
        return vec![Candidate::Path(uri.to_owned())];
    }

    if uri.contains(':') {
        let parts: Vec<&str> = uri.split(':').collect();
        return segments_to_candidate(&parts).into_iter().collect();
    }

    if uri.contains("__") {
        let parts: Vec<&str> = uri.split("__").collect();
        return segments_to_candidate(&parts).into_iter().collect();
    }

    if uri.contains('_') {
        let tokens: Vec<&str> = uri.split('_').collect();
        if tokens.iter().all(|t| t.is_empty()) {
            return Vec::new();
        }
        let mut out: Vec<Candidate> = Vec::new();
        // 2-way partitions → PluginName, skipping any empty-field candidate.
        for i in 1..tokens.len() {
            if let Some(candidate) =
                segments_to_candidate(&[&tokens[..i].join("_"), &tokens[i..].join("_")])
            {
                out.push(candidate);
            }
        }
        // 3-way partitions → Triple, skipping any empty-field candidate.
        for i in 1..tokens.len() {
            for j in (i + 1)..tokens.len() {
                if let Some(candidate) = segments_to_candidate(&[
                    &tokens[..i].join("_"),
                    &tokens[i..j].join("_"),
                    &tokens[j..].join("_"),
                ]) {
                    out.push(candidate);
                }
            }
        }
        // Fallback: the name itself may contain underscores.
        out.push(Candidate::BareName(uri.to_owned()));
        return out;
    }

    // Bare token: try as a relative path fragment AND as a bare name.
    vec![
        Candidate::Path(uri.to_owned()),
        Candidate::BareName(uri.to_owned()),
    ]
}

/// A URI candidate resolved to a concrete enabled entry + its on-disk body.
#[derive(Debug, Clone)]
pub struct ResolvedEntry {
    pub record: skills::SkillRecord,
    pub body_path: PathBuf,
}

/// Lexically normalise a path (resolve `.` and collapse redundant
/// separators) WITHOUT touching the filesystem. `..` is preserved
/// component-for-component so callers can still detect/reject it upstream.
fn normalize_lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// True when any component of `p` is a `..` (parent-dir) segment. The
/// traversal guard `resolve_path` checks before touching the DB, split out
/// so it can be unit-tested directly without a DB/catalog fixture.
fn has_parent_dir_component(p: &Path) -> bool {
    p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
}

/// True when `p` is a symlink (or any of its existing ancestors is).
/// Defence in depth against a hostile catalog committing a symlinked
/// `SKILL.md`.
fn is_symlinked(p: &Path) -> bool {
    let mut cur = Some(p);
    while let Some(c) = cur {
        if let Ok(meta) = std::fs::symlink_metadata(c) {
            if meta.file_type().is_symlink() {
                return true;
            }
        }
        cur = c.parent();
    }
    false
}

/// Pure comparison: does the normalised `target_norm` (or, when present, the
/// canonicalised `target_canon`) match any equivalence form of `body_path`?
///
/// Forms checked: the absolute body path, its parent dir, the parent dir's
/// bare basename, the body path relative to `plugin_root` (+ its parent),
/// and — when `catalog_root` is known — the body path relative to it (+ its
/// parent). Factored out of the DB-backed [`resolve_path`] loop so it can be
/// unit-tested with constructed `PathBuf`s and no filesystem/catalog
/// fixture.
fn body_matches_target(
    target_norm: &Path,
    target_canon: Option<&Path>,
    body_path: &Path,
    plugin_root: &Path,
    catalog_root: Option<&Path>,
) -> bool {
    let body_norm = normalize_lexical(body_path);
    let mut forms: Vec<PathBuf> = vec![body_norm.clone()];

    let parent = body_path.parent().map(Path::to_path_buf);
    if let Some(par) = &parent {
        forms.push(normalize_lexical(par));
        if let Some(base) = par.file_name() {
            forms.push(PathBuf::from(base)); // bare directory basename, e.g. `foo`
        }
    }

    if let Ok(rel) = body_path.strip_prefix(plugin_root) {
        forms.push(normalize_lexical(rel));
        if let Some(rp) = rel.parent() {
            forms.push(normalize_lexical(rp));
        }
    }

    if let Some(croot) = catalog_root {
        if let Ok(rel) = body_path.strip_prefix(croot) {
            forms.push(normalize_lexical(rel));
            if let Some(rp) = rel.parent() {
                forms.push(normalize_lexical(rp));
            }
        }
    }

    // `body_norm` here is the lexically-normalized `body_path`, not a
    // re-canonicalized one — deliberately. `resolve_path` (this function's
    // only caller) already skips symlinked bodies via `is_symlinked` before
    // calling here, so by the time we get `body_path` it has no symlink
    // components left to resolve; lexical normalization is sufficient and
    // avoids a redundant filesystem round-trip.
    forms.iter().any(|f| f == target_norm) || target_canon.is_some_and(|tc| tc == body_norm)
}

/// Resolve a filesystem `target` to enabled entries whose on-disk body
/// matches, per [`body_matches_target`]. Symlinked bodies are skipped.
///
/// DB-backed end-to-end coverage (real `entries_for_workspace` +
/// `resolve_entry_body_path` + the symlink skip) lives in [`resolve`]'s
/// integration tests (`tests/mcp_entries/uri_resolver_int.rs`), where a real
/// catalog + index fixture already exists — `resolve_path` is private and
/// not directly reachable from integration tests, and standing up the
/// content-addressed catalog cache from a `src` unit test is not worth the
/// fixture weight. This module covers the pure pieces
/// (`normalize_lexical`, `has_parent_dir_component`, `body_matches_target`,
/// `is_symlinked`) directly instead.
fn resolve_path(
    conn: &rusqlite::Connection,
    paths: &Paths,
    workspace_name: &str,
    target: &str,
) -> Result<Vec<ResolvedEntry>, TomeError> {
    let raw = Path::new(target);
    // Reject traversal outright.
    if has_parent_dir_component(raw) {
        return Ok(Vec::new());
    }
    let target_norm = normalize_lexical(raw);
    // Canonicalized form when the path exists on disk (absolute inputs).
    let target_canon = std::fs::canonicalize(raw).ok();

    let mut out: Vec<ResolvedEntry> = Vec::new();
    for record in skills::entries_for_workspace(conn, workspace_name)? {
        let body_path = skills::resolve_entry_body_path(
            conn,
            paths,
            workspace_name,
            &record.catalog,
            &record.plugin,
            &record.path,
        )?;
        if is_symlinked(&body_path) {
            continue;
        }
        let plugin_root =
            skills::plugin_root_dir(conn, paths, workspace_name, &record.catalog, &record.plugin)?;
        // The catalog root must come from `resolve_catalog_path`, NOT from
        // `plugin_root.parent()`: `skills::plugin_root_dir` computes
        // `plugin_root = catalog_path.join(&decl.source)`, and a catalog
        // manifest's `source` (see `catalog::manifest::validate_source`) may
        // be a multi-segment relative path (e.g. `vendor/plugin-x`). For
        // such a plugin, `plugin_root.parent()` yields an intermediate
        // directory (e.g. `<catalog_path>/vendor`), not the true catalog
        // root — which would silently break the catalog-relative
        // equivalence forms below.
        let catalog_root =
            workspace_catalogs::resolve_catalog_path(conn, paths, workspace_name, &record.catalog)?;

        if body_matches_target(
            &target_norm,
            target_canon.as_deref(),
            &body_path,
            &plugin_root,
            Some(&catalog_root),
        ) {
            out.push(ResolvedEntry { record, body_path });
        }
    }
    Ok(out)
}

/// The collapsed result of resolving a URI against the index.
#[derive(Debug)]
pub enum ResolveOutcome {
    /// Exactly one enabled entry matched — feed the existing fetch pipeline.
    One(ResolvedEntry),
    /// More than one matched — return previews + next_actions.
    Many(Vec<ResolvedEntry>),
    /// Zero matched — `available` carries the workspace's enabled entries so
    /// the caller can surface guidance without a re-search.
    NoMatch { available: Vec<skills::SkillRecord> },
}

/// Resolve a single non-path candidate to zero or more records.
fn resolve_name(
    conn: &rusqlite::Connection,
    workspace_name: &str,
    candidate: &Candidate,
) -> Result<Vec<skills::SkillRecord>, TomeError> {
    match candidate {
        Candidate::Triple {
            catalog,
            plugin,
            name,
        } => {
            // Try both kinds; caller filters. `find` needs an explicit kind.
            let mut out = Vec::new();
            for kind in [EntryKind::Skill, EntryKind::Command] {
                if let Some(r) = skills::find(conn, workspace_name, catalog, plugin, kind, name)? {
                    if r.enabled {
                        out.push(r);
                    }
                }
            }
            Ok(out)
        }
        Candidate::PluginName { plugin, name } => {
            skills::find_by_plugin_name(conn, workspace_name, plugin, name)
        }
        Candidate::BareName(name) => {
            skills::find_by_name_across_workspace(conn, workspace_name, name)
        }
        Candidate::Path(_) => Ok(Vec::new()), // handled by resolve_path
    }
}

/// Filter `resolved` to `kinds`, de-dupe by `(catalog, plugin, kind, name)`,
/// sort by the same key, and collapse to one/many/none against `available`.
/// Pure — no I/O — so it can be unit-tested with constructed `SkillRecord` /
/// `ResolvedEntry` values and no DB. Factored out of [`resolve`], which
/// supplies `resolved` and `available` from live index queries.
fn collapse(
    mut resolved: Vec<ResolvedEntry>,
    kinds: &[EntryKind],
    available: Vec<skills::SkillRecord>,
) -> ResolveOutcome {
    resolved.retain(|e| kinds.contains(&e.record.kind));
    resolved.sort_by(|a, b| {
        let ka = (
            &a.record.catalog,
            &a.record.plugin,
            a.record.kind.as_str(),
            &a.record.name,
        );
        let kb = (
            &b.record.catalog,
            &b.record.plugin,
            b.record.kind.as_str(),
            &b.record.name,
        );
        ka.cmp(&kb)
    });
    resolved.dedup_by(|a, b| {
        a.record.catalog == b.record.catalog
            && a.record.plugin == b.record.plugin
            && a.record.kind == b.record.kind
            && a.record.name == b.record.name
    });

    match resolved.len() {
        0 => ResolveOutcome::NoMatch { available },
        1 => ResolveOutcome::One(resolved.pop().expect("len==1")),
        _ => ResolveOutcome::Many(resolved),
    }
}

/// Parse + resolve a URI against the workspace index, collapsing to
/// one/many/none. Opens its own read-only connection (as `lookup_entry` does).
pub fn resolve(
    paths: &Paths,
    workspace_name: &str,
    uri: &str,
    kinds: &[EntryKind],
) -> Result<ResolveOutcome, TomeError> {
    let conn = crate::index::db::open_read_only(&paths.index_db)?;

    let mut resolved: Vec<ResolvedEntry> = Vec::new();
    for candidate in parse_uri(uri) {
        match &candidate {
            Candidate::Path(target) => {
                resolved.extend(resolve_path(&conn, paths, workspace_name, target)?);
            }
            _ => {
                for record in resolve_name(&conn, workspace_name, &candidate)? {
                    let body_path = skills::resolve_entry_body_path(
                        &conn,
                        paths,
                        workspace_name,
                        &record.catalog,
                        &record.plugin,
                        &record.path,
                    )?;
                    resolved.push(ResolvedEntry { record, body_path });
                }
            }
        }
    }

    let available = skills::entries_for_workspace(&conn, workspace_name)?;
    Ok(collapse(resolved, kinds, available))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colon_three_segments_is_triple() {
        assert_eq!(
            parse_uri("cat:plug:skill"),
            vec![Candidate::Triple {
                catalog: "cat".into(),
                plugin: "plug".into(),
                name: "skill".into()
            }]
        );
    }

    #[test]
    fn colon_two_segments_is_plugin_name() {
        assert_eq!(
            parse_uri("plug:skill"),
            vec![Candidate::PluginName {
                plugin: "plug".into(),
                name: "skill".into()
            }]
        );
    }

    #[test]
    fn double_underscore_three_segments_is_triple() {
        assert_eq!(
            parse_uri("cat__plug__skill"),
            vec![Candidate::Triple {
                catalog: "cat".into(),
                plugin: "plug".into(),
                name: "skill".into()
            }]
        );
    }

    #[test]
    fn single_underscore_emits_all_partitions_plus_bare() {
        let got = parse_uri("a_b_c");
        // 2-way: (a | b_c), (a_b | c)
        assert!(got.contains(&Candidate::PluginName {
            plugin: "a".into(),
            name: "b_c".into()
        }));
        assert!(got.contains(&Candidate::PluginName {
            plugin: "a_b".into(),
            name: "c".into()
        }));
        // 3-way: (a | b | c)
        assert!(got.contains(&Candidate::Triple {
            catalog: "a".into(),
            plugin: "b".into(),
            name: "c".into()
        }));
        // Bare fallback for a name that itself contains underscores.
        assert!(got.contains(&Candidate::BareName("a_b_c".into())));
    }

    #[test]
    fn absolute_and_dotted_and_md_are_paths() {
        assert_eq!(
            parse_uri("/abs/SKILL.md"),
            vec![Candidate::Path("/abs/SKILL.md".into())]
        );
        assert_eq!(
            parse_uri("./rel/dir"),
            vec![Candidate::Path("./rel/dir".into())]
        );
        assert_eq!(
            parse_uri("SKILL.md"),
            vec![Candidate::Path("SKILL.md".into())]
        );
        assert_eq!(parse_uri("a/b"), vec![Candidate::Path("a/b".into())]);
    }

    #[test]
    fn bare_token_is_path_fragment_and_bare_name() {
        assert_eq!(
            parse_uri("basic-start"),
            vec![
                Candidate::Path("basic-start".into()),
                Candidate::BareName("basic-start".into())
            ]
        );
    }

    #[test]
    fn empty_or_all_delimiters_is_malformed() {
        assert!(parse_uri("").is_empty());
        assert!(parse_uri("   ").is_empty());
        assert!(parse_uri(":").is_empty());
        assert!(parse_uri("::").is_empty());
    }

    #[test]
    fn single_underscore_all_delimiters_is_malformed() {
        assert!(parse_uri("_").is_empty());
        assert!(parse_uri("__").is_empty());
        assert!(parse_uri("___").is_empty());
    }

    #[test]
    fn single_underscore_leading_or_trailing_skips_empty_field_candidates() {
        assert_eq!(parse_uri("_abc"), vec![Candidate::BareName("_abc".into())]);
        assert_eq!(parse_uri("abc_"), vec![Candidate::BareName("abc_".into())]);
    }

    // ---- Task 3: path -> identity resolution ---------------------------
    //
    // `resolve_path` is private and DB-backed (it walks
    // `skills::entries_for_workspace`, which needs a real index + a real
    // on-disk catalog for `plugin_root_dir` to resolve against). Standing
    // that catalog fixture up from a `src` unit test is heavy and not worth
    // it here: `resolve_path` is exercised end-to-end by Task 4's `resolve`
    // tests and Task 7's `StagedWorkspace` integration tests, which already
    // have that fixture. What's covered here instead is every pure piece
    // `resolve_path` is built from, in isolation: `normalize_lexical`,
    // `has_parent_dir_component` (the traversal guard), `body_matches_target`
    // (the per-entry equivalence-form comparison), and `is_symlinked`.

    #[test]
    fn normalize_lexical_collapses_dot_and_redundant_separators() {
        assert_eq!(
            normalize_lexical(Path::new("/a/./b//c")),
            PathBuf::from("/a/b/c")
        );
        assert_eq!(
            normalize_lexical(Path::new("./foo/./bar")),
            PathBuf::from("foo/bar")
        );
        assert_eq!(normalize_lexical(Path::new(".")), PathBuf::new());
    }

    #[test]
    fn normalize_lexical_preserves_parent_dir_components() {
        // `..` is left alone lexically; `resolve_path` rejects it separately
        // via `has_parent_dir_component` rather than normalizing it away.
        assert_eq!(
            normalize_lexical(Path::new("foo/../bar")),
            PathBuf::from("foo/../bar")
        );
    }

    #[test]
    fn has_parent_dir_component_detects_traversal() {
        assert!(has_parent_dir_component(Path::new("../etc/passwd")));
        assert!(has_parent_dir_component(Path::new("foo/../bar")));
        assert!(!has_parent_dir_component(Path::new("foo/bar")));
        assert!(!has_parent_dir_component(Path::new("/abs/path")));
    }

    #[test]
    fn body_matches_target_matches_every_equivalence_form() {
        let body = PathBuf::from("/root/plug/skills/foo/SKILL.md");
        let plugin_root = PathBuf::from("/root/plug");

        let forms = [
            "/root/plug/skills/foo/SKILL.md", // absolute body path
            "/root/plug/skills/foo",          // parent dir
            "foo",                            // parent dir's bare basename
            "skills/foo/SKILL.md",            // relative to plugin root
            "skills/foo",                     // relative parent
        ];
        for form in forms {
            let target_norm = normalize_lexical(Path::new(form));
            assert!(
                body_matches_target(&target_norm, None, &body, &plugin_root, None),
                "expected form {form:?} to match"
            );
        }
    }

    #[test]
    fn body_matches_target_matches_catalog_relative_forms() {
        let body = PathBuf::from("/root/cat/plug/skills/foo/SKILL.md");
        let plugin_root = PathBuf::from("/root/cat/plug");
        let catalog_root = PathBuf::from("/root/cat");

        let target_norm = normalize_lexical(Path::new("plug/skills/foo/SKILL.md"));
        assert!(body_matches_target(
            &target_norm,
            None,
            &body,
            &plugin_root,
            Some(&catalog_root)
        ));

        let target_norm_parent = normalize_lexical(Path::new("plug/skills/foo"));
        assert!(body_matches_target(
            &target_norm_parent,
            None,
            &body,
            &plugin_root,
            Some(&catalog_root)
        ));
    }

    #[test]
    fn body_matches_target_rejects_unrelated_path() {
        let body = PathBuf::from("/root/plug/skills/foo/SKILL.md");
        let plugin_root = PathBuf::from("/root/plug");
        let catalog_root = PathBuf::from("/root");

        let target_norm = normalize_lexical(Path::new("/etc/passwd"));
        assert!(!body_matches_target(
            &target_norm,
            None,
            &body,
            &plugin_root,
            Some(&catalog_root)
        ));
    }

    #[test]
    fn body_matches_target_honours_canonical_form() {
        let body = PathBuf::from("/root/plug/skills/foo/SKILL.md");
        let plugin_root = PathBuf::from("/root/plug");
        // A canonicalized target equal to the body path itself should match
        // even when the lexical target form does not (e.g. it went through
        // a symlinked intermediate directory that canonicalize resolved).
        let target_norm = normalize_lexical(Path::new("/somewhere/else"));
        let target_canon = PathBuf::from("/root/plug/skills/foo/SKILL.md");
        assert!(body_matches_target(
            &target_norm,
            Some(&target_canon),
            &body,
            &plugin_root,
            None
        ));
    }

    #[cfg(unix)]
    #[test]
    fn is_symlinked_true_for_real_symlink_false_for_regular_file() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::TempDir::new().unwrap();
        let real = tmp.path().join("real.txt");
        std::fs::write(&real, "hi").unwrap();
        let link = tmp.path().join("link.txt");
        symlink(&real, &link).unwrap();

        assert!(is_symlinked(&link));
        assert!(!is_symlinked(&real));
    }

    #[cfg(unix)]
    #[test]
    fn is_symlinked_true_for_regular_file_under_symlinked_ancestor_dir() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::TempDir::new().unwrap();
        let real_dir = tmp.path().join("real_dir");
        std::fs::create_dir(&real_dir).unwrap();
        let leaf = real_dir.join("leaf.txt");
        std::fs::write(&leaf, "hi").unwrap();

        let linked_dir = tmp.path().join("linked_dir");
        symlink(&real_dir, &linked_dir).unwrap();
        let leaf_via_link = linked_dir.join("leaf.txt");

        // `leaf_via_link` is itself a regular file (not a symlink), but one
        // of its ancestors (`linked_dir`) is a symlink — `is_symlinked`
        // must still catch that.
        assert!(!leaf_via_link.symlink_metadata().unwrap().is_symlink());
        assert!(is_symlinked(&leaf_via_link));
    }

    // ---- Task 4: `collapse` (filter/dedupe/sort/one-many-none) ---------
    //
    // `resolve` itself is DB-backed (opens a read-only connection, walks
    // `resolve_path` / `resolve_name` against the index) and is exercised
    // end-to-end in `tests/mcp_entries/uri_resolver_int.rs` via the
    // `StagedWorkspace` fixture. `collapse` is the pure decision logic
    // (filter-kinds + dedupe + sort + One/Many/NoMatch) factored out of
    // `resolve` so it can be unit-tested directly with constructed
    // `SkillRecord` / `ResolvedEntry` values and no DB.

    fn mk_record(
        catalog: &str,
        plugin: &str,
        kind: EntryKind,
        name: &str,
        id: i64,
    ) -> skills::SkillRecord {
        skills::SkillRecord {
            id,
            catalog: catalog.to_owned(),
            plugin: plugin.to_owned(),
            name: name.to_owned(),
            kind,
            description: "d".to_owned(),
            plugin_version: "1.0.0".to_owned(),
            path: format!("skills/{name}/SKILL.md"),
            content_hash: "hash".to_owned(),
            when_to_use: None,
            searchable: true,
            user_invocable: true,
            enabled: true,
            indexed_at: "2026-01-01T00:00:00Z".to_owned(),
        }
    }

    fn mk_resolved(
        catalog: &str,
        plugin: &str,
        kind: EntryKind,
        name: &str,
        id: i64,
    ) -> ResolvedEntry {
        ResolvedEntry {
            record: mk_record(catalog, plugin, kind, name, id),
            body_path: PathBuf::from("/x/SKILL.md"),
        }
    }

    #[test]
    fn collapse_dedupes_identical_identities_to_one() {
        let resolved = vec![
            mk_resolved("acme", "plug", EntryKind::Skill, "foo", 1),
            mk_resolved("acme", "plug", EntryKind::Skill, "foo", 1),
        ];
        let out = collapse(resolved, &[EntryKind::Skill], Vec::new());
        match out {
            ResolveOutcome::One(e) => {
                assert_eq!(e.record.catalog, "acme");
                assert_eq!(e.record.name, "foo");
            }
            other => panic!("expected One, got {other:?}"),
        }
    }

    #[test]
    fn collapse_filters_out_kind_not_requested() {
        let resolved = vec![
            mk_resolved("acme", "plug", EntryKind::Skill, "foo", 1),
            mk_resolved("acme", "plug", EntryKind::Command, "foo", 2),
        ];
        let out = collapse(resolved, &[EntryKind::Skill], Vec::new());
        match out {
            ResolveOutcome::One(e) => assert_eq!(e.record.kind, EntryKind::Skill),
            other => panic!("expected One, got {other:?}"),
        }
    }

    #[test]
    fn collapse_two_distinct_identities_returns_many_in_deterministic_order() {
        let resolved = vec![
            mk_resolved("beta", "plug2", EntryKind::Skill, "foo", 2),
            mk_resolved("acme", "plug", EntryKind::Skill, "foo", 1),
        ];
        let out = collapse(resolved, &[EntryKind::Skill], Vec::new());
        match out {
            ResolveOutcome::Many(v) => {
                assert_eq!(v.len(), 2);
                assert_eq!(v[0].record.catalog, "acme");
                assert_eq!(v[1].record.catalog, "beta");
            }
            other => panic!("expected Many, got {other:?}"),
        }
    }

    #[test]
    fn collapse_empty_returns_nomatch_with_available() {
        let available = vec![mk_record("acme", "plug", EntryKind::Skill, "foo", 1)];
        let out = collapse(Vec::new(), &[EntryKind::Skill], available);
        match out {
            ResolveOutcome::NoMatch { available } => {
                assert!(available.iter().any(|r| r.name == "foo"));
            }
            other => panic!("expected NoMatch, got {other:?}"),
        }
    }

    #[test]
    fn collapse_exactly_one_returns_one() {
        let resolved = vec![mk_resolved("acme", "plug", EntryKind::Skill, "foo", 1)];
        let out = collapse(resolved, &[EntryKind::Skill], Vec::new());
        match out {
            ResolveOutcome::One(e) => assert_eq!(e.record.name, "foo"),
            other => panic!("expected One, got {other:?}"),
        }
    }
}
