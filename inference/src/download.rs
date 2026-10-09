//! Streaming model downloads, shared by every capability's provisioning.

use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Duration;
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tracing::warn;

/// How long to wait for the next chunk of a model download before giving up.
/// This bounds idle time, not total transfer time: a slow-but-steady
/// multi-hundred-megabyte download on a modest link is not a stall, but a
/// connection with no bytes arriving for this long is.
const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Why a download failed, as a message for the caller's own error type.
#[derive(Debug, Error)]
#[error("{0}")]
pub struct DownloadError(pub String);

/// Streams `url` into `dest` with the default idle timeout.
pub async fn download_to(url: &str, dest: &Path) -> Result<(), DownloadError> {
    download_with_idle(url, dest, DOWNLOAD_IDLE_TIMEOUT, None).await
}

/// Like [`download_to`], but installs the file only if its SHA-256 equals
/// `sha256` (lowercase hex). The models are fetched from a third-party host
/// and loaded into a native runtime, so a truncated or tampered file must be
/// refused before it can ever be seen at `dest`.
pub async fn download_verified(url: &str, dest: &Path, sha256: &str) -> Result<(), DownloadError> {
    download_with_idle(url, dest, DOWNLOAD_IDLE_TIMEOUT, Some(sha256)).await
}

/// Streams `url` into `dest` via a `.tmp` file that is renamed into place
/// only on success (and, when `expected_sha256` is given, only if the
/// checksum matches), so anything watching `dest` never sees a partial or
/// unverified file.
/// There is deliberately no total-transfer timeout, only `idle`, applied to
/// each chunk: the models are hundreds of megabytes, and a total timeout
/// fails deterministically on any link slower than it assumes.
async fn download_with_idle(
    url: &str,
    dest: &Path,
    idle: Duration,
    expected_sha256: Option<&str>,
) -> Result<(), DownloadError> {
    let tmp_dest = dest.with_extension("tmp");

    let streamed = stream_to_file(url, &tmp_dest, idle)
        .await
        .and_then(|actual| verify_checksum(url, expected_sha256, &actual));
    match streamed {
        Ok(()) => tokio::fs::rename(&tmp_dest, dest)
            .await
            .map_err(|e| DownloadError(format!("Failed to persist model file: {}", e))),
        Err(err) => {
            remove_partial_download(&tmp_dest).await;
            Err(err)
        }
    }
}

fn verify_checksum(url: &str, expected: Option<&str>, actual: &str) -> Result<(), DownloadError> {
    match expected {
        Some(expected) if !expected.eq_ignore_ascii_case(actual) => Err(DownloadError(format!(
            "Downloaded file failed its checksum (expected {}, got {}): {}",
            expected, actual, url
        ))),
        _ => Ok(()),
    }
}

/// Deletes a partial `.tmp` download after a failed attempt. Already-gone is
/// not an error: the failure may have happened before the file was created.
async fn remove_partial_download(tmp_dest: &Path) {
    if let Err(err) = tokio::fs::remove_file(tmp_dest).await {
        if err.kind() != std::io::ErrorKind::NotFound {
            warn!("Failed to remove partial download {:?}: {}", tmp_dest, err);
        }
    }
}

/// Requests `url` and writes its body to `tmp_dest` chunk by chunk, failing
/// if no chunk arrives within `idle`. Returns the body's SHA-256 as lowercase
/// hex, hashed as it streams so verifying costs no second pass over the file.
async fn stream_to_file(
    url: &str,
    tmp_dest: &Path,
    idle: Duration,
) -> Result<String, DownloadError> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| DownloadError(e.to_string()))?;

    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| DownloadError(format!("Failed to request {}: {}", url, e)))?;

    if !response.status().is_success() {
        return Err(DownloadError(format!(
            "Download failed with HTTP status {}: {}",
            response.status(),
            url
        )));
    }

    let mut file = tokio::fs::File::create(tmp_dest)
        .await
        .map_err(|e| DownloadError(format!("Failed to create tmp model file: {}", e)))?;

    let mut hasher = Sha256::new();
    loop {
        let chunk = tokio::time::timeout(idle, response.chunk())
            .await
            .map_err(|_| {
                DownloadError(format!(
                    "Download stalled: no data received for {:?} from {}",
                    idle, url
                ))
            })?
            .map_err(|e| DownloadError(format!("Failed to read stream from {}: {}", url, e)))?;

        let Some(bytes) = chunk else {
            break;
        };

        hasher.update(&bytes);
        file.write_all(&bytes)
            .await
            .map_err(|e| DownloadError(format!("Failed to write tmp model file: {}", e)))?;
    }

    file.flush()
        .await
        .map_err(|e| DownloadError(format!("Failed to flush tmp model file: {}", e)))?;

    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch_dest(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("download-{}-{}.bin", tag, std::process::id()))
    }

    #[tokio::test]
    async fn test_download_with_idle_streams_a_full_body_to_disk() {
        let body: Vec<u8> = vec![7u8; 1024 * 1024];
        let expected = body.clone();
        let app = axum::Router::new().route(
            "/model.bin",
            axum::routing::get(move || {
                let body = body.clone();
                async move { body }
            }),
        );
        let base = crate::test_support::spawn_server(app).await;
        let dest = scratch_dest("ok");
        let tmp_dest = dest.with_extension("tmp");
        std::fs::remove_file(&dest).ok();
        std::fs::remove_file(&tmp_dest).ok();

        download_with_idle(
            &format!("{}/model.bin", base),
            &dest,
            Duration::from_secs(5),
            None,
        )
        .await
        .expect("download of a well-behaved body should succeed");

        let written = std::fs::read(&dest).expect("destination file should exist");
        assert_eq!(written, expected);
        assert!(
            !tmp_dest.exists(),
            "tmp file should not remain after a successful download"
        );

        std::fs::remove_file(&dest).ok();
    }

    #[tokio::test]
    async fn test_download_with_idle_fails_and_cleans_up_on_a_stalled_stream() {
        use futures_util::{stream, StreamExt};
        use std::convert::Infallible;

        let app = axum::Router::new().route(
            "/model.bin",
            axum::routing::get(|| async {
                let first = stream::iter(vec![Ok::<Vec<u8>, Infallible>(vec![1, 2, 3])]);
                let stalled = stream::once(async {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    Ok::<Vec<u8>, Infallible>(vec![4, 5, 6])
                });
                axum::body::Body::from_stream(first.chain(stalled))
            }),
        );
        let base = crate::test_support::spawn_server(app).await;
        let dest = scratch_dest("stall");
        let tmp_dest = dest.with_extension("tmp");
        std::fs::remove_file(&dest).ok();
        std::fs::remove_file(&tmp_dest).ok();

        let result = download_with_idle(
            &format!("{}/model.bin", base),
            &dest,
            Duration::from_millis(200),
            None,
        )
        .await;

        assert!(
            result.is_err(),
            "a stalled stream must fail, got {:?}",
            result
        );
        assert!(
            !dest.exists(),
            "destination must not exist after a stalled download"
        );
        assert!(
            !tmp_dest.exists(),
            "tmp file must not remain after a stalled download"
        );
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(bytes))
    }

    async fn serve_body(body: Vec<u8>) -> String {
        let app = axum::Router::new().route(
            "/model.bin",
            axum::routing::get(move || {
                let body = body.clone();
                async move { body }
            }),
        );
        format!("{}/model.bin", crate::test_support::spawn_server(app).await)
    }

    #[tokio::test]
    async fn test_download_with_a_matching_checksum_is_kept() {
        let body = vec![9u8; 4096];
        let url = serve_body(body.clone()).await;
        let dest = scratch_dest("sum-ok");
        std::fs::remove_file(&dest).ok();

        download_with_idle(
            &url,
            &dest,
            Duration::from_secs(5),
            Some(&sha256_hex(&body)),
        )
        .await
        .expect("a body matching its checksum should be accepted");

        assert_eq!(std::fs::read(&dest).unwrap(), body);
        std::fs::remove_file(&dest).ok();
    }

    #[tokio::test]
    async fn test_download_with_a_wrong_checksum_is_rejected_and_leaves_nothing() {
        let url = serve_body(vec![9u8; 4096]).await;
        let dest = scratch_dest("sum-bad");
        let tmp_dest = dest.with_extension("tmp");
        std::fs::remove_file(&dest).ok();

        let wrong = sha256_hex(b"some other model");
        let result = download_with_idle(&url, &dest, Duration::from_secs(5), Some(&wrong)).await;

        let err = result.expect_err("a corrupt or tampered body must be refused");
        assert!(err.0.contains("checksum"), "unhelpful error: {}", err.0);
        assert!(!dest.exists(), "a rejected model must never be installed");
        assert!(!tmp_dest.exists(), "the rejected tmp file must be removed");
    }
}
