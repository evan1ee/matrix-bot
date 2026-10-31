//! Voice notes from channels that deliver them as files (Telegram, Discord).
//!
//! The adapter detects the audio, downloads it within bounds and validates it
//! into an [`AudioPayload`]; everything after that is channel-neutral and lives
//! here: identity, rate limit, then a saved note or an AI turn, exactly as a
//! Matrix voice note is handled.

use std::time::Duration;

use tracing::warn;

use super::inbound::{Inbound, UNLINKED_REPLY};
use super::message::Reply;
use crate::application::identity::Resolution;
use crate::config::AiEnvConfig;
use crate::domain::channels::audio::AudioPayload;
use crate::domain::channels::{ConversationAddress, PlatformIdentity};
use crate::infrastructure::inference::stt::SttClient;
use crate::interface::chat::voice::VoiceConfig;
use crate::interface::http::rate_limit::InMemoryRateLimiter;
use crate::llm::progress::ProgressSink;

/// Transcriptions one account may start per [`RATE_WINDOW`], to protect the
/// inference server.
pub const AUDIO_PER_WINDOW: u32 = 5;
pub const RATE_WINDOW: Duration = Duration::from_secs(60);

const RATE_LIMITED_REPLY: &str = "⚠️ Too many voice messages. Please wait a minute and try again.";
const UNAVAILABLE_REPLY: &str = "⚠️ Voice messages are not available on this deployment.";
const RESOLVE_FAILED_REPLY: &str = "⚠️ Could not verify your account. Please try again later.";

/// What a voice note needs beyond the application services.
pub struct VoiceRuntime {
    pub config: VoiceConfig,
    pub stt: SttClient,
    pub ai: AiEnvConfig,
    pub limiter: InMemoryRateLimiter,
}

impl VoiceRuntime {
    pub fn new(config: VoiceConfig, stt: SttClient, ai: AiEnvConfig) -> Self {
        Self {
            config,
            stt,
            ai,
            limiter: InMemoryRateLimiter::new(AUDIO_PER_WINDOW, RATE_WINDOW),
        }
    }
}

/// A caption that asks for the transcript to be saved as a note, spelled as
/// on Matrix.
pub fn is_note_caption(caption: &str) -> bool {
    let caption = caption.trim();
    caption.starts_with("/note") || caption.starts_with("!note")
}

/// Transcribes `payload` for `sender` and answers it. Never panics and never
/// returns an error: every failure is a reply the sender can act on.
pub async fn ingest_audio(
    inbound: &Inbound,
    sender: &PlatformIdentity,
    conversation: &ConversationAddress,
    payload: AudioPayload,
    caption: Option<&str>,
    progress: Option<&dyn ProgressSink>,
) -> Reply {
    let services = inbound.services();
    let owner = match services.identity.resolve(sender).await {
        Ok(Resolution::Account(owner)) => owner,
        Ok(Resolution::Unlinked) => return Reply::html(UNLINKED_REPLY),
        Err(err) => {
            warn!(error = %err, "could not resolve the sender of a voice message");
            return Reply::html(RESOLVE_FAILED_REPLY);
        }
    };
    let Some(voice) = inbound.voice() else {
        return Reply::html(UNAVAILABLE_REPLY);
    };
    if !voice.limiter.check(&owner.to_string()) {
        return Reply::html(RATE_LIMITED_REPLY);
    }

    let storage = conversation.to_storage();
    let mime = Some(payload.mime().as_mime());
    let caption = caption.unwrap_or("").trim();
    if is_note_caption(caption) {
        let channel = conversation.channel().map_or("web", |c| c.as_str());
        let html = crate::interface::chat::knowledge::capture_voice_note(
            services,
            channel,
            &storage,
            &owner,
            caption,
            payload.bytes(),
            mime,
            &voice.config,
            &voice.stt,
            progress,
        )
        .await;
        let mut reply = Reply::html(html);
        reply.owner_id = Some(owner);
        return reply;
    }

    let media = crate::interface::chat::voice::process_audio(
        inbound.db(),
        services,
        &voice.ai,
        &storage,
        &owner,
        payload.bytes(),
        mime,
        payload
            .duration_secs()
            .map(|secs| Duration::from_secs(secs.into())),
        &voice.config,
        &voice.stt,
        progress,
    )
    .await;
    let mut reply = Reply::html(media.html);
    reply.is_ai = true;
    reply.owner_id = Some(owner);
    reply
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::application::AppServices;
    use crate::db::{Database, DbPool};
    use crate::domain::accounts::{AccountId, STATUS_ACTIVE};
    use crate::domain::channels::{Channel, PlatformUserId};
    use crate::infrastructure::inference::stt::stub::{stub_stt, CallCount};
    use crate::infrastructure::postgres::repositories::accounts::core;
    use crate::infrastructure::postgres::repositories::identities;
    use crate::interface::channels::fake::FakeAdapter;
    use crate::interface::channels::registry::ChannelRegistry;
    use crate::interface::chat::ModuleRegistry;

    struct Fixture {
        inbound: Inbound,
        pool: DbPool,
        calls: CallCount,
        _registry: Arc<ChannelRegistry>,
    }

    async fn fixture(text: &'static str, with_voice: bool) -> Fixture {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let registry = Arc::new(
            ChannelRegistry::new(vec![Arc::new(FakeAdapter::connected(Channel::Telegram))])
                .expect("one adapter per channel"),
        );
        let embedder =
            Arc::new(crate::infrastructure::inference::embedding::MockEmbeddingProvider::new(384));
        let mut inbound = Inbound::new(
            AppServices::new(&pool).with_embedder(embedder),
            Database { pool: pool.clone() },
            Arc::new(ModuleRegistry::new()),
            Arc::downgrade(&registry),
        );
        let (stt, calls) = stub_stt(text).await;
        if with_voice {
            inbound = inbound.with_voice(VoiceRuntime::new(
                VoiceConfig::default(),
                stt,
                AiEnvConfig::default(),
            ));
        }
        Fixture {
            inbound,
            pool,
            calls,
            _registry: registry,
        }
    }

    fn tg(raw: &str) -> PlatformIdentity {
        let id = PlatformUserId::parse(Channel::Telegram, raw).unwrap();
        PlatformIdentity::new(Channel::Telegram, id, None)
    }

    async fn linked(pool: &DbPool, raw: &str) -> (AccountId, PlatformIdentity) {
        let account = core::create(pool, "a", STATUS_ACTIVE).await.unwrap();
        let identity = tg(raw);
        identities::bind(pool, &account, &identity).await.unwrap();
        (account, identity)
    }

    fn clip() -> AudioPayload {
        AudioPayload::new(vec![1, 2, 3], Some("audio/ogg"), None, Some(3)).unwrap()
    }

    async fn ingest(f: &Fixture, who: &PlatformIdentity, caption: Option<&str>) -> Reply {
        ingest_audio(
            &f.inbound,
            who,
            &ConversationAddress::Telegram(42),
            clip(),
            caption,
            None,
        )
        .await
    }

    #[test]
    fn note_captions_are_spelled_as_on_matrix() {
        assert!(is_note_caption("/note"));
        assert!(is_note_caption("  !note shopping"));
        assert!(!is_note_caption("please note this"));
        assert!(!is_note_caption(""));
    }

    #[tokio::test]
    async fn unlinked_sender_gets_the_link_prompt_and_stt_is_not_called() {
        let f = fixture("hello", true).await;
        let reply = ingest(&f, &tg("555"), None).await;
        assert!(
            reply.html_body().contains("not linked"),
            "{}",
            reply.html_body()
        );
        assert_eq!(f.calls.get(), 0);
    }

    #[tokio::test]
    async fn note_caption_creates_a_note_for_the_owner() {
        let f = fixture("remember the milk", true).await;
        let (owner, who) = linked(&f.pool, "7").await;
        let reply = ingest(&f, &who, Some("/note")).await;
        assert!(
            reply.html_body().contains("Voice Note"),
            "{}",
            reply.html_body()
        );
        assert_eq!(f.calls.get(), 1);
        let notes = f
            .inbound
            .services()
            .notes
            .list(&owner, None, None, false, 10)
            .await
            .unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].source_channel, "telegram");
    }

    #[tokio::test]
    async fn a_clip_without_a_caption_reaches_stt() {
        // Silent audio stops before the AI turn, so no model is contacted.
        let f = fixture("", true).await;
        let (_, who) = linked(&f.pool, "8").await;
        let reply = ingest(&f, &who, None).await;
        assert_eq!(f.calls.get(), 1);
        assert!(reply.is_ai);
    }

    #[tokio::test]
    async fn sixth_clip_is_refused_without_calling_stt() {
        let f = fixture("", true).await;
        let (_, who) = linked(&f.pool, "9").await;
        for _ in 0..AUDIO_PER_WINDOW {
            ingest(&f, &who, None).await;
        }
        assert_eq!(f.calls.get(), AUDIO_PER_WINDOW as usize);
        let sixth = ingest(&f, &who, None).await;
        assert!(
            sixth.html_body().contains("Too many"),
            "{}",
            sixth.html_body()
        );
        assert_eq!(f.calls.get(), AUDIO_PER_WINDOW as usize);

        let (_, other) = linked(&f.pool, "10").await;
        ingest(&f, &other, None).await;
        assert_eq!(f.calls.get(), AUDIO_PER_WINDOW as usize + 1);
    }

    #[tokio::test]
    async fn unconfigured_stt_replies_instead_of_panicking() {
        let mut f = fixture("", false).await;
        f.inbound = f.inbound.with_voice(VoiceRuntime::new(
            VoiceConfig::default(),
            SttClient::new(None, None, "none".into()),
            AiEnvConfig::default(),
        ));
        let (_, who) = linked(&f.pool, "11").await;
        let reply = ingest(&f, &who, None).await;
        assert!(
            reply.html_body().contains("语音处理不可用"),
            "{}",
            reply.html_body()
        );
    }

    #[tokio::test]
    async fn a_pipeline_without_voice_says_so() {
        let f = fixture("", false).await;
        let (_, who) = linked(&f.pool, "12").await;
        let reply = ingest(&f, &who, None).await;
        assert!(
            reply.html_body().contains("not available"),
            "{}",
            reply.html_body()
        );
    }
}
