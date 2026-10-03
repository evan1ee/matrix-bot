//! IMAP client and polling runtime.
//! Connects via IMAPS (port 993 default), searches `UNSEEN`, fetches RFC822 payloads
//! (capped at 20), marks `\Seen`, and optionally moves to a processed folder.

use std::sync::Arc;
use std::time::Duration;

use futures_util::TryStreamExt;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;
use tracing::{debug, info, warn};

use crate::config::EmailConfig;

/// Maximum number of messages fetched in a single poll batch.
pub const IMAP_BATCH_LIMIT: usize = 20;

/// Connect and command timeout.
pub const IMAP_TIMEOUT: Duration = Duration::from_secs(30);

/// Builds a comma-separated IMAP sequence set from a slice of sequence numbers.
/// Sorts them and takes at most `limit` items.
pub fn build_seq_set(seqs: &[u32], limit: usize) -> Option<String> {
    if seqs.is_empty() {
        return None;
    }
    let mut sorted = seqs.to_vec();
    sorted.sort_unstable();
    let set: Vec<String> = sorted
        .into_iter()
        .take(limit)
        .map(|n| n.to_string())
        .collect();
    Some(set.join(","))
}

/// Builds a `TlsConnector` using Mozilla's root certificates via `webpki-roots`
/// and the `ring` crypto provider.
pub fn build_tls_connector() -> anyhow::Result<TlsConnector> {
    let mut root_store = RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| anyhow::anyhow!("failed to set TLS protocol versions: {e}"))?
        .with_root_certificates(root_store)
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

pub type ImapTlsStream = tokio_rustls::client::TlsStream<tokio::net::TcpStream>;

/// Connects to an IMAPS server and reads the initial server greeting.
pub async fn connect_imaps(
    host: &str,
    port: u16,
) -> anyhow::Result<async_imap::Client<ImapTlsStream>> {
    let connector = build_tls_connector()?;
    let server_name = ServerName::try_from(host.to_owned())
        .map_err(|e| anyhow::anyhow!("invalid DNS hostname `{host}`: {e}"))?;

    let tcp_stream = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::net::TcpStream::connect((host, port)),
    )
    .await
    .map_err(|_| anyhow::anyhow!("connection timeout to {host}:{port}"))??;

    let tls_stream = tokio::time::timeout(
        Duration::from_secs(15),
        connector.connect(server_name, tcp_stream),
    )
    .await
    .map_err(|_| anyhow::anyhow!("TLS handshake timeout with {host}:{port}"))??;

    let mut client = async_imap::Client::new(tls_stream);

    // Read the server greeting banner
    tokio::time::timeout(Duration::from_secs(15), client.read_response())
        .await
        .map_err(|_| anyhow::anyhow!("timeout waiting for IMAP greeting from {host}:{port}"))?
        .map_err(|e| anyhow::anyhow!("failed to read IMAP server greeting: {e}"))?;

    Ok(client)
}

/// Poller that logs into IMAP, selects INBOX, and fetches unseen messages.
#[derive(Clone)]
pub struct ImapPoller {
    config: EmailConfig,
}

impl ImapPoller {
    pub fn new(config: EmailConfig) -> Self {
        Self { config }
    }

    /// Connects to the configured IMAP server, fetches at most 20 UNSEEN messages,
    /// marks them as \Seen, and optionally copies to processed folder.
    pub async fn fetch_unseen_raw(&self) -> anyhow::Result<Vec<Vec<u8>>> {
        let host = self
            .config
            .imap_url
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("EMAIL_IMAP_URL must specify a host"))?;
        let port = self.config.imap_url.port().unwrap_or(993);

        debug!(host, port, "connecting to IMAP server");
        let client = connect_imaps(host, port).await?;

        let mut session = client
            .login(&self.config.username, &self.config.password)
            .await
            .map_err(|(e, _)| anyhow::anyhow!("IMAP login failed: {e}"))?;

        session
            .select("INBOX")
            .await
            .map_err(|e| anyhow::anyhow!("failed to select INBOX: {e}"))?;

        let unseen_set = session
            .search("UNSEEN")
            .await
            .map_err(|e| anyhow::anyhow!("IMAP search UNSEEN failed: {e}"))?;

        let seqs: Vec<u32> = unseen_set.into_iter().collect();
        let Some(seq_set) = build_seq_set(&seqs, IMAP_BATCH_LIMIT) else {
            let _ = session.logout().await;
            return Ok(Vec::new());
        };

        info!(messages = %seq_set, "fetching unseen IMAP messages");

        let messages_stream = session
            .fetch(&seq_set, "RFC822")
            .await
            .map_err(|e| anyhow::anyhow!("IMAP fetch failed: {e}"))?;

        let fetch_results: Vec<async_imap::types::Fetch> = messages_stream
            .try_collect()
            .await
            .map_err(|e| anyhow::anyhow!("failed reading fetched messages stream: {e}"))?;

        let raw_messages: Vec<Vec<u8>> = fetch_results
            .iter()
            .filter_map(|m| m.body().map(|b| b.to_vec()))
            .collect();

        // Mark as \Seen
        match session.store(&seq_set, "+FLAGS (\\Seen)").await {
            Ok(stream) => {
                futures_util::pin_mut!(stream);
                while let Ok(Some(_)) = stream.try_next().await {}
            }
            Err(e) => {
                warn!(error = %e, "failed to mark messages as \\Seen in IMAP");
            }
        }

        // Optionally move to processed folder
        if let Some(folder) = &self.config.processed_folder {
            if let Err(e) = session.copy(&seq_set, folder).await {
                warn!(folder, error = %e, "failed to copy messages to processed folder");
            } else {
                match session.store(&seq_set, "+FLAGS (\\Deleted)").await {
                    Ok(stream) => {
                        futures_util::pin_mut!(stream);
                        while let Ok(Some(_)) = stream.try_next().await {}
                    }
                    Err(e) => {
                        warn!(error = %e, "failed to mark messages as \\Deleted");
                    }
                }
                match session.expunge().await {
                    Ok(stream) => {
                        futures_util::pin_mut!(stream);
                        while let Ok(Some(_)) = stream.try_next().await {}
                    }
                    Err(e) => {
                        warn!(error = %e, "failed to expunge deleted messages");
                    }
                }
            }
        }

        let _ = session.logout().await;
        Ok(raw_messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_seq_set_empty() {
        assert_eq!(build_seq_set(&[], 20), None);
    }

    #[test]
    fn test_build_seq_set_sorted_and_capped() {
        let seqs = [5, 1, 10, 3, 2];
        assert_eq!(build_seq_set(&seqs, 3), Some("1,2,3".to_owned()));
        assert_eq!(build_seq_set(&seqs, 20), Some("1,2,3,5,10".to_owned()));
    }

    #[test]
    fn test_tls_connector_builds_cleanly() {
        assert!(build_tls_connector().is_ok());
    }
}
