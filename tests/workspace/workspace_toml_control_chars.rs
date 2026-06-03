//! F-WS-TOML-NEWLINE (Phase 7 / FR-005) — workspace settings.toml cannot be
//! poisoned by a control-char catalog name.
//!
//! Two legs, mirroring the two-pronged defence:
//!
//! (a) BOUNDARY REJECT — `CatalogManifest::parse_and_validate` refuses a
//!     recognised `name` value carrying a newline / control char at ingest,
//!     reusing the same `ManifestInvalid::MissingField { key: "name" }`
//!     variant the empty-name check emits (no new variant / exit code).
//!
//! (b) EMISSION ROBUSTNESS — defence-in-depth for a poisoned name ALREADY in
//!     the `workspace_catalogs` DB. `tome workspace init --inherit-global`
//!     copies pre-existing global catalog names from the index into the new
//!     workspace's settings.toml, BYPASSING the manifest boundary. The
//!     emitted settings.toml MUST stay parseable (toml_edit escapes any
//!     string), so a subsequent read does not brick with exit-70
//!     `WorkspaceMalformed`.

use crate::common::lifecycle_paths;
use tempfile::TempDir;
use tome::catalog::manifest::CatalogManifest;
use tome::error::ManifestInvalid;
use tome::index::{self, OpenOptions, workspace_catalogs};
use tome::workspace::{self, WorkspaceName};

// ---------------------------------------------------------------------------
// (a) Manifest boundary: control chars in the recognised `name` are rejected.
// ---------------------------------------------------------------------------

fn parse_manifest(text: &str) -> Result<CatalogManifest, ManifestInvalid> {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let manifest = root.join("tome-catalog.toml");
    std::fs::write(&manifest, text).unwrap();
    CatalogManifest::parse_and_validate(&manifest, &root, text.as_bytes())
}

/// A clean catalog name still parses — guards against the reject being too
/// aggressive.
#[test]
fn clean_catalog_name_parses() {
    let good = r#"
name = "tidy-catalog"
description = "y"
version = "0.1.0"

[owner]
name = "n"
email = "n@e.co"
"#;
    parse_manifest(good).expect("clean catalog name should parse");
}

/// A `name` whose VALUE carries a literal newline is rejected at the manifest
/// boundary. The newline rides inside a TOML basic string with an escape
/// (`\n`) so the file itself is syntactically valid TOML — the rejection must
/// come from the semantic value check, not a TOML parse error.
#[test]
fn catalog_name_with_newline_rejected() {
    let bad = r#"
name = "evil\nname"
description = "y"
version = "0.1.0"

[owner]
name = "n"
email = "n@e.co"
"#;
    let err = parse_manifest(bad).unwrap_err();
    assert!(
        matches!(err, ManifestInvalid::MissingField { ref key, .. } if key == "name"),
        "expected the empty-name-class reject (MissingField key=name), got: {err:?}",
    );
}

/// A `name` carrying a non-newline control char (BEL, U+0007) is likewise
/// rejected — the check is on the control-char class, not newline alone.
///
/// The char is written into the TOML as the six-byte escape sequence
/// backslash-u-0-0-0-7 so the file is syntactically valid TOML — raw control
/// chars are a TOML *parse* error and would never reach the semantic check.
/// TOML decodes the escape to a real BEL, which the value check must reject.
#[test]
fn catalog_name_with_bell_control_char_rejected() {
    // `\\u0007` in a normal (non-raw) Rust string yields the literal
    // backslash-u-0-0-0-7 in the TOML text; TOML then decodes it to U+0007.
    let bad = "
name = \"evil\\u0007name\"
description = \"y\"
version = \"0.1.0\"

[owner]
name = \"n\"
email = \"n@e.co\"
";
    // Sanity: the source carries the escape, not a pre-decoded raw BEL.
    assert!(
        bad.contains("\\u0007"),
        "fixture must embed the TOML escape"
    );
    let err = parse_manifest(bad).unwrap_err();
    assert!(
        matches!(err, ManifestInvalid::MissingField { ref key, .. } if key == "name"),
        "expected the empty-name-class reject (MissingField key=name), got: {err:?}",
    );
}

// ---------------------------------------------------------------------------
// (b) Emission robustness: a poisoned name already in the DB still emits
//     parseable settings.toml via the --inherit-global path.
// ---------------------------------------------------------------------------

fn parse(name: &str) -> WorkspaceName {
    WorkspaceName::parse(name).expect("valid workspace name")
}

fn open_central(paths: &tome::paths::Paths) -> rusqlite::Connection {
    let (e, r, s) = tome::commands::plugin::registry_seeds();
    index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: e,
            reranker: r,
            summariser: s,
        },
    )
    .expect("open central DB")
}

/// Clean inherited catalog name: the emitted settings.toml round-trips and
/// reads back without error. Pins the normal-name path as still green.
#[test]
fn inherit_global_clean_name_emits_parseable_settings() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    {
        let conn = open_central(&paths);
        workspace_catalogs::insert(&conn, "global", "tidy", "https://example.com/a", "main")
            .unwrap();
    }

    let outcome = workspace::init::init(parse("clean-ws"), true, &paths).expect("init");
    let body = std::fs::read_to_string(paths.workspace_settings_file(&outcome.name)).unwrap();

    // The live read path (the one that bricks at exit 70 on bad TOML).
    let settings = tome::settings::parser::parse_workspace(&body)
        .expect("clean settings.toml should parse via the live read path");
    assert_eq!(settings.name.as_str(), "clean-ws");
    assert_eq!(settings.catalogs.len(), 1);
    assert_eq!(settings.catalogs[0].name, "tidy");
}

/// A catalog name carrying a newline is seeded DIRECTLY into the
/// `workspace_catalogs` table (bypassing the manifest boundary, exactly as a
/// pre-existing poisoned global enrolment would). `init --inherit-global`
/// copies it into the new workspace's settings.toml.
///
/// TODAY (hand-rolled `escape_toml_basic`): the raw newline lands inside a
/// basic string and breaks TOML → the read path returns an error (the exit-70
/// brick this fix prevents).
///
/// AFTER (toml_edit emission): the newline is escaped, the document parses,
/// and the live read path round-trips the literal newline back into the
/// catalog name.
#[test]
fn inherit_global_poisoned_catalog_name_emits_parseable_settings() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    let poisoned = "evil\nname = \"smuggled\"\n[[catalogs]]\nname = \"pwned";
    {
        let conn = open_central(&paths);
        // insert() is raw SQL with no name validation — this models a
        // poisoned global enrolment already resident in the DB.
        workspace_catalogs::insert(&conn, "global", poisoned, "https://example.com/a", "main")
            .unwrap();
    }

    let outcome =
        workspace::init::init(parse("poisoned-ws"), true, &paths).expect("init should succeed");

    let settings_path = paths.workspace_settings_file(&outcome.name);
    let body = std::fs::read_to_string(&settings_path).unwrap();

    // 1. The raw document must parse as TOML (toml_edit is the emission
    //    mechanism; this is the structural defence).
    body.parse::<toml_edit::DocumentMut>().unwrap_or_else(|e| {
        panic!("emitted settings.toml must be parseable TOML, got error: {e}\n---\n{body}\n---")
    });

    // 2. The live read path (the one that surfaces WorkspaceMalformed / exit
    //    70) must succeed — no brick.
    let settings = tome::settings::parser::parse_workspace(&body).unwrap_or_else(|e| {
        panic!(
            "live read path must not brick on a poisoned catalog name, got: {e}\n---\n{body}\n---"
        )
    });

    // 3. The poisoned name round-trips verbatim (escaping is lossless) and did
    //    NOT smuggle a second catalog entry into the parsed structure.
    assert_eq!(settings.name.as_str(), "poisoned-ws");
    assert_eq!(
        settings.catalogs.len(),
        1,
        "exactly one catalog entry — the newline must not inject a second [[catalogs]]: {:?}",
        settings.catalogs,
    );
    assert_eq!(
        settings.catalogs[0].name, poisoned,
        "poisoned name must round-trip verbatim",
    );
}
