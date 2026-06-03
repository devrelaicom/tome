//! Integration tests for [`StubEmbedder`] / [`StubReranker`].
//!
//! The stub stands in for the real ONNX-backed embedder in CI: it never
//! touches a model file, runs in microseconds, and is exercised here for
//! the three properties later layers rely on (research §R10):
//!
//! * **Length** — every vector is exactly 384 elements.
//! * **Determinism** — same input always produces the same vector.
//! * **Distinguishability** — different inputs produce vectors whose cosine
//!   similarity is `< 0.99`, matching the real model's separation behaviour
//!   for non-near-duplicate inputs.
//!
//! Spec: tasks.md T058, research §R10.

use tome::embedding::Embedder;
use tome::embedding::stub::{ReverseStubReranker, StubEmbedder, StubReranker};
use tome::embedding::{Reranker, Scored};
use tome::index::query::Candidate;

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    dot / (na.sqrt() * nb.sqrt()).max(1e-12)
}

#[test]
fn vector_length_is_384() {
    let embedder = StubEmbedder::new();
    for text in [
        "",
        "short",
        "a much longer passage that spans many tokens and characters",
    ] {
        let v = embedder.embed(text).expect("embed");
        assert_eq!(v.len(), 384, "input `{text}` produced {} dims", v.len());
    }
}

#[test]
fn embedding_is_deterministic() {
    let embedder = StubEmbedder::new();
    let a = embedder
        .embed("Compact contract for a simple counter")
        .expect("first");
    let b = embedder
        .embed("Compact contract for a simple counter")
        .expect("second");
    assert_eq!(a, b, "identical input must produce identical vector");
}

#[test]
fn different_inputs_produce_distinguishable_vectors() {
    let embedder = StubEmbedder::new();
    let pairs = [
        ("react component", "rust struct"),
        ("python script", "kubernetes manifest"),
        ("sha256 of nothing", "sha256 of nothing much"),
    ];
    for (left, right) in pairs {
        let a = embedder.embed(left).expect("left");
        let b = embedder.embed(right).expect("right");
        let sim = cosine(&a, &b);
        assert!(
            sim < 0.99,
            "inputs `{left}` and `{right}` produced near-identical vectors (cos={sim:.6})"
        );
    }
}

#[test]
fn embedding_is_l2_normalised() {
    // A consequence of the L2 normalisation in the stub: every vector has
    // unit length within floating-point tolerance.
    let embedder = StubEmbedder::new();
    for text in ["foo", "bar", "baz quux"] {
        let v = embedder.embed(text).expect("embed");
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-5,
            "vector for `{text}` has L2 norm {norm}, expected 1.0"
        );
    }
}

fn synthetic_candidate(name: &str, distance: f32) -> Candidate {
    Candidate {
        skill_id: 0,
        catalog: "cat".to_owned(),
        plugin: "plug".to_owned(),
        name: name.to_owned(),
        kind: tome::plugin::identity::EntryKind::Skill,
        description: format!("description of {name}"),
        plugin_version: "1.0.0".to_owned(),
        path: format!("/tmp/{name}/SKILL.md"),
        distance,
    }
}

#[test]
fn identity_reranker_preserves_order_and_assigns_decreasing_scores() {
    let reranker = StubReranker::new();
    let candidates = vec![
        synthetic_candidate("alpha", 0.1),
        synthetic_candidate("beta", 0.4),
        synthetic_candidate("gamma", 0.7),
    ];
    let scored: Vec<Scored> = reranker.rerank("any query", candidates).expect("rerank");
    assert_eq!(scored.len(), 3);
    assert_eq!(scored[0].candidate.name, "alpha");
    assert_eq!(scored[1].candidate.name, "beta");
    assert_eq!(scored[2].candidate.name, "gamma");
    // Scores are 1 - distance: 0.9, 0.6, 0.3.
    assert!(scored[0].score > scored[1].score);
    assert!(scored[1].score > scored[2].score);
}

#[test]
fn reverse_reranker_flips_order() {
    let reranker = ReverseStubReranker::new();
    let candidates = vec![
        synthetic_candidate("alpha", 0.1),
        synthetic_candidate("beta", 0.4),
        synthetic_candidate("gamma", 0.7),
    ];
    let scored = reranker.rerank("any query", candidates).expect("rerank");
    let names: Vec<&str> = scored.iter().map(|s| s.candidate.name.as_str()).collect();
    assert_eq!(names, vec!["gamma", "beta", "alpha"]);
    // Top of the reversed list still gets the highest score.
    assert!(scored[0].score > scored[1].score);
    assert!(scored[1].score > scored[2].score);
}
