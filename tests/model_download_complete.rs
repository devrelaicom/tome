//! Real-model embedder gate (Phase 7 beta hardening).
//!
//! This test downloads the REAL embedder model (~34 MB) over the network and
//! drives the full spec path `download → all-files-present → load → embed`. It
//! is `#[ignore]`d so it never runs in the fast CI lane; run it explicitly
//! with:
//!
//! ```sh
//! cargo test --test model_download_complete -- --ignored --nocapture
//! ```
//!
//! HISTORY — two stacked beta-blockers this gate has guarded:
//!
//! F-MODEL-FILES (download completeness, FIXED #146): `download_model`
//! streamed ONLY the primary `.onnx` artefact and never fetched
//! `tokenizer.json` (declared in `entry.files` since Phase 2). On any real
//! install the model directory was incomplete, so `FastembedEmbedder::load`
//! → `build_tokenizer_files` → `read_required("tokenizer.json")` returned
//! `ModelMissing`. The fix fetches every `entry.files[1..]` from
//! `entry.aux_urls` before the atomic rename. INVARIANTS 1 + 2 below guard it.
//!
//! F-MODEL-ONNX-CPU (wrong primary artefact, FIXED — this branch): the pinned
//! embedder primary used to be qdrant's `model_optimized.onnx`, whose
//! `ort_config.json` is `fp16:true` / `optimize_for_gpu:true` /
//! `enable_transformers_specific_optimizations:true` — a GPU-optimised, fp16,
//! transformer-fused graph, NOT the CPU INT8 model. On Tome's CPU-only `ort`
//! stack `embed()` failed with `Missing Input:
//! encoder.layer.0.attention.output.LayerNorm.weight` inside a fused
//! `SkipLayerNormalization` op — regardless of how many files landed — so
//! `tome query` + MCP `search_skills` returned errors/garbage. The fix re-pins
//! the embedder to `Xenova/bge-small-en-v1.5` `onnx/model_quantized.onnx`, the
//! canonical self-contained CPU INT8 graph (the publisher fastembed-rs itself
//! uses). INVARIANT 3 below — now a HARD len-384 assertion — guards it.
//!
//! With both fixes landed this gate asserts the complete invariant:
//! download lands every declared file, `load` succeeds, and `embed()` returns
//! the canonical length-384 vector of finite values (the `FLOAT[384]` vec0
//! column the whole index assumes). The entire fast suite uses `StubEmbedder`,
//! so none of this is exercised there; this gate is the real-model proof.

use tempfile::TempDir;
use tome::embedding::Embedder;
use tome::embedding::fastembed::FastembedEmbedder;
use tome::embedding::registry::{ModelKind, lookup};
use tome::paths::Paths;

/// Canonical embedding width. Mirrors `src/index/query.rs`'s `FLOAT[384]` vec0
/// column; an embedder that returns any other length cannot be indexed.
const EMBED_DIM: usize = 384;

#[test]
#[ignore = "real network: downloads ~34 MB embedder; run with --ignored"]
fn download_model_leaves_a_complete_loadable_embedder() {
    let tmp = TempDir::new().expect("tempdir");
    let paths = Paths::from_root(tmp.path().to_path_buf());
    std::fs::create_dir_all(&paths.models_dir).expect("create models_dir");

    let entry = lookup("bge-small-en-v1.5").expect("embedder registered");
    assert_eq!(entry.kind, ModelKind::Embedder);

    // Download the primary artefact AND every aux file. `download_model`
    // lands the model at `models_dir/<entry.name>/`.
    let manifest = tome::embedding::download::download_model(entry, &paths.models_dir, None)
        .expect("download_model must succeed against the live upstream");

    let model_dir = paths.model_path(entry.name).expect("model_path");

    // INVARIANT 1 (the F-MODEL-FILES bug): every file the registry declares is
    // present on disk. This is exactly what `verify_embedder_artefacts` /
    // `models list` require, so a freshly-downloaded model must no longer
    // report Corrupt/Missing.
    for file in entry.files {
        let p = model_dir.join(file);
        assert!(
            p.is_file(),
            "declared file `{file}` missing after download_model — \
             this is the F-MODEL-FILES bug (only the primary .onnx was fetched)",
        );
    }
    // The manifest must list the same set (it serialises `entry.files`).
    assert_eq!(
        manifest.files,
        entry
            .files
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>(),
    );

    // INVARIANT 2 (the F-MODEL-FILES bug): the real embedder LOADS. A
    // successful `load` proves the model directory is complete (in particular
    // `tokenizer.json` is present — `build_tokenizer_files` reads it via
    // `read_required`).
    let embedder = FastembedEmbedder::load(entry, &model_dir)
        .expect("FastembedEmbedder::load must succeed on a complete model dir");

    // INVARIANT 3 (F-MODEL-ONNX-CPU — the CPU-compatibility fix): `embed()`
    // must succeed on Tome's CPU-only `ort` stack and return EXACTLY a
    // length-384 vector of finite values. This is the proof the re-pinned
    // Xenova INT8 artefact runs on CPU `ort` (the prior GPU/fp16-fused qdrant
    // graph failed here with a `SkipLayerNormalization` / `LayerNorm` /
    // `Missing Input` error). A different length would mean the artefact does
    // not match the index's `FLOAT[384]` vec0 column.
    let vector = embedder
        .embed("a representative query for the recall gate")
        .expect(
            "embed() must succeed on the CPU-only ort stack — a LayerNorm / \
             SkipLayerNormalization / Missing-Input failure here means the pinned \
             primary artefact is GPU/fp16-fused again (F-MODEL-ONNX-CPU regression)",
        );
    assert_eq!(
        vector.len(),
        EMBED_DIM,
        "embed() returned width {} (expected {EMBED_DIM}); the pinned artefact \
         does not match the index's FLOAT[{EMBED_DIM}] vec0 column",
        vector.len(),
    );
    assert!(
        vector.iter().all(|v| v.is_finite()),
        "embed() returned a non-finite value (NaN/inf) — the CPU INT8 graph \
         produced garbage rather than a usable embedding",
    );
}
