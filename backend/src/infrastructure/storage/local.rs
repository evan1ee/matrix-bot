//! Local filesystem storage provider.
//!
//! Stores binary assets in a local directory structure while enforcing
//! path safety and atomic file operations.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bytes::Bytes;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::domain::storage::{StorageError, StorageKey};
use crate::infrastructure::storage::StorageService;
use crate::utils::secure::constant_time_eq;

type HmacSha256 = Hmac<Sha256>;

/// Local filesystem implementation of [`StorageService`].
#[derive(Clone)]
pub struct LocalStorageService {
    root_dir: PathBuf,
    web_url: Option<String>,
    signing_secret: String,
}

impl LocalStorageService {
    /// Creates a new local storage service. Ensures the root directory exists.
    pub fn new(
        root_dir: impl AsRef<Path>,
        web_url: Option<String>,
        signing_secret: impl Into<String>,
    ) -> std::io::Result<Self> {
        let root = root_dir.as_ref();
        std::fs::create_dir_all(root)?;
        let canonical_root = root.canonicalize()?;
        let web_url = web_url.map(|u| u.trim_end_matches('/').to_string());

        Ok(Self {
            root_dir: canonical_root,
            web_url,
            signing_secret: signing_secret.into(),
        })
    }

    /// Resolves the filesystem path for a given storage key.
    ///
    /// Double checks that the joined path starts with `root_dir`.
    fn resolve_path(&self, key: &StorageKey) -> Result<PathBuf, StorageError> {
        let target = self.root_dir.join(key.as_str());

        // Defense in depth against traversal:
        if !target.starts_with(&self.root_dir) {
            return Err(StorageError::InvalidKey(
                "storage path escapes base directory".to_string(),
            ));
        }

        Ok(target)
    }

    /// Computes the HMAC-SHA256 signature for a key and expiration timestamp.
    pub fn sign(&self, key_str: &str, exp: i64) -> String {
        sign_storage_key(&self.signing_secret, key_str, exp)
    }

    /// Verifies that an HMAC-SHA256 signature matches and has not expired.
    pub fn verify_signature(&self, key: &StorageKey, exp: i64, sig_hex: &str) -> bool {
        verify_storage_signature(&self.signing_secret, key, exp, sig_hex)
    }
}

/// Computes the HMAC-SHA256 signature for a key and expiration timestamp.
pub fn sign_storage_key(signing_secret: &str, key_str: &str, exp: i64) -> String {
    let mut mac =
        HmacSha256::new_from_slice(signing_secret.as_bytes()).expect("HMAC can take any key size");
    mac.update(key_str.as_bytes());
    mac.update(b":");
    mac.update(exp.to_string().as_bytes());
    let result = mac.finalize().into_bytes();
    result.iter().fold(String::with_capacity(64), |mut acc, b| {
        use std::fmt::Write;
        let _ = write!(acc, "{b:02x}");
        acc
    })
}

/// Verifies that an HMAC-SHA256 signature matches and has not expired.
pub fn verify_storage_signature(
    signing_secret: &str,
    key: &StorageKey,
    exp: i64,
    sig_hex: &str,
) -> bool {
    let now = chrono::Utc::now().timestamp();
    if now > exp {
        return false;
    }

    let expected_sig = sign_storage_key(signing_secret, key.as_str(), exp);
    constant_time_eq(expected_sig.as_bytes(), sig_hex.as_bytes())
}

#[async_trait::async_trait]
impl StorageService for LocalStorageService {
    async fn put(
        &self,
        key: &StorageKey,
        data: Bytes,
        _content_type: &str,
    ) -> Result<(), StorageError> {
        let target = self.resolve_path(key)?;
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| StorageError::Backend(format!("failed to create directory: {e}")))?;
        }

        // Atomic write: write to temp file then rename
        let tmp_file = target.with_extension(format!("tmp.{}", uuid::Uuid::new_v4()));
        tokio::fs::write(&tmp_file, &data)
            .await
            .map_err(|e| StorageError::Backend(format!("failed to write temp file: {e}")))?;

        tokio::fs::rename(&tmp_file, &target)
            .await
            .map_err(|e| StorageError::Backend(format!("failed to commit file: {e}")))?;

        Ok(())
    }

    async fn get(&self, key: &StorageKey) -> Result<Bytes, StorageError> {
        let target = self.resolve_path(key)?;
        match tokio::fs::read(&target).await {
            Ok(bytes) => Ok(Bytes::from(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(StorageError::NotFound(key.to_string()))
            }
            Err(e) => Err(StorageError::Backend(format!("failed to read file: {e}"))),
        }
    }

    async fn delete(&self, key: &StorageKey) -> Result<(), StorageError> {
        let target = self.resolve_path(key)?;
        match tokio::fs::remove_file(&target).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StorageError::Backend(format!("failed to delete file: {e}"))),
        }
    }

    async fn get_presigned_url(
        &self,
        key: &StorageKey,
        ttl: Duration,
    ) -> Result<String, StorageError> {
        let exp = chrono::Utc::now().timestamp() + ttl.as_secs() as i64;
        let sig = self.sign(key.as_str(), exp);
        let base = self.web_url.as_deref().unwrap_or("http://localhost:8080");
        Ok(format!(
            "{base}/api/v1/storage/{}?sig={sig}&exp={exp}",
            key.as_str()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::AccountId;
    use crate::domain::storage::StorageCategory;

    fn test_storage() -> (tempfile::TempDir, LocalStorageService) {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let service = LocalStorageService::new(
            temp_dir.path(),
            Some("http://127.0.0.1:8080".to_string()),
            "test-secret-key-12345",
        )
        .expect("local storage init");
        (temp_dir, service)
    }

    #[tokio::test]
    async fn test_local_storage_put_get_delete_round_trip() {
        let (_tmp, storage) = test_storage();
        let account = AccountId::from_seed("local-user");
        let key = StorageKey::generate(&account, StorageCategory::Receipts, "jpg");

        // Initially not found
        assert!(matches!(
            storage.get(&key).await,
            Err(StorageError::NotFound(_))
        ));

        // Put data
        let payload = Bytes::from_static(b"fake image bytes content");
        storage
            .put(&key, payload.clone(), "image/jpeg")
            .await
            .expect("put must succeed");

        // Get data back
        let retrieved = storage.get(&key).await.expect("get must succeed");
        assert_eq!(retrieved, payload);

        // Delete data
        storage.delete(&key).await.expect("delete must succeed");
        assert!(matches!(
            storage.get(&key).await,
            Err(StorageError::NotFound(_))
        ));

        // Delete again is idempotent
        storage.delete(&key).await.expect("delete missing is ok");
    }

    #[tokio::test]
    async fn test_local_storage_presigned_url_and_signature_verification() {
        let (_tmp, storage) = test_storage();
        let account = AccountId::from_seed("signer-user");
        let key = StorageKey::generate(&account, StorageCategory::Audio, "ogg");

        let url = storage
            .get_presigned_url(&key, Duration::from_secs(300))
            .await
            .expect("presigned url");

        assert!(url.starts_with("http://127.0.0.1:8080/api/v1/storage/"));
        assert!(url.contains("?sig="));
        assert!(url.contains("&exp="));

        // Extract sig and exp from query string
        let query = url.split('?').nth(1).expect("query part");
        let mut sig = "";
        let mut exp_str = "";
        for pair in query.split('&') {
            if let Some(s) = pair.strip_prefix("sig=") {
                sig = s;
            } else if let Some(e) = pair.strip_prefix("exp=") {
                exp_str = e;
            }
        }
        let exp: i64 = exp_str.parse().expect("exp int");

        // Verify valid signature
        assert!(storage.verify_signature(&key, exp, sig));

        // Expired signature fails
        assert!(!storage.verify_signature(&key, exp - 1000, sig));

        // Tampered key fails
        let other_key = StorageKey::generate(&account, StorageCategory::Audio, "ogg");
        assert!(!storage.verify_signature(&other_key, exp, sig));

        // Tampered sig fails
        assert!(!storage.verify_signature(&key, exp, "invalidhexsignature"));
    }
}
