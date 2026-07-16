//! URI resolution for `get_skill` — parse a loose URI into candidate
//! identities, resolve each against the index, collapse to one/many/none.

/// One candidate interpretation of a URI, to be resolved against the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Candidate {
    /// A filesystem path (absolute or relative fragment) to match against
    /// enabled entries' resolved body paths.
    Path(String),
    /// A fully-qualified `(catalog, plugin, name)`.
    Triple { catalog: String, plugin: String, name: String },
    /// A `(plugin, name)` to resolve across all catalogs.
    PluginName { plugin: String, name: String },
    /// A bare entry name to resolve across the whole workspace.
    BareName(String),
}

/// True when `s` should be treated as a filesystem path rather than a
/// delimited name.
fn looks_like_path(s: &str) -> bool {
    s.contains('/')
        || s.contains(std::path::MAIN_SEPARATOR)
        || s.starts_with('.')
        || s.starts_with('~')
        || std::path::Path::new(s).is_absolute()
        || s.ends_with(".md")
}

/// Split `s` on `delim`; return `Triple` (3 parts) or `PluginName` (2 parts),
/// or `None` when the segment count or any segment is invalid.
fn segments_to_candidate(parts: &[&str]) -> Option<Candidate> {
    if parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    match parts.len() {
        3 => Some(Candidate::Triple {
            catalog: parts[0].to_owned(),
            plugin: parts[1].to_owned(),
            name: parts[2].to_owned(),
        }),
        2 => Some(Candidate::PluginName {
            plugin: parts[0].to_owned(),
            name: parts[1].to_owned(),
        }),
        _ => None,
    }
}

/// Parse a loose URI into candidate identities. Pure — performs no I/O. An
/// empty result means the URI is malformed/empty. See the module rules.
pub fn parse_uri(uri: &str) -> Vec<Candidate> {
    let uri = uri.trim();
    if uri.is_empty() {
        return Vec::new();
    }

    if looks_like_path(uri) {
        return vec![Candidate::Path(uri.to_owned())];
    }

    if uri.contains(':') {
        let parts: Vec<&str> = uri.split(':').collect();
        return segments_to_candidate(&parts).into_iter().collect();
    }

    if uri.contains("__") {
        let parts: Vec<&str> = uri.split("__").collect();
        return segments_to_candidate(&parts).into_iter().collect();
    }

    if uri.contains('_') {
        let tokens: Vec<&str> = uri.split('_').collect();
        let mut out: Vec<Candidate> = Vec::new();
        // 2-way partitions → PluginName.
        for i in 1..tokens.len() {
            out.push(Candidate::PluginName {
                plugin: tokens[..i].join("_"),
                name: tokens[i..].join("_"),
            });
        }
        // 3-way partitions → Triple.
        for i in 1..tokens.len() {
            for j in (i + 1)..tokens.len() {
                out.push(Candidate::Triple {
                    catalog: tokens[..i].join("_"),
                    plugin: tokens[i..j].join("_"),
                    name: tokens[j..].join("_"),
                });
            }
        }
        // Fallback: the name itself may contain underscores.
        out.push(Candidate::BareName(uri.to_owned()));
        return out;
    }

    // Bare token: try as a relative path fragment AND as a bare name.
    vec![Candidate::Path(uri.to_owned()), Candidate::BareName(uri.to_owned())]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colon_three_segments_is_triple() {
        assert_eq!(
            parse_uri("cat:plug:skill"),
            vec![Candidate::Triple {
                catalog: "cat".into(),
                plugin: "plug".into(),
                name: "skill".into()
            }]
        );
    }

    #[test]
    fn colon_two_segments_is_plugin_name() {
        assert_eq!(
            parse_uri("plug:skill"),
            vec![Candidate::PluginName { plugin: "plug".into(), name: "skill".into() }]
        );
    }

    #[test]
    fn double_underscore_three_segments_is_triple() {
        assert_eq!(
            parse_uri("cat__plug__skill"),
            vec![Candidate::Triple {
                catalog: "cat".into(),
                plugin: "plug".into(),
                name: "skill".into()
            }]
        );
    }

    #[test]
    fn single_underscore_emits_all_partitions_plus_bare() {
        let got = parse_uri("a_b_c");
        // 2-way: (a | b_c), (a_b | c)
        assert!(got.contains(&Candidate::PluginName { plugin: "a".into(), name: "b_c".into() }));
        assert!(got.contains(&Candidate::PluginName { plugin: "a_b".into(), name: "c".into() }));
        // 3-way: (a | b | c)
        assert!(got.contains(&Candidate::Triple {
            catalog: "a".into(), plugin: "b".into(), name: "c".into()
        }));
        // Bare fallback for a name that itself contains underscores.
        assert!(got.contains(&Candidate::BareName("a_b_c".into())));
    }

    #[test]
    fn absolute_and_dotted_and_md_are_paths() {
        assert_eq!(parse_uri("/abs/SKILL.md"), vec![Candidate::Path("/abs/SKILL.md".into())]);
        assert_eq!(parse_uri("./rel/dir"), vec![Candidate::Path("./rel/dir".into())]);
        assert_eq!(parse_uri("SKILL.md"), vec![Candidate::Path("SKILL.md".into())]);
        assert_eq!(parse_uri("a/b"), vec![Candidate::Path("a/b".into())]);
    }

    #[test]
    fn bare_token_is_path_fragment_and_bare_name() {
        assert_eq!(
            parse_uri("basic-start"),
            vec![Candidate::Path("basic-start".into()), Candidate::BareName("basic-start".into())]
        );
    }

    #[test]
    fn empty_or_all_delimiters_is_malformed() {
        assert!(parse_uri("").is_empty());
        assert!(parse_uri("   ").is_empty());
        assert!(parse_uri(":").is_empty());
        assert!(parse_uri("::").is_empty());
    }
}
