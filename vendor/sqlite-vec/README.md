# Vendored: sqlite-vec

Source: https://github.com/asg017/sqlite-vec
Version: **v0.1.9** (sqlite-vec-0.1.9-amalgamation.tar.gz)
Pinned: 2026-05-12
Licence: MIT OR Apache-2.0 (both LICENSE files in this directory)

## Why vendored

Tome's constitution (principle XII) inherits where the host system already
does the job, and statically links small upstream sources rather than
reimplementing capability. `sqlite-vec` is the embedded vector-search
extension for the bundled SQLite that `rusqlite`'s `bundled` feature ships.
Vendoring as a single C file with its header is the upstream-recommended
distribution path for embedded use.

## Updating

1. Download a new amalgamation tarball from
   https://github.com/asg017/sqlite-vec/releases.
2. Replace `sqlite-vec.c` and `sqlite-vec.h`.
3. Re-fetch `LICENSE-MIT` and `LICENSE-APACHE` if they have changed.
4. Bump the "Version" / "Pinned" lines above.
5. Run `cargo test` — the `tests/index_schema_bootstrap.rs` suite exercises
   the registered virtual table; that's the smoke test that the new amalgamation
   builds cleanly and the API has not shifted.

## Compilation

Compiled into the Tome binary by `build.rs` at the repository root, against
the SQLite headers shipped by `rusqlite`'s `bundled` feature. See
`build.rs` for details.
