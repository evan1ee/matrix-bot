//! Remote S3 / Cloudflare R2 / MinIO compatible object storage provider.

use std::time::Duration;

use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::Client;
use bytes::Bytes;

use crate::domain::storage::{StorageError, StorageKey};
use crate::infrastructure::storage::{StorageConfig, StorageService};

/// S3-compatible implementation of [`StorageService`].
#[derive(Clone, Debug)]
pub struct S3StorageService {
    client: Client,
    bucket: String,
}

impl S3StorageService {
    /// Creates a new S3 storage service from [`StorageConfig`].
    pub fn new(config: &StorageConfig) -> Result<Self, StorageError> {
        let bucket = config.s3_bucket.clone().ok_or_else(|| {
            StorageError::Backend("S3_BUCKET is required for S3 storage driver".to_string())
        })?;

        let region = config
            .s3_region
            .clone()
            .unwrap_or_else(|| "auto".to_string());

        let mut s3_config_builder = aws_sdk_s3::config::Builder::new()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(region))
            .force_path_style(config.s3_force_path_style);

        if let Some(endpoint) = &config.s3_endpoint {
            s3_config_builder = s3_config_builder.endpoint_url(endpoint);
        }

        if let (Some(access_key), Some(secret_key)) = (&config.s3_access_key, &config.s3_secret_key)
        {
            let creds = Credentials::new(
                access_key.clone(),
                secret_key.clone(),
                None,
                None,
                "matrix-bot-static",
            );
            s3_config_builder = s3_config_builder.credentials_provider(creds);
        }

        let s3_config = s3_config_builder.build();
        let client = Client::from_conf(s3_config);

        Ok(Self { client, bucket })
    }

    /// Internal constructor taking an initialized S3 client and bucket name (useful for testing).
    pub fn from_client(client: Client, bucket: impl Into<String>) -> Self {
        Self {
            client,
            bucket: bucket.into(),
        }
    }
}

#[async_trait::async_trait]
impl StorageService for S3StorageService {
    async fn put(
        &self,
        key: &StorageKey,
        data: Bytes,
        content_type: &str,
    ) -> Result<(), StorageError> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .body(data.into())
            .content_type(content_type)
            .send()
            .await
            .map_err(|err| StorageError::Backend(format!("S3 put_object failed: {err}")))?;

        Ok(())
    }

    async fn get(&self, key: &StorageKey) -> Result<Bytes, StorageError> {
        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .send()
            .await
            .map_err(|err| {
                let service_err = err.into_service_error();
                if service_err.is_no_such_key() {
                    StorageError::NotFound(key.to_string())
                } else {
                    StorageError::Backend(format!("S3 get_object failed: {service_err}"))
                }
            })?;

        let body = resp
            .body
            .collect()
            .await
            .map_err(|err| StorageError::Backend(format!("failed to read S3 stream: {err}")))?;

        Ok(body.into_bytes())
    }

    async fn delete(&self, key: &StorageKey) -> Result<(), StorageError> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .send()
            .await
            .map_err(|err| StorageError::Backend(format!("S3 delete_object failed: {err}")))?;

        Ok(())
    }

    async fn get_presigned_url(
        &self,
        key: &StorageKey,
        ttl: Duration,
    ) -> Result<String, StorageError> {
        let presigning_config = PresigningConfig::expires_in(ttl).map_err(|err| {
            StorageError::Backend(format!("failed to configure presigning TTL: {err}"))
        })?;

        let presigned = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .presigned(presigning_config)
            .await
            .map_err(|err| StorageError::Backend(format!("S3 presigning failed: {err}")))?;

        Ok(presigned.uri().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::AccountId;
    use crate::domain::storage::StorageCategory;
    use crate::infrastructure::storage::StorageDriver;

    #[test]
    fn test_s3_storage_requires_bucket() {
        let config = StorageConfig {
            driver: StorageDriver::S3,
            local_dir: "data/storage".to_string(),
            web_url: None,
            signing_secret: "secret".to_string(),
            s3_endpoint: None,
            s3_bucket: None, // Missing bucket
            s3_region: Some("us-east-1".to_string()),
            s3_access_key: Some("key".to_string()),
            s3_secret_key: Some("secret".to_string()),
            s3_force_path_style: false,
        };

        let err = S3StorageService::new(&config).unwrap_err();
        assert!(matches!(err, StorageError::Backend(_)));
    }

    #[test]
    fn test_s3_storage_initializes_with_valid_config() {
        let config = StorageConfig {
            driver: StorageDriver::S3,
            local_dir: "data/storage".to_string(),
            web_url: None,
            signing_secret: "secret".to_string(),
            s3_endpoint: Some("https://r2.cloudflarestorage.com".to_string()),
            s3_bucket: Some("my-bucket".to_string()),
            s3_region: Some("auto".to_string()),
            s3_access_key: Some("test-key".to_string()),
            s3_secret_key: Some("test-secret".to_string()),
            s3_force_path_style: true,
        };

        let service = S3StorageService::new(&config).expect("S3 service should initialize");
        assert_eq!(service.bucket, "my-bucket");

        let account = AccountId::from_seed("s3-user");
        let key = StorageKey::generate(&account, StorageCategory::Receipts, "jpg");
        assert_eq!(key.category(), StorageCategory::Receipts);
    }
}
