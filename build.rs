//! Compile the vendored `sqlite-vec` amalgamation into the Tome binary so it
//! can be registered as a SQLite virtual-table extension at runtime. The
//! upstream amalgamation is one C file + one header; we hand it to the `cc`
//! crate, which links the resulting object into our binary alongside the
//! statically-linked SQLite that `rusqlite`'s `bundled` feature provides.
//!
//! Tome's constitution §XII inherits upstream where reasonable; here that
//! means we vendor `sqlite-vec` rather than reimplement vector search. See
//! `vendor/sqlite-vec/README.md` for the pinned version and update procedure.

use std::env;
use std::path::PathBuf;

fn main() {
    let vendor = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
        .join("vendor/sqlite-vec");

    println!(
        "cargo:rerun-if-changed={}",
        vendor.join("sqlite-vec.c").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        vendor.join("sqlite-vec.h").display()
    );

    let mut build = cc::Build::new();
    build
        .file(vendor.join("sqlite-vec.c"))
        .include(&vendor)
        // The amalgamation is intended for static linking inside a host that
        // already statically links SQLite. We rely on rusqlite's `bundled`
        // feature for the SQLite symbols and headers; `libsqlite3-sys` (the
        // rusqlite back-end) sets DEP_SQLITE3_INCLUDE for downstream build
        // scripts.
        .opt_level(3);

    if let Some(include) = env::var_os("DEP_SQLITE3_INCLUDE") {
        build.include(PathBuf::from(include));
    }

    // Disable warnings we have no way to fix in vendored upstream code.
    build
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-unused-function")
        .flag_if_supported("-Wno-implicit-fallthrough")
        .flag_if_supported("-Wno-sign-compare");

    build.compile("sqlite_vec");
}
