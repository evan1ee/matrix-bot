//! Object storage abstractions and implementations.
//!
//! Provides the [`StorageService`] trait along with concrete implementations for
//! local filesystem storage and remote S3/R2-compatible storage.

use bytes::Bytes;
use std::time::Duration;

use crate::domain::storage::{StorageError, StorageKey};

pub mod local;
pub mod s3;

pub use local::LocalStorageService;
pub use s3::S3StorageService;

/// Common contract for object storage providers.
#[async_trait::async_trait]
pub trait StorageService: Send + Sync {
    /// Stores binary data under the canonical storage key.
    async fn put(
        &self,
        key: &StorageKey,
        data: Bytes,
        content_type: &str,
    ) -> Result<(), StorageError>;

    /// Retrieves binary data for the given canonical storage key.
    async fn get(&self, key: &StorageKey) -> Result<Bytes, StorageError>;

    /// Deletes the asset associated with the storage key.
    async fn delete(&self, key: &StorageKey) -> Result<(), StorageError>;

    /// Generates a presigned URL (or signed temporary access link) for the given asset.
    async fn get_presigned_url(
        &self,
        key: &StorageKey,
        ttl: Duration,
    ) -> Result<String, StorageError>;
}

pub use crate::config::{StorageConfig, StorageDriver};

/// Builds the configured [`StorageService`] implementation based on [`StorageConfig`].
pub fn build_storage(
    config: &StorageConfig,
) -> Result<std::sync::Arc<dyn StorageService>, StorageError> {
    match config.driver {
        StorageDriver::Local => {
            let local = LocalStorageService::new(
                &config.local_dir,
                config.web_url.clone(),
                &config.signing_secret,
            )
            .map_err(|e| {
                StorageError::Backend(format!("failed to initialize local storage: {e}"))
            })?;
            Ok(std::sync::Arc::new(local))
        }
        StorageDriver::S3 => {
            let s3 = S3StorageService::new(config)?;
            Ok(std::sync::Arc::new(s3))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_driver_parse() {
        assert_eq!(StorageDriver::parse("local"), Some(StorageDriver::Local));
        assert_eq!(StorageDriver::parse("LOCAL"), Some(StorageDriver::Local));
        assert_eq!(StorageDriver::parse("s3"), Some(StorageDriver::S3));
        assert_eq!(StorageDriver::parse("  S3  "), Some(StorageDriver::S3));
        assert_eq!(StorageDriver::parse("unknown"), None);
    }

    #[tokio::test]
    async fn test_build_storage_local() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = StorageConfig {
            driver: StorageDriver::Local,
            local_dir: temp_dir.path().to_str().unwrap().to_string(),
            web_url: Some("http://localhost:3000".to_string()),
            signing_secret: "secret-key".to_string(),
            s3_endpoint: None,
            s3_bucket: None,
            s3_region: None,
            s3_access_key: None,
            s3_secret_key: None,
            s3_force_path_style: false,
        };

        let storage = build_storage(&config).expect("build local storage");
        assert!(storage
            .get_presigned_url(
                &crate::domain::storage::StorageKey::generate(
                    &crate::domain::accounts::AccountId::from_seed("u"),
                    crate::domain::storage::StorageCategory::Receipts,
                    "jpg"
                ),
                Duration::from_secs(60)
            )
            .await
            .is_ok());
    }

    #[test]
    fn test_build_storage_s3() {
        let config = StorageConfig {
            driver: StorageDriver::S3,
            local_dir: "data/storage".to_string(),
            web_url: None,
            signing_secret: "secret-key".to_string(),
            s3_endpoint: Some("https://s3.example.com".to_string()),
            s3_bucket: Some("bucket".to_string()),
            s3_region: Some("auto".to_string()),
            s3_access_key: Some("key".to_string()),
            s3_secret_key: Some("secret".to_string()),
            s3_force_path_style: true,
        };

        assert!(build_storage(&config).is_ok());
    }
}
