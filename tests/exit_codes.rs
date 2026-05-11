//! Exhaustive match over every `TomeError` variant. Adding a variant without
//! updating this test is a compile error, which is the entire point of the
//! closed-set guarantee in FR-022.

use std::io;
use std::path::PathBuf;

use tome::error::{ManifestInvalid, TomeError};

fn dummy_io_error() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "x")
}

fn build_each_variant() -> Vec<(TomeError, i32, &'static str)> {
    // The exhaustive arm below makes this the source of coverage: every variant
    // must produce a row. The compiler refuses to compile this file if a new
    // variant is added without touching it.
    vec![
        (TomeError::Internal(anyhow::anyhow!("boom")), 1, "internal"),
        (TomeError::Usage("bad flag".into()), 2, "usage"),
        (
            TomeError::CatalogNotFound("foo".into()),
            3,
            "catalog_not_found",
        ),
        (
            TomeError::CatalogAlreadyExists("foo".into()),
            4,
            "catalog_already_exists",
        ),
        (
            TomeError::ManifestInvalid(ManifestInvalid::MissingField {
                file: PathBuf::from("tome-catalog.toml"),
                key: "name".into(),
            }),
            5,
            "manifest_invalid",
        ),
        (
            TomeError::GitFailed {
                catalog: "foo".into(),
                detail: "fatal: …".into(),
            },
            6,
            "git_failed",
        ),
        (TomeError::Io(dummy_io_error()), 7, "io"),
        (TomeError::Interrupted, 8, "interrupted"),
    ]
}

#[test]
fn every_variant_has_documented_exit_code_and_category() {
    for (err, expected_code, expected_category) in build_each_variant() {
        assert_eq!(
            err.exit_code(),
            expected_code,
            "variant {:?} produced unexpected exit code",
            err
        );
        assert_eq!(
            err.category(),
            expected_category,
            "variant {:?} produced unexpected category",
            err
        );
    }
}

#[test]
fn exhaustive_match_compile_check() {
    // If a new variant is added to `TomeError`, this match stops being
    // exhaustive and the file fails to compile. That failure is the test.
    fn _code_for(err: &TomeError) -> i32 {
        match err {
            TomeError::Internal(_) => 1,
            TomeError::Usage(_) => 2,
            TomeError::CatalogNotFound(_) => 3,
            TomeError::CatalogAlreadyExists(_) => 4,
            TomeError::ManifestInvalid(_) => 5,
            TomeError::GitFailed { .. } => 6,
            TomeError::Io(_) => 7,
            TomeError::Interrupted => 8,
        }
    }
}
