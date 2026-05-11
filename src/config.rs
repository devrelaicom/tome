//! The top-level config document and the per-catalog registry entry.
//! `BTreeMap` keying ensures deterministic ordering for `tome catalog list`
//! (FR-006).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub name: String,
    pub url: String,
    #[serde(rename = "ref")]
    pub ref_: String,
    pub path: PathBuf,
    #[serde(with = "time::serde::rfc3339")]
    pub last_synced: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub catalogs: BTreeMap<String, CatalogEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_round_trips() {
        let c = Config::default();
        let s = toml::to_string(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn entry_round_trips_through_toml() {
        let mut c = Config::default();
        c.catalogs.insert(
            "midnight-experts".into(),
            CatalogEntry {
                name: "midnight-experts".into(),
                url: "https://github.com/midnight/midnight-experts".into(),
                ref_: "main".into(),
                path: PathBuf::from("/tmp/x"),
                last_synced: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            },
        );
        let s = toml::to_string(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn unknown_top_level_field_rejected() {
        let toml = r#"
unexpected = "value"

[catalogs.foo]
name = "foo"
url = "https://example/"
ref = "main"
path = "/x"
last_synced = "2026-01-01T00:00:00Z"
"#;
        let err = toml::from_str::<Config>(toml).unwrap_err();
        assert!(err.to_string().to_lowercase().contains("unknown"));
    }

    #[test]
    fn unknown_field_inside_catalog_rejected() {
        let toml = r#"
[catalogs.foo]
name = "foo"
url = "https://example/"
ref = "main"
path = "/x"
last_synced = "2026-01-01T00:00:00Z"
extra = "nope"
"#;
        let err = toml::from_str::<Config>(toml).unwrap_err();
        assert!(err.to_string().to_lowercase().contains("unknown"));
    }
}
