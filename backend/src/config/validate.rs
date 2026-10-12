//! Boot-time validation: refuses to start on a configuration that cannot work.

use anyhow::Result;

use super::{channels, is_production, Config, MAX_JOB_WORKERS};
use crate::domain::channels::Channel;

impl Config {
    pub fn validate(&self) -> Result<()> {
        if self.homeserver.is_empty() {
            anyhow::bail!("MATRIX_HOMESERVER cannot be empty");
        }
        if self.username.is_empty() {
            anyhow::bail!("MATRIX_USERNAME cannot be empty");
        }
        if self.password.is_empty() {
            anyhow::bail!("MATRIX_PASSWORD cannot be empty");
        }

        if !(1..=MAX_JOB_WORKERS).contains(&self.job_workers) {
            anyhow::bail!(
                "JOB_WORKERS must be between 1 and {MAX_JOB_WORKERS} (got {})",
                self.job_workers
            );
        }

        if self.job_workers >= self.db_max_connections as usize {
            anyhow::bail!(
                "JOB_WORKERS ({}) must be strictly less than DATABASE_MAX_CONNECTIONS ({}) to prevent pool starvation",
                self.job_workers,
                self.db_max_connections
            );
        }

        let is_prod = is_production();

        // A production deployment still on the development Relying Party would
        // mint passkeys bound to `localhost`, which no real browser would ever
        // send back to it — every registration would appear to succeed and
        // every subsequent login would fail. Refuse to boot instead.
        if is_prod
            && (self.webauthn_rp_id == "localhost" || self.webauthn_origin.contains("localhost"))
        {
            anyhow::bail!(
                "WEBAUTHN_RP_ID/WEBAUTHN_ORIGIN are still the localhost defaults in production. \
                 Set them to the real host — they are baked into every passkey and cannot be \
                 changed later without invalidating all of them."
            );
        }

        if self.webauthn_rp_id.contains("://") || self.webauthn_rp_id.contains(':') {
            anyhow::bail!(
                "WEBAUTHN_RP_ID must be a bare hostname with no scheme or port (got '{}')",
                self.webauthn_rp_id
            );
        }

        if !self.webauthn_origin.contains("://") {
            anyhow::bail!(
                "WEBAUTHN_ORIGIN must be a full origin including the scheme (got '{}')",
                self.webauthn_origin
            );
        }

        let allowed = |channel| {
            self.link_signup_allowlist
                .get(&channel)
                .map_or(&[][..], |v| v)
        };
        channels::validate_discord_allowlist(allowed(Channel::Discord))?;
        channels::validate_telegram(
            self.telegram.as_ref(),
            &self.web_url,
            allowed(Channel::Telegram),
        )?;
        if let Some(cfg) = &self.email {
            channels::validate_email(cfg)?;
        }
        channels::validate_email_allowlist(allowed(Channel::Email))?;

        let has_encryption_key = self
            .secrets_encryption_key
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
            || self
                .secrets_encryption_keys
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty());
        if has_encryption_key || is_prod {
            crate::infrastructure::crypto::aes_gcm::MasterKeyProvider::from_key_strings(
                self.secrets_encryption_key.as_deref(),
                self.secrets_encryption_keys.as_deref(),
            )
            .map_err(|e| anyhow::anyhow!("Invalid secret encryption configuration: {e}"))?;
        }

        if is_prod
            && (self.storage.signing_secret.trim().is_empty()
                || self.storage.signing_secret == super::storage::DEFAULT_SIGNING_SECRET)
        {
            anyhow::bail!(
                "STORAGE_SIGNING_SECRET is still the default or empty in production. \
                 Set a unique secret to sign storage download URLs."
            );
        }

        Ok(())
    }
}
