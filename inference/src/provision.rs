use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::config::InferenceConfig;

/// The first retry delay after a provisioning attempt fails.
const FIRST_BACKOFF: Duration = Duration::from_secs(30);
/// The longest a retry ever waits, so a long outage is still retried every
/// few minutes rather than being backed off into silence.
const MAX_BACKOFF: Duration = Duration::from_secs(600);

/// The wait before the next provisioning attempt: doubling from the first
/// delay, capped so a long outage is still retried every few minutes.
fn next_backoff(current: Duration) -> Duration {
    (current * 2).min(MAX_BACKOFF)
}

/// Retries `attempt` until it succeeds, with doubling backoff between
/// failures. Never gives up: a capability that cannot provision stays
/// reporting unavailable via its route's `models_present` check, but a
/// transient failure (a flaky download, a registry hiccup) recovers on its
/// own instead of requiring someone to notice and restart the container.
async fn retry_until_ready<F, Fut, E>(label: &str, mut attempt: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<(), E>>,
    E: std::fmt::Display,
{
    let mut delay = FIRST_BACKOFF;
    let mut attempt_number: u32 = 1;

    loop {
        match attempt().await {
            Ok(()) => {
                info!(
                    "{} provisioning succeeded on attempt {}",
                    label, attempt_number
                );
                return;
            }
            Err(err) => {
                warn!(
                    "{} provisioning attempt {} failed, retrying in {:?}: {}",
                    label, attempt_number, delay, err
                );
                tokio::time::sleep(delay).await;
                delay = next_backoff(delay);
                attempt_number += 1;
            }
        }
    }
}

/// Downloads any model a capability is missing, retrying each capability
/// independently and indefinitely with doubling backoff until it succeeds.
///
/// Spawned beside the listener rather than awaited in front of it, so a fresh
/// deployment answers `/health` at once instead of reading unhealthy for the
/// minutes a download takes. Each route gates on its own `models_present` and
/// answers 503 until its files land; downloads write to a temporary file and
/// rename, so a route never sees a partial model. OCR, STT and TTS provision
/// concurrently, so one retrying forever never holds up the others.
pub async fn provision_models(config: Arc<InferenceConfig>) {
    let ocr_config = Arc::clone(&config);
    let ocr = async move {
        retry_until_ready("OCR", || {
            crate::ocr::ensure_models(&ocr_config.ocr_model_dir)
        })
        .await;
    };

    let stt_config = Arc::clone(&config);
    let stt = async move {
        retry_until_ready("Speech-to-text", || {
            crate::stt::ensure_models_downloaded(&stt_config.voice_model_dir)
        })
        .await;
    };

    let tts_config = Arc::clone(&config);
    let tts = async move {
        retry_until_ready("Text-to-speech", || {
            crate::tts::ensure_model_downloaded(&tts_config.tts_model_dir)
        })
        .await;
    };

    tokio::join!(ocr, stt, tts);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A directory already holding every model file, so provisioning has
    /// nothing to download. Tests must never reach the network.
    fn provisioned_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("provision-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir should be creatable");
        for name in [
            crate::ocr::DET_MODEL_NAME,
            crate::ocr::CLS_MODEL_NAME,
            crate::ocr::REC_MODEL_NAME,
            crate::stt::SENSEVOICE_MODEL_FILE,
            crate::stt::SENSEVOICE_TOKENS_FILE,
        ] {
            std::fs::write(dir.join(name), b"x").expect("model stub should be writable");
        }
        for name in crate::tts::KOKORO_REQUIRED_FILES {
            std::fs::write(dir.join(name), b"x").expect("model stub should be writable");
        }
        dir
    }

    #[tokio::test]
    async fn test_provisioning_is_a_no_op_when_every_model_is_already_present() {
        let dir = provisioned_dir("noop");
        let config = Arc::new(InferenceConfig {
            port: 0,
            ocr_model_dir: dir.clone(),
            voice_model_dir: dir.clone(),
            tts_model_dir: dir.clone(),
        });

        tokio::time::timeout(Duration::from_secs(5), provision_models(config))
            .await
            .expect("provisioning with every file present must finish without downloading");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_next_backoff_doubles_and_caps_at_ten_minutes() {
        assert_eq!(
            next_backoff(Duration::from_secs(30)),
            Duration::from_secs(60)
        );
        assert_eq!(
            next_backoff(Duration::from_secs(400)),
            Duration::from_secs(600)
        );
        assert_eq!(
            next_backoff(Duration::from_secs(600)),
            Duration::from_secs(600)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_retry_until_ready_retries_after_a_failure_then_succeeds() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let calls = Arc::new(AtomicU32::new(0));
        let calls_clone = Arc::clone(&calls);

        let attempt = move || {
            let calls = calls_clone.clone();
            async move {
                let n = calls.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    Err("transient failure".to_string())
                } else {
                    Ok(())
                }
            }
        };

        // Paused tokio time makes the real 30s backoff sleep resolve
        // instantly instead of making the test slow.
        tokio::time::timeout(Duration::from_secs(60), retry_until_ready("test", attempt))
            .await
            .expect("retry should finish once the second attempt succeeds");

        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
