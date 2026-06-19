//! Build script. Two jobs:
//!
//! 1. Compile the vendored `sqlite-vec` amalgamation into the Tome binary so it
//!    can be registered as a SQLite virtual-table extension at runtime. The
//!    upstream amalgamation is one C file + one header; we hand it to the `cc`
//!    crate, which links the resulting object into our binary alongside the
//!    statically-linked SQLite that `rusqlite`'s `bundled` feature provides.
//!
//! 2. Generate the **embedded meta-skill manifest** (Phase 9). We walk the
//!    authored `assets/meta-skills/**` tree and emit a generated Rust source
//!    into `OUT_DIR` declaring `META_SKILLS: &[EmbeddedMetaSkill]`, pulling every
//!    file in with `include_bytes!`. `src/authoring/meta.rs` `include!`s it. This
//!    keeps the skills offline (versioned with the binary) with O(1) runtime
//!    lookup and **zero new dependency** — `sha2`/`hex` (used only to compute the
//!    per-skill content-hash revision) are already regular dependencies, so
//!    listing them under `[build-dependencies]` adds no package to the lockfile.
//!
//! Tome's constitution §XII inherits upstream where reasonable; here that
//! means we vendor `sqlite-vec` rather than reimplement vector search. See
//! `vendor/sqlite-vec/README.md` for the pinned version and update procedure.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Total embedded meta-skill bytes must stay well under the 50 MB binary cap
/// (NFR-008). The shipped assets are a few KB of prose; this budget is a wide
/// regression backstop that fails the build long before the binary cap is at
/// risk (the CI release-binary-size check is the final backstop). Raise it
/// deliberately if a future skill genuinely needs more.
const META_SKILL_BYTE_BUDGET: u64 = 5 * 1024 * 1024; // 5 MiB

/// Total embedded harness-plugin (TS shim) bytes must stay well under the 50 MB
/// binary cap (Phase 11, R6). The shipped shims are a few KB of TypeScript; this
/// budget is a wide regression backstop, mirroring `META_SKILL_BYTE_BUDGET`.
const HARNESS_PLUGIN_BYTE_BUDGET: u64 = 1024 * 1024; // 1 MiB

fn main() {
    compile_sqlite_vec();
    generate_meta_skill_manifest();
    generate_harness_plugin_manifest();
}

fn compile_sqlite_vec() {
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

// ---------------------------------------------------------------------------
// Embedded meta-skill manifest (Phase 9)
// ---------------------------------------------------------------------------

/// One embedded file: its POSIX-relative path inside the skill folder plus the
/// absolute on-disk source path we `include_bytes!`.
struct ManifestFile {
    rel_path: String,
    abs_path: PathBuf,
    bytes: Vec<u8>,
}

/// One embedded skill folder, fully validated.
struct ManifestSkill {
    id: String,
    summary: String,
    prompt_name: Option<String>,
    revision: String,
    files: Vec<ManifestFile>,
}

fn generate_meta_skill_manifest() {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let assets_root = manifest_dir.join("assets/meta-skills");

    // Rebuild when the asset tree changes (the directory itself catches
    // adds/removes; each file is registered individually below).
    println!("cargo:rerun-if-changed={}", assets_root.display());

    let mut skills: Vec<ManifestSkill> = Vec::new();
    if assets_root.is_dir() {
        let mut entries: Vec<PathBuf> = fs::read_dir(&assets_root)
            .unwrap_or_else(|e| panic!("read assets/meta-skills/: {e}"))
            .map(|e| e.expect("dir entry").path())
            .filter(|p| p.is_dir())
            .collect();
        entries.sort();
        for skill_dir in entries {
            skills.push(load_skill(&skill_dir));
        }
    }

    // Binary-size budget (NFR-008): fail the build well under the 50 MB cap.
    let total: u64 = skills
        .iter()
        .flat_map(|s| s.files.iter())
        .map(|f| f.bytes.len() as u64)
        .sum();
    assert!(
        total <= META_SKILL_BYTE_BUDGET,
        "embedded meta-skill assets total {total} bytes, over the {META_SKILL_BYTE_BUDGET}-byte \
         budget (NFR-008). Trim the assets or raise META_SKILL_BYTE_BUDGET deliberately."
    );

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let generated = out_dir.join("meta_skills_manifest.rs");
    fs::write(&generated, render_manifest(&skills))
        .unwrap_or_else(|e| panic!("write {}: {e}", generated.display()));
}

/// Walk one `assets/meta-skills/<id>/` folder, validate it, and build the
/// in-memory record. Panics (failing the build) on any invariant violation —
/// that is the build-time validation gate the contract requires.
fn load_skill(skill_dir: &Path) -> ManifestSkill {
    let id = skill_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_else(|| panic!("non-UTF-8 skill dir name at {}", skill_dir.display()))
        .to_owned();
    assert!(
        is_safe_segment(&id),
        "meta-skill id `{id}` is not a safe path segment (no empty, `.`, `..`, leading-dot, \
         `/`, `\\`, or NUL)"
    );

    let mut files: Vec<ManifestFile> = Vec::new();
    collect_files(skill_dir, skill_dir, &mut files);
    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    // Exactly one SKILL.md at the folder root.
    let skill_md_count = files.iter().filter(|f| f.rel_path == "SKILL.md").count();
    assert!(
        skill_md_count == 1,
        "meta-skill `{id}` must have exactly one root SKILL.md, found {skill_md_count}"
    );

    // Register each file for change-detection.
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.abs_path.display());
    }

    let revision = content_revision(&files);
    let skill_md = files
        .iter()
        .find(|f| f.rel_path == "SKILL.md")
        .expect("SKILL.md presence asserted above");
    let frontmatter = std::str::from_utf8(&skill_md.bytes)
        .unwrap_or_else(|_| panic!("meta-skill `{id}` SKILL.md is not valid UTF-8"));
    let summary = frontmatter_value(frontmatter, "description").unwrap_or_else(|| {
        panic!("meta-skill `{id}` SKILL.md frontmatter has no single-line `description:`")
    });
    let prompt_name = frontmatter_value(frontmatter, "tome_prompt_name");

    ManifestSkill {
        id,
        summary,
        prompt_name,
        revision,
        files,
    }
}

/// Recursively collect every file under `dir`, recording its path relative to
/// `root` (POSIX `/` separators). Rejects any non-`Normal` / absolute rel path.
fn collect_files(root: &Path, dir: &Path, out: &mut Vec<ManifestFile>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|e| e.expect("dir entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_files(root, &path, out);
        } else if path.is_file() {
            let rel = path
                .strip_prefix(root)
                .expect("path is under root by construction");
            let rel_path = rel_to_posix(rel);
            let bytes = fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            out.push(ManifestFile {
                rel_path,
                abs_path: path,
                bytes,
            });
        }
    }
}

/// Convert a relative path to a POSIX string, asserting every component is a
/// plain `Normal` segment (no `..`, no absolute prefix, no root).
fn rel_to_posix(rel: &Path) -> String {
    use std::path::Component;
    let mut segs: Vec<String> = Vec::new();
    for comp in rel.components() {
        match comp {
            Component::Normal(s) => segs.push(
                s.to_str()
                    .unwrap_or_else(|| panic!("non-UTF-8 path component in {}", rel.display()))
                    .to_owned(),
            ),
            other => panic!(
                "meta-skill file path {} contains a non-Normal component {other:?}",
                rel.display()
            ),
        }
    }
    segs.join("/")
}

/// Deterministic content fingerprint over the sorted file set: for each file,
/// hash its rel path then its bytes (both NUL-terminated so concatenation is
/// unambiguous). Returns a short hex prefix — equality is the only operation
/// drift performs (R-2), so collision resistance of the full digest is ample.
fn content_revision(files: &[ManifestFile]) -> String {
    let mut hasher = Sha256::new();
    for f in files {
        hasher.update(f.rel_path.as_bytes());
        hasher.update([0u8]);
        hasher.update(&f.bytes);
        hasher.update([0u8]);
    }
    let digest = hasher.finalize();
    hex::encode(&digest[..8]) // 16 hex chars
}

/// Extract a single-line frontmatter scalar `key: value` from the leading
/// `---`-delimited YAML block. Authored embedded skills keep these keys on one
/// line by convention (the lint gate + a meta.rs test enforce the rest);
/// surrounding single/double quotes are stripped. Returns `None` if absent.
fn frontmatter_value(content: &str, key: &str) -> Option<String> {
    let mut lines = content.lines();
    // First non-empty line must be the opening `---`.
    let opened = lines.by_ref().find(|l| !l.trim().is_empty());
    if opened.map(str::trim) != Some("---") {
        return None;
    }
    for line in lines {
        if line.trim() == "---" {
            break; // end of frontmatter
        }
        if let Some(rest) = line.trim_start().strip_prefix(key)
            && let Some(value) = rest.strip_prefix(':')
        {
            let v = value.trim();
            let v = v
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                .unwrap_or(v)
                .trim();
            if !v.is_empty() {
                return Some(v.to_owned());
            }
        }
    }
    None
}

/// A safe single path segment: non-empty, none of `/ \ NUL . ..`, no leading dot.
fn is_safe_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s.starts_with('.')
        && !s.contains('/')
        && !s.contains('\\')
        && !s.contains('\0')
}

/// Render the generated Rust source. Types (`EmbeddedMetaSkill`/`EmbeddedFile`)
/// are defined at the `include!` site in `src/authoring/meta.rs`.
fn render_manifest(skills: &[ManifestSkill]) -> String {
    let mut out = String::new();
    out.push_str("// @generated by build.rs — do not edit. Source: assets/meta-skills/**\n");
    out.push_str("pub static META_SKILLS: &[EmbeddedMetaSkill] = &[\n");
    for s in skills {
        out.push_str("    EmbeddedMetaSkill {\n");
        out.push_str(&format!("        id: {},\n", rust_str(&s.id)));
        out.push_str(&format!("        summary: {},\n", rust_str(&s.summary)));
        out.push_str(&format!("        revision: {},\n", rust_str(&s.revision)));
        match &s.prompt_name {
            Some(p) => out.push_str(&format!("        prompt_name: Some({}),\n", rust_str(p))),
            None => out.push_str("        prompt_name: None,\n"),
        }
        out.push_str("        files: &[\n");
        for f in &s.files {
            out.push_str("            EmbeddedFile {\n");
            out.push_str(&format!(
                "                rel_path: {},\n",
                rust_str(&f.rel_path)
            ));
            out.push_str(&format!(
                "                bytes: include_bytes!({}),\n",
                rust_str(&f.abs_path.to_string_lossy())
            ));
            out.push_str("            },\n");
        }
        out.push_str("        ],\n");
        out.push_str("    },\n");
    }
    out.push_str("];\n");
    out
}

/// Emit a Rust string literal for an arbitrary string (escapes via `{:?}`,
/// which produces a valid Rust/`str` literal for our ASCII-ish inputs).
fn rust_str(s: &str) -> String {
    format!("{s:?}")
}

// ---------------------------------------------------------------------------
// Embedded harness-plugin (TS shim) manifest (Phase 11, R6)
// ---------------------------------------------------------------------------
//
// Some harnesses (Cline, Pi, OpenCode) deliver Tome's session-start steering via
// a small Tome-shipped TypeScript plugin shim rather than a native session hook.
// We embed each shim tree in the binary (offline, versioned with the binary,
// zero new dependency) exactly the way the meta-skill manifest does, and the
// shim is executed by the *harness's* own runtime — so the sync boundary holds.
//
// Reuses the meta-skill helpers (`collect_files`, `rel_to_posix`,
// `is_safe_segment`, `rust_str`) and the `ManifestFile` record — only the
// per-subdir validation gate (exactly one root `tome.ts` entrypoint) and the
// generated struct names differ.

/// One embedded harness shim folder, fully validated.
struct ManifestHarnessPlugin {
    /// The subdir name = the harness id (a safe path segment).
    harness: String,
    /// The rel path of the required entrypoint (`tome.ts`).
    entrypoint: String,
    files: Vec<ManifestFile>,
}

fn generate_harness_plugin_manifest() {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let assets_root = manifest_dir.join("assets/harness-plugins");

    // Rebuild when the asset tree changes (the directory itself catches
    // adds/removes; each file is registered individually below).
    println!("cargo:rerun-if-changed={}", assets_root.display());

    let mut plugins: Vec<ManifestHarnessPlugin> = Vec::new();
    if assets_root.is_dir() {
        // Immediate subdirectories only — each is one harness shim. The
        // top-level `README.md` (and any other loose file) is NOT a harness
        // dir and is skipped by the `is_dir()` filter, exactly as the
        // meta-skills walker skips non-dirs.
        let mut entries: Vec<PathBuf> = fs::read_dir(&assets_root)
            .unwrap_or_else(|e| panic!("read assets/harness-plugins/: {e}"))
            .map(|e| e.expect("dir entry").path())
            .filter(|p| p.is_dir())
            .collect();
        entries.sort();
        for plugin_dir in entries {
            plugins.push(load_harness_plugin(&plugin_dir));
        }
    }

    // Binary-size budget: fail the build well under the 50 MB cap.
    let total: u64 = plugins
        .iter()
        .flat_map(|p| p.files.iter())
        .map(|f| f.bytes.len() as u64)
        .sum();
    assert!(
        total <= HARNESS_PLUGIN_BYTE_BUDGET,
        "embedded harness-plugin shims total {total} bytes, over the \
         {HARNESS_PLUGIN_BYTE_BUDGET}-byte budget. Trim the shims or raise \
         HARNESS_PLUGIN_BYTE_BUDGET deliberately."
    );

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let generated = out_dir.join("harness_plugins_manifest.rs");
    fs::write(&generated, render_harness_plugin_manifest(&plugins))
        .unwrap_or_else(|e| panic!("write {}: {e}", generated.display()));
}

/// Walk one `assets/harness-plugins/<harness>/` folder, validate it, and build
/// the in-memory record. Panics (failing the build) on any invariant violation
/// — the build-time validation gate FR-022 requires.
fn load_harness_plugin(plugin_dir: &Path) -> ManifestHarnessPlugin {
    let harness = plugin_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_else(|| {
            panic!(
                "non-UTF-8 harness-plugin dir name at {}",
                plugin_dir.display()
            )
        })
        .to_owned();
    assert!(
        is_safe_segment(&harness),
        "harness-plugin id `{harness}` is not a safe path segment (no empty, `.`, `..`, \
         leading-dot, `/`, `\\`, or NUL)"
    );

    let mut files: Vec<ManifestFile> = Vec::new();
    // `collect_files` rejects any non-`Normal`/absolute rel path at build time
    // via `rel_to_posix`.
    collect_files(plugin_dir, plugin_dir, &mut files);
    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    // Exactly one `tome.ts` entrypoint at the folder root (FR-022).
    let entrypoint = "tome.ts";
    let entry_count = files.iter().filter(|f| f.rel_path == entrypoint).count();
    assert!(
        entry_count == 1,
        "harness-plugin `{harness}` must have exactly one root `{entrypoint}` entrypoint, \
         found {entry_count}"
    );

    // Register each file for change-detection.
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.abs_path.display());
    }

    ManifestHarnessPlugin {
        harness,
        entrypoint: entrypoint.to_owned(),
        files,
    }
}

/// Render the generated Rust source. Types (`EmbeddedHarnessPlugin`/
/// `EmbeddedFile`) are defined at the `include!` site in
/// `src/harness/plugin_assets.rs`.
fn render_harness_plugin_manifest(plugins: &[ManifestHarnessPlugin]) -> String {
    let mut out = String::new();
    out.push_str("// @generated by build.rs — do not edit. Source: assets/harness-plugins/**\n");
    out.push_str("pub static HARNESS_PLUGINS: &[EmbeddedHarnessPlugin] = &[\n");
    for p in plugins {
        out.push_str("    EmbeddedHarnessPlugin {\n");
        out.push_str(&format!("        harness: {},\n", rust_str(&p.harness)));
        out.push_str(&format!(
            "        entrypoint: {},\n",
            rust_str(&p.entrypoint)
        ));
        out.push_str("        files: &[\n");
        for f in &p.files {
            out.push_str("            EmbeddedFile {\n");
            out.push_str(&format!(
                "                rel_path: {},\n",
                rust_str(&f.rel_path)
            ));
            out.push_str(&format!(
                "                bytes: include_bytes!({}),\n",
                rust_str(&f.abs_path.to_string_lossy())
            ));
            out.push_str("            },\n");
        }
        out.push_str("        ],\n");
        out.push_str("    },\n");
    }
    out.push_str("];\n");
    out
}
