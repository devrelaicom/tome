//! `comfy-table` helpers. Tables are the default Phase 2 rendering for any
//! list (catalogs, plugins, models, query results). The same data is also
//! available in `--json` form (see [`crate::output`]); structured output is
//! byte-stable regardless of terminal context (FR-041).
//!
//! Rendering choices:
//! - When stdout is a real terminal: a UTF-8 rounded preset, dim divider.
//! - When stdout is not a terminal: ASCII pipes and dashes, no glyphs, no
//!   colour — clean to grep and to redirect into files (FR-046).
//!
//! Colour application is the caller's responsibility (via
//! [`crate::presentation::colour`]); this module never injects ANSI itself.

use comfy_table::{ContentArrangement, Table, presets};

use crate::output;

/// Build a [`comfy_table::Table`] pre-configured for the current terminal
/// context. The caller adds the header and rows.
pub fn new_table() -> Table {
    let mut t = Table::new();
    if output::stdout_is_tty() {
        t.load_preset(presets::UTF8_FULL_CONDENSED)
            .set_content_arrangement(ContentArrangement::Dynamic);
    } else {
        // Plain ASCII grid — readable when redirected to a file or piped.
        t.load_preset(presets::ASCII_MARKDOWN)
            .set_content_arrangement(ContentArrangement::Disabled);
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_table_returns_a_usable_table_in_either_mode() {
        // The TTY-detection branch flips at runtime; we cannot easily force
        // it from a unit test. We can still assert the function produces a
        // non-empty rendered string for either branch.
        let mut t = new_table();
        t.set_header(vec!["catalog", "plugin"]);
        t.add_row(vec!["midnight-experts", "compact-expert"]);
        let rendered = t.to_string();
        assert!(rendered.contains("catalog"));
        assert!(rendered.contains("compact-expert"));
        // The rendered table is at least one line.
        assert!(rendered.lines().count() >= 1);
    }

    #[test]
    fn new_table_handles_empty_body() {
        let mut t = new_table();
        t.set_header(vec!["a", "b"]);
        let rendered = t.to_string();
        assert!(rendered.contains("a"));
        assert!(rendered.contains("b"));
    }
}
