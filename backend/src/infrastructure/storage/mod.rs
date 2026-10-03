//! Object storage abstractions and implementations.
//!
//! Provides the [`StorageService`] trait along with concrete implementations for
//! local filesystem storage and remote S3/R2-compatible storage.

use bytes::Bytes;
use std::time::Duration;

use crate::domain::storage::{StorageError, StorageKey};

pub mod local;

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

/// Selected storage driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageDriver {
    Local,
    S3,
}

impl StorageDriver {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "local" => Some(Self::Local),
            "s3" => Some(Self::S3),
            _ => None,
        }
    }
}

/// Configuration for object storage.
#[derive(Clone, Debug)]
pub struct StorageConfig {
    pub driver: StorageDriver,
    pub local_dir: String,
    pub web_url: Option<String>,
    pub signing_secret: String,
    pub s3_endpoint: Option<String>,
    pub s3_bucket: Option<String>,
    pub s3_region: Option<String>,
    pub s3_access_key: Option<String>,
    pub s3_secret_key: Option<String>,
    pub s3_force_path_style: bool,
}

impl StorageConfig {
    /// Builds configuration from environment variables.
    pub fn from_env(web_url: Option<String>, session_secret: &str) -> Self {
        let driver_str = std::env::var("STORAGE_DRIVER").unwrap_or_else(|_| "local".to_string());
        let driver = StorageDriver::parse(&driver_str).unwrap_or(StorageDriver::Local);
        let local_dir =
            std::env::var("STORAGE_LOCAL_DIR").unwrap_or_else(|_| "data/storage".to_string());
        let signing_secret =
            std::env::var("STORAGE_SIGNING_SECRET").unwrap_or_else(|_| session_secret.to_string());

        let s3_endpoint = std::env::var("S3_ENDPOINT")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let s3_bucket = std::env::var("S3_BUCKET")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let s3_region = std::env::var("S3_REGION")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let s3_access_key = std::env::var("S3_ACCESS_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let s3_secret_key = std::env::var("S3_SECRET_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let s3_force_path_style = std::env::var("S3_FORCE_PATH_STYLE")
            .map(|s| s.trim().eq_ignore_ascii_case("true") || s.trim() == "1")
            .unwrap_or(false);

        Self {
            driver,
            local_dir,
            web_url,
            signing_secret,
            s3_endpoint,
            s3_bucket,
            s3_region,
            s3_access_key,
            s3_secret_key,
            s3_force_path_style,
        }
    }
}
