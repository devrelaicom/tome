//! Static registration of the vendored `sqlite-vec` extension.
//!
//! The C amalgamation is compiled into the binary by `build.rs`. We register
//! its init function with SQLite's auto-extension hook so every subsequent
//! `Connection::open` automatically loads it — there is no system extension
//! file to find and no `sqlite3_load_extension` call.
//!
//! Registration is process-global and idempotent: a `Once` guards the
//! `sqlite3_auto_extension` call, and the outcome is cached for subsequent
//! callers. Failures map to [`TomeError::VectorExtensionInitFailure`]
//! (exit 35).

use std::os::raw::{c_char, c_int};
use std::sync::Mutex;
use std::sync::Once;

use rusqlite::Connection;
use rusqlite::ffi;

use crate::error::TomeError;

// Bound to the vendored amalgamation by build.rs. The C signature uses the
// loadable-extension calling convention because sqlite-vec is compiled
// without SQLITE_CORE (see vendor/sqlite-vec/sqlite-vec.h).
unsafe extern "C" {
    fn sqlite3_vec_init(
        db: *mut ffi::sqlite3,
        pz_err_msg: *mut *mut c_char,
        api: *const ffi::sqlite3_api_routines,
    ) -> c_int;
}

static REGISTER_ONCE: Once = Once::new();
static REGISTER_RC: Mutex<c_int> = Mutex::new(0);

/// Register `sqlite3_vec_init` with SQLite's auto-extension hook so every
/// subsequent connection picks it up. Idempotent. Returns the cached result
/// on calls after the first.
pub fn register_globally() -> Result<(), TomeError> {
    REGISTER_ONCE.call_once(|| {
        // libsqlite3-sys exposes `sqlite3_auto_extension` with the full C
        // signature, so we can hand the bound `sqlite3_vec_init` straight in.
        let rc = unsafe { ffi::sqlite3_auto_extension(Some(sqlite3_vec_init)) };
        *REGISTER_RC.lock().expect("REGISTER_RC poisoned") = rc;
    });

    let rc = *REGISTER_RC.lock().expect("REGISTER_RC poisoned");
    if rc != ffi::SQLITE_OK {
        return Err(TomeError::VectorExtensionInitFailure(format!(
            "sqlite3_auto_extension returned {rc}"
        )));
    }
    Ok(())
}

/// Confirm the extension is reachable on `conn` by invoking `vec_version()`.
/// Returns the version string on success.
pub fn verify(conn: &Connection) -> Result<String, TomeError> {
    conn.query_row("SELECT vec_version()", [], |row| row.get::<_, String>(0))
        .map_err(|e| TomeError::VectorExtensionInitFailure(format!("vec_version() failed: {e}")))
}
