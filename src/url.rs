//! URL detection for the launcher: decides whether typed text looks like
//! something to open in a browser, and opens it in the default browser.

/// TLDs accepted for bare (scheme-less) domains.
const TLDS: &[&str] = &[
    "com", "org", "net", "io", "dev", "rs", "app", "ai", "co", "me", "info", "edu", "gov", "mil",
    "int", "biz", "xyz", "tech", "site", "online", "cloud", "page", "sh", "gg", "tv", "fm", "ly",
    "to", "so", "is", "it", "uk", "de", "fr", "lt", "lv", "ee", "eu", "us", "ca", "au", "nz", "jp",
    "cn", "kr", "in", "ru", "ua", "pl", "nl", "be", "ch", "at", "se", "no", "dk", "fi", "es", "pt",
    "ie", "cz", "br", "mx", "ar", "za", "news", "blog", "wiki", "zone", "codes", "tools",
];

/// Return a normalized URL to open if `input` looks like one.
pub fn detect(input: &str) -> Option<String> {
    let t = input.trim();
    if t.is_empty() || t.chars().any(char::is_whitespace) {
        return None;
    }
    let lower = t.to_ascii_lowercase();

    for scheme in ["http://", "https://"] {
        if let Some(rest) = lower.strip_prefix(scheme) {
            let authority = authority_of(rest);
            let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
            let host = host_port.split(':').next().unwrap_or("");
            return (!host.is_empty()).then(|| t.to_string());
        }
    }
    for scheme in ["file://", "ftp://", "mailto:"] {
        if let Some(rest) = lower.strip_prefix(scheme) {
            return (!rest.is_empty()).then(|| t.to_string());
        }
    }

    let authority = authority_of(t);
    if authority.contains('@') {
        return None;
    }
    let (host, port) = match authority.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (authority, None),
    };
    if let Some(p) = port
        && !valid_port(p)
    {
        return None;
    }
    let host_l = host.to_ascii_lowercase();
    if host_l == "localhost" || is_ipv4(host) {
        return Some(format!("http://{t}"));
    }
    if is_bare_domain(&host_l) {
        return Some(format!("https://{t}"));
    }
    None
}

/// The part before the first `/`, `?` or `#`.
fn authority_of(s: &str) -> &str {
    s.split(['/', '?', '#']).next().unwrap_or("")
}

fn valid_port(p: &str) -> bool {
    !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()) && p.parse::<u32>().is_ok_and(|n| n <= 65535)
}

fn is_ipv4(host: &str) -> bool {
    let parts: Vec<&str> = host.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|p| {
            !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()) && p.parse::<u16>().is_ok_and(|n| n <= 255)
        })
}

fn is_bare_domain(host: &str) -> bool {
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    let valid_label = |l: &&str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    if !labels.iter().all(valid_label) {
        return false;
    }
    let tld = labels[labels.len() - 1];
    tld.len() >= 2 && tld.bytes().all(|b| b.is_ascii_alphabetic()) && TLDS.contains(&tld)
}

/// Open `url` in the default browser (`xdg-open` / `open`, see `sys`); does not wait.
pub fn open(url: &str) -> std::io::Result<()> {
    crate::sys::open_url(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Option<String> {
        detect(s)
    }

    #[test]
    fn schemes_kept_as_is() {
        assert_eq!(d("https://example.com"), Some("https://example.com".into()));
        assert_eq!(d("  http://foo.bar/baz?q=1  "), Some("http://foo.bar/baz?q=1".into()));
        assert_eq!(d("HTTPS://Example.com"), Some("HTTPS://Example.com".into()));
        assert_eq!(d("http://localhost:8080"), Some("http://localhost:8080".into()));
        assert_eq!(d("https://intranet"), Some("https://intranet".into()));
        assert_eq!(d("file:///home/user/a.txt"), Some("file:///home/user/a.txt".into()));
        assert_eq!(d("ftp://ftp.example.org/pub"), Some("ftp://ftp.example.org/pub".into()));
        assert_eq!(d("mailto:me@example.com"), Some("mailto:me@example.com".into()));
    }

    #[test]
    fn schemes_invalid() {
        assert_eq!(d("http://"), None);
        assert_eq!(d("https:///path"), None);
        assert_eq!(d("https://:80"), None);
        assert_eq!(d("mailto:"), None);
        assert_eq!(d("https://exa mple.com"), None);
    }

    #[test]
    fn localhost_and_ip() {
        assert_eq!(d("localhost"), Some("http://localhost".into()));
        assert_eq!(d("localhost:3000/path"), Some("http://localhost:3000/path".into()));
        assert_eq!(d("192.168.1.1:8080"), Some("http://192.168.1.1:8080".into()));
        assert_eq!(d("10.0.0.1"), Some("http://10.0.0.1".into()));
        assert_eq!(d("256.1.1.1"), None);
        assert_eq!(d("1.2.3"), None);
        assert_eq!(d("localhost:abc"), None);
        assert_eq!(d("localhost:99999"), None);
    }

    #[test]
    fn bare_domains() {
        assert_eq!(d("github.com"), Some("https://github.com".into()));
        assert_eq!(d("www.rust-lang.org/learn"), Some("https://www.rust-lang.org/learn".into()));
        assert_eq!(d("docs.rs/rune?x=1#y"), Some("https://docs.rs/rune?x=1#y".into()));
        assert_eq!(d("news.ycombinator.com"), Some("https://news.ycombinator.com".into()));
        assert_eq!(d("example.lt:8443/x"), Some("https://example.lt:8443/x".into()));
        assert_eq!(d("bbc.co.uk"), Some("https://bbc.co.uk".into()));
    }

    #[test]
    fn bare_domains_rejected() {
        for s in [
            "2.5", "1+1", "2+2", "hello world", "firefox", "term", "", "   ", "foo.txt",
            "notes.md", "-bad.com", "bad-.com", "a..com", "github.c", "github.com.", ".com",
            "foo.123", "under_score.com", "user@github.com", "example.com:", "example.notatld",
        ] {
            assert_eq!(d(s), None, "{s:?}");
        }
    }
}
