//! `GET /notes/export` and `POST /notes/import`: a zip of Markdown files.
//!
//! Both need a recently verified session (`VerifiedUser`): an export hands out
//! every note, and an import rewrites notes in bulk. Zip work runs on the
//! blocking pool, and the archive's shape is limited in `note_archive`.

use axum::{
    extract::{Multipart, State},
    http::{header, HeaderValue},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::note_archive::{self, ArchiveError, Limits};
use crate::application::knowledge::{ImportReport, SkippedFile};
use crate::interface::http::auth::extractor::VerifiedUser;
use crate::interface::http::auth::ApiState;
use crate::interface::http::error::ApiError;

/// Largest import upload. The route raises the default body limit to this.
pub const IMPORT_BODY_LIMIT_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct SkippedFileDto {
    pub file: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ImportReportDto {
    pub created: usize,
    pub updated: usize,
    pub skipped: Vec<SkippedFileDto>,
}

impl From<ImportReport> for ImportReportDto {
    fn from(report: ImportReport) -> Self {
        Self {
            created: report.created,
            updated: report.updated,
            skipped: report
                .skipped
                .into_iter()
                .map(SkippedFileDto::from)
                .collect(),
        }
    }
}

impl From<SkippedFile> for SkippedFileDto {
    fn from(skipped: SkippedFile) -> Self {
        Self {
            file: skipped.file,
            reason: skipped.reason,
        }
    }
}

impl From<ArchiveError> for ApiError {
    fn from(err: ArchiveError) -> Self {
        match err {
            ArchiveError::Build => ApiError::internal("note archive could not be built"),
            other => ApiError::bad_request(other.to_string()),
        }
    }
}

/// Downloads every note as `notes-<date>.zip`, one `notes/<slug>-<id>.md` per
/// note with YAML frontmatter.
#[utoipa::path(
    get,
    tag = "knowledge",
    path = "/api/v1/notes/export",
    responses(
        (status = 200, description = "A zip archive of Markdown files (application/zip)"),
        (status = 400, description = "More notes than one export holds"),
        (status = 403, description = "The session needs to be verified again")
    ),
    security(("session_cookie" = []))
)]
pub async fn export_notes(
    user: VerifiedUser,
    State(state): State<ApiState>,
) -> Result<Response, ApiError> {
    let files = state.services.notes.export(&user.account_id).await?;
    let bytes = tokio::task::spawn_blocking(move || note_archive::build_archive(&files))
        .await
        .map_err(|e| ApiError::internal(format!("archive task failed: {e}")))??;

    let filename = format!("notes-{}.zip", chrono::Utc::now().format("%Y-%m-%d"));
    let disposition = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
        .map_err(|e| ApiError::internal(format!("bad content disposition: {e}")))?;
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/zip"),
            ),
            (header::CONTENT_DISPOSITION, disposition),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        bytes,
    )
        .into_response())
}

/// Imports the `*.md` files of an uploaded zip (multipart field `file`, at most
/// 20 MB, 5,000 entries, 1 MB per entry and 100 MB unpacked). A file whose `id`
/// is one of the caller's notes updates it; any other id creates a new note.
#[utoipa::path(
    post,
    tag = "knowledge",
    path = "/api/v1/notes/import",
    responses(
        (status = 200, body = ImportReportDto),
        (status = 400, description = "Not a zip, or over one of the archive limits"),
        (status = 403, description = "The session needs to be verified again")
    ),
    security(("session_cookie" = []))
)]
pub async fn import_notes(
    user: VerifiedUser,
    State(state): State<ApiState>,
    mut multipart: Multipart,
) -> Result<Json<ImportReportDto>, ApiError> {
    let bytes = read_upload(&mut multipart).await?;
    let archive =
        tokio::task::spawn_blocking(move || note_archive::read_archive(&bytes, &Limits::IMPORT))
            .await
            .map_err(|e| ApiError::internal(format!("archive task failed: {e}")))??;

    let mut report = state
        .services
        .notes
        .import(&user.account_id, archive.files)
        .await;
    report.skipped.extend(archive.skipped);
    Ok(Json(report.into()))
}

async fn read_upload(multipart: &mut Multipart) -> Result<Vec<u8>, ApiError> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(format!("Failed to read multipart payload: {e}")))?
    {
        if matches!(field.name(), Some("file") | None | Some("")) {
            let bytes = field
                .bytes()
                .await
                .map_err(|e| ApiError::bad_request(format!("Failed to read the upload: {e}")))?;
            if bytes.is_empty() {
                break;
            }
            return Ok(bytes.to_vec());
        }
    }
    Err(ApiError::bad_request(
        "No zip file provided in multipart request",
    ))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use tower::ServiceExt;

    use super::*;
    use crate::application::knowledge::ExportFile;
    use crate::domain::knowledge::note::NewNote;
    use crate::interface::http::routes::api_routes;

    const ORIGIN: &str = "http://localhost:8080";
    const BOUNDARY: &str = "XBOUNDARYX";

    struct Harness {
        app: Router,
        cookie: String,
        state: ApiState,
        owner: crate::domain::accounts::AccountId,
    }

    async fn harness() -> Harness {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let state = ApiState::new(
            crate::db::Database { pool },
            None,
            ORIGIN.to_string(),
            reqwest::Client::new(),
        );
        let accounts = &state.services.accounts;
        let account = accounts
            .create("transfer", crate::domain::accounts::STATUS_ACTIVE)
            .await
            .unwrap();
        let cookie = accounts
            .create_session(&account, None, None, None)
            .await
            .unwrap();
        accounts
            .update_last_verified_at(&account, chrono::Utc::now())
            .await
            .unwrap();
        let app = Router::new()
            .nest("/api/v1", api_routes())
            .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024))
            .with_state(state.clone());
        Harness {
            app,
            cookie,
            state,
            owner: account,
        }
    }

    fn multipart(file: &[u8]) -> Vec<u8> {
        let mut body = format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"notes.zip\"\r\nContent-Type: application/zip\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(file);
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
        body
    }

    async fn send(
        h: &Harness,
        request: Request<Body>,
    ) -> (StatusCode, Vec<u8>, axum::http::HeaderMap) {
        let response = h.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, bytes.to_vec(), headers)
    }

    fn import_request(h: &Harness, body: Vec<u8>) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/v1/notes/import")
            .header(header::COOKIE, format!("mb_session={}", h.cookie))
            .header(header::ORIGIN, ORIGIN)
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={BOUNDARY}"),
            )
            .body(Body::from(body))
            .unwrap()
    }

    /// Text zlib cannot shrink much, so a few entries make a zip over 1 MB.
    fn pseudo_random_text(len: usize, seed: u64) -> String {
        let mut state = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (0..len)
            .map(|_| {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                char::from(b'!' + ((state >> 33) % 90) as u8)
            })
            .collect()
    }

    #[tokio::test]
    async fn export_downloads_a_zip_of_the_callers_notes() {
        let h = harness().await;
        h.state
            .services
            .notes
            .create(&h.owner, NewNote::new("猫 and dogs", "# Pets\n\nbody"))
            .await
            .unwrap();

        let request = Request::builder()
            .uri("/api/v1/notes/export")
            .header(header::COOKIE, format!("mb_session={}", h.cookie))
            .body(Body::empty())
            .unwrap();
        let (status, bytes, headers) = send(&h, request).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CONTENT_TYPE], "application/zip");
        let disposition = headers[header::CONTENT_DISPOSITION].to_str().unwrap();
        assert!(
            disposition.starts_with("attachment; filename=\"notes-")
                && disposition.ends_with(".zip\""),
            "{disposition}"
        );
        let read = note_archive::read_archive(&bytes, &Limits::IMPORT).unwrap();
        assert_eq!(read.files.len(), 1);
        assert!(read.files[0].name.starts_with("notes/猫-and-dogs-"));
        assert!(read.files[0].text.ends_with("# Pets\n\nbody"));
    }

    #[tokio::test]
    async fn import_reads_the_uploaded_zip_and_reports_what_it_did() {
        let h = harness().await;
        let zip = note_archive::build_archive(&[
            ExportFile {
                path: "notes/new.md".into(),
                text: "# Fresh\nfrom a zip".into(),
            },
            ExportFile {
                path: "notes/empty.md".into(),
                text: "---\ntitle: Nothing\n---\n".into(),
            },
        ])
        .unwrap();

        let (status, bytes, _) = send(&h, import_request(&h, multipart(&zip))).await;

        assert_eq!(
            status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        let report: ImportReportDto = serde_json::from_slice(&bytes).unwrap();
        assert_eq!((report.created, report.updated), (1, 0));
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].file, "notes/empty.md");
        let notes = h
            .state
            .services
            .notes
            .list(&h.owner, None, None, true, 10)
            .await
            .unwrap();
        assert_eq!(notes[0].title, "Fresh");
    }

    #[tokio::test]
    async fn import_accepts_an_upload_over_the_default_one_megabyte_limit() {
        let h = harness().await;
        let files: Vec<ExportFile> = (0..20)
            .map(|n| ExportFile {
                path: format!("notes/big-{n}.md"),
                text: format!("# Big {n}\n{}", pseudo_random_text(90_000, n)),
            })
            .collect();
        let zip = note_archive::build_archive(&files).unwrap();
        assert!(zip.len() > 1024 * 1024, "zip is only {} bytes", zip.len());

        let (status, bytes, _) = send(&h, import_request(&h, multipart(&zip))).await;

        assert_eq!(
            status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        let report: ImportReportDto = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(report.created, 20, "{:?}", report.skipped);
    }

    #[tokio::test]
    async fn import_refuses_a_file_that_is_not_a_zip_and_a_missing_file() {
        let h = harness().await;

        let (status, bytes, _) = send(&h, import_request(&h, multipart(b"not a zip"))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(String::from_utf8_lossy(&bytes).contains("zip"));

        let empty = format!("--{BOUNDARY}--\r\n").into_bytes();
        let (status, _, _) = send(&h, import_request(&h, empty)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn other_routes_keep_the_one_megabyte_body_limit() {
        let h = harness().await;
        let big = serde_json::json!({"title": "t", "content": "x".repeat(2 * 1024 * 1024)});
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/notes")
            .header(header::COOKIE, format!("mb_session={}", h.cookie))
            .header(header::ORIGIN, ORIGIN)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(big.to_string()))
            .unwrap();

        let (status, _, _) = send(&h, request).await;

        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }
}
