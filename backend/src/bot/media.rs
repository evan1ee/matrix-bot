//! Authenticated Matrix Media Ingestion
//!
//! Downloads media files (images, audio, etc.) from the Matrix homeserver.
//! Supports Matrix 1.11 / MSC3916 Authenticated Media and transparently decrypts
//! encrypted attachments (AES-CTR) in encrypted rooms.

use anyhow::{anyhow, Context, Result};
use matrix_sdk::{
    crypto::AttachmentDecryptor, ruma::events::room::MediaSource, ruma::MxcUri, Client,
};
use std::io::{Cursor, Read};
use std::time::Duration;
use tracing::{debug, warn};

/// Downloads a media file (plain or encrypted) using authenticated endpoints when available.
pub async fn download_media_file(client: &Client, source: &MediaSource) -> Result<Vec<u8>> {
    match source {
        MediaSource::Plain(uri) => download_raw_media(client, uri).await,
        MediaSource::Encrypted(file) => {
            let ciphertext = download_raw_media(client, &file.url).await?;
            let mut cursor = Cursor::new(ciphertext);
            let mut decryptor = AttachmentDecryptor::new(&mut cursor, file.as_ref().clone().into())
                .map_err(|e| anyhow!("Failed to initialize attachment decryptor: {:?}", e))?;

            let mut decrypted = Vec::new();
            decryptor
                .read_to_end(&mut decrypted)
                .context("Failed to decrypt attachment ciphertext")?;
            Ok(decrypted)
        }
    }
}

/// Downloads raw media bytes from an MXC URI using authenticated endpoints.
async fn download_raw_media(client: &Client, uri: &MxcUri) -> Result<Vec<u8>> {
    let (server_name, media_id) = uri
        .parts()
        .map_err(|e| anyhow!("Invalid MXC URI '{}': {:?}", uri, e))?;

    let homeserver = client.homeserver();
    let base_url = homeserver.as_str().trim_end_matches('/');

    let token = client
        .access_token()
        .or_else(|| client.session().map(|s| s.access_token().to_string()))
        .or_else(|| {
            std::fs::read_to_string("data/matrix_token.json")
                .ok()
                .map(|s| s.trim().trim_matches('"').to_string())
        });

    let http = reqwest::Client::builder()
        .use_rustls_tls()
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    // Candidates in priority order:
    // 1. Matrix 1.11 / MSC3916 client endpoint
    // 2. MSC3916 legacy / Synapse media v3 endpoint
    let candidates = [
        format!(
            "{}/_matrix/client/v1/media/download/{}/{}",
            base_url, server_name, media_id
        ),
        format!(
            "{}/_matrix/media/v3/download/{}/{}",
            base_url, server_name, media_id
        ),
    ];

    let mut last_error = None;

    for url in &candidates {
        let mut req = http.get(url);
        if let Some(ref tok) = token {
            req = req.bearer_auth(tok);
        }

        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    debug!("Successfully downloaded media from {}", url);
                    return resp
                        .bytes()
                        .await
                        .map(|b| b.to_vec())
                        .context("Failed to read media response body bytes");
                } else {
                    let err_msg = format!("HTTP {} from {}", status, url);
                    debug!("{}", err_msg);
                    last_error = Some(anyhow!(err_msg));
                }
            }
            Err(e) => {
                debug!("Request to {} failed: {:?}", url, e);
                last_error = Some(anyhow!("Request to {} failed: {:?}", url, e));
            }
        }
    }

    // Fallback to matrix-sdk unauthenticated media download as last resort
    match client
        .media()
        .get_media_content(
            &matrix_sdk::media::MediaRequest {
                source: MediaSource::Plain(uri.to_owned()),
                format: matrix_sdk::media::MediaFormat::File,
            },
            true,
        )
        .await
    {
        Ok(bytes) => Ok(bytes),
        Err(e) => {
            warn!(
                "Fallback get_media_content also failed for {}: {:?}",
                uri, e
            );
            Err(last_error.unwrap_or_else(|| anyhow!("Failed to download media: {:?}", e)))
        }
    }
}
