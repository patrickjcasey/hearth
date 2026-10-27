//! Checks on links that come from outside: imported pages, library files and
//! exports. The webview hands any `href` to the system, and the app fetches
//! photo links on every launch, so only web links are kept and only public
//! ones are fetched.

/// An `http://` or `https://` link. Anything else (`file:`, `javascript:`,
/// `data:`, …) must not reach an `href` or `src`.
pub fn is_web_url(u: &str) -> bool {
    let lower = u.trim().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// A web link to a named public host: not an IP address, not localhost, not
/// a `.local` name. The device should never fetch anything else on its own,
/// since that would let a recipe file probe the user's LAN.
pub fn is_public_web_url(u: &str) -> bool {
    is_web_url(u) && host_of(u).as_deref().is_some_and(is_public_host)
}

/// The host part of a web link, lower-cased, without userinfo or port.
fn host_of(u: &str) -> Option<String> {
    let rest = u.trim().split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    // `[::1]:80` keeps its brackets so it is still seen as an IP literal.
    let host = match host.strip_prefix('[') {
        Some(_) => host,
        None => host.rsplit_once(':').map_or(host, |(h, _)| h),
    };
    Some(host.trim_end_matches('.').to_ascii_lowercase())
}

fn is_public_host(host: &str) -> bool {
    if host.is_empty() || host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return false;
    }
    if host.starts_with('[') || host.parse::<std::net::IpAddr>().is_ok() {
        return false;
    }
    // "http://2130706433/" and "http://0x7f.1/" are 127.0.0.1 to a URL
    // parser: a last label that is a number means the host is an address.
    let last = host.rsplit('.').next().unwrap_or(host);
    let numeric = last.strip_prefix("0x").map_or(last.bytes().all(|b| b.is_ascii_digit()), |hex| {
        hex.bytes().all(|b| b.is_ascii_hexdigit())
    });
    !numeric
}

/// Whether a server address baked into a build is acceptable: `https://`,
/// or plain http to this machine for local testing. Unset is fine. Usable
/// in a `const` assertion over an `option_env!` value.
pub const fn is_build_endpoint(url: Option<&str>) -> bool {
    match url {
        None => true,
        Some(u) => starts_with(u, "https://") || starts_with(u, "http://127.0.0.1") || starts_with(u, "http://localhost"),
    }
}

const fn starts_with(s: &str, prefix: &str) -> bool {
    let (s, prefix) = (s.as_bytes(), prefix.as_bytes());
    if s.len() < prefix.len() {
        return false;
    }
    let mut i = 0;
    while i < prefix.len() {
        if s[i] != prefix[i] {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_urls() {
        for ok in ["https://example.com/x", "HTTP://example.com", "  https://a.b "] {
            assert!(is_web_url(ok), "{ok}");
        }
        for bad in ["file:///etc/hosts", "javascript:alert(1)", "data:text/html,hi", "ftp://x", "example.com", ""] {
            assert!(!is_web_url(bad), "{bad}");
        }
    }

    #[test]
    fn public_web_urls() {
        for ok in [
            "https://example.com/mac.jpg",
            "http://images.example.co.uk:8080/a?b=c",
            "https://user:pw@example.com/",
            "https://example.com./x",
        ] {
            assert!(is_public_web_url(ok), "{ok}");
        }
        for bad in [
            "file:///etc/hosts",
            "javascript:alert(1)",
            "http://127.0.0.1:8000/x.jpg",
            "http://10.0.0.5/x.jpg",
            "http://[::1]/x.jpg",
            "http://[fe80::1]:80/x",
            "http://localhost/x",
            "http://LOCALHOST:3000/x",
            "http://app.localhost/x",
            "http://printer.local/x",
            "http://nas.LOCAL/x",
            "http://2130706433/",
            "http://0x7f000001/",
            "http://0x7f.1/",
            "http://",
            "http:///path",
            "http://user@/x",
            "http://@127.0.0.1/x",
        ] {
            assert!(!is_public_web_url(bad), "{bad}");
        }
    }

    #[test]
    fn build_endpoints() {
        // Usable in `const` context, as the app's compile-time checks need.
        const _: () = assert!(is_build_endpoint(Some("https://hestia.example.com/api/share")));
        const _: () = assert!(is_build_endpoint(Some("http://127.0.0.1:8000/api/events")));
        assert!(is_build_endpoint(None));
        assert!(is_build_endpoint(Some("http://localhost:8000/api/events")));
        for bad in ["http://hestia.example.com/api/share", "ftp://x", "", "https:/"] {
            assert!(!is_build_endpoint(Some(bad)), "{bad}");
        }
    }
}
