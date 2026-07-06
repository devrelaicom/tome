//! `tome exit-codes [<code>]` (issue #436).
//!
//! Prints the exit-code reference — every code with its `--json` error
//! `category` slug and a one-line meaning — or a single code's row. The data
//! is [`crate::error::EXIT_CODES`], the one static table pinned against both
//! the `TomeError` mapping and the docs page (see its doc comment), so this
//! command cannot drift from either.
//!
//! Pure static lookup: no HOME, index, config, or lock. Like `completions`,
//! it is intercepted pre-dispatch in `main.rs` (before `Paths::resolve()` and
//! scope resolution) so it works on a completely unconfigured machine.

use std::io::Write;

use serde::Serialize;

use crate::cli::ExitCodesArgs;
use crate::error::{EXIT_CODES, ExitCodeInfo, TomeError};
use crate::output::{self, Mode};
use crate::presentation::tables;

pub fn run(args: &ExitCodesArgs, mode: Mode) -> Result<(), TomeError> {
    let rows: Vec<&'static ExitCodeInfo> = match args.code {
        Some(code) => vec![find(code)?],
        None => EXIT_CODES.iter().collect(),
    };
    match mode {
        Mode::Json => emit_json(&rows),
        Mode::Human => emit_human(&rows),
    }
}

/// Resolve one code to its table row. An unknown code is a plain usage error
/// (exit 2) — deterministic, and it points back at the full table.
fn find(code: i32) -> Result<&'static ExitCodeInfo, TomeError> {
    EXIT_CODES.iter().find(|r| r.code == code).ok_or_else(|| {
        TomeError::Usage(format!(
            "unknown exit code {code}; run `tome exit-codes` for the full table"
        ))
    })
}

/// The `--json` envelope. Each record mirrors the hand-curated `exitCodes`
/// entries in `site/specs/reference/cli-surface.json` (`code` / `category` /
/// `meaning`, with `category: null` for success) so the two machine surfaces
/// share one shape.
#[derive(Serialize)]
struct Envelope<'a> {
    exit_codes: Vec<Record<'a>>,
}

#[derive(Serialize)]
struct Record<'a> {
    code: i32,
    category: Option<&'a str>,
    meaning: &'a str,
}

fn emit_json(rows: &[&'static ExitCodeInfo]) -> Result<(), TomeError> {
    let env = Envelope {
        exit_codes: rows
            .iter()
            .map(|r| Record {
                code: r.code,
                category: r.category,
                meaning: r.meaning,
            })
            .collect(),
    };
    output::write_json(&env)
}

fn emit_human(rows: &[&'static ExitCodeInfo]) -> Result<(), TomeError> {
    let mut table = tables::new_table();
    table.set_header(vec!["Code", "Category", "Meaning"]);
    for r in rows {
        table.add_row(vec![
            r.code.to_string(),
            // An em-dash for the category-less success row, matching the
            // docs page's rendering.
            r.category.unwrap_or("—").to_owned(),
            r.meaning.to_owned(),
        ]);
    }
    let mut out = std::io::stdout().lock();
    writeln!(out, "{table}")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #436: byte-stable wire pin for the `--json` record shape — `category`
    /// is `null` (not omitted) for the success row, mirroring the
    /// `cli-surface.json` `exitCodes` entries.
    #[test]
    fn json_record_wire_shape_is_pinned() {
        let success = Record {
            code: 0,
            category: None,
            meaning: "Success.",
        };
        assert_eq!(
            serde_json::to_string(&success).unwrap(),
            r#"{"code":0,"category":null,"meaning":"Success."}"#,
        );
        let busy = Record {
            code: 50,
            category: Some("index_busy"),
            meaning: "The index is locked by another process.",
        };
        assert_eq!(
            serde_json::to_string(&busy).unwrap(),
            r#"{"code":50,"category":"index_busy","meaning":"The index is locked by another process."}"#,
        );
    }

    /// A known code resolves to its row; an unknown one is a usage error
    /// (exit 2) naming the code.
    #[test]
    fn find_resolves_known_and_rejects_unknown_codes() {
        assert_eq!(find(50).expect("50 is documented").code, 50);
        let err = find(11).expect_err("11 has never been assigned");
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().contains("11"), "{err}");
    }
}
