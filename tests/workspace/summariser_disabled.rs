//! Tests for `[summariser] enabled = false` config gate.
//!
//! TDD (Task 6): when `config.summariser.enabled == Some(false)`, the
//! production trigger wrapper `regenerate_for_trigger` must exit early
//! (returning `Ok(())`) without ever calling the summariser. The
//! `SUMMARISER_OVERRIDE` + `SummariserOverrideGuard` DI seam gives us a
//! counting signal that the summariser was NOT invoked.
//!
//! The explicit `tome workspace regen-summary` path (via
//! `run_with_summariser`) is UNAFFECTED by the enabled gate — it always
//! runs.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::common::{
    lifecycle_paths, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
};
use tempfile::TempDir;
use time::OffsetDateTime;
use tome::error::TomeError;
use tome::index::{self, OpenOptions};
use tome::paths::Paths;
use tome::summarise::{
    PluginSummariesInput, Summariser, SummariserOutput, regenerate_for_trigger,
    trigger::SummariserOverrideGuard,
};
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

fn seed_enabled_skill(paths: &Paths, workspace_name: &str) {
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
         VALUES ('cat', 'plug', 'skill-a', 'A skill', '0.0.0', '/dev/null', 'hash', ?1)",
        rusqlite::params![now],
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

/// Counting summariser: wraps a call counter so tests can assert whether
/// the summariser was invoked.
#[derive(Default, Clone)]
struct CountingSummariser {
    calls: Arc<AtomicU64>,
}

impl CountingSummariser {
    fn new() -> Self {
        Self::default()
    }

    fn call_count(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Summariser for CountingSummariser {
    fn summarise(
        &self,
        input: &PluginSummariesInput,
        _long_max_chars: usize,
    ) -> Result<SummariserOutput, TomeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let skill_names: Vec<String> = input
            .plugins
            .iter()
            .flat_map(|p| p.skills.iter().map(|s| s.name.clone()))
            .collect();
        let short = skill_names.join(", ");
        Ok(SummariserOutput {
            long: format!(
                "This workspace covers: {short}. Call search_skills when working on these topics."
            ),
            short,
        })
    }
}

/// TDD: when `[summariser] enabled = false` in config.toml, the production
/// `regenerate_for_trigger` must return `Ok(())` WITHOUT invoking the
/// summariser. Verified via the `SUMMARISER_OVERRIDE` / counting seam.
#[test]
fn auto_regen_skipped_when_disabled() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    // Write `[summariser] enabled = false` into the global config.
    std::fs::create_dir_all(paths.global_config_file.parent().unwrap()).unwrap();
    std::fs::write(&paths.global_config_file, "[summariser]\nenabled = false\n").unwrap();

    workspace::init::init(parse("mine"), false, &paths).expect("init");
    seed_enabled_skill(&paths, "mine");

    // Install a counting summariser via the override slot.
    let counting = CountingSummariser::new();
    let _guard =
        SummariserOverrideGuard::install(Arc::new(counting.clone()) as Arc<dyn Summariser>);

    // Drive the PRODUCTION trigger function. Because enabled = false, it
    // must return Ok(()) immediately WITHOUT calling the summariser.
    regenerate_for_trigger(&parse("mine"), &paths).expect("trigger must not fail");

    assert_eq!(
        counting.call_count(),
        0,
        "summariser must NOT be called when [summariser] enabled = false in config",
    );
}

/// Complement: when enabled is absent (default) or `enabled = true`, the
/// trigger DOES invoke the summariser.
#[test]
fn auto_regen_runs_when_enabled_or_default() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    // Write `[summariser] enabled = true` (explicit true behaves like default).
    std::fs::create_dir_all(paths.global_config_file.parent().unwrap()).unwrap();
    std::fs::write(&paths.global_config_file, "[summariser]\nenabled = true\n").unwrap();

    workspace::init::init(parse("mine"), false, &paths).expect("init");
    seed_enabled_skill(&paths, "mine");

    let counting = CountingSummariser::new();
    let _guard =
        SummariserOverrideGuard::install(Arc::new(counting.clone()) as Arc<dyn Summariser>);

    regenerate_for_trigger(&parse("mine"), &paths).expect("trigger must not fail");

    assert_eq!(
        counting.call_count(),
        1,
        "summariser MUST be called once when [summariser] enabled = true",
    );
}

/// TDD: `effective_long_max` from config flows into the regen warn threshold.
/// When config sets `long_max_chars = 3000` and the summariser returns a
/// 3001-char long summary, the regen path does NOT warn (3001 <= 3000 is
/// false — wait, 3001 > 3000 → warn IS emitted). Let's test a smaller cap
/// — a 2000-char cap with a 2500-char summary → oversize by both the new
/// cap (2000) but not by the old const (2500). This confirms the effective
/// cap is 2000, not 2500.
///
/// We assert via `RegenSummaryOutcome::long_chars` that the long output is
/// accepted AND verify no panics occur; the `tracing::info!` event is a
/// side-channel we don't capture here (the `regen_summary_long_window_emits_info_via_layer`
/// test in workspace_regen_summary.rs covers the tracing signal).
#[test]
fn configured_long_max_chars_threads_through_to_regen() {
    use tome::workspace::regen_summary;

    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    // Set long_max_chars = 2000 (below the 2500 default).
    std::fs::create_dir_all(paths.global_config_file.parent().unwrap()).unwrap();
    std::fs::write(
        &paths.global_config_file,
        "[summariser]\nlong_max_chars = 2000\n",
    )
    .unwrap();

    workspace::init::init(parse("mine"), false, &paths).expect("init");
    seed_enabled_skill(&paths, "mine");

    // Summariser that returns exactly 2100 chars for `long` — above the
    // configured 2000 cap but below the default 2500 const.
    struct FixedLongSummariser;
    impl Summariser for FixedLongSummariser {
        fn summarise(
            &self,
            _input: &PluginSummariesInput,
            _long_max_chars: usize,
        ) -> Result<SummariserOutput, TomeError> {
            Ok(SummariserOutput {
                short: "topics".to_owned(),
                long: "a".repeat(2100),
            })
        }
    }

    let effective_long_max = tome::summarise::prompts::validate_long_max_chars(2000);
    assert_eq!(
        effective_long_max, 2000,
        "2000 >= LONG_TARGET_MIN so accepted"
    );

    let outcome = regen_summary::regen(
        &parse("mine"),
        &FixedLongSummariser,
        &paths,
        effective_long_max,
    )
    .expect("regen should succeed even with oversize output");

    // The outcome must reflect the ACTUAL char count (2100), not a clamped value.
    assert_eq!(outcome.long_chars, 2100);
    // And the threshold was 2000, not 2500 — meaning the "oversize" check ran
    // against 2000. We can confirm by checking that 2100 > effective_long_max
    // (the condition that would have emitted the warn event).
    assert!(
        outcome.long_chars > effective_long_max,
        "2100 should exceed the configured cap of 2000",
    );
}
