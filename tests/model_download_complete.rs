//! F-MODEL-FILES empirical repro/regression gate (Phase 7 beta hardening).
//!
//! This test downloads a REAL embedder model (~67 MB) over the network and
//! drives the install → load path (see the SCOPE note below for why it stops
//! at `load`). It is `#[ignore]`d so it never runs in the fast CI lane; run
//! it explicitly with:
//!
//! ```sh
//! cargo test --test model_download_complete -- --ignored --nocapture
//! ```
//!
//! The bug it guards (present since Phase 2): `download_model` streamed ONLY
//! the primary `.onnx` artefact and never fetched `tokenizer.json` (declared
//! in `entry.files` since Phase 2). On any real install the model directory
//! was incomplete, so `FastembedEmbedder::load` → `build_tokenizer_files`
//! → `read_required("tokenizer.json")` returned `ModelMissing`, leaving
//! `tome query` + the MCP `search_skills` tool non-functional. The entire
//! fast suite uses `StubEmbedder`, so it was never caught here; the SC-001
//! real-model recall gate (on another branch) caught it empirically.
//!
//! Mirrors the doc-comment + `--ignored` convention of the SC-001 gate.
//!
//! SCOPE — what this gate asserts and what it deliberately defers:
//! F-MODEL-FILES is a *download-completeness* bug. The precise behaviour it
//! fixes is: `download_model` must land EVERY file in `entry.files` (the
//! primary `.onnx` PLUS the non-primary files), so `FastembedEmbedder::load`
//! no longer returns `ModelMissing`. This test asserts exactly that. Pre-fix
//! it panics at the "declared file present" check (tokenizer.json absent);
//! post-fix every file lands and `load` succeeds.
//!
//! It then drives the full spec path `download → all-files-present → load →
//! embed`, asserting the canonical length-384 vector (the `FLOAT[384]` vec0
//! column the whole index assumes) — BUT behind a tripwire, because a
//! *separate* pre-existing beta-blocker (BLOCKER-2 below) currently makes
//! `embed()` fail on the pinned artefact:
//!
//! BLOCKER-2 (WRONG PRIMARY ARTEFACT PIN — out of scope for F-MODEL-FILES):
//! `bge-small-en-v1.5.source_url` points at qdrant's `model_optimized.onnx`,
//! which (verified live against its `ort_config.json`) is `fp16:true`,
//! `optimize_for_gpu:true`, `enable_transformers_specific_optimizations:true`,
//! `quantization:{}` — a GPU-optimised, fp16, transformer-fused graph, NOT the
//! INT8/quantised model. On Tome's CPU-only `ort` stack `embed()` fails with
//! `Missing Input: encoder.layer.0.attention.output.LayerNorm.weight` inside a
//! fused `SkipLayerNormalization` op — regardless of how many files land.
//! Fixing it requires re-pinning `source_url`/`sha256`/`size_bytes`, which
//! F-MODEL-FILES explicitly forbids; the embed-len-384 acceptance criterion is
//! therefore unsatisfiable on this branch. See the handoff note for the
//! verified self-contained INT8 replacement.
//!
//! Rather than silently skip `embed()`, INVARIANT 3 makes that deferral an
//! ACTIVE TRIPWIRE: it calls `embed()` and accepts ONLY two outcomes — the
//! satisfied spec invariant (a length-384 vector, which flips this test green
//! the instant BLOCKER-2's re-pin lands) or the exact known-bad fused-op
//! signature (which keeps BLOCKER-2 traceable in code). ANY other error —
//! notably a missing tokenizer/config, i.e. an F-MODEL-FILES regression —
//! fails the test. The deferral cannot mask the bug this gate exists to catch.

use tempfile::TempDir;
use tome::embedding::Embedder;
use tome::embedding::fastembed::FastembedEmbedder;
use tome::embedding::registry::{ModelKind, lookup};
use tome::error::TomeError;
use tome::paths::Paths;

/// Canonical embedding width. Mirrors `src/index/query.rs`'s `FLOAT[384]` vec0
/// column; an embedder that returns any other length cannot be indexed.
const EMBED_DIM: usize = 384;

#[test]
#[ignore = "real network: downloads ~67 MB embedder; run with --ignored"]
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
    // report Corrupt/Missing. Pre-fix, `tokenizer.json` is absent here.
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

    // INVARIANT 2 (the F-MODEL-FILES bug): the real embedder LOADS. Pre-fix
    // this returns `ModelMissing { "bge-small-en-v1.5" }` because
    // `build_tokenizer_files` → `read_required("tokenizer.json")` fails. A
    // successful `load` proves the model directory is now complete.
    let embedder = FastembedEmbedder::load(entry, &model_dir)
        .expect("FastembedEmbedder::load must succeed on a complete model dir");

    // INVARIANT 3 (spec path `…→load→embed len 384`, deferred via tripwire):
    // drive `embed()` and accept ONLY two outcomes.
    //
    //   * Ok(v)  → assert `v.len() == EMBED_DIM`. This IS the spec's required
    //     invariant (a); it goes green the instant BLOCKER-2's artefact re-pin
    //     lands (see the module SCOPE note), with no further test edit.
    //   * Err(e) → tolerate ONLY the known BLOCKER-2 signature (a fused-op /
    //     `LayerNorm` / `Missing Input` failure from running qdrant's
    //     GPU/fp16 graph on CPU `ort`). Anything else — above all a
    //     `ModelMissing`/missing-tokenizer error, which is exactly the
    //     F-MODEL-FILES regression this gate guards — fails the test.
    //
    // The deferral is thus self-resolving and cannot hide a download-
    // completeness regression behind "embed is expected to fail".
    match embedder.embed("a representative query for the recall gate") {
        Ok(vector) => assert_eq!(
            vector.len(),
            EMBED_DIM,
            "embed() returned width {} (expected {EMBED_DIM}); the pinned artefact \
             does not match the index's FLOAT[{EMBED_DIM}] vec0 column",
            vector.len(),
        ),
        Err(TomeError::EmbeddingGenerationFailure { detail, .. })
            if is_known_artefact_incompatibility(&detail) =>
        {
            // BLOCKER-2 still open: the wrong primary artefact is pinned. The
            // download is complete (we got past load + into ort inference);
            // the artefact itself is GPU/fp16-fused and unusable on CPU. This
            // is a SECOND beta-blocker, tracked here so it is not lost — it
            // requires the source_url/sha256/size_bytes re-pin that
            // F-MODEL-FILES forbids.
            eprintln!(
                "BLOCKER-2 (artefact re-pin) still open: embed() failed on the \
                 GPU/fp16 `model_optimized.onnx` graph as expected — `{detail}`. \
                 F-MODEL-FILES (download completeness) is satisfied; re-pin the \
                 INT8 artefact to flip INVARIANT 3 green."
            );
        }
        Err(other) => panic!(
            "embed() failed with an UNEXPECTED error — this is not BLOCKER-2's \
             known GPU/fp16 fused-op signature, so it likely signals an \
             F-MODEL-FILES regression (incomplete model dir) or a new defect: \
             {other}"
        ),
    }
}

/// True iff `detail` matches BLOCKER-2's known CPU-vs-GPU/fp16 inference
/// failure: the qdrant `model_optimized.onnx` graph is transformer-fused, so
/// CPU `ort` reports a missing fused-op input (`SkipLayerNormalization` /
/// `LayerNorm` weights). Matching the signature — not blanket-tolerating every
/// embed error — is what keeps a genuine download regression failing the test.
fn is_known_artefact_incompatibility(detail: &str) -> bool {
    let d = detail.to_ascii_lowercase();
    d.contains("layernorm")
        || d.contains("skiplayernormalization")
        || (d.contains("missing input") && d.contains("encoder.layer"))
}
