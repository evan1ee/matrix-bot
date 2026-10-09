//! Downloads the embedding model and tokenizer, verified against pinned checksums.

use std::path::Path;

use thiserror::Error;
use tracing::info;

use crate::download::download_verified;

use super::{MODEL_FILE, TOKENIZER_FILE};

/// Revision of `Xenova/multilingual-e5-small` the checksums below were taken
/// from. URLs are pinned to this commit, not `main`, so a later upstream
/// re-export can never silently change the vectors (or fail the checksum on a
/// healthy deployment).
macro_rules! revision {
    () => {
        "761b726dd34fb83930e26aab4e9ac3899aa1fa78"
    };
}

macro_rules! model_url {
    ($path:literal) => {
        concat!(
            "https://huggingface.co/Xenova/multilingual-e5-small/resolve/",
            revision!(),
            "/",
            $path
        )
    };
}

/// One file to install: where it comes from and the SHA-256 it must have.
pub struct ModelFile {
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
}

/// The int8-quantised ONNX export of `intfloat/multilingual-e5-small` (the
/// one transformers.js loads by default, ~118 MB) and its tokenizer.
pub const MODEL_FILES: [ModelFile; 2] = [
    ModelFile {
        name: MODEL_FILE,
        url: model_url!("onnx/model_quantized.onnx"),
        sha256: "f80102d3f2a1229f387d3c81909990d8945513e347b0eab049f7de3c6f98c193",
    },
    ModelFile {
        name: TOKENIZER_FILE,
        url: model_url!("tokenizer.json"),
        sha256: "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
    },
];

#[derive(Debug, Error)]
#[error("{0}")]
pub struct ProvisionError(String);

/// Installs whichever of the model files `dir` is missing.
pub async fn ensure_models(dir: &Path) -> Result<(), ProvisionError> {
    ensure_files(dir, &MODEL_FILES).await
}

async fn ensure_files(dir: &Path, files: &[ModelFile]) -> Result<(), ProvisionError> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| ProvisionError(format!("Failed to create {}: {e}", dir.display())))?;

    for file in files {
        let dest = dir.join(file.name);
        if dest.is_file() {
            continue;
        }
        info!("Downloading embedding model file {}", file.name);
        download_verified(file.url, &dest, file.sha256)
            .await
            .map_err(|e| ProvisionError(format!("{}: {e}", file.name)))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("embed-provision-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        dir
    }

    /// Serves `body` at `/file` and describes it as a `ModelFile` called `name`.
    /// The test builds its own checksum so no test ever reaches the network.
    async fn served(name: &'static str, body: &'static [u8], sha256: String) -> ModelFile {
        let app =
            axum::Router::new().route("/file", axum::routing::get(move || async move { body }));
        let base = crate::test_support::spawn_server(app).await;
        ModelFile {
            name,
            url: Box::leak(format!("{base}/file").into_boxed_str()),
            sha256: Box::leak(sha256.into_boxed_str()),
        }
    }

    fn sha(body: &[u8]) -> String {
        format!("{:x}", Sha256::digest(body))
    }

    #[tokio::test]
    async fn missing_files_are_downloaded_into_a_new_directory() {
        let dir = scratch_dir("fresh");
        let files = [
            served("a.bin", b"alpha", sha(b"alpha")).await,
            served("b.bin", b"beta", sha(b"beta")).await,
        ];

        ensure_files(&dir, &files).await.expect("should download");

        assert_eq!(std::fs::read(dir.join("a.bin")).unwrap(), b"alpha");
        assert_eq!(std::fs::read(dir.join("b.bin")).unwrap(), b"beta");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn files_already_present_are_not_fetched_again() {
        let dir = scratch_dir("present");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.bin"), b"installed").unwrap();
        // Unreachable URL: any attempt to download would fail the test.
        let files = [ModelFile {
            name: "a.bin",
            url: "http://127.0.0.1:1/never",
            sha256: "0000",
        }];

        ensure_files(&dir, &files).await.expect("nothing to do");

        assert_eq!(std::fs::read(dir.join("a.bin")).unwrap(), b"installed");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_file_failing_its_checksum_is_not_installed() {
        let dir = scratch_dir("tampered");
        let files = [served("a.bin", b"tampered", sha(b"original")).await];

        let err = ensure_files(&dir, &files).await.expect_err("must refuse");

        assert!(err.to_string().contains("a.bin"), "unhelpful: {err}");
        assert!(!dir.join("a.bin").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_pinned_urls_point_at_the_pinned_revision() {
        for file in &MODEL_FILES {
            assert!(file.url.contains(revision!()), "{} is not pinned", file.url);
            assert_eq!(file.sha256.len(), 64, "{} is not a SHA-256", file.name);
        }
    }
}
