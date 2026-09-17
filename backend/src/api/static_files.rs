use axum::{
    body::Body,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

/// Embedded static web assets compiled from the React SPA (`backend/web/dist`).
#[derive(RustEmbed)]
#[folder = "web/dist/"]
pub struct WebAssets;

/// Serves static frontend assets with disk fallback, embedded fallback, and SPA client-side routing.
pub async fn static_handler(uri: Uri) -> Response {
    let raw_path = uri.path().trim_start_matches('/');
    let path = if raw_path.is_empty() {
        "index.html"
    } else {
        raw_path
    };

    // 1. Try serving from local disk first if present (supports frontend dev and custom overrides)
    if let Some(disk_path) = find_disk_file(path) {
        if let Ok(bytes) = tokio::fs::read(&disk_path).await {
            let content_type = get_content_type(path);
            let cache_control = get_cache_control(path);
            return Response::builder()
                .header(header::CONTENT_TYPE, content_type)
                .header(header::CACHE_CONTROL, cache_control)
                .body(Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    }

    // 2. Try serving from embedded static assets in the binary
    if let Some(content) = WebAssets::get(path) {
        let content_type = get_content_type(path);
        let cache_control = get_cache_control(path);
        return Response::builder()
            .header(header::CONTENT_TYPE, content_type)
            .header(header::CACHE_CONTROL, cache_control)
            .body(Body::from(content.data))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }

    // 3. SPA routing fallback:
    // If the path does not have a file extension (e.g. /todos, /finance, /reminders, /settings),
    // return index.html with 200 OK so client-side React Router handles the route.
    if !path.contains('.') {
        if let Some(index_disk) = find_disk_file("index.html") {
            if let Ok(bytes) = tokio::fs::read(&index_disk).await {
                return Response::builder()
                    .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                    .header(header::CACHE_CONTROL, "no-cache")
                    .body(Body::from(bytes))
                    .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
            }
        }

        if let Some(content) = WebAssets::get("index.html") {
            return Response::builder()
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CACHE_CONTROL, "no-cache")
                .body(Body::from(content.data))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    }

    // Asset missing (e.g., missing /assets/*.js or /nonexistent.png)
    (StatusCode::NOT_FOUND, "404 Not Found").into_response()
}

fn find_disk_file(path: &str) -> Option<std::path::PathBuf> {
    for base in &["web/dist", "backend/web/dist"] {
        let p = std::path::Path::new(base).join(path);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn get_content_type(path: &str) -> String {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    if mime == "text/html" {
        "text/html; charset=utf-8".to_string()
    } else {
        mime.to_string()
    }
}

fn get_cache_control(path: &str) -> &'static str {
    if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_static_handler_root() {
        let uri: Uri = "/".parse().unwrap();
        let res = static_handler(uri).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()
                .get(header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap(),
            "text/html; charset=utf-8"
        );
    }

    #[tokio::test]
    async fn test_static_handler_spa_fallback() {
        let uri: Uri = "/reminders/history".parse().unwrap();
        let res = static_handler(uri).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()
                .get(header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap(),
            "text/html; charset=utf-8"
        );
    }

    #[tokio::test]
    async fn test_static_handler_missing_file_404() {
        let uri: Uri = "/assets/non_existent_file.js".parse().unwrap();
        let res = static_handler(uri).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}
