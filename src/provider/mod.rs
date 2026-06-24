//! BYOK/BYOM model providers — the new top-level **sync** provider layer.
//!
//! Phase 12 lets each of Tome's three model capabilities (summarisation,
//! embedding, reranking) be pointed independently at an external provider
//! instead of the bundled local models. The default stays local; this layer is
//! engaged only when a `[providers.<name>]` registry entry is referenced from a
//! capability section in `~/.tome/config.toml`.
//!
//! **Sync-only, deliberately.** Every transport here is built on
//! `reqwest::blocking` (wired in a later phase): the constitution permits a
//! single async island under `src/mcp/`, and the user explicitly chose to
//! hand-roll synchronous HTTP over `reqwest::blocking` + `serde_json` rather
//! than open a second async island. `tests/harness_settings/sync_boundary.rs`
//! greps this tree for async constructs and must stay green — nothing under
//! `src/provider/` may reach for the async runtime.
//!
//! Submodules:
//! - [`config`] — provider-registry resolution (`Config` → `ResolvedProvider`).
//! - [`http`] — the synchronous transport seam (request → response bytes).
//! - [`error`] — the structured `ProviderError` mapped once to `TomeError`.
//! - [`openai`], [`anthropic`], [`gemini`], [`voyage`] — per-kind wire shapes.
//!
//! Phase 1 (Setup) lands this as a compiling, inert skeleton: the submodules
//! are near-empty stubs. Behaviour arrives in later phases.

pub mod anthropic;
pub mod config;
pub mod error;
pub mod gemini;
pub mod http;
pub mod openai;
pub mod voyage;
