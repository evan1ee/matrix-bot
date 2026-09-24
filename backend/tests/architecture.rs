//! Source-level guard for the dependency rule in
//! docs/superpowers/specs/2026-09-24-postgres-foundation-design.md §3.1.
//! A scan, not a compiler guarantee: cheap, and it catches the regressions
//! that matter (an adapter type leaking into domain, SQL in a handler).

use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}

fn src(sub: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(sub)
}

/// Every `(file, line)` in `dir` whose non-comment text contains a forbidden needle.
fn violations(dir: &str, forbidden: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for file in rust_files(&src(dir)) {
        let text = fs::read_to_string(&file).expect("readable source file");
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if forbidden.iter().any(|needle| code.contains(needle)) {
                found.push(format!("{}:{}: {}", file.display(), n + 1, line.trim()));
            }
        }
    }
    found
}

#[test]
fn domain_depends_on_nothing_outside_itself() {
    let v = violations(
        "domain",
        &[
            "sqlx",
            "axum",
            "matrix_sdk",
            "reqwest",
            "utoipa",
            "crate::application",
            "crate::interface",
            "crate::infrastructure",
            "crate::modules",
            "crate::db",
        ],
    );
    assert!(v.is_empty(), "domain/ must stay pure:\n{}", v.join("\n"));
}

#[test]
fn application_does_not_know_transports() {
    let v = violations(
        "application",
        &[
            "axum",
            "matrix_sdk",
            "reqwest",
            "utoipa",
            "crate::interface",
        ],
    );
    assert!(
        v.is_empty(),
        "application/ must not depend on transports:\n{}",
        v.join("\n")
    );
}

#[test]
fn infrastructure_does_not_reach_upwards() {
    let v = violations(
        "infrastructure",
        &["crate::interface", "crate::application", "crate::modules"],
    );
    assert!(
        v.is_empty(),
        "infrastructure/ must not import upper layers:\n{}",
        v.join("\n")
    );
}

#[test]
fn no_code_searches_parent_directories_for_env_files() {
    let v = violations("", &["dotenvy::dotenv("]);
    assert!(
        v.is_empty(),
        "src/ must use dotenvy::from_path, not dotenvy::dotenv():\n{}",
        v.join("\n")
    );
}
