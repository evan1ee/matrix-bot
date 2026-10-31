//! The recording that accompanies a scheduled morning briefing. Every step
//! is fail-open: a missing TTS service or a storage hiccup costs the audio,
//! never the text briefing or the job.

use bytes::Bytes;
use chrono::NaiveDate;
use tracing::warn;

use super::agent::audio::compile_audio_digest;
use crate::application::AppServices;
use crate::domain::accounts::AccountId;
use crate::domain::storage::{StorageCategory, StorageKey};
use crate::interface::channels::message::Attachment;

const FILENAME: &str = "morning-briefing.ogg";

/// Synthesizes `script`, keeps the recording and records it for `local_date`.
/// Returns the attachment for the chat message whenever synthesis worked, even
/// if keeping the recording did not.
pub(super) async fn prepare(
    services: &AppServices,
    owner: &AccountId,
    local_date: NaiveDate,
    script: &str,
) -> Option<Attachment> {
    let mut audio = compile_audio_digest(services, owner, script, None).await?;
    audio.filename = FILENAME.to_string();
    store(services, owner, local_date, &audio).await;
    Some(audio)
}

async fn store(
    services: &AppServices,
    owner: &AccountId,
    local_date: NaiveDate,
    audio: &Attachment,
) {
    let key = StorageKey::generate(owner, StorageCategory::Audio, "ogg");
    if let Err(err) = services
        .storage
        .put(&key, Bytes::from(audio.bytes.clone()), &audio.mime)
        .await
    {
        warn!(%owner, error = %err, "Briefing audio not stored");
        return;
    }
    match services
        .briefing
        .record_audio(owner, local_date, key.as_str())
        .await
    {
        Ok(replaced) => {
            if let Some(old) = replaced {
                delete_object(services, &old).await;
            }
        }
        Err(err) => {
            warn!(%owner, error = %err, "Briefing audio not recorded");
            // Nothing points at the object, so nothing would ever delete it.
            if let Err(err) = services.storage.delete(&key).await {
                warn!(%owner, error = %err, "Unrecorded briefing audio not deleted");
            }
        }
    }
}

/// Drops the owner's recordings past the retention window.
pub(super) async fn prune(services: &AppServices, owner: &AccountId, today: NaiveDate) {
    match services.briefing.prune_audio(owner, today).await {
        Ok(keys) => {
            for key in keys {
                delete_object(services, &key).await;
            }
        }
        Err(err) => warn!(%owner, error = %err, "Briefing audio retention failed"),
    }
}

async fn delete_object(services: &AppServices, key: &str) {
    let Ok(key) = StorageKey::parse(key) else {
        warn!(key, "Stored briefing audio key is not a storage key");
        return;
    };
    if let Err(err) = services.storage.delete(&key).await {
        warn!(key = key.as_str(), error = %err, "Briefing audio object not deleted");
    }
}
