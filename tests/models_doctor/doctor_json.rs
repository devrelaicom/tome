//! Phase 6 / US4 — `tome doctor --json` wire-shape regression net.
//!
//! Pins the JSON envelope's field names + values per
//! `contracts/doctor.md` §"Output (`--json`)" and data-model §5. If the
//! shape drifts, this test breaks loudly — the JSON is consumed by
//! harnesses and by users piping into `jq`.

use crate::common::{Fixture, ToolEnv, fabricate_all_registry_models, paths_for};
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn doctor_json_shape_is_pinned_on_healthy_install() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let out = env.cmd().args(["--json", "doctor"]).output().unwrap();
    assert!(out.status.success(), "exit={:?}", out.status.code());

    let v: Value = serde_json::from_slice(&out.stdout).expect("doctor --json parses");

    // Top-level fields, in any order — but every one of these must be
    // present.
    for field in [
        "tome_version",
        "workspace",
        "embedder",
        "reranker",
        // Phase 4 / US5.a additions (T-M9, PR-D):
        "summariser",
        "index",
        "drift",
        "catalogs",
        "workspace_registry",
        "harnesses",
        // Phase 4 / US5.a additions (T-M9, PR-D):
        // `project_binding` + `effective_harness_list` are both
        // `#[serde(skip_serializing_if = "Option::is_none")]` per
        // data-model §15; they MAY be absent from a healthy report.
        // `harness_rules` + `harness_mcp` + `detected_uninstalled_harnesses`
        // are ALWAYS present (default empty vec).
        "harness_rules",
        "harness_mcp",
        "detected_uninstalled_harnesses",
        "overall",
        "suggested_fixes",
    ] {
        assert!(v.get(field).is_some(), "missing top-level field `{field}`");
    }

    // Phase 4 / US5.a: summariser is a ModelHealth (same shape as
    // embedder + reranker).
    let summ = &v["summariser"];
    for field in ["name", "version", "state"] {
        assert!(summ.get(field).is_some(), "summariser.{field} missing");
    }

    // harness_rules + harness_mcp are arrays; on a healthy install with
    // no effective harness list they're empty.
    assert!(v["harness_rules"].is_array(), "harness_rules array");
    assert!(v["harness_mcp"].is_array(), "harness_mcp array");
    assert!(
        v["detected_uninstalled_harnesses"].is_array(),
        "detected_uninstalled_harnesses array",
    );

    // Workspace embeds the WorkspaceInfo schema verbatim.
    let ws = &v["workspace"];
    assert_eq!(ws["scope"], "global");
    assert!(ws.get("path").is_some());
    assert!(ws.get("source").is_some());

    // ModelHealth has name / version / state.
    let emb = &v["embedder"];
    for field in ["name", "version", "state"] {
        assert!(emb.get(field).is_some(), "embedder.{field} missing");
    }
    assert_eq!(emb["state"], "ok");

    // IndexHealth has present / schema_version / plugins_enabled /
    // skills_indexed / size_bytes / integrity_ok.
    let idx = &v["index"];
    for field in [
        "present",
        "schema_version",
        "plugins_enabled",
        "skills_indexed",
        "size_bytes",
        "integrity_ok",
    ] {
        assert!(idx.get(field).is_some(), "index.{field} missing");
    }

    // catalogs + harnesses are arrays.
    assert!(v["catalogs"].is_array());
    assert!(v["harnesses"].is_array());

    // Harnesses array contains every known name in fixed order; first
    // entry is claude-code per the contract example.
    let harnesses = v["harnesses"].as_array().unwrap();
    assert_eq!(harnesses.len(), 6);
    assert_eq!(harnesses[0]["name"], "claude-code");
    for h in harnesses {
        for field in ["name", "path", "present"] {
            assert!(h.get(field).is_some(), "harness.{field} missing");
        }
    }

    // overall is one of "ok" | "degraded" | "unhealthy".
    assert!(
        matches!(
            v["overall"].as_str(),
            Some("ok") | Some("degraded") | Some("unhealthy")
        ),
        "overall: {}",
        v["overall"],
    );

    // suggested_fixes is an array; on a healthy install it's empty.
    assert!(v["suggested_fixes"].is_array());
    assert_eq!(v["suggested_fixes"].as_array().unwrap().len(), 0);
}

#[test]
fn doctor_json_includes_suggested_fix_record_on_broken_catalog() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let fix = Fixture::build_sample();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();

    // Break the catalog cache: remove `.git/`.
    let cache_dir = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(fix.url.as_bytes());
        env.catalogs_dir().join(hex::encode(h.finalize()))
    };
    std::fs::remove_dir_all(cache_dir.join(".git")).unwrap();

    let out = env.cmd().args(["--json", "doctor"]).output().unwrap();
    // Exit 1 — degraded — but JSON still emitted on stdout.
    assert_eq!(out.status.code(), Some(1));
    let v: Value = serde_json::from_slice(&out.stdout).expect("doctor --json parses");

    assert_eq!(v["overall"], "degraded");
    let catalogs = v["catalogs"].as_array().unwrap();
    assert_eq!(catalogs.len(), 1);
    assert_eq!(catalogs[0]["state"], "not_a_repo");
    let fixes = v["suggested_fixes"].as_array().unwrap();
    assert!(!fixes.is_empty(), "expected suggested fix");
    let cat_fix = fixes
        .iter()
        .find(|f| {
            f["subsystem"]
                .as_str()
                .map(|s| s.starts_with("catalog:"))
                .unwrap_or(false)
        })
        .expect("catalog suggested fix");
    assert_eq!(cat_fix["auto_fixable"], true);
    assert!(
        cat_fix["command"]
            .as_str()
            .unwrap()
            .contains("catalog update")
    );
}

// ---- T-M9 (PR-D): byte-stable JSON wire shape ----------------------------
//
// Pin the entire DoctorReport JSON envelope for a minimal bootstrap-state
// report. The shape is the wire contract per `contracts/doctor.md` +
// `contracts/doctor-extensions-p4.md`; if a field order or rename drifts,
// every downstream `jq` consumer breaks silently in the absence of this
// test.
//
// `tome_version` is substituted from `env!("CARGO_PKG_VERSION")` because
// the field carries the literal package version at build time; the
// substitution keeps the test stable across version bumps without
// hand-editing the expected JSON.

#[test]
fn doctor_json_shape_is_byte_stable_for_minimal_report() {
    use tome::commands::status::{IndexHealth, ModelHealth};
    use tome::doctor::report::WorkspaceRegistryStatus;
    use tome::doctor::{CatalogCacheHealth, DoctorClassification, DoctorReport};
    use tome::index::meta::DriftStatus;
    use tome::workspace::{ScopeKind, WorkspaceInfo, scope::ScopeSource};

    // Literal minimal report — empty everywhere a field is Vec /
    // Option / default. Pin every Phase 4 addition into the wire shape.
    let report = DoctorReport {
        tome_version: env!("CARGO_PKG_VERSION").to_owned(),
        workspace: WorkspaceInfo {
            scope: ScopeKind::Global,
            path: None,
            source: ScopeSource::GlobalFallback,
            catalogs: 0,
            plugins_total: 0,
            plugins_enabled: 0,
            skills_indexed: 0,
            schema_version: None,
            embedder: None,
            enrolled_catalogs: Vec::new(),
            enabled_plugins: Vec::new(),
            bound_projects: Vec::new(),
            summary_cache: None,
        },
        project_binding: None,
        embedder: ModelHealth {
            name: "bge-small-en-v1.5".to_owned(),
            version: "1.5".to_owned(),
            state: "ok".to_owned(),
        },
        reranker: ModelHealth {
            name: "bge-reranker-base".to_owned(),
            version: "1".to_owned(),
            state: "ok".to_owned(),
        },
        summariser: ModelHealth {
            name: "qwen2.5-0.5b-instruct".to_owned(),
            version: "2.5".to_owned(),
            state: "ok".to_owned(),
        },
        index: IndexHealth {
            present: false,
            schema_version: None,
            plugins_enabled: 0,
            skills_indexed: 0,
            size_bytes: 0,
            integrity_ok: true,
        },
        drift: DriftStatus::None,
        catalogs: Vec::<CatalogCacheHealth>::new(),
        workspace_registry: WorkspaceRegistryStatus {
            present: false,
            tracked: 0,
        },
        harnesses: Vec::new(),
        effective_harness_list: None,
        harness_rules: Vec::new(),
        harness_mcp: Vec::new(),
        detected_uninstalled_harnesses: Vec::new(),
        // Phase 5 / US5.b: three new Option fields. All None for the
        // minimal report — `#[serde(skip_serializing_if = "Option::is_none")]`
        // keeps them absent from the wire shape, preserving byte
        // stability with the Phase 4 pin.
        prompts: None,
        orphan_data_dirs: None,
        entry_counts: None,
        hooks: None,
        guardrails: None,
        agents: None,
        privilege_escalation: None,
        personas: None,
        legacy_model_manifests: Vec::new(),
        unconverted_plugins: Vec::new(),
        // Phase 9 / US4: empty → `skip_serializing_if = "Vec::is_empty"` omits
        // it from the wire shape, so the byte-stable pin below stays unchanged.
        meta_skills: Vec::new(),
        overall: DoctorClassification::Ok,
        suggested_fixes: Vec::new(),
    };

    let json = serde_json::to_string(&report).expect("serialise");
    let expected = format!(
        r#"{{"tome_version":"{v}","workspace":{{"scope":"global","path":null,"source":"global_fallback","catalogs":0,"plugins_total":0,"plugins_enabled":0,"skills_indexed":0,"schema_version":null,"embedder":null,"enrolled_catalogs":[],"enabled_plugins":[],"bound_projects":[],"summary_cache":null}},"embedder":{{"name":"bge-small-en-v1.5","version":"1.5","state":"ok"}},"reranker":{{"name":"bge-reranker-base","version":"1","state":"ok"}},"summariser":{{"name":"qwen2.5-0.5b-instruct","version":"2.5","state":"ok"}},"index":{{"present":false,"schema_version":null,"plugins_enabled":0,"skills_indexed":0,"size_bytes":0,"integrity_ok":true}},"drift":{{"kind":"none"}},"catalogs":[],"workspace_registry":{{"present":false,"tracked":0}},"harnesses":[],"harness_rules":[],"harness_mcp":[],"detected_uninstalled_harnesses":[],"overall":"ok","suggested_fixes":[]}}"#,
        v = env!("CARGO_PKG_VERSION"),
    );
    assert_eq!(json, expected);
}

#[test]
fn doctor_json_overall_unhealthy_when_models_missing() {
    let env = ToolEnv::new();
    // No fabricate — both models are absent.
    let out = env.cmd().args(["--json", "doctor"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: Value = serde_json::from_slice(&out.stdout).expect("doctor --json parses");
    assert_eq!(v["overall"], "unhealthy");
    assert_eq!(v["embedder"]["state"], "missing");
}

/// FIX G (Test Minor #3): a SPAWNED `tome --json doctor` under a HOME with
/// `~/.claude` present (so claude-code is detected) but NO meta-skill install
/// emits a POPULATED `meta_skills` array — exactly one object carrying all five
/// documented keys and `state:"missing-but-expected"`. The pre-existing
/// `doctor_json` spawned tests all run under an empty home (no detected
/// harness), so the populated path was untested end-to-end through the binary.
#[test]
fn doctor_json_meta_skills_populated_when_harness_detected() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    // Detect claude-code (existence-only) under the isolated HOME; install nothing.
    std::fs::create_dir_all(env.home_path().join(".claude")).unwrap();

    let out = env.cmd().args(["--json", "doctor"]).output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).expect("doctor --json parses");

    let rows = v["meta_skills"]
        .as_array()
        .expect("meta_skills present + an array when a harness is detected");
    let cc = rows
        .iter()
        .find(|r| r["harness"] == "claude-code" && r["scope"] == "global")
        .expect("claude-code/global missing-but-expected row present");

    // Exactly the five documented keys.
    let mut keys: Vec<&str> = cc.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["dir", "harness", "scope", "skill_id", "state"],
        "the meta_skills row carries exactly the five keys: {cc}",
    );
    assert_eq!(cc["skill_id"], "convert-marketplace");
    assert_eq!(cc["harness"], "claude-code");
    assert_eq!(cc["scope"], "global");
    assert_eq!(cc["state"], "missing-but-expected");
    assert!(
        cc["dir"].as_str().unwrap().ends_with(".claude/skills"),
        "dir is the claude-code/global skills root: {}",
        cc["dir"],
    );
}

// Silence unused-import warning when no harness check uses TempDir.
#[allow(dead_code)]
fn _silence(_: TempDir) {}

// ---- Phase 5 / US5.b — JSON pins for the three new fields ---------------
//
// `prompts`, `orphan_data_dirs`, and `entry_counts` are
// `#[serde(skip_serializing_if = "Option::is_none")]`, so an
// outside-project doctor pass MUST NOT emit any of them. A
// workspace-scoped pass populates them; the populated shape carries
// the documented field set per `contracts/doctor-extensions-p5.md`.

#[test]
fn doctor_json_phase5_fields_absent_outside_workspace() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let out = env.cmd().args(["--json", "doctor"]).output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).expect("doctor --json parses");
    // GlobalFallback scope → all three Phase 5 fields skipped.
    assert!(
        v.get("prompts").is_none(),
        "GlobalFallback: prompts must be absent (skip_serializing_if), got {:?}",
        v.get("prompts"),
    );
    assert!(
        v.get("orphan_data_dirs").is_none(),
        "GlobalFallback: orphan_data_dirs must be absent",
    );
    assert!(
        v.get("entry_counts").is_none(),
        "GlobalFallback: entry_counts must be absent",
    );
}

#[test]
fn doctor_json_phase5_fields_serialise_correctly_when_populated() {
    // Use the library API to build a known-populated DoctorReport and
    // serialise it directly. This pins the wire shape independently
    // of any test harness producing data on-disk.
    use std::path::PathBuf;
    use tome::commands::status::{IndexHealth, ModelHealth};
    use tome::doctor::report::{
        EntryCountsByKind, OrphanDataDirReport, PromptsReport, WorkspaceRegistryStatus,
    };
    use tome::doctor::{CatalogCacheHealth, DoctorClassification, DoctorReport};
    use tome::index::meta::DriftStatus;
    use tome::workspace::{ScopeKind, WorkspaceInfo, scope::ScopeSource};

    let report = DoctorReport {
        tome_version: env!("CARGO_PKG_VERSION").to_owned(),
        workspace: WorkspaceInfo {
            scope: ScopeKind::Global,
            path: None,
            source: ScopeSource::Flag,
            catalogs: 0,
            plugins_total: 0,
            plugins_enabled: 0,
            skills_indexed: 0,
            schema_version: None,
            embedder: None,
            enrolled_catalogs: Vec::new(),
            enabled_plugins: Vec::new(),
            bound_projects: Vec::new(),
            summary_cache: None,
        },
        project_binding: None,
        embedder: ModelHealth {
            name: "bge-small-en-v1.5".to_owned(),
            version: "1.5".to_owned(),
            state: "ok".to_owned(),
        },
        reranker: ModelHealth {
            name: "bge-reranker-base".to_owned(),
            version: "1".to_owned(),
            state: "ok".to_owned(),
        },
        summariser: ModelHealth {
            name: "qwen2.5-0.5b-instruct".to_owned(),
            version: "2.5".to_owned(),
            state: "ok".to_owned(),
        },
        index: IndexHealth {
            present: false,
            schema_version: None,
            plugins_enabled: 0,
            skills_indexed: 0,
            size_bytes: 0,
            integrity_ok: true,
        },
        drift: DriftStatus::None,
        catalogs: Vec::<CatalogCacheHealth>::new(),
        workspace_registry: WorkspaceRegistryStatus {
            present: false,
            tracked: 0,
        },
        harnesses: Vec::new(),
        effective_harness_list: None,
        harness_rules: Vec::new(),
        harness_mcp: Vec::new(),
        detected_uninstalled_harnesses: Vec::new(),
        prompts: Some(PromptsReport {
            prompts: Vec::new(),
            collisions: Vec::new(),
        }),
        orphan_data_dirs: Some(OrphanDataDirReport {
            plugin_data: vec![PathBuf::from("/tmp/orphan/p")],
            workspace_data: vec![PathBuf::from("/tmp/orphan/w")],
        }),
        entry_counts: Some(EntryCountsByKind {
            skills: 12,
            commands: 3,
            agents: 2,
            pending_re_embedding: 1,
        }),
        hooks: None,
        guardrails: None,
        agents: None,
        privilege_escalation: None,
        personas: None,
        legacy_model_manifests: Vec::new(),
        unconverted_plugins: Vec::new(),
        // Phase 9 / US4: empty → `skip_serializing_if = "Vec::is_empty"` omits
        // it from the wire shape, so the byte-stable pin below stays unchanged.
        meta_skills: Vec::new(),
        overall: DoctorClassification::Ok,
        suggested_fixes: Vec::new(),
    };

    let json = serde_json::to_value(&report).expect("serialise");

    // Prompts: { prompts: [], collisions: [] }
    let p = &json["prompts"];
    assert!(p["prompts"].is_array(), "prompts.prompts must be an array");
    assert!(
        p["collisions"].is_array(),
        "prompts.collisions must be an array"
    );

    // Orphan data dirs: { plugin_data: [...], workspace_data: [...] }
    let o = &json["orphan_data_dirs"];
    let pd = o["plugin_data"].as_array().expect("plugin_data array");
    assert_eq!(pd.len(), 1);
    assert_eq!(pd[0], "/tmp/orphan/p");
    let wd = o["workspace_data"]
        .as_array()
        .expect("workspace_data array");
    assert_eq!(wd.len(), 1);
    assert_eq!(wd[0], "/tmp/orphan/w");

    // Entry counts: { skills: N, commands: M, agents: A, pending_re_embedding: K }
    let c = &json["entry_counts"];
    assert_eq!(c["skills"], 12);
    assert_eq!(c["commands"], 3);
    assert_eq!(c["agents"], 2);
    assert_eq!(c["pending_re_embedding"], 1);
}

// TEST-2: with all five Phase 6 fields PRESENT, pin their "appended LAST"
// envelope ORDER. The minimal pin above sets them to `None`, so
// `skip_serializing_if` elides them and nothing freezes their position. Use
// `to_string` + byte offsets (NOT `to_value`, which is key-order-insensitive)
// to prove `hooks` < `guardrails` < `agents` < `privilege_escalation` <
// `personas`, and that all five follow `entry_counts` (NFR-011).
#[test]
fn doctor_json_phase6_fields_appended_last_in_order() {
    use tome::commands::status::{IndexHealth, ModelHealth};
    use tome::doctor::report::{
        AgentsReport, EntryCountsByKind, GuardrailsReport, HooksReport, PersonaReport,
        PrivilegeEscalationReport, WorkspaceRegistryStatus,
    };
    use tome::doctor::{CatalogCacheHealth, DoctorClassification, DoctorReport};
    use tome::index::meta::DriftStatus;
    use tome::workspace::{ScopeKind, WorkspaceInfo, scope::ScopeSource};

    let report = DoctorReport {
        tome_version: env!("CARGO_PKG_VERSION").to_owned(),
        workspace: WorkspaceInfo {
            scope: ScopeKind::Global,
            path: None,
            source: ScopeSource::Flag,
            catalogs: 0,
            plugins_total: 0,
            plugins_enabled: 0,
            skills_indexed: 0,
            schema_version: None,
            embedder: None,
            enrolled_catalogs: Vec::new(),
            enabled_plugins: Vec::new(),
            bound_projects: Vec::new(),
            summary_cache: None,
        },
        project_binding: None,
        embedder: ModelHealth {
            name: "bge-small-en-v1.5".to_owned(),
            version: "1.5".to_owned(),
            state: "ok".to_owned(),
        },
        reranker: ModelHealth {
            name: "bge-reranker-base".to_owned(),
            version: "1".to_owned(),
            state: "ok".to_owned(),
        },
        summariser: ModelHealth {
            name: "qwen2.5-0.5b-instruct".to_owned(),
            version: "2.5".to_owned(),
            state: "ok".to_owned(),
        },
        index: IndexHealth {
            present: false,
            schema_version: None,
            plugins_enabled: 0,
            skills_indexed: 0,
            size_bytes: 0,
            integrity_ok: true,
        },
        drift: DriftStatus::None,
        catalogs: Vec::<CatalogCacheHealth>::new(),
        workspace_registry: WorkspaceRegistryStatus {
            present: false,
            tracked: 0,
        },
        harnesses: Vec::new(),
        effective_harness_list: None,
        harness_rules: Vec::new(),
        harness_mcp: Vec::new(),
        detected_uninstalled_harnesses: Vec::new(),
        prompts: None,
        orphan_data_dirs: None,
        // `entry_counts` is the last pre-Phase-6 field; the five below must all
        // serialise after it.
        entry_counts: Some(EntryCountsByKind {
            skills: 0,
            commands: 0,
            agents: 0,
            pending_re_embedding: 0,
        }),
        // Minimal-but-Some sub-values so `skip_serializing_if` does NOT elide
        // them and their positions are pinned.
        hooks: Some(HooksReport {
            plugins: Vec::new(),
        }),
        guardrails: Some(GuardrailsReport { files: Vec::new() }),
        agents: Some(AgentsReport {
            harnesses: Vec::new(),
        }),
        privilege_escalation: Some(PrivilegeEscalationReport {
            plugins: Vec::new(),
        }),
        personas: Some(PersonaReport {
            personas: Vec::new(),
            drop_persona: "drop-persona".to_owned(),
        }),
        legacy_model_manifests: Vec::new(),
        unconverted_plugins: Vec::new(),
        // Phase 9 / US4: empty → `skip_serializing_if = "Vec::is_empty"` omits
        // it from the wire shape, so the byte-stable pin below stays unchanged.
        meta_skills: Vec::new(),
        overall: DoctorClassification::Ok,
        suggested_fixes: Vec::new(),
    };

    let json = serde_json::to_string(&report).expect("serialise");
    // Match each TOP-LEVEL key by its object-open form (`"key":{`). All six are
    // objects on the wire, so this disambiguates from a same-named scalar field
    // nested elsewhere (e.g. `entry_counts` carries its own `"agents":0`).
    let at = |key: &str| {
        let needle = format!("\"{key}\":{{");
        json.find(&needle)
            .unwrap_or_else(|| panic!("missing {needle}: {json}"))
    };

    let entry_counts = at("entry_counts");
    let hooks = at("hooks");
    let guardrails = at("guardrails");
    let agents = at("agents");
    let privilege = at("privilege_escalation");
    let personas = at("personas");

    // All five Phase 6 fields follow `entry_counts` …
    for (label, pos) in [
        ("hooks", hooks),
        ("guardrails", guardrails),
        ("agents", agents),
        ("privilege_escalation", privilege),
        ("personas", personas),
    ] {
        assert!(
            entry_counts < pos,
            "`{label}` must serialise after `entry_counts`: {json}"
        );
    }
    // … in the documented appended-LAST order.
    assert!(hooks < guardrails, "hooks < guardrails: {json}");
    assert!(guardrails < agents, "guardrails < agents: {json}");
    assert!(agents < privilege, "agents < privilege_escalation: {json}");
    assert!(
        privilege < personas,
        "privilege_escalation < personas: {json}"
    );
}
