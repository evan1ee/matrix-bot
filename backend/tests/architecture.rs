//! Source-level guard for the dependency rule in
//! docs/superpowers/specs/2026-09-24-postgres-foundation-design.md §3.1.
//! A scan, not a compiler guarantee: cheap, and it catches the regressions
//! that matter (an adapter type leaking into domain, SQL in a handler).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Recursively lists every `.rs` file under `dir`.
///
/// Panics if `dir` does not exist, rather than returning an empty list — a
/// typo'd path (e.g. `violations("interfce", ...)`) must fail loudly instead
/// of vacuously passing because nothing was scanned.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("expected {} to exist: {err}", dir.display()));
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

/// `file`'s path relative to `src/`, using `/` regardless of platform, so
/// entries in `INTERFACE_SQL_ALLOW_LIST` are portable.
fn relative_to_src(file: &Path) -> String {
    file.strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))
        .expect("file under src/")
        .to_string_lossy()
        .replace('\\', "/")
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
    // The bare needle (no trailing `(`) also catches `use dotenvy::dotenv;`,
    // `dotenvy::dotenv_override(...)` and `dotenvy::dotenv_iter(...)` — every
    // dotenvy entry point that walks upward from the current directory looking
    // for `.env`, not just a direct call to `dotenv()`.
    let v = violations("", &["dotenvy::dotenv"]);
    assert!(
        v.is_empty(),
        "src/ must use dotenvy::from_path, not dotenvy::dotenv/dotenv_override/dotenv_iter:\n{}",
        v.join("\n")
    );
}

/// Files under `interface/` that still run SQL directly, pending the
/// feature-by-feature move to the target layer-first layout (see
/// docs/ARCHITECTURE.md's "Target layer-first layout"). This list may only
/// shrink: `interface_runs_no_sql_outside_the_allow_list` fails if a listed
/// file no longer offends, so a stale entry can't linger unnoticed.
const INTERFACE_SQL_ALLOW_LIST: &[&str] = &[
    "interface/channels/matrix/invite.rs",
    "interface/channels/matrix/linking.rs",
    "interface/channels/matrix/reaction.rs",
    "interface/channels/matrix/reaction_targets.rs",
    "interface/http/account/mod.rs",
    "interface/http/auth/mod.rs",
    "interface/http/auth/webauthn.rs",
    "interface/http/life_activity.rs",
    "interface/http/mcp.rs",
    "interface/http/mod.rs",
    "interface/http/reminders.rs",
    "interface/http/webhooks.rs",
];

#[test]
fn interface_runs_no_sql_outside_the_allow_list() {
    let dir = src("interface");
    assert!(
        dir.is_dir(),
        "expected {} to exist — did interface/ move or get renamed?",
        dir.display()
    );

    let forbidden = [
        "sqlx::query",
        "db_fetch",
        "db_scalar",
        "db_execute",
        "db_ddl",
        "tx_execute",
        "tx_fetch",
        "tx_scalar",
    ];

    let mut offending_files = BTreeSet::new();
    for file in rust_files(&dir) {
        let text = fs::read_to_string(&file).expect("readable source file");
        let offends = text.lines().any(|line| {
            let code = line.split("//").next().unwrap_or("");
            forbidden.iter().any(|needle| code.contains(needle))
        });
        if offends {
            offending_files.insert(relative_to_src(&file));
        }
    }

    let allow_list: BTreeSet<&str> = INTERFACE_SQL_ALLOW_LIST.iter().copied().collect();

    let unlisted: Vec<&String> = offending_files
        .iter()
        .filter(|f| !allow_list.contains(f.as_str()))
        .collect();
    assert!(
        unlisted.is_empty(),
        "interface/ files run SQL directly but are not in INTERFACE_SQL_ALLOW_LIST \
         (move the SQL into application/infrastructure, or add the file to the list \
         if it is a known interim offender):\n{}",
        unlisted
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );

    let stale: Vec<&str> = allow_list
        .iter()
        .filter(|f| !offending_files.contains(**f))
        .copied()
        .collect();
    assert!(
        stale.is_empty(),
        "INTERFACE_SQL_ALLOW_LIST entries no longer run SQL directly — remove them \
         (the list only shrinks):\n{}",
        stale.join("\n")
    );
}
