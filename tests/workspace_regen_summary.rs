//! Phase 4 / US2.a-2 — `tome workspace regen-summary [<name>]` tests.
//!
//! Exercises [`tome::workspace::regen_summary::regen`] using the
//! deterministic [`tome::summarise::StubSummariser`]. The production
//! `LlamaSummariser` is currently a `BackendInitFailed` stub (the
//! production wiring lands in US4.a); these tests verify the library
//! plumbing end-to-end with the stub.

mod common;

use std::path::Path;

use common::{lifecycle_paths, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed};
use tempfile::TempDir;
use time::OffsetDateTime;
use tome::error::{SummariserFailureKind, TomeError};
use tome::index::{self, OpenOptions};
use tome::paths::Paths;
use tome::summarise::{PluginSummariesInput, StubSummariser, Summariser, SummariserOutput};
use tome::workspace::{self, WorkspaceName};

fn parse(name: &str) -> WorkspaceName {
    WorkspaceName::parse(name).expect("valid workspace name")
}

fn open_central(paths: &Paths) -> rusqlite::Connection {
    index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .expect("open central DB")
}

/// Seed one `skills` row + one `workspace_skills` row for the given
/// `(catalog, plugin, name, description)` against `workspace_name`.
fn seed_enabled_skill(
    paths: &Paths,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
    skill_name: &str,
    description: &str,
) {
    let conn = open_central(paths);
    let workspace_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = ?1",
            rusqlite::params![workspace_name],
            |row| row.get(0),
        )
        .expect("lookup workspace_id");
    let now = OffsetDateTime::now_utc().unix_timestamp();
    conn.execute(
        "INSERT INTO skills
           (catalog, plugin, name, description, plugin_version, path, content_hash, indexed_at)
         VALUES (?1, ?2, ?3, ?4, '0.0.0', '/dev/null', 'hash', ?5)",
        rusqlite::params![catalog, plugin, skill_name, description, now],
    )
    .expect("insert skill");
    let skill_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![workspace_id, skill_id, now],
    )
    .expect("insert workspace_skills");
}

fn seed_bound_project(paths: &Paths, workspace_name: &str, project_root: &Path) {
    std::fs::create_dir_all(project_root.join(".tome")).expect("create .tome");
    std::fs::write(
        project_root.join(".tome").join("config.toml"),
        format!("workspace = \"{workspace_name}\"\n"),
    )
    .expect("write project config.toml");
    let conn = open_central(paths);
    let workspace_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = ?1",
            rusqlite::params![workspace_name],
            |row| row.get(0),
        )
        .expect("lookup workspace_id");
    let now = OffsetDateTime::now_utc().unix_timestamp();
    conn.execute(
        "INSERT INTO workspace_projects (project_path, workspace_id, bound_at)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![
            project_root.to_string_lossy().to_string(),
            workspace_id,
            now
        ],
    )
    .expect("seed workspace_projects");
}

#[test]
fn regen_summary_writes_settings_and_rules() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init");
    seed_enabled_skill(&paths, "mine", "cat1", "plugA", "skill-x", "First skill");
    seed_enabled_skill(&paths, "mine", "cat1", "plugA", "skill-y", "Second skill");

    let stub = StubSummariser::new();
    let outcome = workspace::regen_summary::regen(&parse("mine"), &stub, &paths).expect("regen");

    assert!(outcome.short_chars > 0);
    assert!(outcome.long_chars > 0);

    // settings.toml has [summaries] section with three fields.
    let settings_body =
        std::fs::read_to_string(paths.workspace_settings_file(&parse("mine"))).unwrap();
    assert!(
        settings_body.contains("[summaries]"),
        "missing [summaries]: {settings_body}",
    );
    assert!(
        settings_body.contains("short ="),
        "missing short field: {settings_body}",
    );
    assert!(
        settings_body.contains("long ="),
        "missing long field: {settings_body}",
    );
    assert!(
        settings_body.contains("generated_at ="),
        "missing generated_at field: {settings_body}",
    );

    // The original `name = "mine"` scaffold survived the rewrite.
    assert!(
        settings_body.contains("name = \"mine\""),
        "lost `name` field after rewrite: {settings_body}",
    );

    // RULES.md is the long summary body.
    let rules_body = std::fs::read_to_string(paths.workspace_rules_file(&parse("mine"))).unwrap();
    assert!(
        rules_body.contains("This workspace covers"),
        "RULES.md should match stub long summary: {rules_body}",
    );
}

#[test]
fn regen_summary_syncs_bound_projects() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init");
    seed_enabled_skill(&paths, "mine", "cat", "p", "s1", "");

    let project_a = tmp.path().join("project-a");
    let project_b = tmp.path().join("project-b");
    seed_bound_project(&paths, "mine", &project_a);
    seed_bound_project(&paths, "mine", &project_b);

    let stub = StubSummariser::new();
    let outcome = workspace::regen_summary::regen(&parse("mine"), &stub, &paths).expect("regen");

    assert_eq!(outcome.bound_projects_synced, 2);

    let central_rules =
        std::fs::read(paths.workspace_rules_file(&parse("mine"))).expect("read central RULES.md");
    for project in [&project_a, &project_b] {
        let body = std::fs::read(project.join(".tome/RULES.md")).expect("read project RULES.md");
        assert_eq!(
            body,
            central_rules,
            "project {} RULES.md should match central",
            project.display(),
        );
    }
}

#[test]
fn regen_summary_invokes_summariser_with_enabled_plugins() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init");
    seed_enabled_skill(&paths, "mine", "c", "p", "alpha", "describe alpha");
    seed_enabled_skill(&paths, "mine", "c", "p", "beta", "describe beta");

    let stub = StubSummariser::new();
    assert_eq!(stub.call_count(), 0);

    let outcome = workspace::regen_summary::regen(&parse("mine"), &stub, &paths).expect("regen");
    assert_eq!(stub.call_count(), 1);

    // Stub's short = topics.join(", "); topics = skill names. Order is
    // (catalog, plugin, name) per the regen-summary query.
    let settings_body =
        std::fs::read_to_string(paths.workspace_settings_file(&parse("mine"))).unwrap();
    assert!(
        settings_body.contains("alpha, beta") || settings_body.contains("\"alpha, beta\""),
        "settings should carry stub's topic-joined short: {settings_body}",
    );
    assert!(outcome.short_chars >= "alpha, beta".len());

    // Second call increments the counter.
    let _ = workspace::regen_summary::regen(&parse("mine"), &stub, &paths).expect("regen #2");
    assert_eq!(stub.call_count(), 2);
}

/// Inline failing summariser used to verify failure semantics.
struct FailingSummariser;

impl Summariser for FailingSummariser {
    fn summarise(&self, _input: &PluginSummariesInput) -> Result<SummariserOutput, TomeError> {
        Err(TomeError::SummariserFailure {
            kind: SummariserFailureKind::ModelMissing,
        })
    }
}

#[test]
fn regen_summary_failure_keeps_prior_cached_summaries() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init");
    seed_enabled_skill(&paths, "mine", "c", "p", "s", "");

    // Pre-seed a [summaries] block via a successful first run.
    let stub = StubSummariser::new();
    workspace::regen_summary::regen(&parse("mine"), &stub, &paths).expect("seed cache");
    let body_before =
        std::fs::read_to_string(paths.workspace_settings_file(&parse("mine"))).unwrap();
    let rules_before = std::fs::read_to_string(paths.workspace_rules_file(&parse("mine"))).unwrap();

    // Second run with a failing summariser must NOT touch the cache.
    let failing = FailingSummariser;
    let err = workspace::regen_summary::regen(&parse("mine"), &failing, &paths).unwrap_err();
    // T-M4: tighten the matcher to assert the `kind` payload too — the
    // `ModelMissing` discriminant is what makes the test actually
    // exercise the documented failure path.
    assert!(
        matches!(
            err,
            TomeError::SummariserFailure {
                kind: SummariserFailureKind::ModelMissing,
                ..
            }
        ),
        "expected SummariserFailure {{ kind: ModelMissing }}, got {err:?}",
    );
    // Exit code 24 in implementation (closed-set; contract typo'd as
    // 20, see `error.rs` exit_code() note).
    assert_eq!(err.exit_code(), 24);

    let body_after =
        std::fs::read_to_string(paths.workspace_settings_file(&parse("mine"))).unwrap();
    let rules_after = std::fs::read_to_string(paths.workspace_rules_file(&parse("mine"))).unwrap();
    assert_eq!(
        body_before, body_after,
        "prior settings.toml [summaries] must survive failure",
    );
    assert_eq!(
        rules_before, rules_after,
        "prior RULES.md must survive failure",
    );
}

/// Stub that returns oversize summaries to exercise the length-window
/// warning path (FR-425). The value is still cached.
struct OversizeSummariser;

impl Summariser for OversizeSummariser {
    fn summarise(&self, _input: &PluginSummariesInput) -> Result<SummariserOutput, TomeError> {
        Ok(SummariserOutput {
            short: "x".repeat(900),
            long: "y".repeat(3000),
        })
    }
}

#[test]
fn regen_summary_long_window_oversize_is_still_cached() {
    // FR-425: too-long output emits a tracing::warn but the value is
    // still written. We don't try to capture the warn here — the
    // forward-progress assertion (value cached) is the user-facing
    // contract. The tracing layer's behaviour is library-internal.
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init");
    seed_enabled_skill(&paths, "mine", "c", "p", "s", "");

    let s = OversizeSummariser;
    let outcome = workspace::regen_summary::regen(&parse("mine"), &s, &paths).expect("regen");
    assert_eq!(outcome.short_chars, 900);
    assert_eq!(outcome.long_chars, 3000);

    let rules_body = std::fs::read_to_string(paths.workspace_rules_file(&parse("mine"))).unwrap();
    assert_eq!(rules_body.len(), 3000);
}

/// T-M5: a settings.toml that already carries `[[catalogs]]` arrays
/// and a top-level `harnesses` field must survive the regen rewrite —
/// `toml_edit::DocumentMut`'s structural editing leaves untouched keys
/// alone. The new `[summaries]` section must also be present.
#[test]
fn regen_summary_preserves_existing_catalogs_and_harnesses() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init");
    seed_enabled_skill(&paths, "mine", "c", "p", "s", "");

    // Overwrite settings.toml with a richer scaffold than `init`
    // produces, mirroring the shape data-model §6 documents.
    let settings_path = paths.workspace_settings_file(&parse("mine"));
    let pre = "\
name = \"mine\"
harnesses = [\"claude-code\", \"!cursor\"]

[summaries]

[[catalogs]]
name = \"primary\"
url = \"https://example.com/primary.git\"
ref = \"main\"

[[catalogs]]
name = \"secondary\"
url = \"https://example.com/secondary.git\"
ref = \"v1\"
";
    std::fs::write(&settings_path, pre).expect("seed settings.toml");

    let stub = StubSummariser::new();
    let _ = workspace::regen_summary::regen(&parse("mine"), &stub, &paths).expect("regen");

    let post = std::fs::read_to_string(&settings_path).expect("read post");
    // The toml_edit rewrite must keep the developer-authored top-level
    // fields and array-of-tables intact.
    assert!(post.contains("name = \"mine\""), "name field lost: {post}");
    assert!(
        post.contains("harnesses = [\"claude-code\", \"!cursor\"]"),
        "harnesses array lost: {post}",
    );
    assert!(
        post.contains("name = \"primary\""),
        "[[catalogs]] primary entry lost: {post}",
    );
    assert!(
        post.contains("url = \"https://example.com/primary.git\""),
        "primary url lost: {post}",
    );
    assert!(
        post.contains("name = \"secondary\""),
        "[[catalogs]] secondary entry lost: {post}",
    );
    assert!(
        post.contains("url = \"https://example.com/secondary.git\""),
        "secondary url lost: {post}",
    );
    // [summaries] is populated.
    assert!(
        post.contains("[summaries]"),
        "summaries section lost: {post}"
    );
    assert!(
        post.contains("short ="),
        "summaries.short not written: {post}"
    );
    assert!(
        post.contains("long ="),
        "summaries.long not written: {post}"
    );
    assert!(
        post.contains("generated_at = "),
        "summaries.generated_at not written: {post}",
    );
    // C-M5: generated_at lands as an UNQUOTED TOML datetime literal,
    // not a basic-string. The unquoted form has no surrounding double
    // quotes after the `= `.
    let needle = "generated_at = ";
    let idx = post.find(needle).expect("generated_at present");
    let after = &post[idx + needle.len()..];
    assert!(
        !after.starts_with('"'),
        "generated_at must be unquoted datetime literal, got starts-with-quote: {after}",
    );

    // Parse the doc back to confirm `generated_at` is a Datetime not a
    // string. `toml::Value::Datetime` lands when the literal was
    // unquoted; `toml::Value::String` lands when it was quoted.
    let parsed: toml::Value = toml::from_str(&post).expect("re-parse");
    let summaries = parsed
        .get("summaries")
        .expect("summaries table")
        .as_table()
        .expect("summaries is a table");
    let generated = summaries.get("generated_at").expect("generated_at present");
    assert!(
        generated.is_datetime(),
        "generated_at should parse as toml::Value::Datetime, got {generated:?}",
    );
}

#[test]
fn regen_summary_bumps_last_used_at() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init");
    seed_enabled_skill(&paths, "mine", "c", "p", "s", "");

    // Snapshot last_used_at after init.
    let prior: i64 = {
        let conn = open_central(&paths);
        conn.query_row(
            "SELECT last_used_at FROM workspaces WHERE name = 'mine'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };

    // Force a clock gap so the post-regen timestamp can't tie.
    std::thread::sleep(std::time::Duration::from_secs(1));

    let stub = StubSummariser::new();
    let _ = workspace::regen_summary::regen(&parse("mine"), &stub, &paths).expect("regen");

    let post: i64 = {
        let conn = open_central(&paths);
        conn.query_row(
            "SELECT last_used_at FROM workspaces WHERE name = 'mine'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert!(
        post > prior,
        "regen-summary should bump last_used_at (prior={prior}, post={post})",
    );
}

#[test]
fn regen_summary_missing_workspace_exits_13() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    // No `init` for "ghost"; only the privileged `global` is seeded on
    // bootstrap.
    let stub = StubSummariser::new();
    let err = workspace::regen_summary::regen(&parse("ghost"), &stub, &paths).unwrap_err();
    assert!(
        matches!(err, TomeError::WorkspaceNotFound { .. }),
        "expected WorkspaceNotFound, got {err:?}",
    );
    assert_eq!(err.exit_code(), 13);
}
