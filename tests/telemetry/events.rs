//! Phase 10 / US2 (T041) — byte-stable wire-shape pins for the anonymous
//! telemetry events, asserted across the CRATE boundary (`tome::telemetry::*`).
//!
//! These integration pins are the cross-crate stability guarantee that underpins
//! the US5 `TELEMETRY.md` pin and the schema-stability contract: any field
//! reorder/rename, enum-token drift, or `skip_serializing_if` regression breaks
//! CI here. Some of these deliberately duplicate the in-module lib tests in
//! `src/telemetry/event.rs` — the duplication IS the point: the lib test proves
//! the constructor; this proves the same bytes are observable to an external
//! consumer of the published API.
//!
//! Pin policy (documented per the T041 brief):
//! - **FULL-LITERAL pins** (the field-richest / contract-defining events): every
//!   one of the 18 anonymous events is pinned to its EXACT JSON line here — there
//!   was no need to fall back to field-presence-only. The brief's minimum set
//!   (install/upgrade/heartbeat/search/entry_invoked/model_download/error/
//!   doctor_run/reindex/catalog_action) is a strict subset of what is pinned.
//! - The expected strings are committed as LITERALS (captured once by running the
//!   constructors, then frozen) — they are real pins, not values recomputed from
//!   the same code path under test.

use tome::error::ErrorCategory;
use tome::telemetry::buckets::{
    CountBucket, FindingsBucket, LatencyBucket, LoadBucket, RankBucket,
};
use tome::telemetry::event::{
    AnonymousEvent, fixed_attributed_envelope_for_tests, fixed_envelope_for_tests, to_line,
};
use tome::telemetry::event::{
    Artifact, AttributedEntryInvoked, AuthoringActionEvent, AuthoringOutcome, AuthoringVerb,
    CatalogAction, CatalogActionEvent, ColdStart, DoctorRun, EntryInfo, EntryInvoked, EntryKind,
    ErrorEvent, Harness, HarnessAction, HarnessActionEvent, Heartbeat, Install, InstallMethod,
    MetaAction, MetaActionEvent, ModelDownload, Outcome, PluginAction, PluginActionEvent,
    PromptInvoked, PromptKind, ProviderKind, Reindex, ReindexScope, Search, SourceFormat,
    SourceType, Summary, Surface, Upgrade, VersionStr, WorkspaceAction, WorkspaceActionEvent,
};

/// Serialize `event` behind the canonical fixed envelope for its event type and
/// return the exact JSONL body (no trailing newline).
fn line<E: AnonymousEvent>(event: &E) -> String {
    let envelope = fixed_envelope_for_tests(E::EVENT_TYPE);
    to_line(&envelope, event).expect("event serialises")
}

/// The fixed envelope prefix shared by every pinned line, parameterised on the
/// per-event `event_type`. Building it here (rather than copy-pasting the prefix
/// into 18 literals) keeps the install/session uuid + os/arch + timestamp +
/// sample_rate in ONE place — but the *event tail* of each line below is still a
/// hand-frozen literal, which is what the schema-stability pin actually guards.
fn envelope_prefix(event_type: &str) -> String {
    format!(
        "{{\"schema_version\":1,\
\"install_uuid\":\"0b9c1f2e-3a4d-4b6c-8e1f-2a3b4c5d6e7f\",\
\"session_uuid\":\"7f6e5d4c-3b2a-4f1e-9c8b-1a2b3c4d5e6f\",\
\"tome_version\":\"0.6.0\",\
\"os\":\"macos\",\
\"arch\":\"aarch64\",\
\"timestamp\":\"2026-06-11T14:11:45.123Z\",\
\"event_type\":\"{event_type}\",\
\"sample_rate\":1.0"
    )
}

// ---------------------------------------------------------------------------
// Envelope pin — the data-model §10 `tome.install` worked example, byte-exact.
//
// This duplicates the `src/telemetry/event.rs` lib test deliberately (see the
// module doc): the integration assertion is the CROSS-CRATE guarantee.
// ---------------------------------------------------------------------------

#[test]
fn install_envelope_matches_data_model_worked_example() {
    let event = Install {
        install_method: InstallMethod::Brew,
    };
    let got = line(&event);
    // Pinned byte-for-byte against `specs/010-phase-10-telemetry/data-model.md` §10.
    let expected = "{\"schema_version\":1,\"install_uuid\":\"0b9c1f2e-3a4d-4b6c-8e1f-2a3b4c5d6e7f\",\"session_uuid\":\"7f6e5d4c-3b2a-4f1e-9c8b-1a2b3c4d5e6f\",\"tome_version\":\"0.6.0\",\"os\":\"macos\",\"arch\":\"aarch64\",\"timestamp\":\"2026-06-11T14:11:45.123Z\",\"event_type\":\"tome.install\",\"sample_rate\":1.0,\"install_method\":\"brew\"}";
    assert_eq!(
        got, expected,
        "tome.install drifted from the data-model §10 pin"
    );
}

// ---------------------------------------------------------------------------
// Attributed pin — the data-model §10 `catalog.midnight.entry_invoked` worked
// example 2, byte-exact across the crate boundary. Note: the attributed envelope
// has NO `sample_rate` (FR-058, attributed events are never sampled).
// ---------------------------------------------------------------------------

#[test]
fn attributed_entry_invoked_matches_data_model_worked_example_2() {
    let envelope =
        fixed_attributed_envelope_for_tests("catalog.midnight.entry_invoked".to_string());
    let event = AttributedEntryInvoked {
        entry_name: "midnight-compact-debug".to_string(),
        entry_kind: EntryKind::Skill,
        plugin_name: "midnight-expert".to_string(),
        plugin_version: "1.2.0".to_string(),
        catalog_id: "midnight",
        calling_harness: Some(Harness::ClaudeCode),
    };
    let got = to_line(&envelope, &event).expect("attributed event serialises");
    // Pinned byte-for-byte against `specs/010-phase-10-telemetry/data-model.md` §10.
    let expected = "{\"schema_version\":1,\"install_uuid\":\"0b9c1f2e-3a4d-4b6c-8e1f-2a3b4c5d6e7f\",\"session_uuid\":\"7f6e5d4c-3b2a-4f1e-9c8b-1a2b3c4d5e6f\",\"tome_version\":\"0.6.0\",\"os\":\"macos\",\"arch\":\"aarch64\",\"timestamp\":\"2026-06-11T14:12:03.456Z\",\"event_type\":\"catalog.midnight.entry_invoked\",\"entry_name\":\"midnight-compact-debug\",\"entry_kind\":\"skill\",\"plugin_name\":\"midnight-expert\",\"plugin_version\":\"1.2.0\",\"catalog_id\":\"midnight\",\"calling_harness\":\"claude-code\"}";
    assert_eq!(
        got, expected,
        "catalog.midnight.entry_invoked drifted from the data-model §10 pin"
    );
}

// ---------------------------------------------------------------------------
// Every-event shape pin — all 18 anonymous events, EXACT JSON, envelope-first
// field order then event fields. Each `expected` tail is a frozen literal.
// ---------------------------------------------------------------------------

#[test]
fn install_full_literal_pin() {
    let event = Install {
        install_method: InstallMethod::Brew,
    };
    let expected = envelope_prefix("tome.install") + ",\"install_method\":\"brew\"}";
    assert_eq!(line(&event), expected);
}

#[test]
fn upgrade_full_literal_pin() {
    let event = Upgrade {
        from_version: VersionStr::from_last_version("0.5.0"),
    };
    let expected = envelope_prefix("tome.upgrade") + ",\"from_version\":\"0.5.0\"}";
    assert_eq!(line(&event), expected);
}

#[test]
fn heartbeat_full_literal_pin() {
    let event = Heartbeat {
        skills_bucket: CountBucket::FiveToNineteen,
        commands_bucket: CountBucket::OneToFour,
        agents_bucket: CountBucket::Zero,
        workspaces_bucket: CountBucket::OneToFour,
        catalogs_bucket: CountBucket::OneToFour,
        harnesses_detected: vec![Harness::ClaudeCode, Harness::Cursor],
    };
    let expected = envelope_prefix("tome.heartbeat")
        + ",\"skills_bucket\":\"5-19\"\
,\"commands_bucket\":\"1-4\"\
,\"agents_bucket\":\"0\"\
,\"workspaces_bucket\":\"1-4\"\
,\"catalogs_bucket\":\"1-4\"\
,\"harnesses_detected\":[\"claude-code\",\"cursor\"]}";
    assert_eq!(line(&event), expected);
}

#[test]
fn search_full_literal_pin() {
    let event = Search {
        surface: Surface::Cli,
        latency_bucket: LatencyBucket::From50To200,
        candidates_returned: CountBucket::FiveToNineteen,
        reranker_used: true,
        strict: false,
        corpus_size_bucket: CountBucket::TwentyToNinetyNine,
        embedder_model_id: Some("bge-small-en-v1.5"),
        calling_harness: Some(Harness::ClaudeCode),
    };
    let expected = envelope_prefix("tome.search")
        + ",\"surface\":\"cli\"\
,\"latency_bucket\":\"50-200ms\"\
,\"candidates_returned\":\"5-19\"\
,\"reranker_used\":true\
,\"strict\":false\
,\"corpus_size_bucket\":\"20-99\"\
,\"embedder_model_id\":\"bge-small-en-v1.5\"\
,\"calling_harness\":\"claude-code\"}";
    assert_eq!(line(&event), expected);
}

#[test]
fn entry_info_full_literal_pin() {
    let event = EntryInfo {
        rank_bucket: RankBucket::One,
        calling_harness: Some(Harness::Cursor),
    };
    let expected = envelope_prefix("tome.entry_info")
        + ",\"rank_bucket\":\"1\",\"calling_harness\":\"cursor\"}";
    assert_eq!(line(&event), expected);
}

#[test]
fn entry_invoked_full_literal_pin() {
    let event = EntryInvoked {
        entry_kind: EntryKind::Skill,
        rank_bucket: RankBucket::One,
        calling_harness: Some(Harness::ClaudeCode),
    };
    let expected = envelope_prefix("tome.entry_invoked")
        + ",\"entry_kind\":\"skill\"\
,\"rank_bucket\":\"1\"\
,\"calling_harness\":\"claude-code\"}";
    assert_eq!(line(&event), expected);
}

#[test]
fn prompt_invoked_full_literal_pin() {
    let event = PromptInvoked {
        prompt_kind: PromptKind::Command,
        calling_harness: Some(Harness::Codex),
    };
    let expected = envelope_prefix("tome.prompt_invoked")
        + ",\"prompt_kind\":\"command\",\"calling_harness\":\"codex\"}";
    assert_eq!(line(&event), expected);
}

#[test]
fn catalog_action_full_literal_pin() {
    let event = CatalogActionEvent {
        action: CatalogAction::Added,
        source_type: SourceType::Git,
    };
    let expected =
        envelope_prefix("tome.catalog_action") + ",\"action\":\"added\",\"source_type\":\"git\"}";
    assert_eq!(line(&event), expected);
}

#[test]
fn plugin_action_full_literal_pin() {
    let event = PluginActionEvent {
        action: PluginAction::Enabled,
    };
    let expected = envelope_prefix("tome.plugin_action") + ",\"action\":\"enabled\"}";
    assert_eq!(line(&event), expected);
}

#[test]
fn workspace_action_full_literal_pin() {
    let event = WorkspaceActionEvent {
        action: WorkspaceAction::Init,
    };
    let expected = envelope_prefix("tome.workspace_action") + ",\"action\":\"init\"}";
    assert_eq!(line(&event), expected);
}

#[test]
fn harness_action_full_literal_pin() {
    let event = HarnessActionEvent {
        action: HarnessAction::Sync,
        harness: Harness::ClaudeCode,
    };
    let expected = envelope_prefix("tome.harness_action")
        + ",\"action\":\"sync\",\"harness\":\"claude-code\"}";
    assert_eq!(line(&event), expected);
}

#[test]
fn authoring_action_full_literal_pin() {
    let event = AuthoringActionEvent {
        verb: AuthoringVerb::Convert,
        artifact: Artifact::Plugin,
        source_format: SourceFormat::ClaudeCode,
        outcome: AuthoringOutcome::Ok,
    };
    let expected = envelope_prefix("tome.authoring_action")
        + ",\"verb\":\"convert\"\
,\"artifact\":\"plugin\"\
,\"source_format\":\"claude_code\"\
,\"outcome\":\"ok\"}";
    assert_eq!(line(&event), expected);
}

#[test]
fn meta_action_full_literal_pin() {
    let event = MetaActionEvent {
        action: MetaAction::Add,
        outcome: Outcome::Ok,
    };
    let expected = envelope_prefix("tome.meta_action") + ",\"action\":\"add\",\"outcome\":\"ok\"}";
    assert_eq!(line(&event), expected);
}

#[test]
fn model_download_full_literal_pin() {
    // `error_class: None` MUST omit the key (skip_serializing_if).
    let event = ModelDownload {
        model_id: "bge-small-en-v1.5",
        outcome: Outcome::Ok,
        error_class: None,
    };
    let expected = envelope_prefix("tome.model_download")
        + ",\"model_id\":\"bge-small-en-v1.5\",\"outcome\":\"ok\"}";
    assert_eq!(line(&event), expected);
}

#[test]
fn model_download_with_error_class_full_literal_pin() {
    // `error_class: Some(..)` serialises as the ErrorCategory snake_case slug.
    let event = ModelDownload {
        model_id: "bge-reranker-base",
        outcome: Outcome::Failed,
        error_class: Some(ErrorCategory::TelemetryEndpointUnreachable),
    };
    let expected = envelope_prefix("tome.model_download")
        + ",\"model_id\":\"bge-reranker-base\"\
,\"outcome\":\"failed\"\
,\"error_class\":\"telemetry_endpoint_unreachable\"}";
    assert_eq!(line(&event), expected);
}

#[test]
fn cold_start_full_literal_pin() {
    let event = ColdStart {
        embedder_load_bucket: LoadBucket::From100To300,
        index_ready_bucket: LoadBucket::Under100,
        embedder_model_id: Some("bge-small-en-v1.5"),
    };
    let expected = envelope_prefix("tome.cold_start")
        + ",\"embedder_load_bucket\":\"100-300ms\"\
,\"index_ready_bucket\":\"<100ms\"\
,\"embedder_model_id\":\"bge-small-en-v1.5\"}";
    assert_eq!(line(&event), expected);
}

#[test]
fn doctor_run_full_literal_pin() {
    let event = DoctorRun {
        fix: false,
        findings_bucket: FindingsBucket::OneToFour,
    };
    let expected =
        envelope_prefix("tome.doctor_run") + ",\"fix\":false,\"findings_bucket\":\"1-4\"}";
    assert_eq!(line(&event), expected);
}

#[test]
fn reindex_full_literal_pin() {
    let event = Reindex {
        scope: ReindexScope::All,
        forced: false,
        outcome: Outcome::Ok,
    };
    let expected =
        envelope_prefix("tome.reindex") + ",\"scope\":\"all\",\"forced\":false,\"outcome\":\"ok\"}";
    assert_eq!(line(&event), expected);
}

#[test]
fn error_full_literal_pin() {
    let event = ErrorEvent {
        error_class: ErrorCategory::CatalogNotFound,
        surface: Surface::Cli,
        calling_harness: Some(Harness::ClaudeCode),
    };
    let expected = envelope_prefix("tome.error")
        + ",\"error_class\":\"catalog_not_found\"\
,\"surface\":\"cli\"\
,\"calling_harness\":\"claude-code\"}";
    assert_eq!(line(&event), expected);
}

// ---------------------------------------------------------------------------
// Phase 12 — `tome.summary` (provider-kind attribution). Byte-stable pin +
// privacy assertion: only the closed `ProviderKind` token can appear; no
// provider name / model id / base url is on the wire.
// ---------------------------------------------------------------------------

#[test]
fn summary_full_literal_pin() {
    let event = Summary {
        summariser_provider_kind: ProviderKind::Anthropic,
        outcome: Outcome::Ok,
    };
    let expected = envelope_prefix("tome.summary")
        + ",\"summariser_provider_kind\":\"anthropic\",\"outcome\":\"ok\"}";
    assert_eq!(line(&event), expected);
}

#[test]
fn summary_provider_kind_only_closed_tokens_no_free_form_string() {
    // Every ProviderKind variant must serialise to its closed lowercase token —
    // there is structurally no way to put a registry name / model / url on the
    // wire (the field type is a closed enum, not a String).
    let cases = [
        (ProviderKind::Bundled, "bundled"),
        (ProviderKind::Openai, "openai"),
        (ProviderKind::Anthropic, "anthropic"),
        (ProviderKind::Gemini, "gemini"),
        (ProviderKind::Voyage, "voyage"),
    ];
    for (kind, expected_token) in cases {
        let event = Summary {
            summariser_provider_kind: kind,
            outcome: Outcome::Failed,
        };
        let serialised = line(&event);
        assert!(
            serialised.contains(&format!(
                "\"summariser_provider_kind\":\"{expected_token}\""
            )),
            "expected closed token `{expected_token}` in: {serialised}"
        );
        // Defence-in-depth: a representative secret-shaped string must NEVER
        // appear — the type system already prevents it, this catches a future
        // regression that swaps the field to a String.
        assert!(
            !serialised.contains("api.openai.com")
                && !serialised.contains("sk-")
                && !serialised.contains("gpt-4")
                && !serialised.contains("my-provider"),
            "summary event must carry no provider name/model/url: {serialised}"
        );
    }
}

// ---------------------------------------------------------------------------
// Bucket token pins — exact wire token per enum variant. (Mirrors the lib unit
// tests; the cross-crate assertion guards the public buckets API.)
// ---------------------------------------------------------------------------

/// Serialize a value and strip the surrounding JSON quotes to get the bare token.
fn token<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v)
        .expect("bucket serialises")
        .trim_matches('"')
        .to_string()
}

#[test]
fn count_bucket_tokens_pinned() {
    assert_eq!(token(&CountBucket::Zero), "0");
    assert_eq!(token(&CountBucket::OneToFour), "1-4");
    assert_eq!(token(&CountBucket::FiveToNineteen), "5-19");
    assert_eq!(token(&CountBucket::TwentyToNinetyNine), "20-99");
    assert_eq!(token(&CountBucket::HundredPlus), "100+");
}

#[test]
fn latency_bucket_tokens_pinned() {
    assert_eq!(token(&LatencyBucket::Under50), "<50ms");
    assert_eq!(token(&LatencyBucket::From50To200), "50-200ms");
    assert_eq!(token(&LatencyBucket::From200To500), "200-500ms");
    assert_eq!(token(&LatencyBucket::From500To1000), "500ms-1s");
    assert_eq!(token(&LatencyBucket::Over1s), "1s+");
}

#[test]
fn rank_bucket_tokens_pinned() {
    assert_eq!(token(&RankBucket::One), "1");
    assert_eq!(token(&RankBucket::Two), "2");
    assert_eq!(token(&RankBucket::Three), "3");
    assert_eq!(token(&RankBucket::Four), "4");
    assert_eq!(token(&RankBucket::Five), "5");
    assert_eq!(token(&RankBucket::SixToTen), "6-10");
    assert_eq!(token(&RankBucket::ElevenPlus), "11+");
    assert_eq!(token(&RankBucket::None), "none");
}

#[test]
fn load_bucket_tokens_pinned() {
    assert_eq!(token(&LoadBucket::Under100), "<100ms");
    assert_eq!(token(&LoadBucket::From100To300), "100-300ms");
    assert_eq!(token(&LoadBucket::From300To1000), "300-1000ms");
    assert_eq!(token(&LoadBucket::Over1s), "1s+");
}

#[test]
fn findings_bucket_tokens_pinned() {
    assert_eq!(token(&FindingsBucket::Zero), "0");
    assert_eq!(token(&FindingsBucket::OneToFour), "1-4");
    assert_eq!(token(&FindingsBucket::FivePlus), "5+");
}

// ---------------------------------------------------------------------------
// Boundary `From` mappings — the bucket cutovers, asserted from the public API.
// ---------------------------------------------------------------------------

#[test]
fn count_bucket_from_boundaries() {
    assert_eq!(CountBucket::from(4u64), CountBucket::OneToFour);
    assert_eq!(CountBucket::from(5u64), CountBucket::FiveToNineteen);
    assert_eq!(CountBucket::from(100u64), CountBucket::HundredPlus);
}

#[test]
fn latency_bucket_from_boundaries() {
    use std::time::Duration;
    assert_eq!(
        LatencyBucket::from(Duration::from_millis(49)),
        LatencyBucket::Under50
    );
    assert_eq!(
        LatencyBucket::from(Duration::from_millis(1000)),
        LatencyBucket::Over1s
    );
}

#[test]
fn rank_bucket_from_boundaries() {
    assert_eq!(RankBucket::from_rank(6), RankBucket::SixToTen);
    assert_eq!(RankBucket::from_rank(11), RankBucket::ElevenPlus);
}

// ---------------------------------------------------------------------------
// Optional-field omission — `skip_serializing_if = "Option::is_none"` on the
// `Search` optionals: omitted when `None`, present (and correct) when `Some`.
// ---------------------------------------------------------------------------

#[test]
fn search_optionals_are_omitted_when_none() {
    let event = Search {
        surface: Surface::Cli,
        latency_bucket: LatencyBucket::Under50,
        candidates_returned: CountBucket::OneToFour,
        reranker_used: true,
        strict: false,
        corpus_size_bucket: CountBucket::FiveToNineteen,
        embedder_model_id: None,
        calling_harness: None,
    };
    let got = line(&event);
    assert!(
        !got.contains("embedder_model_id"),
        "None embedder_model_id must be omitted: {got}"
    );
    assert!(
        !got.contains("calling_harness"),
        "None calling_harness must be omitted: {got}"
    );
    // Exact tail with both optionals absent.
    let expected = envelope_prefix("tome.search")
        + ",\"surface\":\"cli\"\
,\"latency_bucket\":\"<50ms\"\
,\"candidates_returned\":\"1-4\"\
,\"reranker_used\":true\
,\"strict\":false\
,\"corpus_size_bucket\":\"5-19\"}";
    assert_eq!(got, expected);
}

#[test]
fn search_optionals_are_present_when_some() {
    let event = Search {
        surface: Surface::Mcp,
        latency_bucket: LatencyBucket::Under50,
        candidates_returned: CountBucket::OneToFour,
        reranker_used: false,
        strict: true,
        corpus_size_bucket: CountBucket::FiveToNineteen,
        embedder_model_id: Some("bge-small-en-v1.5"),
        calling_harness: Some(Harness::Opencode),
    };
    let got = line(&event);
    assert!(
        got.contains("\"embedder_model_id\":\"bge-small-en-v1.5\""),
        "Some embedder_model_id must be present: {got}"
    );
    assert!(
        got.contains("\"calling_harness\":\"opencode\""),
        "Some calling_harness must be present: {got}"
    );
}
