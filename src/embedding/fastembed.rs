//! [`fastembed`](https://crates.io/crates/fastembed) wrappers — the real
//! [`Embedder`] / [`Reranker`] implementations Tome ships in user-stories
//! phase.
//!
//! Each wrapper loads its ONNX model from a directory under
//! `${XDG_DATA_HOME}/tome/models/<name>/` rather than letting fastembed pull
//! from its own cache directory (FR-021 — must be data dir, not cache dir).
//!
//! Slice-5 scope: the type surface compiles and round-trips through
//! `Embedder` / `Reranker`. Real end-to-end exercise (loading actual BGE
//! ONNX bytes, embedding text) lands when the model-download integration
//! test (T057) and the US1 wiring tasks land — those depend on the
//! registry's pinned checksums being verified by CI.

use std::fs;
use std::path::{Path, PathBuf};

use fastembed::{
    InitOptionsUserDefined, RerankInitOptionsUserDefined, TextEmbedding, TextRerank,
    TokenizerFiles, UserDefinedEmbeddingModel, UserDefinedRerankingModel,
};

use crate::embedding::registry::ModelEntry;
use crate::embedding::runtime;
use crate::embedding::{Embedder, Reranker, Scored};
use crate::error::TomeError;
use crate::index::query::Candidate;

/// fastembed-backed text embedder.
pub struct FastembedEmbedder {
    inner: TextEmbedding,
    name: String,
    version: String,
}

impl FastembedEmbedder {
    /// Load the embedder from `model_dir`. Files expected:
    ///
    /// * `model.onnx`
    /// * `tokenizer.json`
    /// * `config.json` (optional)
    /// * `special_tokens_map.json` (optional)
    /// * `tokenizer_config.json` (optional)
    pub fn load(entry: &ModelEntry, model_dir: &Path) -> Result<Self, TomeError> {
        runtime::ensure_initialised()?;
        let onnx = read_required(
            entry,
            &model_dir.join(entry.files.first().copied().unwrap_or("model.onnx")),
        )?;
        let tokenizer = build_tokenizer_files(entry, model_dir)?;

        let model = UserDefinedEmbeddingModel::new(onnx, tokenizer);
        let inner =
            TextEmbedding::try_new_from_user_defined(model, InitOptionsUserDefined::default())
                .map_err(|e| TomeError::ModelCorrupt {
                    model: entry.name.to_owned(),
                    detail: format!("fastembed init: {e}"),
                })?;
        Ok(Self {
            inner,
            name: entry.name.to_owned(),
            version: entry.version.to_owned(),
        })
    }
}

impl Embedder for FastembedEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, TomeError> {
        let mut vectors = self.inner.embed(vec![text], None).map_err(|e| {
            TomeError::EmbeddingGenerationFailure {
                input_desc: truncate_for_diag(text),
                detail: e.to_string(),
            }
        })?;
        vectors
            .pop()
            .ok_or_else(|| TomeError::EmbeddingGenerationFailure {
                input_desc: truncate_for_diag(text),
                detail: "fastembed returned zero vectors".to_owned(),
            })
    }

    fn model_name(&self) -> &str {
        &self.name
    }

    fn model_version(&self) -> &str {
        &self.version
    }
}

/// fastembed-backed cross-encoder reranker.
pub struct FastembedReranker {
    inner: TextRerank,
    name: String,
    version: String,
}

impl FastembedReranker {
    pub fn load(entry: &ModelEntry, model_dir: &Path) -> Result<Self, TomeError> {
        runtime::ensure_initialised()?;
        let onnx = read_required(
            entry,
            &model_dir.join(entry.files.first().copied().unwrap_or("model.onnx")),
        )?;
        let tokenizer = build_tokenizer_files(entry, model_dir)?;

        let model = UserDefinedRerankingModel::new(onnx, tokenizer);
        let inner =
            TextRerank::try_new_from_user_defined(model, RerankInitOptionsUserDefined::default())
                .map_err(|e| TomeError::ModelCorrupt {
                model: entry.name.to_owned(),
                detail: format!("fastembed init: {e}"),
            })?;
        Ok(Self {
            inner,
            name: entry.name.to_owned(),
            version: entry.version.to_owned(),
        })
    }
}

impl Reranker for FastembedReranker {
    fn rerank(&self, query: &str, candidates: Vec<Candidate>) -> Result<Vec<Scored>, TomeError> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        let documents: Vec<String> = candidates
            .iter()
            .map(|c| format!("{}\n\n{}", c.name, c.description))
            .collect();
        let document_refs: Vec<&str> = documents.iter().map(String::as_str).collect();
        let scored = self
            .inner
            .rerank(query, document_refs, true, None)
            .map_err(|e| TomeError::RerankingFailure(e.to_string()))?;

        // fastembed's RerankResult exposes (index, score); zip back with
        // the original candidates while preserving the reranker's order.
        let mut out: Vec<Scored> = Vec::with_capacity(scored.len());
        for result in scored {
            let candidate = candidates.get(result.index).cloned().ok_or_else(|| {
                TomeError::RerankingFailure(format!(
                    "reranker returned out-of-range index {} (n={})",
                    result.index,
                    candidates.len()
                ))
            })?;
            out.push(Scored {
                candidate,
                score: result.score,
            });
        }
        Ok(out)
    }

    fn model_name(&self) -> &str {
        &self.name
    }

    fn model_version(&self) -> &str {
        &self.version
    }
}

fn read_required(entry: &ModelEntry, path: &Path) -> Result<Vec<u8>, TomeError> {
    fs::read(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            TomeError::ModelMissing {
                model: entry.name.to_owned(),
            }
        } else {
            TomeError::Io(e)
        }
    })
}

fn build_tokenizer_files(
    entry: &ModelEntry,
    model_dir: &Path,
) -> Result<TokenizerFiles, TomeError> {
    Ok(TokenizerFiles {
        tokenizer_file: read_required(entry, &model_dir.join("tokenizer.json"))?,
        config_file: read_optional(model_dir.join("config.json"))?,
        special_tokens_map_file: read_optional(model_dir.join("special_tokens_map.json"))?,
        tokenizer_config_file: read_optional(model_dir.join("tokenizer_config.json"))?,
    })
}

fn read_optional(path: PathBuf) -> Result<Vec<u8>, TomeError> {
    match fs::read(&path) {
        Ok(bytes) => Ok(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(TomeError::Io(e)),
    }
}

fn truncate_for_diag(s: &str) -> String {
    const CAP: usize = 80;
    if s.chars().count() <= CAP {
        s.to_owned()
    } else {
        let prefix: String = s.chars().take(CAP).collect();
        format!("{prefix}…")
    }
}
