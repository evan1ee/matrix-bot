use std::env;

/// Selected storage driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StorageDriver {
    #[default]
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

pub const DEFAULT_SIGNING_SECRET: &str = "matrix-bot-default-signing-secret";

impl StorageConfig {
    /// Builds configuration from environment variables.
    pub fn from_env(web_url: Option<String>) -> Self {
        let driver_str = env::var("STORAGE_DRIVER").unwrap_or_else(|_| "local".to_string());
        let driver = StorageDriver::parse(&driver_str).unwrap_or(StorageDriver::Local);
        let local_dir =
            env::var("STORAGE_LOCAL_DIR").unwrap_or_else(|_| "data/storage".to_string());
        let signing_secret = env::var("STORAGE_SIGNING_SECRET")
            .unwrap_or_else(|_| DEFAULT_SIGNING_SECRET.to_string());

        let s3_endpoint = env::var("S3_ENDPOINT")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let s3_bucket = env::var("S3_BUCKET").ok().filter(|s| !s.trim().is_empty());
        let s3_region = env::var("S3_REGION").ok().filter(|s| !s.trim().is_empty());
        let s3_access_key = env::var("S3_ACCESS_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let s3_secret_key = env::var("S3_SECRET_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let s3_force_path_style = env::var("S3_FORCE_PATH_STYLE")
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

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            driver: StorageDriver::Local,
            local_dir: "data/storage".to_string(),
            web_url: None,
            signing_secret: DEFAULT_SIGNING_SECRET.to_string(),
            s3_endpoint: None,
            s3_bucket: None,
            s3_region: None,
            s3_access_key: None,
            s3_secret_key: None,
            s3_force_path_style: false,
        }
    }
}
