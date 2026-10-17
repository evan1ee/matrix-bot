//! The `SearchService` and its configuration. Querying lives in
//! `retrieval.rs`; embedding items and the backfill live in `indexing.rs`.

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;

use crate::domain::knowledge::search::DEFAULT_EMBEDDING_MODEL;
use crate::infrastructure::inference::embedding::EmbeddingProvider;

/// How long a query waits for the embedder before search degrades to keywords.
const DEFAULT_EMBED_TIMEOUT: Duration = Duration::from_secs(3);

type EmbedderCache = Arc<std::sync::RwLock<Option<(String, String, Arc<dyn EmbeddingProvider>)>>>;

/// Service executing cross-domain hybrid semantic and keyword searches and
/// keeping the embeddings behind them current.
#[derive(Clone)]
pub struct SearchService {
    pub(super) pool: PgPool,
    pub(super) embedder: Arc<dyn EmbeddingProvider>,
    /// The model new chunks are tagged with and the only one whose chunks take
    /// part in vector search. Changing it re-embeds everything via the backfill.
    pub(super) model: String,
    pub(super) embed_timeout: Duration,
    pub(super) app_config: Option<crate::application::config::AppConfigService>,
    pub(super) custom_embedder_cache: EmbedderCache,
}

impl SearchService {
    pub fn new(pool: PgPool, embedder: Arc<dyn EmbeddingProvider>) -> Self {
        Self {
            pool,
            embedder,
            model: DEFAULT_EMBEDDING_MODEL.to_string(),
            embed_timeout: DEFAULT_EMBED_TIMEOUT,
            app_config: None,
            custom_embedder_cache: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    pub fn with_embedder(&self, embedder: Arc<dyn EmbeddingProvider>) -> Self {
        Self {
            embedder,
            ..self.clone()
        }
    }

    /// Sets the configured embedding model.
    pub fn with_model(&self, model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            ..self.clone()
        }
    }

    /// Attaches the dynamic runtime configuration service for `inference.embedding.base_url`.
    pub fn with_app_config(
        &self,
        app_config: crate::application::config::AppConfigService,
    ) -> Self {
        Self {
            app_config: Some(app_config),
            ..self.clone()
        }
    }

    /// Resolves the active embedding provider, applying a hot-reloaded
    /// `inference.embedding.base_url` override when customized in `app_settings`.
    pub(super) fn active_embedder(&self) -> Arc<dyn EmbeddingProvider> {
        if let Some(cfg) = &self.app_config {
            let snap = cfg.snapshot();
            if snap.is_customized(crate::domain::config::SettingKey::InferenceEmbeddingBaseUrl) {
                let url = snap.inference_embedding_base_url().trim();
                if let Ok(cache) = self.custom_embedder_cache.read() {
                    if let Some((cached_url, cached_model, client)) = cache.as_ref() {
                        if cached_url == url && cached_model == &self.model {
                            return client.clone();
                        }
                    }
                }
                if let Ok(mut cache) = self.custom_embedder_cache.write() {
                    if let Some((cached_url, cached_model, client)) = cache.as_ref() {
                        if cached_url == url && cached_model == &self.model {
                            return client.clone();
                        }
                    }
                    let client: Arc<dyn EmbeddingProvider> = Arc::new(
                        crate::infrastructure::inference::embedding::LocalOnnxEmbeddingClient::new(
                            (!url.is_empty()).then(|| url.to_string()),
                            Some(self.model.clone()),
                        ),
                    );
                    *cache = Some((url.to_string(), self.model.clone(), client.clone()));
                    return client;
                }
            }
        }
        self.embedder.clone()
    }

    /// Overrides the embed timeout; tests use it to exercise the slow path.
    #[cfg(test)]
    pub(super) fn with_embed_timeout(&self, embed_timeout: Duration) -> Self {
        Self {
            embed_timeout,
            ..self.clone()
        }
    }
}

impl Default for SearchService {
    fn default() -> Self {
        panic!("SearchService must be initialized with a PgPool");
    }
}
