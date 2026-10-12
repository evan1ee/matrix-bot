use std::net::{IpAddr, SocketAddr};
use url::Url;

pub use crate::domain::llm::is_disallowed_ip;

/// Validates that a target URL is an acceptable public HTTP/HTTPS URL and resolves its host
/// to a non-empty list of vetted public `SocketAddr`s so callers can pin the connection
/// with `reqwest::ClientBuilder::resolve_to_addrs` and prevent DNS rebinding between check and connect.
pub async fn resolve_safe_target_url(raw_url: &str) -> Option<(String, Vec<SocketAddr>)> {
    let parsed = Url::parse(raw_url).ok()?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return None;
    }
    let host_str = parsed.host_str()?;
    let host_lower = host_str.to_lowercase();
    if host_lower == "localhost"
        || host_lower.ends_with(".localhost")
        || host_lower.ends_with(".local")
        || host_lower.ends_with(".internal")
        || host_lower == "localtest.me"
        || host_lower.ends_with(".localtest.me")
        || host_lower == "lvh.me"
        || host_lower.ends_with(".lvh.me")
    {
        return None;
    }

    let port = parsed.port_or_known_default().unwrap_or(80);

    match parsed.host()? {
        url::Host::Ipv4(v4) => {
            let ip = IpAddr::V4(v4);
            if is_disallowed_ip(ip) {
                return None;
            }
            Some((host_lower, vec![SocketAddr::new(ip, port)]))
        }
        url::Host::Ipv6(v6) => {
            let ip = IpAddr::V6(v6);
            if is_disallowed_ip(ip) {
                return None;
            }
            Some((host_lower, vec![SocketAddr::new(ip, port)]))
        }
        url::Host::Domain(domain) => {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((domain, port))
                .await
                .ok()?
                .collect();
            if addrs.is_empty() || addrs.iter().any(|a| is_disallowed_ip(a.ip())) {
                return None;
            }
            Some((domain.to_lowercase(), addrs))
        }
    }
}

/// Validates that a target URL is an acceptable public HTTP/HTTPS URL and does not resolve to private IPs.
pub async fn is_safe_target_url(raw_url: &str) -> bool {
    resolve_safe_target_url(raw_url).await.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_is_disallowed_ip() {
        assert!(is_disallowed_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(is_disallowed_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(is_disallowed_ip(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));
        assert!(is_disallowed_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        assert!(is_disallowed_ip(IpAddr::V4(Ipv4Addr::new(
            169, 254, 169, 254
        ))));
        assert!(is_disallowed_ip(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))));
        assert!(is_disallowed_ip(IpAddr::V4(Ipv4Addr::new(
            255, 255, 255, 255
        ))));
        assert!(!is_disallowed_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
        assert!(!is_disallowed_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }

    #[tokio::test]
    async fn test_is_safe_target_url() {
        assert!(!is_safe_target_url("http://127.0.0.1:8080/secret").await);
        assert!(!is_safe_target_url("http://localhost:8080").await);
        assert!(!is_safe_target_url("http://service.internal/status").await);
        assert!(!is_safe_target_url("http://169.254.169.254/latest/meta-data").await);
        assert!(!is_safe_target_url("ftp://example.com").await);
    }
}
