//! `StubSummariser` correctness + call-count assertions.
//!
//! F6 ships the summariser skeleton: the trait surface, the
//! [`tome::summarise::LlamaBackend`] singleton, the prompt + length
//! constants, and the deterministic `StubSummariser` exercised here.
//! Real-model production wiring lands in US4.a; until then this stub
//! is the only `Summariser` impl the test suite uses.
//!
//! Coverage:
//!
//! * `new()` produces a fresh stub with `call_count() == 0`.
//! * `summarise` returns deterministic `(short, long)` content
//!   addressed against the input's flattened skill names.
//! * `summarise` is idempotent — invoking twice with the same input
//!   produces byte-identical output (the stub is pure).
//! * `call_count()` increments exactly once per `summarise` call.
//! * Clones share the call counter (the `Arc<AtomicU64>` discipline
//!   the stub inherits from `StubEmbedder`).
//! * Empty input is not an error — the contract pins emptiness as a
//!   production-side concern (output-empty → exit 24), not an input
//!   precondition.

use tome::summarise::{
    PluginSummariesInput, PluginSummaryItem, SkillSummaryItem, StubSummariser, Summariser,
};

fn skill(name: &str, desc: &str) -> SkillSummaryItem {
    SkillSummaryItem {
        name: name.to_owned(),
        description: desc.to_owned(),
    }
}

fn plugin(catalog: &str, name: &str, skills: Vec<SkillSummaryItem>) -> PluginSummaryItem {
    PluginSummaryItem {
        catalog: catalog.to_owned(),
        plugin: name.to_owned(),
        description: String::new(),
        skills,
    }
}

fn input_two_plugins() -> PluginSummariesInput {
    PluginSummariesInput {
        plugins: vec![
            plugin(
                "core",
                "rust",
                vec![
                    skill("rust-core", "Rust language fundamentals"),
                    skill("cargo", "Cargo build tool"),
                ],
            ),
            plugin(
                "core",
                "ts",
                vec![skill("typescript-core", "TypeScript language")],
            ),
        ],
    }
}

#[test]
fn new_stub_has_zero_call_count() {
    let s = StubSummariser::new();
    assert_eq!(s.call_count(), 0);
}

#[test]
fn summarise_returns_deterministic_short_output() {
    let s = StubSummariser::new();
    let out = s
        .summarise(&input_two_plugins())
        .expect("stub never errors");

    // Per `StubSummariser`'s algorithm: skill names flattened in order,
    // comma-joined.
    assert_eq!(out.short, "rust-core, cargo, typescript-core");
}

#[test]
fn summarise_returns_deterministic_long_output() {
    let s = StubSummariser::new();
    let out = s
        .summarise(&input_two_plugins())
        .expect("stub never errors");
    assert_eq!(
        out.long,
        "This workspace covers: rust-core, cargo, typescript-core. \
         Call search_skills when working on these topics."
    );
}

#[test]
fn summarise_is_idempotent_for_identical_input() {
    let s = StubSummariser::new();
    let a = s
        .summarise(&input_two_plugins())
        .expect("stub never errors");
    let b = s
        .summarise(&input_two_plugins())
        .expect("stub never errors");
    assert_eq!(a.short, b.short);
    assert_eq!(a.long, b.long);
}

#[test]
fn call_count_increments_once_per_invocation() {
    let s = StubSummariser::new();
    let _ = s.summarise(&input_two_plugins()).unwrap();
    assert_eq!(s.call_count(), 1);
    let _ = s.summarise(&input_two_plugins()).unwrap();
    assert_eq!(s.call_count(), 2);
    let _ = s.summarise(&input_two_plugins()).unwrap();
    assert_eq!(s.call_count(), 3);
}

#[test]
fn clones_share_the_call_counter() {
    // The `Arc<AtomicU64>` discipline: cloning the stub produces a
    // sibling that observes the same total — same shape `StubEmbedder`
    // uses for its force-fail-after counter.
    let original = StubSummariser::new();
    let clone = original.clone();
    let _ = original.summarise(&input_two_plugins()).unwrap();
    let _ = clone.summarise(&input_two_plugins()).unwrap();
    assert_eq!(original.call_count(), 2);
    assert_eq!(clone.call_count(), 2);
}

#[test]
fn empty_input_does_not_error() {
    // Empty input is allowed at the stub level — the production-side
    // "output empty → exit 24" rule from the contract is about the
    // model's OUTPUT, not the input. The stub returns an empty short
    // summary and a long summary with no topics.
    let s = StubSummariser::new();
    let out = s
        .summarise(&PluginSummariesInput::default())
        .expect("stub never errors");
    assert_eq!(out.short, "");
    assert!(out.long.starts_with("This workspace covers: ."));
}

#[test]
fn summariser_coerces_to_trait_object() {
    // Tests that the trait surface is `dyn`-compatible — US4 will
    // hold an `Arc<dyn Summariser>` across summary-regeneration
    // triggers.
    let boxed: Box<dyn Summariser> = Box::new(StubSummariser::new());
    let out = boxed
        .summarise(&input_two_plugins())
        .expect("trait dispatch fires the stub");
    assert!(!out.short.is_empty());
}
