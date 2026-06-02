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
//! SCOPE — what this gate asserts and what it deliberately does NOT:
//! F-MODEL-FILES is a *download-completeness* bug. The precise behaviour it
//! fixes is: `download_model` must land EVERY file in `entry.files` (the
//! primary `.onnx` PLUS the non-primary files), so `FastembedEmbedder::load`
//! no longer returns `ModelMissing`. This test asserts exactly that. Pre-fix
//! it panics at the "declared file present" check (tokenizer.json absent);
//! post-fix every file lands and `load` succeeds.
//!
//! It does NOT assert that `embed()` returns a vector, because a *separate*
//! pre-existing beta-blocker makes the embedder's currently-pinned primary
//! artefact unusable on Tome's CPU-only `ort` stack: `source_url` points at
//! qdrant's `model_optimized.onnx`, which (per its `ort_config.json`) is a
//! GPU-optimised, fp16, transformer-fused graph. CPU inference fails with
//! `Missing Input: encoder.layer.0.attention.output.LayerNorm.weight` inside
//! a fused `SkipLayerNormalization` op — regardless of how many files we
//! download. That artefact re-pin is out of scope for F-MODEL-FILES (it
//! changes `source_url`/`sha256`/`size_bytes`); see the agent's handoff note
//! for the verified self-contained INT8 replacement.

use tempfile::TempDir;
use tome::embedding::fastembed::FastembedEmbedder;
use tome::embedding::registry::{ModelKind, lookup};
use tome::paths::Paths;

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
    // successful `load` proves the model directory is now complete. (Whether
    // `embed()` then succeeds depends on the artefact itself — see the
    // module-level SCOPE note for the separate primary-artefact blocker.)
    FastembedEmbedder::load(entry, &model_dir)
        .expect("FastembedEmbedder::load must succeed on a complete model dir");
}
