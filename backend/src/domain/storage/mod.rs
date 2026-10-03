//! Pure domain types for object storage.
//!
//! Enforces path safety, canonical key formatting, and tenant/owner isolation
//! without depending on I/O or filesystem runtime.

use std::fmt;
use std::ops::Deref;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::domain::accounts::AccountId;

/// Maximum allowed length of a storage key string.
pub const MAX_KEY_LEN: usize = 256;

/// Storage domain error. Pure: std, serde, thiserror only.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StorageError {
    #[error("storage key not found: {0}")]
    NotFound(String),
    #[error("invalid storage key: {0}")]
    InvalidKey(String),
    #[error("access denied to storage key")]
    AccessDenied,
    #[error("storage backend error: {0}")]
    Backend(String),
    #[error("unsupported storage operation: {0}")]
    Unsupported(String),
}

/// Category of stored binary assets under an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageCategory {
    Receipts,
    Audio,
    Attachments,
    Exports,
}

impl StorageCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Receipts => "receipts",
            Self::Audio => "audio",
            Self::Attachments => "attachments",
            Self::Exports => "exports",
        }
    }

    pub fn from_str_slug(s: &str) -> Option<Self> {
        match s {
            "receipts" => Some(Self::Receipts),
            "audio" => Some(Self::Audio),
            "attachments" => Some(Self::Attachments),
            "exports" => Some(Self::Exports),
            _ => None,
        }
    }
}

impl fmt::Display for StorageCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A validated, canonical storage key scoped to an account.
///
/// Canonical structure:
/// `accounts/{account_id}/{category}/{filename}`
///
/// Invariants:
/// - Begins with `accounts/`
/// - Next segment must be a valid [`AccountId`] (`acct_<32hex>` or UUID)
/// - Next segment must be a recognized [`StorageCategory`]
/// - Final segment is a filename with extension
/// - Allowed characters: `[a-zA-Z0-9_.-]`
/// - Path traversal (`..`, `\`, leading/trailing `/`, `//`) is strictly forbidden
/// - Maximum total length is 256 bytes
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StorageKey {
    raw: String,
    account_id: AccountId,
    category: StorageCategory,
    filename_offset: usize,
}

impl StorageKey {
    /// Generates a new canonical storage key for the given account and category.
    pub fn generate(owner: &AccountId, category: StorageCategory, ext: &str) -> Self {
        let clean_ext = ext.trim_start_matches('.').to_ascii_lowercase();
        let uuid_str = uuid::Uuid::new_v4().to_string();
        let filename = if clean_ext.is_empty() {
            uuid_str
        } else {
            format!("{uuid_str}.{clean_ext}")
        };
        let raw = format!("accounts/{}/{}/{}", owner, category, filename);
        let filename_offset = format!("accounts/{}/{}/", owner, category).len();

        Self {
            raw,
            account_id: *owner,
            category,
            filename_offset,
        }
    }

    /// Parses and validates a canonical storage key string.
    pub fn parse(s: &str) -> Result<Self, StorageError> {
        if s.is_empty() || s.len() > MAX_KEY_LEN {
            return Err(StorageError::InvalidKey(format!(
                "key length must be between 1 and {MAX_KEY_LEN} bytes"
            )));
        }

        // Reject Windows separators, null bytes, and traversal markers
        if s.contains('\\') || s.contains('\0') {
            return Err(StorageError::InvalidKey(
                "key contains forbidden separators or null bytes".to_string(),
            ));
        }

        if s.starts_with('/') || s.ends_with('/') || s.contains("//") {
            return Err(StorageError::InvalidKey(
                "key must not have leading, trailing, or duplicate slashes".to_string(),
            ));
        }

        let segments: Vec<&str> = s.split('/').collect();
        if segments.len() != 4 {
            return Err(StorageError::InvalidKey(
                "key must have exactly 4 segments: accounts/{account_id}/{category}/{filename}"
                    .to_string(),
            ));
        }

        if segments[0] != "accounts" {
            return Err(StorageError::InvalidKey(
                "key must begin with 'accounts'".to_string(),
            ));
        }

        // Validate AccountId
        let account_id = AccountId::from_str(segments[1]).map_err(|_| {
            StorageError::InvalidKey(format!("invalid account id: {}", segments[1]))
        })?;

        // Validate Category
        let category = StorageCategory::from_str_slug(segments[2]).ok_or_else(|| {
            StorageError::InvalidKey(format!("unrecognized storage category: {}", segments[2]))
        })?;

        // Validate Filename
        let filename = segments[3];
        if filename.is_empty() {
            return Err(StorageError::InvalidKey("filename is empty".to_string()));
        }

        if filename == "." || filename == ".." || filename.contains("..") {
            return Err(StorageError::InvalidKey(
                "filename must not contain traversal elements".to_string(),
            ));
        }

        // Filename character whitelist: alphanumeric, dash, underscore, dot
        if !filename
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            return Err(StorageError::InvalidKey(
                "filename contains forbidden characters".to_string(),
            ));
        }

        let filename_offset = segments[0].len() + 1 + segments[1].len() + 1 + segments[2].len() + 1;

        Ok(Self {
            raw: s.to_string(),
            account_id,
            category,
            filename_offset,
        })
    }

    /// Returns the account ID owning this storage key.
    pub fn account_id(&self) -> AccountId {
        self.account_id
    }

    /// Returns the storage category.
    pub fn category(&self) -> StorageCategory {
        self.category
    }

    /// Returns the filename part of the key.
    pub fn filename(&self) -> &str {
        &self.raw[self.filename_offset..]
    }

    /// Returns the canonical key as a string slice.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Consumes the key and returns the inner string.
    pub fn into_inner(self) -> String {
        self.raw
    }
}

impl Deref for StorageKey {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}

impl AsRef<str> for StorageKey {
    fn as_ref(&self) -> &str {
        &self.raw
    }
}

impl fmt::Display for StorageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl fmt::Debug for StorageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StorageKey({:?})", self.raw)
    }
}

impl FromStr for StorageKey {
    type Err = StorageError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for StorageKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.raw)
    }
}

impl<'de> Deserialize<'de> for StorageKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_key_generate_and_parse() {
        let account = AccountId::from_seed("test-user");
        let key = StorageKey::generate(&account, StorageCategory::Receipts, "jpg");

        assert_eq!(key.account_id(), account);
        assert_eq!(key.category(), StorageCategory::Receipts);
        assert!(key.filename().ends_with(".jpg"));
        assert!(key
            .as_str()
            .starts_with(&format!("accounts/{account}/receipts/")));

        // Round trip parse
        let parsed = StorageKey::parse(key.as_str()).expect("valid key should parse");
        assert_eq!(key, parsed);
        assert_eq!(key.filename(), parsed.filename());
    }

    #[test]
    fn test_storage_key_serde_roundtrip() {
        let account = AccountId::from_seed("user-1");
        let key = StorageKey::generate(&account, StorageCategory::Audio, "ogg");

        let json = serde_json::to_string(&key).expect("serialize");
        let deserialized: StorageKey = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(key, deserialized);
    }

    #[test]
    fn test_storage_key_rejects_path_traversal() {
        let bad_keys = [
            "../etc/passwd",
            "accounts/../foo/bar",
            "accounts/acct_00000000000000000000000000000000/receipts/..",
            "accounts/acct_00000000000000000000000000000000/receipts/../other.jpg",
            "accounts/acct_00000000000000000000000000000000/receipts/....jpg",
            "/accounts/acct_00000000000000000000000000000000/receipts/file.jpg",
            "accounts/acct_00000000000000000000000000000000/receipts/file.jpg/",
            "accounts//acct_00000000000000000000000000000000/receipts/file.jpg",
            "accounts\\acct_00000000000000000000000000000000\\receipts\\file.jpg",
            "accounts/acct_00000000000000000000000000000000/receipts/file\0.jpg",
        ];

        for bad in bad_keys {
            assert!(
                StorageKey::parse(bad).is_err(),
                "expected '{bad}' to be rejected"
            );
        }
    }

    #[test]
    fn test_storage_key_rejects_invalid_segments() {
        // Bad account
        assert!(StorageKey::parse("accounts/not-an-account/receipts/file.jpg").is_err());
        // Bad category
        assert!(StorageKey::parse(
            "accounts/acct_00000000000000000000000000000000/unknown_cat/file.jpg"
        )
        .is_err());
        // Not starting with accounts
        assert!(
            StorageKey::parse("other/acct_00000000000000000000000000000000/receipts/file.jpg")
                .is_err()
        );
        // Too few segments
        assert!(
            StorageKey::parse("accounts/acct_00000000000000000000000000000000/receipts").is_err()
        );
        // Too many segments
        assert!(StorageKey::parse(
            "accounts/acct_00000000000000000000000000000000/receipts/sub/file.jpg"
        )
        .is_err());
    }

    #[test]
    fn test_storage_key_rejects_bad_characters_and_length() {
        let account = AccountId::from_seed("user-2");
        let bad_filename = format!("accounts/{account}/receipts/file with spaces.jpg");
        assert!(StorageKey::parse(&bad_filename).is_err());

        let bad_chars = format!("accounts/{account}/receipts/file;rm -rf.jpg");
        assert!(StorageKey::parse(&bad_chars).is_err());

        let long_name = "a".repeat(300);
        let too_long = format!("accounts/{account}/receipts/{long_name}.jpg");
        assert!(StorageKey::parse(&too_long).is_err());
    }
}
