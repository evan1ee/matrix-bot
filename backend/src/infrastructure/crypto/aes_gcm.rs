//! AES-256-GCM authenticated secret protection.
//!
//! Encrypts and decrypts secrets at rest using 256-bit symmetric keys, 96-bit
//! random nonces, and AAD bound to `{provider}:{name}:{key_version}`.

use std::collections::HashMap;

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;

use crate::domain::secrets::{EncryptedSecret, SecretProvider, SecretValue};

/// Errors encountered during secret encryption or decryption.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("Encryption master key unavailable: {0}")]
    KeyUnavailable(String),
    #[error("Invalid key format: {0}")]
    InvalidKey(String),
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),
    #[error("Decryption failed (integrity or authenticity check failed): {0}")]
    DecryptionFailed(String),
    #[error("Key version {0} not found")]
    UnknownKeyVersion(i32),
    #[error("Invalid nonce length: expected 12 bytes, got {0}")]
    InvalidNonce(usize),
}

/// Trait for encrypting and decrypting credential values at rest.
pub trait SecretProtector: Send + Sync {
    fn is_configured(&self) -> bool {
        true
    }

    fn encrypt(
        &self,
        provider: SecretProvider,
        name: &str,
        secret: &SecretValue,
    ) -> Result<EncryptedSecret, CryptoError>;

    fn decrypt(
        &self,
        provider: SecretProvider,
        name: &str,
        encrypted: &EncryptedSecret,
    ) -> Result<SecretValue, CryptoError>;
}

/// Provider for AES-256 master symmetric keys with version support.
#[derive(Clone)]
pub struct MasterKeyProvider {
    primary_version: i32,
    keys: HashMap<i32, [u8; 32]>,
}

impl std::fmt::Debug for MasterKeyProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut versions: Vec<i32> = self.keys.keys().copied().collect();
        versions.sort_unstable();
        f.debug_struct("MasterKeyProvider")
            .field("primary_version", &self.primary_version)
            .field("key_versions", &versions)
            .finish()
    }
}

impl MasterKeyProvider {
    fn ensure_non_zero(key: &[u8; 32]) -> Result<(), CryptoError> {
        if key.iter().all(|&b| b == 0) {
            return Err(CryptoError::InvalidKey(
                "Encryption key must not be all zeros".into(),
            ));
        }
        Ok(())
    }

    /// Constructs a key provider with a single raw 32-byte key at version 1.
    pub fn from_raw_key(key: [u8; 32]) -> Self {
        let mut keys = HashMap::new();
        keys.insert(1, key);
        Self {
            primary_version: 1,
            keys,
        }
    }

    /// Constructs a key provider with multiple key versions.
    pub fn from_key_map(
        keys: HashMap<i32, [u8; 32]>,
        primary_version: i32,
    ) -> Result<Self, CryptoError> {
        if keys.is_empty() {
            return Err(CryptoError::KeyUnavailable(
                "No encryption keys provided".into(),
            ));
        }
        for (&ver, key) in &keys {
            if ver <= 0 {
                return Err(CryptoError::InvalidKey(
                    "Key version must be a positive integer (>= 1)".into(),
                ));
            }
            Self::ensure_non_zero(key)?;
        }
        if !keys.contains_key(&primary_version) {
            return Err(CryptoError::UnknownKeyVersion(primary_version));
        }
        Ok(Self {
            primary_version,
            keys,
        })
    }

    /// Resolves encryption master keys from optional key strings.
    ///
    /// Reads `primary` for a single master key (version 1),
    /// or `versioned` for a versioned map (e.g. `1:<key1>,2:<key2>`).
    /// Keys may be hex-encoded (64 chars) or base64-encoded (32 raw bytes).
    pub fn from_key_strings(
        primary: Option<&str>,
        versioned: Option<&str>,
    ) -> Result<Self, CryptoError> {
        if let Some(keys_str) = versioned {
            let trimmed = keys_str.trim();
            if !trimmed.is_empty() {
                return Self::parse_versioned_keys(trimmed);
            }
        }

        if let Some(key_str) = primary {
            let trimmed = key_str.trim();
            if !trimmed.is_empty() {
                let raw = Self::parse_key(trimmed)?;
                return Ok(Self::from_raw_key(raw));
            }
        }

        Err(CryptoError::KeyUnavailable(
            "SECRETS_ENCRYPTION_KEY is not configured".into(),
        ))
    }

    /// Parses a 32-byte key from a 64-char hex string or base64 string.
    pub fn parse_key(raw: &str) -> Result<[u8; 32], CryptoError> {
        let trimmed = raw.trim();
        // Try hex if 64 hex characters
        if trimmed.len() == 64 {
            if let Ok(bytes) = hex::decode(trimmed) {
                if bytes.len() == 32 {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&bytes);
                    Self::ensure_non_zero(&arr)?;
                    return Ok(arr);
                }
            }
        }

        // Try standard base64
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(trimmed) {
            if bytes.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                Self::ensure_non_zero(&arr)?;
                return Ok(arr);
            }
        }

        // Try URL-safe base64
        if let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(trimmed) {
            if bytes.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                Self::ensure_non_zero(&arr)?;
                return Ok(arr);
            }
        }

        Err(CryptoError::InvalidKey(
            "Key must be 64-character hex or valid 32-byte base64".into(),
        ))
    }

    fn parse_versioned_keys(raw: &str) -> Result<Self, CryptoError> {
        let mut keys = HashMap::new();

        for (idx, part) in raw.split(',').enumerate() {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let entry_num = idx + 1;
            let (ver_str, key_str) = part.split_once(':').ok_or_else(|| {
                CryptoError::InvalidKey(format!("entry {entry_num} is not in version:key format"))
            })?;
            let ver: i32 = ver_str.trim().parse().map_err(|_| {
                CryptoError::InvalidKey(format!(
                    "entry {entry_num} has an invalid positive integer version"
                ))
            })?;
            if ver <= 0 {
                return Err(CryptoError::InvalidKey(format!(
                    "entry {entry_num} has a non-positive version (must be >= 1)"
                )));
            }
            if keys.contains_key(&ver) {
                return Err(CryptoError::InvalidKey(format!(
                    "duplicate key version {ver} in SECRETS_ENCRYPTION_KEYS"
                )));
            }
            let key = Self::parse_key(key_str)?;
            keys.insert(ver, key);
        }

        let primary_version = keys.keys().copied().max().ok_or_else(|| {
            CryptoError::KeyUnavailable("No keys found in SECRETS_ENCRYPTION_KEYS".into())
        })?;

        Ok(Self {
            primary_version,
            keys,
        })
    }

    /// Fetches the raw key for the specified version.
    pub fn get_key(&self, version: i32) -> Result<&[u8; 32], CryptoError> {
        self.keys
            .get(&version)
            .ok_or(CryptoError::UnknownKeyVersion(version))
    }

    /// Returns the active primary version and key for new encryptions.
    pub fn primary_key(&self) -> (i32, &[u8; 32]) {
        (self.primary_version, &self.keys[&self.primary_version])
    }
}

/// AES-256-GCM authenticated secret protector.
#[derive(Clone)]
pub struct AesGcmSecretProtector {
    keys: MasterKeyProvider,
}

impl AesGcmSecretProtector {
    pub fn new(keys: MasterKeyProvider) -> Self {
        Self { keys }
    }

    pub fn from_key_strings(
        primary: Option<&str>,
        versioned: Option<&str>,
    ) -> Result<Self, CryptoError> {
        MasterKeyProvider::from_key_strings(primary, versioned).map(Self::new)
    }

    fn aad_string(provider: SecretProvider, name: &str, key_version: i32) -> String {
        format!("{}:{}:{}", provider.as_str(), name, key_version)
    }
}

impl SecretProtector for AesGcmSecretProtector {
    fn encrypt(
        &self,
        provider: SecretProvider,
        name: &str,
        secret: &SecretValue,
    ) -> Result<EncryptedSecret, CryptoError> {
        let (key_version, key_bytes) = self.keys.primary_key();
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

        let aad = Self::aad_string(provider, name, key_version);
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key_bytes));

        let payload = Payload {
            msg: secret.expose_secret().as_bytes(),
            aad: aad.as_bytes(),
        };

        let ciphertext = cipher
            .encrypt(&nonce, payload)
            .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

        Ok(EncryptedSecret {
            ciphertext,
            nonce: nonce.to_vec(),
            key_version,
        })
    }

    fn decrypt(
        &self,
        provider: SecretProvider,
        name: &str,
        encrypted: &EncryptedSecret,
    ) -> Result<SecretValue, CryptoError> {
        let key_bytes = self.keys.get_key(encrypted.key_version)?;
        if encrypted.nonce.len() != 12 {
            return Err(CryptoError::InvalidNonce(encrypted.nonce.len()));
        }

        let aad = Self::aad_string(provider, name, encrypted.key_version);
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key_bytes));
        let nonce = Nonce::from_slice(&encrypted.nonce);

        let payload = Payload {
            msg: &encrypted.ciphertext,
            aad: aad.as_bytes(),
        };

        let decrypted = cipher
            .decrypt(nonce, payload)
            .map_err(|e| CryptoError::DecryptionFailed(e.to_string()))?;

        let secret_str = String::from_utf8(decrypted).map_err(|e| {
            CryptoError::DecryptionFailed(format!("Decrypted secret is not valid UTF-8: {e}"))
        })?;

        Ok(SecretValue::new(secret_str))
    }
}

/// Failsafe protector that fails all operations when encryption key is unconfigured.
#[derive(Debug, Clone, Copy, Default)]
pub struct FailClosedSecretProtector;

impl SecretProtector for FailClosedSecretProtector {
    fn is_configured(&self) -> bool {
        false
    }

    fn encrypt(
        &self,
        _provider: SecretProvider,
        _name: &str,
        _secret: &SecretValue,
    ) -> Result<EncryptedSecret, CryptoError> {
        Err(CryptoError::KeyUnavailable(
            "SECRETS_ENCRYPTION_KEY environment variable is not configured".into(),
        ))
    }

    fn decrypt(
        &self,
        _provider: SecretProvider,
        _name: &str,
        _encrypted: &EncryptedSecret,
    ) -> Result<SecretValue, CryptoError> {
        Err(CryptoError::KeyUnavailable(
            "SECRETS_ENCRYPTION_KEY environment variable is not configured".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_protector() -> AesGcmSecretProtector {
        let key = [0x42u8; 32];
        AesGcmSecretProtector::new(MasterKeyProvider::from_raw_key(key))
    }

    #[test]
    fn test_round_trip_encryption_and_decryption() {
        let protector = test_protector();
        let secret = SecretValue::new("sk-test-secret-value-12345");

        let encrypted = protector
            .encrypt(SecretProvider::OpenAi, "default", &secret)
            .expect("encryption succeeds");

        assert_eq!(encrypted.nonce.len(), 12);
        assert_eq!(encrypted.key_version, 1);
        assert_ne!(encrypted.ciphertext, secret.expose_secret().as_bytes());

        let decrypted = protector
            .decrypt(SecretProvider::OpenAi, "default", &encrypted)
            .expect("decryption succeeds");

        assert_eq!(decrypted.expose_secret(), secret.expose_secret());
    }

    #[test]
    fn test_nonce_uniqueness_across_encryptions() {
        let protector = test_protector();
        let secret = SecretValue::new("constant-secret");

        let enc1 = protector
            .encrypt(SecretProvider::OpenAi, "default", &secret)
            .unwrap();
        let enc2 = protector
            .encrypt(SecretProvider::OpenAi, "default", &secret)
            .unwrap();

        assert_ne!(enc1.nonce, enc2.nonce);
        assert_ne!(enc1.ciphertext, enc2.ciphertext);
    }

    #[test]
    fn test_aad_provider_mismatch_fails() {
        let protector = test_protector();
        let secret = SecretValue::new("provider-bound-token");

        let encrypted = protector
            .encrypt(SecretProvider::OpenAi, "default", &secret)
            .unwrap();

        let err = protector
            .decrypt(SecretProvider::Anthropic, "default", &encrypted)
            .unwrap_err();

        assert!(matches!(err, CryptoError::DecryptionFailed(_)));
    }

    #[test]
    fn test_aad_name_mismatch_fails() {
        let protector = test_protector();
        let secret = SecretValue::new("custom-openai-key");

        let encrypted = protector
            .encrypt(SecretProvider::OpenAi, "primary", &secret)
            .unwrap();

        let err = protector
            .decrypt(SecretProvider::OpenAi, "secondary", &encrypted)
            .unwrap_err();

        assert!(matches!(err, CryptoError::DecryptionFailed(_)));
    }

    #[test]
    fn test_tampered_ciphertext_fails() {
        let protector = test_protector();
        let secret = SecretValue::new("super-sensitive-token");

        let mut encrypted = protector
            .encrypt(SecretProvider::Tavily, "default", &secret)
            .unwrap();

        // Flip a byte in ciphertext
        if let Some(first) = encrypted.ciphertext.first_mut() {
            *first ^= 0x01;
        }

        let err = protector
            .decrypt(SecretProvider::Tavily, "default", &encrypted)
            .unwrap_err();

        assert!(matches!(err, CryptoError::DecryptionFailed(_)));
    }

    #[test]
    fn test_invalid_nonce_length_fails() {
        let protector = test_protector();
        let encrypted = EncryptedSecret {
            ciphertext: vec![1, 2, 3, 4],
            nonce: vec![1, 2, 3], // invalid nonce
            key_version: 1,
        };

        let err = protector
            .decrypt(SecretProvider::OpenAi, "default", &encrypted)
            .unwrap_err();

        assert!(matches!(err, CryptoError::InvalidNonce(3)));
    }

    #[test]
    fn test_unknown_key_version_fails() {
        let protector = test_protector();
        let encrypted = EncryptedSecret {
            ciphertext: vec![1, 2, 3, 4],
            nonce: vec![0u8; 12],
            key_version: 99,
        };

        let err = protector
            .decrypt(SecretProvider::OpenAi, "default", &encrypted)
            .unwrap_err();

        assert!(matches!(err, CryptoError::UnknownKeyVersion(99)));
    }

    #[test]
    fn test_parse_key_hex_and_base64() {
        // 32-byte key hex encoded (64 chars)
        let hex_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let parsed = MasterKeyProvider::parse_key(hex_key).expect("parses hex");
        assert_eq!(parsed[0], 0x01);
        assert_eq!(parsed[15], 0xef);

        // 32-byte key base64 encoded
        let b64_key = base64::engine::general_purpose::STANDARD.encode([7u8; 32]);
        let parsed_b64 = MasterKeyProvider::parse_key(&b64_key).expect("parses base64");
        assert_eq!(parsed_b64, [7u8; 32]);

        // Invalid key
        assert!(MasterKeyProvider::parse_key("not-a-valid-key").is_err());
    }

    #[test]
    fn test_versioned_keys_parsing() {
        let k1 = hex::encode([1u8; 32]);
        let k2 = hex::encode([2u8; 32]);
        let input = format!("1:{k1}, 2:{k2}");

        let provider = MasterKeyProvider::parse_versioned_keys(&input).unwrap();
        assert_eq!(provider.primary_version, 2);
        assert_eq!(*provider.get_key(1).unwrap(), [1u8; 32]);
        assert_eq!(*provider.get_key(2).unwrap(), [2u8; 32]);

        let dbg = format!("{provider:?}");
        assert!(dbg.contains("primary_version: 2"));
        assert!(!dbg.contains(&k1));
        assert!(!dbg.contains(&k2));
    }

    #[test]
    fn test_malformed_versioned_keys_never_leak_key_material_and_reject_bad_versions() {
        let secret_hex = "deadbeefcafebabe0123456789abcdefdeadbeefcafebabe0123456789abcdef";

        // Missing version prefix: must not echo raw key material in error message
        let err = MasterKeyProvider::parse_versioned_keys(secret_hex).unwrap_err();
        let msg = err.to_string();
        assert!(
            !msg.contains(secret_hex) && !msg.contains("deadbeef"),
            "error leaked key material: {msg}"
        );

        // Non-integer version prefix: must not echo ver_str in error message
        let err = MasterKeyProvider::parse_versioned_keys(&format!("{secret_hex}:{secret_hex}"))
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            !msg.contains(secret_hex) && !msg.contains("deadbeef"),
            "error leaked version prefix: {msg}"
        );

        // Non-positive version rejected
        assert!(MasterKeyProvider::parse_versioned_keys(&format!("0:{secret_hex}")).is_err());
        assert!(MasterKeyProvider::parse_versioned_keys(&format!("-1:{secret_hex}")).is_err());

        // Duplicate version rejected
        assert!(
            MasterKeyProvider::parse_versioned_keys(&format!("1:{secret_hex},1:{secret_hex}"))
                .is_err()
        );

        // All-zero key rejected
        let zero_hex = "0".repeat(64);
        assert!(MasterKeyProvider::parse_key(&zero_hex).is_err());
    }
}
