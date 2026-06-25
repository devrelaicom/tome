//! PRIVACY CANARY — the structural privacy backstop for the typed-event model.
//!
//! Every event field is a closed enum / int / bool EXCEPT the Tier-2 attributed
//! carve-out (published artefact names + versions). This suite proves two things:
//!
//! 1. The Tier-2 attributed events carry ONLY the intended bounded strings — no
//!    surprise free-form field sneaks in (a `String` field still compiles, so the
//!    structural `to_attributes` validator alone can't catch a NEW free-string).
//! 2. NO event — anonymous or attributed — leaks a forbidden substring: a path,
//!    a URL, an `@`, a SQL fragment, a credential, or a Tome-specific marker
//!    (`.tome/`, `github.com/`). Each event is built with benign data and asserted
//!    clean; a planted leak is asserted CAUGHT (the negative control).
//!
//! The kernel's `assert_no_forbidden(event, forbidden)` takes the forbidden slice
//! explicitly. We pass the kernel's [`FORBIDDEN_SUBSTRINGS`] EXTENDED with
//! Tome-specific markers via [`FORBIDDEN`] so the canary is strictly stronger than
//! the kernel default.

use gauge_telemetry::canary::{FORBIDDEN_SUBSTRINGS, assert_no_forbidden};
use gauge_telemetry::env::EnvAttributes;
use gauge_telemetry::event::to_attributes;
use tome::telemetry::event::*;

/// The kernel default forbidden set EXTENDED with Tome-specific markers. The
/// kernel set already forbids `@`, `/Users/`, `/home/`, `C:\`, `http://`,
/// `https://`, `SELECT `, `password`, `secret`; we add the Tome filesystem +
/// source-URL markers a leak through Tome's own paths would carry, plus a bare
/// `http` (catches a scheme-less reflected URL fragment).
const TOME_MARKERS: &[&str] = &[
    ".tome/",
    "github.com/",
    "http",
    "/private/",
    "Bearer ",
    "sk-",
];

/// Build the effective forbidden set: the kernel default plus the Tome markers.
fn forbidden() -> Vec<&'static str> {
    FORBIDDEN_SUBSTRINGS
        .iter()
        .copied()
        .chain(TOME_MARKERS.iter().copied())
        .collect()
}

/// A benign env snapshot with no host-identifying free-form fields (so the
/// `Install`/`Heartbeat` flatten can't carry an OS string that trips `:`-free
/// markers). Counts only.
fn benign_env() -> EnvAttributes {
    EnvAttributes {
        cpu_cores: Some(8),
        ram_gb: Some(16),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// 1. The Tier-2 attributed event carries ONLY the intended bounded strings.
// ---------------------------------------------------------------------------

#[test]
fn tier2_entry_invoked_carries_only_the_intended_bounded_strings() {
    let e = AttributedEntryInvoked {
        catalog: "midnight",
        entry_name: "midnight-compact-debug".into(),
        entry_kind: EntryKind::Skill,
        plugin_name: "midnight-expert".into(),
        plugin_version: "1.2.0".into(),
        calling_harness: None,
    };
    let attrs = to_attributes(&e).expect("serializes to scalar attributes");
    // Exactly these STRING keys; no surprise free-form field. `entry_kind` is a
    // closed enum (renders as a string token), the rest are the bounded
    // artefact-name carve-out.
    let mut string_keys: Vec<&str> = attrs
        .iter()
        .filter(|(_, v)| v.is_string())
        .map(|(k, _)| k.as_str())
        .collect();
    string_keys.sort_unstable();
    assert_eq!(
        string_keys,
        [
            "catalog",
            "entry_kind",
            "entry_name",
            "plugin_name",
            "plugin_version"
        ],
        "the attributed entry_invoked must carry EXACTLY the intended string fields",
    );
}

#[test]
fn tier2_search_result_carries_only_the_intended_bounded_strings() {
    let e = SearchResult {
        catalog: "midnight",
        entry_name: "midnight-compact-debug".into(),
        entry_kind: EntryKind::Skill,
        plugin_name: "midnight-expert".into(),
        rank: 3,
        calling_harness: Some(Harness::ClaudeCode),
    };
    let attrs = to_attributes(&e).expect("serializes to scalar attributes");
    let mut string_keys: Vec<&str> = attrs
        .iter()
        .filter(|(_, v)| v.is_string())
        .map(|(k, _)| k.as_str())
        .collect();
    string_keys.sort_unstable();
    // `rank` is a number (not a string) — excluded. `calling_harness`/`entry_kind`
    // are closed-enum string tokens. No free-form field. (Sorted lexically:
    // `entry_kind` < `entry_name`.)
    assert_eq!(
        string_keys,
        [
            "calling_harness",
            "catalog",
            "entry_kind",
            "entry_name",
            "plugin_name"
        ],
    );
}

// ---------------------------------------------------------------------------
// 2. No event leaks a forbidden substring (benign passes; planted leak caught).
// ---------------------------------------------------------------------------

#[test]
fn benign_anonymous_events_pass_the_canary() {
    let forbidden = forbidden();

    assert_no_forbidden(
        &Search {
            surface: Surface::Cli,
            latency_ms: 1,
            candidates_returned: 0,
            reranker_used: false,
            strict: false,
            corpus_size: 0,
            embedder_model_id: None,
            embedding_provider_kind: ProviderKind::Bundled,
            reranker_provider_kind: ProviderKind::Bundled,
            calling_harness: None,
        },
        &forbidden,
    );

    assert_no_forbidden(
        &Install {
            install_method: InstallMethod::Brew,
            env: benign_env(),
        },
        &forbidden,
    );

    assert_no_forbidden(
        &Heartbeat {
            skills: 3,
            commands: 1,
            agents: 0,
            workspaces: 2,
            catalogs: 1,
            harnesses_detected: "claude-code,cursor".into(),
            env: benign_env(),
        },
        &forbidden,
    );

    assert_no_forbidden(
        &CatalogActionEvent {
            action: CatalogAction::Added,
            source_type: SourceType::Local,
        },
        &forbidden,
    );

    assert_no_forbidden(
        &ErrorEvent {
            error_class: tome::error::ErrorCategory::Io,
            surface: Surface::Mcp,
            calling_harness: Some(Harness::ClaudeCode),
        },
        &forbidden,
    );
}

#[test]
fn benign_attributed_events_pass_the_canary() {
    let forbidden = forbidden();

    // The bounded artefact names are NOT secrets and contain none of the forbidden
    // markers (no path, URL, `@`, or credential). The canary still runs over them
    // to prove the carve-out fields stay marker-free.
    assert_no_forbidden(
        &AttributedEntryInvoked {
            catalog: "midnight",
            entry_name: "midnight-compact-debug".into(),
            entry_kind: EntryKind::Skill,
            plugin_name: "midnight-expert".into(),
            plugin_version: "1.2.0".into(),
            calling_harness: None,
        },
        &forbidden,
    );

    assert_no_forbidden(
        &PluginEnabled {
            catalog: "midnight",
            plugin_name: "midnight-expert".into(),
            plugin_version: "1.2.0".into(),
        },
        &forbidden,
    );
}

#[test]
#[should_panic(expected = "leaked forbidden substring")]
fn planted_path_leak_is_caught_negative_control() {
    // If a future change ever let a path-shaped value into a string field, the
    // canary MUST catch it. Plant a `/Users/...` value in the attributed
    // `entry_name` carve-out and assert the canary panics. This proves the canary
    // is not vacuous.
    let leaky = AttributedEntryInvoked {
        catalog: "midnight",
        entry_name: "/Users/alice/secret-plugin".into(),
        entry_kind: EntryKind::Skill,
        plugin_name: "midnight-expert".into(),
        plugin_version: "1.2.0".into(),
        calling_harness: None,
    };
    assert_no_forbidden(&leaky, &forbidden());
}

#[test]
#[should_panic(expected = "leaked forbidden substring")]
fn planted_tome_marker_leak_is_caught_negative_control() {
    // The Tome-specific extension (`github.com/`) must also be enforced: a planted
    // source-URL fragment in a carve-out string is caught.
    let leaky = PluginEnabled {
        catalog: "midnight",
        plugin_name: "github.com/someone/leak".into(),
        plugin_version: "1.0.0".into(),
    };
    assert_no_forbidden(&leaky, &forbidden());
}
