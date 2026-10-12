use reqwest::Client;
use std::time::Duration;
use url::Url;

use super::ssrf::resolve_safe_target_url;
use crate::domain::bookmarks::html::{
    clean_text, extract_domain, extract_youtube_video_id, parse_html_metadata, ExtractedMetadata,
};

#[derive(serde::Deserialize)]
struct OEmbedResponse {
    title: Option<String>,
    author_name: Option<String>,
    thumbnail_url: Option<String>,
}

async fn fetch_youtube_oembed(
    http: &Client,
    url: &str,
) -> Option<(String, Option<String>, Option<String>)> {
    let resp = http
        .get("https://www.youtube.com/oembed")
        .query(&[("url", url), ("format", "json")])
        .timeout(Duration::from_secs(4))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let data = resp.json::<OEmbedResponse>().await.ok()?;
    let title = clean_text(&data.title?);
    let author = data.author_name.map(|a| clean_text(&a));
    let thumb = data.thumbnail_url;
    Some((title, author, thumb))
}

pub async fn fetch_and_extract_metadata(
    client: Option<&Client>,
    target_url: &str,
    user_title: Option<&str>,
) -> ExtractedMetadata {
    let domain = extract_domain(target_url);
    let yt_id = extract_youtube_video_id(target_url);

    let default_title = user_title
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if let Some(ref id) = yt_id {
                return format!("YouTube Video ({id})");
            }
            if let Ok(parsed) = Url::parse(target_url) {
                let path = parsed.path().trim_matches('/');
                if !path.is_empty() {
                    let segment = path.split('/').next_back().unwrap_or(path);
                    return segment.replace(['-', '_'], " ");
                }
            }
            domain.clone()
        });

    let mut meta = ExtractedMetadata {
        title: default_title,
        description: None,
        author: None,
        image_url: yt_id
            .as_ref()
            .map(|id| format!("https://img.youtube.com/vi/{id}/hqdefault.jpg")),
        domain: domain.clone(),
        reader_content: None,
        reading_time_minutes: 1,
    };

    let Some(http) = client else {
        return meta;
    };

    // For YouTube links, prefer the official oEmbed API for fast, unblocked metadata
    if yt_id.is_some() || domain == "youtube.com" || domain == "youtu.be" {
        if let Some((yt_title, yt_author, yt_thumb)) = fetch_youtube_oembed(http, target_url).await
        {
            if user_title.is_none() || user_title.unwrap().trim().is_empty() {
                meta.title = yt_title;
            }
            if let Some(ref author) = yt_author {
                meta.description = Some(format!("YouTube video by {author}"));
            }
            meta.author = yt_author;
            if yt_thumb.is_some() {
                meta.image_url = yt_thumb;
            }
            return meta;
        }
    }

    // SSRF protection: reject loopback, link-local, private IP ranges, and internal domains,
    // and pin the resolved socket addresses so DNS cannot rebind between check and connect.
    let Some((resolved_host, resolved_addrs)) = resolve_safe_target_url(target_url).await else {
        tracing::warn!("Blocked SSRF request to unsafe target URL: {}", target_url);
        return meta;
    };

    // Attempt HTTP GET with a 5s timeout, pinned DNS resolution, and strict redirect refusal
    let mut builder = reqwest::Client::builder()
        .use_rustls_tls()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5));
    if resolved_host.parse::<std::net::IpAddr>().is_err() && !resolved_host.starts_with('[') {
        builder = builder.resolve_to_addrs(&resolved_host, &resolved_addrs);
    }
    let Ok(fetch_client) = builder.build() else {
        return meta;
    };

    let resp_res = fetch_client
        .get(target_url)
        .header(
            reqwest::header::USER_AGENT,
            "Mozilla/5.0 (compatible; MatrixBot/1.0; +https://matrix.org)",
        )
        .send()
        .await;

    let Ok(mut resp) = resp_res else {
        return meta;
    };

    if !resp.status().is_success() {
        return meta;
    }

    // Limit body read to 2 MB to prevent memory exhaustion / OOM
    let mut bytes = Vec::new();
    while let Ok(Some(chunk)) = resp.chunk().await {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > 2 * 1024 * 1024 {
            break;
        }
    }

    let html = String::from_utf8_lossy(&bytes);
    parse_html_metadata(&html, &mut meta);

    if let Some(title) = user_title {
        if !title.trim().is_empty() {
            meta.title = title.trim().to_string();
        }
    }

    meta
}
