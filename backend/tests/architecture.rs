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

/// `file`'s path relative to `src/`, using `/` regardless of platform.
fn relative_to_src(file: &Path) -> String {
    file.strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))
        .expect("file under src/")
        .to_string_lossy()
        .replace('\\', "/")
}

/// Strips line comments (`//...`) while ignoring `//` inside string literals (`"..."`),
/// and removes string literal content so string text does not trigger forbidden needles.
fn strip_comments_and_strings(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars().peekable();
    let mut in_string = false;
    let mut escape = false;

    while let Some(c) = chars.next() {
        if in_string {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
                out.push('"');
            }
        } else if c == '"' {
            in_string = true;
            out.push('"');
        } else if c == '/' && chars.peek() == Some(&'/') {
            break;
        } else {
            out.push(c);
        }
    }
    out
}

/// Every `(file, line)` in `dir` whose non-comment text contains a forbidden needle.
fn violations(dir: &str, forbidden: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for file in rust_files(&src(dir)) {
        let text = fs::read_to_string(&file).expect("readable source file");
        for (n, line) in text.lines().enumerate() {
            let code = strip_comments_and_strings(line);
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

#[test]
fn interface_runs_no_sql() {
    let dir = src("interface");
    assert!(
        dir.is_dir(),
        "expected {} to exist — did interface/ move or get renamed?",
        dir.display()
    );

    let forbidden = [
        "sqlx::query",
        "sqlx::query_as",
        "sqlx::query_scalar",
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
            let code = strip_comments_and_strings(line);
            forbidden.iter().any(|needle| code.contains(needle))
        });
        if offends {
            offending_files.insert(relative_to_src(&file));
        }
    }

    assert!(
        offending_files.is_empty(),
        "interface/ files must not run SQL directly:\n{}",
        offending_files.into_iter().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn no_interim_query_macros_in_src() {
    let forbidden = [
        "db_execute!",
        "db_fetch_one!",
        "db_fetch_optional!",
        "db_fetch_all!",
        "db_fetch_all_dyn!",
        "db_fetch!",
        "db_scalar!",
        "db_ddl!",
        "tx_execute!",
        "tx_fetch_one!",
        "tx_fetch_optional!",
        "tx_fetch_all!",
        "tx_fetch!",
        "tx_scalar!",
    ];
    let v = violations("", &forbidden);
    assert!(
        v.is_empty(),
        "interim db_*! / tx_*! query macros are removed and must not be used anywhere in src/:\n{}",
        v.join("\n")
    );
}

#[test]
fn assert_sql_safe_only_in_merge() {
    let mut offending_files = BTreeSet::new();
    for file in rust_files(&src("")) {
        let rel = relative_to_src(&file);
        if rel == "infrastructure/postgres/merge.rs" {
            continue;
        }
        let text = fs::read_to_string(&file).expect("readable source file");
        let offends = text.lines().any(|line| {
            let code = strip_comments_and_strings(line);
            code.contains("AssertSqlSafe")
        });
        if offends {
            offending_files.insert(rel);
        }
    }
    assert!(
        offending_files.is_empty(),
        "AssertSqlSafe is strictly restricted to infrastructure/postgres/merge.rs:\n{}",
        offending_files.into_iter().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn no_sqlx_in_interface_or_application() {
    let allowed_idents = ["PgPool", "Transaction", "Error", "Postgres"];
    let mut violations = Vec::new();

    for dir_name in ["interface", "application"] {
        for file in rust_files(&src(dir_name)) {
            let text = fs::read_to_string(&file).expect("readable source file");
            let mut in_test_cfg = false;
            let mut brace_depth = 0usize;
            let mut test_start_depth = None;

            for (n, line) in text.lines().enumerate() {
                let code = strip_comments_and_strings(line);

                let open_braces = code.chars().filter(|&c| c == '{').count();
                let close_braces = code.chars().filter(|&c| c == '}').count();

                if code.contains("#[cfg(test)]") {
                    in_test_cfg = true;
                }

                if in_test_cfg && test_start_depth.is_none() && code.contains('{') {
                    test_start_depth = Some(brace_depth);
                }

                brace_depth += open_braces;
                if brace_depth >= close_braces {
                    brace_depth -= close_braces;
                } else {
                    brace_depth = 0;
                }

                if let Some(start) = test_start_depth {
                    if brace_depth <= start {
                        in_test_cfg = false;
                        test_start_depth = None;
                    }
                    continue;
                } else if in_test_cfg {
                    continue;
                }

                let mut remaining = code.as_str();
                while let Some(idx) = remaining.find("sqlx::") {
                    let after = &remaining[idx + "sqlx::".len()..];
                    let ident: String = after
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !allowed_idents.contains(&ident.as_str()) {
                        violations.push(format!(
                            "{}:{}: unapproved sqlx::{ident}",
                            file.display(),
                            n + 1
                        ));
                    }
                    remaining = after;
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "interface/ and application/ must not reference sqlx outside tests, except PgPool, Transaction, Error:\n{}",
        violations.join("\n")
    );
}
