use std::net::{IpAddr, Ipv4Addr};
use url::Url;

/// Evaluates whether an IP belongs to private, loopback, link-local, or non-routable ranges.
pub fn is_disallowed_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback() // 127.0.0.0/8
            || v4.is_unspecified() // 0.0.0.0
            || v4.is_private() // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
            || v4.is_link_local() // 169.254.0.0/16
            || v4.is_broadcast() // 255.255.255.255
            || v4.is_documentation()
            || (v4.octets()[0] == 100 && (v4.octets()[1] & 0b1100_0000 == 0b0100_0000))
            // 100.64.0.0/10 CGNAT
        }
        IpAddr::V6(v6) => {
            let octets = v6.octets();
            v6.is_loopback() // ::1
            || v6.is_unspecified() // ::
            || (v6.segments()[0] & 0xfe00 == 0xfc00) // Unique local fc00::/7
            || (v6.segments()[0] & 0xffc0 == 0xfe80) // Link local fe80::/10
            // Check for IPv4-mapped IPv6 (::ffff:x.x.x.x)
            || if octets[..10] == [0; 10] && octets[10] == 0xff && octets[11] == 0xff {
                let v4 = Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]);
                is_disallowed_ip(IpAddr::V4(v4))
            } else {
                false
            }
        }
    }
}

/// Validates that a target URL is an acceptable public HTTP/HTTPS URL and does not resolve to private IPs.
pub async fn is_safe_target_url(raw_url: &str) -> bool {
    let Ok(parsed) = Url::parse(raw_url) else {
        return false;
    };
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return false;
    }
    let Some(host_str) = parsed.host_str() else {
        return false;
    };
    let host_lower = host_str.to_lowercase();
    if host_lower == "localhost"
        || host_lower.ends_with(".localhost")
        || host_lower.ends_with(".local")
        || host_lower.ends_with(".internal")
    {
        return false;
    }

    if let Ok(ip) = host_lower.parse::<IpAddr>() {
        return !is_disallowed_ip(ip);
    }

    let port = parsed.port_or_known_default().unwrap_or(80);
    match tokio::net::lookup_host(format!("{host_lower}:{port}")).await {
        Ok(addrs) => {
            let mut count = 0;
            for addr in addrs {
                count += 1;
                if is_disallowed_ip(addr.ip()) {
                    return false;
                }
            }
            count > 0
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
