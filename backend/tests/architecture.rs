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

/// Strips line comments (`//...`), inline block comments (`/* .. */`), string
/// literal content and char literals (`'{'`, `'\''`), so text inside them
/// triggers neither forbidden needles nor brace counting. Lifetimes (`'a`)
/// are left intact. Multi-line block comments are not tracked.
fn strip_comments_and_strings(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;

    while i < chars.len() {
        let c = chars[i];
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
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            break;
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            match (i + 2..chars.len().saturating_sub(1))
                .find(|&j| chars[j] == '*' && chars[j + 1] == '/')
            {
                Some(end) => i = end + 1,
                None => break,
            }
        } else if c == '\'' {
            if chars.get(i + 1) == Some(&'\\') {
                // Escaped char literal: skip to its closing quote.
                match (i + 3..chars.len()).find(|&j| chars[j] == '\'') {
                    Some(end) => i = end,
                    None => break,
                }
            } else if chars.get(i + 2) == Some(&'\'') {
                i += 2;
            } else {
                out.push(c);
            }
        } else {
            out.push(c);
        }
        i += 1;
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

/// Lines of `text` whose non-comment, non-string code contains a forbidden
/// needle, ignoring `#[cfg(test)]` items: a braced block (`mod tests { .. }`)
/// or a single-line item (`mod fake;`, a field or variant ending in `,`).
fn production_hits(text: &str, forbidden: &[&str]) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut pending_test_attr = false;
    let mut skipped_depth = 0i64;
    for (n, line) in text.lines().enumerate() {
        let code = strip_comments_and_strings(line);
        let opens = code.matches('{').count() as i64;
        let closes = code.matches('}').count() as i64;
        if skipped_depth > 0 {
            skipped_depth += opens - closes;
            continue;
        }
        if code.contains("#[cfg(test)]") {
            pending_test_attr = true;
            continue;
        }
        if pending_test_attr {
            let trimmed = code.trim();
            if trimmed.is_empty() || trimmed.starts_with("#[") {
                continue;
            }
            if opens > 0 {
                skipped_depth = opens - closes;
                pending_test_attr = false;
            } else if code.trim_end().ends_with([';', ',']) {
                pending_test_attr = false;
            }
            continue;
        }
        if forbidden.iter().any(|needle| code.contains(needle)) {
            found.push((n + 1, line.trim().to_string()));
        }
    }
    found
}

/// Like `violations`, but ignores `#[cfg(test)]` items, for layers whose
/// production code is restricted while their tests may legitimately read
/// state (`jobs/` runtime, `interface/jobs` handlers).
fn production_violations(dir: &str, forbidden: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for file in rust_files(&src(dir)) {
        let text = fs::read_to_string(&file).expect("readable source file");
        for (n, line) in production_hits(&text, forbidden) {
            found.push(format!("{}:{n}: {line}", file.display()));
        }
    }
    found
}

#[test]
fn scanner_sees_production_code_after_a_cfg_test_field() {
    let text = "struct X {
    #[cfg(test)]
    probe: u8,
    real: u8,
}
impl X {
    fn f() {
        sqlx::query!(\"x\");
    }
}
";
    let hits = production_hits(text, &["sqlx::query"]);
    assert_eq!(hits.len(), 1, "{hits:?}");
}

#[test]
fn scanner_is_not_fooled_by_braces_in_char_literals() {
    let text = "#[cfg(test)]
mod tests {
    fn f() {
        let c = '{';
        let q = '\'';
    }
}
fn prod() {
    sqlx::query!(\"x\");
}
";
    let hits = production_hits(text, &["sqlx::query"]);
    assert_eq!(hits.len(), 1, "{hits:?}");
}

#[test]
fn scanner_skips_cfg_test_modules_and_single_line_items() {
    let text = "#[cfg(test)]
mod fake;
fn prod<'a>(x: &'a str) {}
#[cfg(test)]
mod tests {
    fn f() { sqlx::query!(\"x\"); }
}
";
    assert!(production_hits(text, &["sqlx::query"]).is_empty());
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

#[test]
fn job_runtime_knows_no_transport_and_runs_no_sql() {
    let v = violations(
        "jobs",
        &[
            "axum",
            "matrix_sdk",
            "utoipa",
            "reqwest",
            "crate::interface",
            "sqlx::query",
            "query!",
            "query_as!",
            "query_scalar!",
            "QueryBuilder",
        ],
    );
    assert!(
        v.is_empty(),
        "jobs/ may use services and repositories only:\n{}",
        v.join("\n")
    );
}

#[test]
fn lower_layers_do_not_import_the_job_runtime() {
    for dir in ["domain", "application", "infrastructure"] {
        let v = violations(dir, &["crate::jobs"]);
        assert!(
            v.is_empty(),
            "{dir}/ must not depend on crate::jobs:\n{}",
            v.join("\n")
        );
    }
}

#[test]
fn job_handlers_are_interface_code() {
    // interface_runs_no_sql already scans interface/ recursively; this pins
    // that the handlers live there (a missing dir panics in rust_files).
    let v = production_violations(
        "interface/jobs",
        &["sqlx::", "crate::infrastructure::postgres::repositories"],
    );
    assert!(
        v.is_empty(),
        "job handlers call services, never repositories:\n{}",
        v.join("\n")
    );
}

/// The audit rows for a passkey's lifecycle belong to `AccountService`, which
/// records them exactly once inside `register_passkey`, `rename_passkey` and
/// `delete_passkey`. An interface that names a passkey event kind is writing the
/// row a second time (as `register_finish` once did).
#[test]
fn interface_never_records_passkey_security_events() {
    let dir = src("interface");
    assert!(
        dir.is_dir(),
        "expected {} to exist — did interface/ move or get renamed?",
        dir.display()
    );

    let offending = production_violations("interface", &["SecurityEventKind::Passkey"]);
    assert!(
        offending.is_empty(),
        "interface production code must not name passkey security-event kinds; \
         AccountService records them:\n{}",
        offending.join("\n")
    );
}

/// The audit log has exactly one writer: `AccountService`, which records each
/// row inside the operation that causes it (`claim_link`, `merge`, `unlink`,
/// the recovery and passkey methods). Interface code that records an event
/// writes it a second time (`!link` once doubled `matrix_linked`) or writes one
/// the service does not know about.
///
/// Two needles express that precisely:
/// - the write calls (`record_security_event`, the repository's
///   `security_events::record`) may appear nowhere in `interface/` or `llm/`;
///   reading and listing events for the web UI (`list_security_events`) is not
///   matched;
/// - an event *kind* may be named only where it words a Matrix DM
///   (`announce_account` takes one to pick the message) — never in the HTTP
///   layer, which announces nothing, nor anywhere else. The web DTO maps the
///   kind as a plain string, so it never needs the enum.
#[test]
fn interface_never_writes_security_events() {
    for dir in ["interface", "llm"] {
        assert!(
            src(dir).is_dir(),
            "expected {} to exist — did it move or get renamed?",
            src(dir).display()
        );
    }

    let mut writes = production_violations("interface", &WRITE_NEEDLES);
    writes.extend(production_violations("llm", &WRITE_NEEDLES));
    assert!(
        writes.is_empty(),
        "interface/llm production code must not write security events; \
         AccountService records them:\n{}",
        writes.join("\n")
    );

    // `MergeService::merge` moves the rows but writes no audit row; only
    // `AccountService::merge` does. The type stays private to
    // `application::accounts`, and this needle catches a re-export that would
    // let an adapter skip the row.
    let mut bypass = production_violations("interface", &["MergeService"]);
    bypass.extend(production_violations("llm", &["MergeService"]));
    assert!(
        bypass.is_empty(),
        "adapters must merge through AccountService, which records the audit row:\n{}",
        bypass.join("\n")
    );

    // llm/ names no event kind at all; interface/ only to word a DM.
    let llm_kinds = production_violations("llm", &["SecurityEventKind::"]);
    assert!(
        llm_kinds.is_empty(),
        "llm/ must not name security-event kinds:\n{}",
        llm_kinds.join("\n")
    );

    /// Files that name a kind only to word an announcement DM.
    const DM_WORDING: [&str; 2] = [
        "interface/channels/matrix/notify.rs",
        "interface/chat/account.rs",
    ];
    let stray_kinds: Vec<_> = production_violations("interface", &["SecurityEventKind::"])
        .into_iter()
        .filter(|hit| {
            let hit = hit.replace(std::path::MAIN_SEPARATOR, "/");
            !DM_WORDING.iter().any(|file| hit.contains(file))
        })
        .collect();
    assert!(
        stray_kinds.is_empty(),
        "security-event kinds may be named only to word a DM ({DM_WORDING:?}):\n{}",
        stray_kinds.join("\n")
    );
}

const WRITE_NEEDLES: [&str; 3] = [
    "record_security_event",
    "security_events::record",
    "repositories::accounts::security_events",
];

/// R15: account ids reach the program only as `AccountId` values. The legacy
/// `accounts/` facade carried them as strings and a `From<&str>` fallback mapped
/// garbage to the nil UUID; both are gone and must stay gone.
#[test]
fn the_legacy_accounts_module_is_gone() {
    // The directory check also covers lib.rs: `pub mod accounts;` without the
    // directory would not compile.
    assert!(
        !src("accounts").exists(),
        "backend/src/accounts/ was retired; use domain/application/infrastructure accounts"
    );
    // Matches `crate::accounts::...` and `use crate::accounts;`, never
    // `crate::domain::accounts` / `crate::application::accounts`.
    let v = violations(
        "",
        &[
            "crate::accounts::",
            "crate::accounts;",
            "crate::accounts,",
            "crate::accounts}",
        ],
    );
    assert!(
        v.is_empty(),
        "nothing may reference the retired crate::accounts:\n{}",
        v.join("\n")
    );
    let lib = fs::read_to_string(src("lib.rs")).expect("lib.rs is readable");
    assert!(
        !lib.lines()
            .any(|l| l.trim_start().starts_with("pub mod accounts")),
        "lib.rs must not declare `pub mod accounts`"
    );
}

#[test]
fn account_ids_are_never_built_from_strings_implicitly() {
    let v = violations(
        "domain/accounts",
        &[
            "impl From<&str> for AccountId",
            "impl From<String> for AccountId",
            "impl From<&String> for AccountId",
        ],
    );
    assert!(
        v.is_empty(),
        "string ids go through FromStr (R15 closed):\n{}",
        v.join("\n")
    );
}

#[test]
fn events_runtime_knows_no_transport_and_runs_no_sql() {
    let v = violations(
        "events",
        &[
            "axum",
            "matrix_sdk",
            "utoipa",
            "reqwest",
            "crate::interface",
            "sqlx::query",
            "query!",
            "query_as!",
            "query_scalar!",
            "QueryBuilder",
        ],
    );
    assert!(
        v.is_empty(),
        "events/ may use services only:\n{}",
        v.join("\n")
    );
}

#[test]
fn lower_layers_do_not_import_the_events_runtime() {
    for dir in ["domain", "application", "infrastructure"] {
        let v = violations(dir, &["crate::events::"]);
        assert!(
            v.is_empty(),
            "{dir}/ must not depend on crate::events:\n{}",
            v.join("\n")
        );
    }
}

#[test]
fn no_in_process_event_broadcast_remains() {
    for dir in [
        "domain",
        "application",
        "infrastructure",
        "interface",
        "llm",
        "jobs",
        "events",
    ] {
        let v = violations(
            dir,
            &[
                "EventBroadcaster",
                "tokio::sync::broadcast",
                "BroadcastStream",
            ],
        );
        assert!(
            v.is_empty(),
            "events go through the outbox:\n{}",
            v.join("\n")
        );
    }
}

#[test]
fn only_application_publishes_to_the_outbox() {
    for dir in ["interface", "llm", "jobs", "events"] {
        // Production code only: hub tests seed events through the repository.
        let v = production_violations(dir, &["outbox::publish"]);
        assert!(
            v.is_empty(),
            "{dir}/ must publish through a service:\n{}",
            v.join("\n")
        );
    }
}

#[test]
fn the_agent_knows_no_channel() {
    let hits = production_violations(
        "llm",
        &[
            "matrix_sdk",
            "interface::channels",
            "serenity",
            "teloxide",
            "lettre",
            "async_imap",
            "mail_parser",
        ],
    );
    assert!(
        hits.is_empty(),
        "llm/ must not know any channel:\n{}",
        hits.join("\n")
    );
}

/// The chat modules answer through `Reply` and know no platform SDK. Voice,
/// OCR, rooms, the outage alert, `account.rs` and `llm.rs` still import
/// `matrix_sdk` (live `Room`/`Client` handles); Task 7 moves them behind the
/// adapter and removes the `ignore`.
#[test]
#[ignore = "enabled in Task 7"]
fn chat_modules_know_no_platform_sdk() {
    let hits = production_violations(
        "interface/chat",
        &["matrix_sdk", "serenity", "teloxide", "lettre", "async_imap"],
    );
    assert!(hits.is_empty(), "{}", hits.join("\n"));
}
