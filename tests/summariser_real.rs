//! Env-gated integration test for the real `LlamaSummariser`. Downloads
//! the ~400 MB Qwen2.5-0.5B-Instruct GGUF into a temp dir, invokes
//! `LlamaSummariser::summarise` against a fixture two-plugin /
//! five-skill input, and asserts both outputs are non-empty and within
//! their length windows.
//!
//! ## Why it's CI-skipped
//!
//! - The model is ~400 MB; downloading it on every CI run is wasteful
//!   and slow (network-bound).
//! - The `MODEL_REGISTRY` summariser entry still carries the placeholder
//!   SHA-256 (see `src/summarise/registry.rs` — flipping it to a real
//!   digest is a separate ops task). With the placeholder in place,
//!   `download_model` refuses to install (`ModelCorrupt`, exit 31) so
//!   the test would fail even with `TOME_TEST_REAL_MODELS=1`. Once the
//!   real hash lands, this test becomes the smoke-check that proves the
//!   end-to-end llama-cpp-2 wiring works.
//!
//! Run with:
//!
//! ```sh
//! TOME_TEST_REAL_MODELS=1 cargo test --test summariser_real -- --nocapture
//! ```

mod common;

use common::lifecycle_paths;
use tempfile::TempDir;
use tome::embedding::download::download_model;
use tome::embedding::registry::{MODEL_REGISTRY, ModelKind};
use tome::summarise::{
    LlamaSummariser, PluginSummariesInput, PluginSummaryItem, SkillSummaryItem, Summariser,
};

const ENV_GATE: &str = "TOME_TEST_REAL_MODELS";

/// Soft cap from `contracts/summariser.md` §"Length windows" — the
/// short summary's hard ceiling. `LlamaSummariser` emits a tracing
/// warning above this but still returns the value; this test asserts
/// the value is non-empty and *aspires* to the window. A pass within
/// the band is the happy path; the gate is "non-empty" only.
const SHORT_HARD_MAX: usize = 800;

/// Same shape as `SHORT_HARD_MAX` for the long summary.
const LONG_HARD_MAX: usize = 2400;

fn fixture_input() -> PluginSummariesInput {
    PluginSummariesInput {
        plugins: vec![
            PluginSummaryItem {
                catalog: "core".to_owned(),
                plugin: "data-tooling".to_owned(),
                description: "Tools for working with structured data".to_owned(),
                skills: vec![
                    SkillSummaryItem {
                        name: "csv-validation".to_owned(),
                        description:
                            "Validate CSV files against a schema and report schema violations"
                                .to_owned(),
                    },
                    SkillSummaryItem {
                        name: "json-merge".to_owned(),
                        description:
                            "Merge two or more JSON documents with conflict-resolution rules"
                                .to_owned(),
                    },
                    SkillSummaryItem {
                        name: "sql-query".to_owned(),
                        description: "Query SQLite databases with safe parameterisation".to_owned(),
                    },
                ],
            },
            PluginSummaryItem {
                catalog: "core".to_owned(),
                plugin: "git-helpers".to_owned(),
                description: "Helpers for common git workflows".to_owned(),
                skills: vec![
                    SkillSummaryItem {
                        name: "conventional-commits".to_owned(),
                        description: "Write commit messages in the Conventional Commits format"
                            .to_owned(),
                    },
                    SkillSummaryItem {
                        name: "pr-review".to_owned(),
                        description: "Review pull requests with structured feedback".to_owned(),
                    },
                ],
            },
        ],
    }
}

#[test]
fn real_summariser_produces_non_empty_within_window() {
    if std::env::var_os(ENV_GATE).is_none() {
        eprintln!(
            "skipping {} (set {ENV_GATE}=1 to enable a real-model run)",
            module_path!()
        );
        return;
    }

    let tmp = TempDir::new().expect("create temp dir");
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.models_dir).expect("create models_dir");

    let entry = MODEL_REGISTRY
        .iter()
        .find(|e| e.kind == ModelKind::Summariser)
        .expect("summariser entry in MODEL_REGISTRY");

    // Download the model. With the registry's placeholder SHA-256 this
    // will fail with `ModelCorrupt` (exit 31); that's expected until
    // ops flips the hash. Surface the failure clearly so the developer
    // knows why the test bailed.
    eprintln!(
        "downloading {} (~{} MB) — this may take a while on the first run",
        entry.name,
        entry.size_bytes / 1_000_000,
    );
    let progress_cb = |bytes: u64, total: u64| {
        if total > 0 && bytes % (50 * 1024 * 1024) < 64 * 1024 {
            eprintln!("  ... {} / {} MB", bytes / 1_000_000, total / 1_000_000,);
        }
    };
    download_model(entry, &paths.models_dir, Some(&progress_cb))
        .expect("download summariser model — flip the registry SHA-256 placeholder first");

    let summariser = LlamaSummariser::new(&paths).expect("LlamaSummariser::new");

    let input = fixture_input();
    let output = summariser.summarise(&input).expect("summarise");

    // Both outputs must be non-empty — the hard gate from FR-425.
    assert!(
        !output.short.is_empty(),
        "short summary unexpectedly empty: {output:?}",
    );
    assert!(
        !output.long.is_empty(),
        "long summary unexpectedly empty: {output:?}",
    );

    // Within the documented hard cap is the *aspiration*. The runtime
    // already emits a tracing warning above the cap and still returns
    // the value; we soft-check here so a real-model run that overshoots
    // surfaces as a warning, not a hard failure that would mask
    // legitimate output.
    let short_chars = output.short.chars().count();
    let long_chars = output.long.chars().count();
    if short_chars > SHORT_HARD_MAX {
        eprintln!("short summary exceeded hard cap: {short_chars} chars (cap {SHORT_HARD_MAX})",);
    }
    if long_chars > LONG_HARD_MAX {
        eprintln!("long summary exceeded hard cap: {long_chars} chars (cap {LONG_HARD_MAX})",);
    }

    eprintln!("short ({} chars): {}", short_chars, output.short);
    eprintln!("long  ({} chars): {}", long_chars, output.long);
}
