//! Quicklinks: a keyword that opens a URL pattern with what you type filled in, from
//! `[quicklinks]` in the config. `jira ABC-1` opens `https://issues.jira.com/browse/ABC-1`
//! for `jira = "issues.jira.com/browse/{a}"`; Enter on plain `jira` asks for the argument.
//! A pattern with no `{…}` is a bookmark and opens as it is.

use crate::config::Config;

#[derive(Debug, Clone, PartialEq)]
pub struct Quicklink {
    /// Lowercase, one word.
    pub keyword: String,
    /// "Jira issue"; the keyword, capitalised, when the config doesn't name it.
    pub name: String,
    pub url: String,
    /// The placeholder's name (`a`, `query`); `None` for a bookmark.
    pub arg: Option<String>,
}

impl Quicklink {
    pub fn new(keyword: &str, url: &str, name: Option<&str>) -> Quicklink {
        let keyword = keyword.trim().to_lowercase();
        let name = name.map(str::trim).filter(|n| !n.is_empty()).map(str::to_string)
            .unwrap_or_else(|| capitalise(&keyword));
        let url = url.trim().to_string();
        let arg = placeholders(&url).next().map(|(_, n)| n.to_string());
        Quicklink { keyword, name, url, arg }
    }

    /// The URL with every placeholder replaced by `arg` (percent-encoded), and `https://`
    /// added when the pattern has no scheme.
    pub fn expand(&self, arg: &str) -> String {
        let mut out = String::new();
        let mut rest = self.url.as_str();
        while let Some((start, name)) = placeholders(rest).next() {
            out.push_str(&rest[..start]);
            out.push_str(&encode(arg.trim()));
            rest = &rest[start + name.len() + 2..];
        }
        out.push_str(rest);
        if has_scheme(&out) { out } else { format!("https://{out}") }
    }

    /// What the field asks for in argument mode: "Query…", "Issue…", or "Argument…" for a
    /// one-letter placeholder.
    pub fn prompt(&self) -> String {
        match self.arg.as_deref() {
            Some(a) if a.chars().count() > 1 => format!("{}…", capitalise(&a.replace('_', " "))),
            _ => "Argument…".into(),
        }
    }

    /// Opens something other than a web page (`obsidian://`, `file://`)?
    pub fn is_web(&self) -> bool {
        let u = self.expand("").to_ascii_lowercase();
        u.starts_with("http://") || u.starts_with("https://")
    }
}

/// The quicklinks in the config, minus the ones `Config::warning` complains about.
pub fn load(config: &Config) -> Vec<Quicklink> {
    config.quicklinks.iter()
        .filter(|(k, l)| !k.trim().is_empty() && !k.trim().contains(char::is_whitespace) && !l.url().trim().is_empty())
        .map(|(k, l)| Quicklink::new(k, l.url(), l.name()))
        .collect()
}

/// A hit for the launcher query: the link, and the argument typed after its keyword.
#[derive(Debug, PartialEq)]
pub struct Hit<'a> {
    pub link: usize,
    pub arg: Option<&'a str>,
    /// The query starts with the whole keyword (`jira`, `jira ABC-1`), not just a prefix.
    pub exact: bool,
}

/// Links matching `query`: `jira ABC-1` → jira with "ABC-1"; `jira` → jira exactly;
/// `ji` (2+ letters) → jira, or any link whose name has a word starting with it.
pub fn matches<'a>(links: &[Quicklink], query: &'a str) -> Vec<Hit<'a>> {
    let q = query.trim_start();
    let (first, rest) = match q.split_once(char::is_whitespace) {
        Some((f, r)) => (f, Some(r.trim()).filter(|r| !r.is_empty())),
        None => (q.trim_end(), None),
    };
    let first = first.to_lowercase();
    if first.is_empty() { return Vec::new(); }
    let mut hits = Vec::new();
    for (i, l) in links.iter().enumerate() {
        if l.keyword == first {
            hits.push(Hit { link: i, arg: if l.arg.is_some() { rest } else { None }, exact: true });
        } else if rest.is_none() && first.chars().count() >= 2 {
            let name = l.name.to_lowercase();
            if l.keyword.starts_with(&first) || name.split_whitespace().any(|w| w.starts_with(&first)) {
                hits.push(Hit { link: i, arg: None, exact: false });
            }
        }
    }
    hits
}

/// `{name}` placeholders in `url`: (byte offset of `{`, name).
fn placeholders(url: &str) -> impl Iterator<Item = (usize, &str)> {
    url.match_indices('{').filter_map(move |(i, _)| {
        let tail = &url[i + 1..];
        let end = tail.find('}')?;
        let name = &tail[..end];
        (!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')).then_some((i, name))
    })
}

/// `https://…`, `obsidian://…`, `mailto:…` have one; `issues.jira.com/…` and
/// `localhost:3000/…` (a port, not a scheme) don't.
fn has_scheme(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once(':') else { return false };
    let valid = scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        && !scheme.contains('/');
    valid && (rest.starts_with("//") || !rest.starts_with(|c: char| c.is_ascii_digit()))
}

/// Percent-encode everything but unreserved characters; a space is `%20` (it may be a path).
fn encode(s: &str) -> String {
    s.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
        _ => format!("%{b:02X}"),
    }).collect()
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links() -> Vec<Quicklink> {
        vec![
            Quicklink::new("gh", "https://github.com/search?q={query}", None),
            Quicklink::new("Jira", "issues.jira.com/browse/{a}", Some("Jira issue")),
            Quicklink::new("news", "https://news.ycombinator.com", None),
        ]
    }

    #[test]
    fn builds() {
        let l = links();
        assert_eq!(l[0].name, "Gh");
        assert_eq!(l[0].arg.as_deref(), Some("query"));
        assert_eq!(l[1].keyword, "jira");
        assert_eq!(l[1].arg.as_deref(), Some("a"));
        assert_eq!(l[2].arg, None);
        assert_eq!(l[0].prompt(), "Query…");
        assert_eq!(l[1].prompt(), "Argument…");
    }

    #[test]
    fn loads_from_config() {
        let config: Config = toml::from_str("[quicklinks]\ngh = \"https://github.com/search?q={query}\"\n\
            Jira = { url = \"issues.jira.com/browse/{issue}\", name = \"Jira issue\" }\n\"bad key\" = \"x\"\nempty = \" \"\n").unwrap();
        let l = load(&config);
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].keyword.as_str(), l[0].name.as_str()), ("jira", "Jira issue"));
        assert_eq!(l[0].prompt(), "Issue…");
        assert_eq!(l[1].keyword, "gh");
    }

    #[test]
    fn expands() {
        let l = links();
        assert_eq!(l[0].expand("rune ui & more/x"), "https://github.com/search?q=rune%20ui%20%26%20more%2Fx");
        assert_eq!(l[1].expand(" ABC-1 "), "https://issues.jira.com/browse/ABC-1");
        assert_eq!(l[2].expand("ignored"), "https://news.ycombinator.com");
        let two = Quicklink::new("t", "x.com/{a}/{a}?{not a placeholder}", None);
        assert_eq!(two.expand("y"), "https://x.com/y/y?{not a placeholder}");
        assert_eq!(Quicklink::new("o", "obsidian://open?vault={v}", None).expand("Notes"), "obsidian://open?vault=Notes");
        assert_eq!(Quicklink::new("l", "localhost:3000/{p}", None).expand("a"), "https://localhost:3000/a");
        assert_eq!(Quicklink::new("m", "mailto:{to}", None).expand("me"), "mailto:me");
        assert!(l[0].is_web() && !Quicklink::new("o", "obsidian://x", None).is_web());
    }

    #[test]
    fn matches_queries() {
        let l = links();
        assert_eq!(matches(&l, "jira ABC-1"), vec![Hit { link: 1, arg: Some("ABC-1"), exact: true }]);
        assert_eq!(matches(&l, "JIRA"), vec![Hit { link: 1, arg: None, exact: true }]);
        assert_eq!(matches(&l, "jira  "), vec![Hit { link: 1, arg: None, exact: true }]);
        assert_eq!(matches(&l, "ji"), vec![Hit { link: 1, arg: None, exact: false }]);
        assert_eq!(matches(&l, "issue"), vec![Hit { link: 1, arg: None, exact: false }]);
        assert!(matches(&l, "j").is_empty());
        assert!(matches(&l, "ji abc").is_empty());
        // A bookmark takes no argument.
        assert_eq!(matches(&l, "news today"), vec![Hit { link: 2, arg: None, exact: true }]);
        assert!(matches(&l, "  ").is_empty());
    }
}
