//! Cryptographic infrastructure for secret credential protection.
//!
//! Provides AES-256-GCM authenticated encryption at rest with 96-bit CSPRNG nonces,
//! AAD slot binding, and master key management.

pub mod aes_gcm;

pub use aes_gcm::{
    AesGcmSecretProtector, CryptoError, FailClosedSecretProtector, MasterKeyProvider,
    SecretProtector,
};
