//! Table-driven integration tests for the SKILL.md frontmatter parser.
//!
//! The matrix exercises the two failure-mode split mandated by FR-013c
//! (delimiter failure → fatal; YAML-body failure → skip-and-warn) plus the
//! FR-011 / FR-012 fallback rules.
//!
//! Spec: tasks.md T036, FR-011, FR-012, FR-013c.

use std::path::PathBuf;

use tome::plugin::frontmatter::{FrontmatterError, parse_skill_frontmatter_str};

fn dummy_path() -> PathBuf {
    PathBuf::from("/fake/skills/example/SKILL.md")
}

/// Expected outcome for one row of the matrix.
enum Outcome {
    /// Parses cleanly; assertions on resolved fields after fallback.
    Parsed {
        /// (resolved name, fallback flag) given the dir name argument.
        expect_name: (&'static str, bool),
        /// (resolved description, fallback flag).
        expect_description: (&'static str, bool),
    },
    /// Caller maps to exit 23 — header could not be located.
    MissingDelimiters,
    /// Caller skips this single skill per FR-013c — header located but YAML invalid.
    InvalidYaml,
}

struct Case {
    name: &'static str,
    contents: &'static str,
    dir_name: &'static str,
    outcome: Outcome,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "valid header with extra fields is accepted",
            contents: "---\nname: pdf-tools\ndescription: Manipulate PDFs.\nwhen_to_use: anytime\n---\nBody text here.\n",
            dir_name: "ignored-dir",
            outcome: Outcome::Parsed {
                expect_name: ("pdf-tools", false),
                expect_description: ("Manipulate PDFs.", false),
            },
        },
        Case {
            name: "missing name falls back to directory name (FR-011)",
            contents: "---\ndescription: Manipulate PDFs.\n---\nBody.\n",
            dir_name: "pdf-tools",
            outcome: Outcome::Parsed {
                expect_name: ("pdf-tools", true),
                expect_description: ("Manipulate PDFs.", false),
            },
        },
        Case {
            name: "empty name string falls back (FR-011)",
            contents: "---\nname: \"   \"\ndescription: ok\n---\nbody\n",
            dir_name: "fallback-dir",
            outcome: Outcome::Parsed {
                expect_name: ("fallback-dir", true),
                expect_description: ("ok", false),
            },
        },
        Case {
            name: "missing description falls back to body prefix (FR-012)",
            contents: "---\nname: pdf-tools\n---\nThis is the body text. It will be quoted by the fallback.\n",
            dir_name: "pdf-tools",
            outcome: Outcome::Parsed {
                expect_name: ("pdf-tools", false),
                expect_description: (
                    "This is the body text. It will be quoted by the fallback.\n",
                    true,
                ),
            },
        },
        Case {
            name: "both name and description missing apply both fallbacks",
            contents: "---\nwhen_to_use: rarely\n---\nA short body.\n",
            dir_name: "tiny-skill",
            outcome: Outcome::Parsed {
                expect_name: ("tiny-skill", true),
                expect_description: ("A short body.\n", true),
            },
        },
        Case {
            name: "missing opening delimiter is a delimiter failure",
            contents: "name: pdf-tools\ndescription: Manipulate PDFs.\nBody.\n",
            dir_name: "pdf-tools",
            outcome: Outcome::MissingDelimiters,
        },
        Case {
            name: "missing closing delimiter is a delimiter failure",
            contents: "---\nname: pdf-tools\ndescription: ok\nbody without close\n",
            dir_name: "pdf-tools",
            outcome: Outcome::MissingDelimiters,
        },
        Case {
            name: "no delimiters at all is a delimiter failure",
            contents: "Just a body with no header.\n",
            dir_name: "skillless",
            outcome: Outcome::MissingDelimiters,
        },
        Case {
            name: "malformed YAML body is a per-skill skip (FR-013c)",
            contents: "---\nname: pdf-tools\ndescription: ok\n  : : not valid\n---\nbody\n",
            dir_name: "pdf-tools",
            outcome: Outcome::InvalidYaml,
        },
        Case {
            name: "empty yaml block parses with both fallbacks",
            contents: "---\n---\nbody text\n",
            dir_name: "empty-header",
            outcome: Outcome::Parsed {
                expect_name: ("empty-header", true),
                expect_description: ("body text\n", true),
            },
        },
        Case {
            name: "BOM-prefixed file is accepted",
            contents: "\u{FEFF}---\nname: with-bom\ndescription: yes\n---\nbody\n",
            dir_name: "ignored",
            outcome: Outcome::Parsed {
                expect_name: ("with-bom", false),
                expect_description: ("yes", false),
            },
        },
        Case {
            name: "CRLF line endings are accepted",
            contents: "---\r\nname: crlf\r\ndescription: yes\r\n---\r\nbody\r\n",
            dir_name: "ignored",
            outcome: Outcome::Parsed {
                expect_name: ("crlf", false),
                expect_description: ("yes", false),
            },
        },
    ]
}

#[test]
fn frontmatter_matrix() {
    let path = dummy_path();
    for case in cases() {
        let result = parse_skill_frontmatter_str(&path, case.contents);
        match (result, &case.outcome) {
            (
                Ok(parsed),
                Outcome::Parsed {
                    expect_name,
                    expect_description,
                },
            ) => {
                let resolved_name = parsed.resolved_name(case.dir_name);
                assert_eq!(
                    resolved_name,
                    (expect_name.0.to_owned(), expect_name.1),
                    "case `{}`: resolved name mismatch",
                    case.name
                );
                let resolved_desc = parsed.resolved_description();
                assert_eq!(
                    resolved_desc,
                    (expect_description.0.to_owned(), expect_description.1),
                    "case `{}`: resolved description mismatch",
                    case.name
                );
            }
            (Err(FrontmatterError::MissingDelimiters { .. }), Outcome::MissingDelimiters) => {}
            (Err(FrontmatterError::InvalidYaml { .. }), Outcome::InvalidYaml) => {}
            (got, expected) => panic!(
                "case `{}`: outcome mismatch — got {:?}, expected {}",
                case.name,
                got.as_ref()
                    .map(|_| "Ok(parsed)")
                    .map_err(|e| format!("{e:?}")),
                match expected {
                    Outcome::Parsed { .. } => "Parsed",
                    Outcome::MissingDelimiters => "MissingDelimiters",
                    Outcome::InvalidYaml => "InvalidYaml",
                }
            ),
        }
    }
}

#[test]
fn description_fallback_caps_at_500_chars() {
    // Build a body of 600 ASCII characters; fallback must keep exactly 500.
    let body: String = std::iter::repeat_n('a', 600).collect();
    let contents = format!("---\nname: long\n---\n{body}");
    let parsed = parse_skill_frontmatter_str(&dummy_path(), &contents)
        .expect("valid frontmatter, missing description");
    let (resolved, applied) = parsed.resolved_description();
    assert!(applied, "fallback must be marked applied");
    assert_eq!(resolved.chars().count(), 500);
}

#[test]
fn description_fallback_counts_chars_not_bytes() {
    // 4-byte char (😀) repeated 250 times = 250 chars, 1000 bytes.
    // Limit is 500 chars; entire string should pass through.
    let body: String = std::iter::repeat_n('😀', 250).collect();
    let contents = format!("---\nname: emoji\n---\n{body}");
    let parsed = parse_skill_frontmatter_str(&dummy_path(), &contents).expect("valid frontmatter");
    let (resolved, applied) = parsed.resolved_description();
    assert!(applied);
    assert_eq!(resolved.chars().count(), 250);
}
