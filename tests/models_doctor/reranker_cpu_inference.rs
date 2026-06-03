//! Real-model reranker CPU-inference gate (F-MODEL-ONNX-CPU, Phase 7).
//!
//! Companion to `model_download_complete.rs`. Where that gate proves the
//! re-pinned EMBEDDER runs on Tome's CPU-only `ort` stack, this one proves the
//! same for the RERANKER — the second stage of `tome query`.
//!
//! The risk is identical: an ONNX artefact built/optimised for GPU + fp16 with
//! transformer-specific graph fusions fails CPU inference with a
//! `Missing Input` / `SkipLayerNormalization` / `LayerNorm` error (this is
//! exactly what bit the embedder's old qdrant pin). The reranker pin —
//! `onnx-community/bge-reranker-base-ONNX/onnx/model_quantized.onnx` — has NO
//! `ort_config.json` (verified 404 at the repo root AND under /onnx/),
//! strongly implying a standard CPU INT8 graph. This test confirms that
//! empirically rather than by inference.
//!
//! It downloads the REAL reranker (~280 MB) over the network, so it is
//! `#[ignore]`d. Run it explicitly with:
//!
//! ```sh
//! cargo test --test reranker_cpu_inference -- --ignored --nocapture
//! ```
//!
//! INVARIANT: `download_model(reranker) → FastembedReranker::load → rerank`
//! returns one finite score per candidate, with NO LayerNorm/Missing-Input
//! CPU-inference failure. If this ever fails with that fused-op signature the
//! reranker pin must be moved to a CPU-safe artefact, exactly as the embedder
//! was.

use tempfile::TempDir;
use tome::embedding::Reranker;
use tome::embedding::fastembed::FastembedReranker;
use tome::embedding::registry::{ModelKind, lookup};
use tome::index::query::Candidate;
use tome::plugin::identity::EntryKind;

/// Build a throwaway `Candidate` for the rerank input. Only `name` +
/// `description` feed the cross-encoder (see `FastembedReranker::rerank`); the
/// rest are identity/scoring fields the reranker never reads.
fn candidate(name: &str, description: &str) -> Candidate {
    Candidate {
        skill_id: 1,
        catalog: "test-catalog".to_owned(),
        plugin: "test-plugin".to_owned(),
        name: name.to_owned(),
        kind: EntryKind::Skill,
        description: description.to_owned(),
        plugin_version: "0.0.0".to_owned(),
        path: "SKILL.md".to_owned(),
        distance: 0.0,
    }
}

#[test]
#[ignore = "real network: downloads ~280 MB reranker; run with --ignored"]
fn reranker_runs_inference_on_the_cpu_ort_stack() {
    let tmp = TempDir::new().expect("tempdir");
    let models_dir = tmp.path().join("models");
    std::fs::create_dir_all(&models_dir).expect("create models_dir");

    let entry = lookup("bge-reranker-base").expect("reranker registered");
    assert_eq!(entry.kind, ModelKind::Reranker);

    // Download primary + every aux file (tokenizer/config) before the rename,
    // landing a complete model dir at `models_dir/<entry.name>/`.
    tome::embedding::download::download_model(entry, &models_dir, None)
        .expect("download_model must succeed against the live upstream");

    let model_dir = models_dir.join(entry.name);
    let reranker = FastembedReranker::load(entry, &model_dir)
        .expect("FastembedReranker::load must succeed on a complete model dir");

    // A tiny, deliberately-ordered (query, [docs]) set: doc 0 is on-topic,
    // doc 1 is off-topic. We assert finite scores + no CPU-inference failure;
    // we deliberately do NOT assert a particular ranking (that would couple the
    // gate to the model's exact numerics — out of scope for a CPU-safety probe).
    let candidates = vec![
        candidate(
            "rust async runtime",
            "Configure and tune the tokio async runtime for a Rust service.",
        ),
        candidate(
            "sourdough starter",
            "Maintain a sourdough starter for home bread baking.",
        ),
    ];
    let n = candidates.len();

    let scored = reranker
        .rerank("how do I set up async in Rust?", candidates)
        .expect(
            "rerank() must succeed on the CPU-only ort stack — a LayerNorm / \
             SkipLayerNormalization / Missing-Input failure here means the pinned \
             reranker artefact is GPU/fp16-fused and a CPU-safe replacement must \
             be pinned (F-MODEL-ONNX-CPU)",
        );

    assert_eq!(
        scored.len(),
        n,
        "rerank() returned {} scored rows for {n} candidates",
        scored.len(),
    );
    assert!(
        scored.iter().all(|s| s.score.is_finite()),
        "rerank() produced a non-finite score (NaN/inf) — the INT8 graph \
         produced garbage rather than usable cross-encoder scores",
    );
}
