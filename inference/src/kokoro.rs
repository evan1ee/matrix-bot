//! Kokoro text-to-speech: the model on disk, and (Task 3) the engine.

use std::path::Path;
use thiserror::Error;
use tracing::info;

/// The sherpa-onnx release archive of the multi-language Kokoro model:
/// 103 voices, English and Mandarin, with per-language lexicons and the
/// Chinese text-normalisation FSTs.
const KOKORO_ARCHIVE_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-multi-lang-v1_1.tar.bz2";

/// The one top-level directory inside that archive.
const KOKORO_ARCHIVE_ROOT: &str = "kokoro-multi-lang-v1_1";

/// Where the archive is downloaded and unpacked, beside the model dir.
const ARCHIVE_FILE_NAME: &str = "kokoro-download.tar.bz2";
const STAGING_DIR_NAME: &str = ".kokoro-staging";

/// The files sherpa-onnx cannot build a Kokoro engine without. The lexicons,
/// `espeak-ng-data`, `dict` and FSTs are optional to it, and a missing one
/// degrades pronunciation rather than failing.
pub const KOKORO_REQUIRED_FILES: [&str; 3] = ["model.onnx", "voices.bin", "tokens.txt"];

/// Task 3 adds `ModelMissing`, `Engine` and `Encode` when the engine that
/// constructs them arrives; adding them now would be dead code.
#[derive(Debug, Error)]
pub enum TtsError {
    #[error("Failed to install the Kokoro TTS model: {0}")]
    ModelDownload(String),
}

/// Whether every required model file is on disk. The route gates on this and
/// answers 503 until provisioning has finished.
pub fn model_present(dir: &Path) -> bool {
    KOKORO_REQUIRED_FILES
        .iter()
        .all(|name| dir.join(name).is_file())
}

/// Installs the Kokoro model into `model_dir` unless it is already there.
pub async fn ensure_model_downloaded(model_dir: &Path) -> Result<(), TtsError> {
    ensure_model_from(KOKORO_ARCHIVE_URL, model_dir).await
}

/// Downloads the archive at `url` beside `model_dir`, unpacks it into a
/// staging directory, and renames the archive's root into place, so the route
/// sees either no model or a whole one. The archive and staging directory are
/// removed whether or not installation succeeds.
///
/// A `model_dir` that exists and holds something other than the model is
/// rejected before the download starts: `retry_until_ready` never gives up,
/// so paying for the multi-hundred-megabyte download on every retry against
/// a directory that can never succeed would waste bandwidth forever.
async fn ensure_model_from(url: &str, model_dir: &Path) -> Result<(), TtsError> {
    if model_present(model_dir) {
        return Ok(());
    }
    reject_if_occupied(model_dir)?;

    let parent = model_dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    tokio::fs::create_dir_all(&parent)
        .await
        .map_err(|e| install_error("create the model parent directory", e))?;

    let archive = parent.join(ARCHIVE_FILE_NAME);
    let staging = parent.join(STAGING_DIR_NAME);

    info!("Kokoro TTS model missing; downloading {}", url);
    let downloaded = crate::download::download_to(url, &archive)
        .await
        .map_err(|e| TtsError::ModelDownload(e.0));

    let installed = match downloaded {
        Ok(()) => {
            let (archive, staging, model_dir) =
                (archive.clone(), staging.clone(), model_dir.to_path_buf());
            tokio::task::spawn_blocking(move || {
                install_from_archive(&archive, &staging, &model_dir)
            })
            .await
            .map_err(|e| TtsError::ModelDownload(format!("Unpack worker panicked: {}", e)))
            .and_then(|result| result)
        }
        Err(err) => Err(err),
    };

    tokio::fs::remove_file(&archive).await.ok();
    tokio::fs::remove_dir_all(&staging).await.ok();

    if installed.is_ok() {
        info!("Kokoro TTS model installed in {:?}", model_dir);
    }
    installed
}

/// Unpacks `archive` into `staging` and moves its root to `model_dir`.
///
/// An existing `model_dir` is replaced only when it is empty. A non-empty
/// one that lacks the model is someone's hand-placed directory, and deleting
/// it to make room is not this function's call.
fn install_from_archive(archive: &Path, staging: &Path, model_dir: &Path) -> Result<(), TtsError> {
    if staging.exists() {
        std::fs::remove_dir_all(staging)
            .map_err(|e| install_error("clear the staging directory", e))?;
    }
    std::fs::create_dir_all(staging)
        .map_err(|e| install_error("create the staging directory", e))?;

    let file = std::fs::File::open(archive).map_err(|e| install_error("open the archive", e))?;
    tar::Archive::new(bzip2::read::BzDecoder::new(std::io::BufReader::new(file)))
        .unpack(staging)
        .map_err(|e| install_error("unpack the archive", e))?;

    let root = staging.join(KOKORO_ARCHIVE_ROOT);
    if !model_present(&root) {
        return Err(TtsError::ModelDownload(format!(
            "the archive has no {}/ holding {}",
            KOKORO_ARCHIVE_ROOT,
            KOKORO_REQUIRED_FILES.join(", ")
        )));
    }

    // Backstop against a directory that appeared or changed while the
    // download was in flight; `ensure_model_from` already checked this
    // before paying for the download.
    reject_if_occupied(model_dir)?;
    if model_dir.exists() {
        std::fs::remove_dir(model_dir)
            .map_err(|e| install_error("replace the empty model directory", e))?;
    }

    std::fs::rename(&root, model_dir).map_err(|e| install_error("move the model into place", e))
}

/// Whether `model_dir` exists and holds something other than an empty
/// directory. A missing directory and an empty one are both fine to install
/// into; anything else was placed there by hand and is not this module's to
/// delete.
fn is_occupied(model_dir: &Path) -> Result<bool, TtsError> {
    if !model_dir.exists() {
        return Ok(false);
    }
    let empty = std::fs::read_dir(model_dir)
        .map_err(|e| install_error("inspect the model directory", e))?
        .next()
        .is_none();
    Ok(!empty)
}

/// Rejects `model_dir` if it is occupied by something other than the model,
/// with the message both callers of `is_occupied` report.
fn reject_if_occupied(model_dir: &Path) -> Result<(), TtsError> {
    if is_occupied(model_dir)? {
        return Err(TtsError::ModelDownload(format!(
            "{:?} exists but does not hold the model; remove it or point TTS_MODEL_DIR elsewhere",
            model_dir
        )));
    }
    Ok(())
}

fn install_error(action: &str, err: std::io::Error) -> TtsError {
    TtsError::ModelDownload(format!("Failed to {}: {}", action, err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kokoro-{}-{}", tag, std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A `.tar.bz2` shaped like the sherpa-onnx release: every entry under
    /// `KOKORO_ARCHIVE_ROOT/`.
    fn archive_with(files: &[&str]) -> Vec<u8> {
        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            for name in files {
                let data = b"x";
                let mut header = tar::Header::new_gnu();
                header.set_size(data.len() as u64);
                header.set_mode(0o644);
                builder
                    .append_data(
                        &mut header,
                        format!("{}/{}", KOKORO_ARCHIVE_ROOT, name),
                        &data[..],
                    )
                    .unwrap();
            }
            builder.finish().unwrap();
        }
        let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
        encoder.write_all(&tar_bytes).unwrap();
        encoder.finish().unwrap()
    }

    async fn serve_archive(bytes: Vec<u8>) -> String {
        let app = axum::Router::new().route(
            "/kokoro.tar.bz2",
            axum::routing::get(move || {
                let bytes = bytes.clone();
                async move { bytes }
            }),
        );
        format!(
            "{}/kokoro.tar.bz2",
            crate::test_support::spawn_server(app).await
        )
    }

    #[test]
    fn test_model_present_needs_every_required_file() {
        let dir = scratch("present");
        assert!(!model_present(&dir));
        for name in KOKORO_REQUIRED_FILES {
            assert!(!model_present(&dir), "incomplete before {}", name);
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        assert!(model_present(&dir));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_an_installed_model_is_never_downloaded_again() {
        let root = scratch("noop");
        let model_dir = root.join(KOKORO_ARCHIVE_ROOT);
        std::fs::create_dir_all(&model_dir).unwrap();
        for name in KOKORO_REQUIRED_FILES {
            std::fs::write(model_dir.join(name), b"x").unwrap();
        }
        // Port 1 refuses connections: reaching the network would fail the test.
        ensure_model_from("http://127.0.0.1:1/never", &model_dir)
            .await
            .expect("a present model must short-circuit");
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn test_installs_the_archive_root_as_the_model_dir_and_cleans_up() {
        let root = scratch("install");
        let model_dir = root.join("kokoro");
        let url = serve_archive(archive_with(&[
            "model.onnx",
            "voices.bin",
            "tokens.txt",
            "espeak-ng-data/phontab",
        ]))
        .await;

        ensure_model_from(&url, &model_dir)
            .await
            .expect("a well-formed archive should install");

        assert!(model_present(&model_dir));
        assert!(model_dir.join("espeak-ng-data/phontab").is_file());
        assert!(
            !root.join(ARCHIVE_FILE_NAME).exists(),
            "archive must be removed"
        );
        assert!(
            !root.join(STAGING_DIR_NAME).exists(),
            "staging must be removed"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn test_an_archive_missing_required_files_installs_nothing() {
        let root = scratch("incomplete");
        let model_dir = root.join("kokoro");
        let url = serve_archive(archive_with(&["model.onnx"])).await;

        let result = ensure_model_from(&url, &model_dir).await;

        assert!(
            matches!(result, Err(TtsError::ModelDownload(_))),
            "{:?}",
            result
        );
        assert!(!model_dir.exists());
        assert!(!root.join(STAGING_DIR_NAME).exists());
        assert!(!root.join(ARCHIVE_FILE_NAME).exists());
        std::fs::remove_dir_all(&root).ok();
    }

    /// A directory someone put there by hand is never deleted to make room,
    /// and is rejected before the archive is even downloaded: `retry_until_ready`
    /// never gives up, so a misconfigured directory must not re-download the
    /// archive on every retry forever.
    #[tokio::test]
    async fn test_an_incomplete_non_empty_model_dir_is_left_alone() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let root = scratch("occupied");
        let model_dir = root.join("kokoro");
        std::fs::create_dir_all(&model_dir).unwrap();
        std::fs::write(model_dir.join("notes.txt"), b"mine").unwrap();

        let hits = Arc::new(AtomicUsize::new(0));
        let hits_clone = Arc::clone(&hits);
        let bytes = archive_with(&KOKORO_REQUIRED_FILES);
        let app = axum::Router::new().route(
            "/kokoro.tar.bz2",
            axum::routing::get(move || {
                hits_clone.fetch_add(1, Ordering::SeqCst);
                let bytes = bytes.clone();
                async move { bytes }
            }),
        );
        let url = format!(
            "{}/kokoro.tar.bz2",
            crate::test_support::spawn_server(app).await
        );

        let result = ensure_model_from(&url, &model_dir).await;

        assert!(
            matches!(result, Err(TtsError::ModelDownload(_))),
            "{:?}",
            result
        );
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "the archive server must never be contacted for an occupied directory"
        );
        assert_eq!(std::fs::read(model_dir.join("notes.txt")).unwrap(), b"mine");
        assert!(!root.join(ARCHIVE_FILE_NAME).exists());
        assert!(!root.join(STAGING_DIR_NAME).exists());
        std::fs::remove_dir_all(&root).ok();
    }
}
