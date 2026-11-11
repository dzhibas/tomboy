//! Installed applications: one entry type and fuzzy search shared by every OS, with
//! discovery and launching in a backend per OS (`linux.rs`, `macos.rs`).

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as os;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as os;

pub use os::{launch, load_apps};

use std::path::PathBuf;

#[derive(Debug, Clone)]
#[cfg_attr(target_os = "macos", allow(dead_code))] // `id` and `terminal` are read on Linux only
pub struct AppEntry {
    /// Linux: desktop file id ("org.gnome.Nautilus.desktop"). macOS: the bundle's file name.
    pub id: String,
    /// Linux: plain (non-localized) Name=. macOS: the bundle name without `.app`.
    pub name: String,
    pub generic_name: Option<String>,
    pub comment: Option<String>,
    /// Linux: Exec= value (string escapes decoded, Exec quoting left intact).
    /// macOS: the bundle path.
    pub exec: String,
    /// Linux: raw Icon= value. macOS: set when the bundle has an icon to load.
    pub icon: Option<String>,
    pub keywords: Vec<String>,
    pub terminal: bool,
    /// The file Rune reads the icon from: the `.desktop` file (Linux) or `.app` bundle (macOS).
    pub path: PathBuf,
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

fn words(s: &str) -> Vec<&str> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect()
}

fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|n| it.any(|h| h == n))
}

/// Score an app against a lowercased, trimmed, non-empty query.
/// Returns `None` if it does not match at all.
fn score(app: &AppEntry, q: &str) -> Option<u32> {
    let name = app.name.to_lowercase();
    if name == q {
        return Some(1000);
    }
    if name.starts_with(q) {
        return Some(900);
    }

    let name_words = words(&name);
    let q_words: Vec<&str> = q.split_whitespace().collect();
    let initials: String = name_words.iter().filter_map(|w| w.chars().next()).collect();
    let word_prefix = name_words.iter().any(|w| w.starts_with(q))
        || (q.chars().count() >= 2 && initials.starts_with(q))
        || (q_words.len() > 1
            && q_words.iter().all(|qw| name_words.iter().any(|w| w.starts_with(qw))));
    if word_prefix {
        return Some(800);
    }
    if name.contains(q) {
        return Some(700);
    }
    let q_compact: String = q.chars().filter(|c| !c.is_whitespace()).collect();
    // A fuzzy name hit ranks below a word-prefix hit in GenericName/Keywords, so "term"
    // finds terminals before "Telegram".
    let fuzzy = is_subsequence(&q_compact, &name);

    let mut secondary: Vec<String> = Vec::new();
    if let Some(g) = &app.generic_name {
        secondary.push(g.to_lowercase());
    }
    secondary.extend(app.keywords.iter().map(|k| k.to_lowercase()));
    let base = os::program_name(&app.exec);
    if !base.is_empty() {
        secondary.push(base);
    }
    if secondary
        .iter()
        .any(|s| s.starts_with(q) || words(s).iter().any(|w| w.starts_with(q)))
    {
        return Some(650);
    }
    if fuzzy {
        return Some(600);
    }
    if secondary.iter().any(|s| s.contains(q)) {
        return Some(500);
    }

    if let Some(c) = &app.comment
        && c.to_lowercase().contains(q)
    {
        return Some(400);
    }
    None
}

/// Rank `apps` against `query`; returns indices into `apps`, best first,
/// at most `limit` of them. An empty query returns the first `limit` apps.
pub fn search(apps: &[AppEntry], query: &str, limit: usize) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (0..apps.len().min(limit)).collect();
    }
    let mut scored: Vec<(u32, usize, String, usize)> = apps
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            score(a, &q).map(|s| (s, a.name.chars().count(), a.name.to_lowercase(), i))
        })
        .collect();
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
            .then(a.3.cmp(&b.3))
    });
    scored.into_iter().take(limit).map(|(_, _, _, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn app(name: &str, generic: Option<&str>, exec: &str, kw: &[&str], comment: Option<&str>) -> AppEntry {
        AppEntry {
            id: format!("{}.desktop", name.to_lowercase().replace(' ', "-")),
            name: name.into(),
            generic_name: generic.map(Into::into),
            comment: comment.map(Into::into),
            exec: exec.into(),
            icon: None,
            keywords: v(kw),
            terminal: false,
            path: PathBuf::new(),
        }
    }

    fn fixture() -> Vec<AppEntry> {
        let mut apps = vec![
            app("Visual Studio Code", Some("Text Editor"), "/usr/bin/code %F", &["vscode"], None),
            app("Code", None, "code-oss", &[], None),
            app("Codex Reader", None, "codexr", &[], None),
            app("Unicode Table", None, "unitable", &[], None),
            app("Files", Some("File Manager"), "nautilus %U", &["folder"], Some("Access files")),
            app("Firefox", Some("Web Browser"), "firefox %u", &["internet"], None),
            app("Alacritty", Some("Terminal"), "alacritty", &["shell"], None),
            app("Calculator", None, "gnome-calculator", &[], Some("Do arithmetic with code")),
            app("Clocks Of Death", None, "clocks", &[], None),
        ];
        apps.sort_by_key(|a| a.name.to_lowercase());
        apps
    }

    fn names(apps: &[AppEntry], idx: &[usize]) -> Vec<String> {
        idx.iter().map(|&i| apps[i].name.clone()).collect()
    }

    #[test]
    fn search_ranking() {
        let apps = fixture();
        let r = names(&apps, &search(&apps, "code", 10));
        assert_eq!(
            r,
            v(&[
                "Code",               // exact
                "Codex Reader",       // prefix
                "Visual Studio Code", // word start
                "Unicode Table",      // substring
                "Clocks Of Death",    // subsequence
                "Calculator",         // comment only
            ])
        );
        let r = names(&apps, &search(&apps, "vsc", 10));
        assert_eq!(r[0], "Visual Studio Code");
        let r = names(&apps, &search(&apps, "CODE", 2));
        assert_eq!(r, v(&["Code", "Codex Reader"]));
        // subsequence vs secondary fields
        let r = names(&apps, &search(&apps, "term", 10));
        assert_eq!(r, v(&["Alacritty"]));
        let r = names(&apps, &search(&apps, "fi", 10));
        assert_eq!(r[..2], v(&["Files", "Firefox"])[..]);
        let r = names(&apps, &search(&apps, "cod", 10));
        assert_eq!(r[2..4], v(&["Clocks Of Death", "Visual Studio Code"])[..]); // initials/word start
        let r = names(&apps, &search(&apps, "nautilus", 10));
        assert_eq!(r, v(&["Files"]));
        assert!(search(&apps, "zzzzqqq", 10).is_empty());
    }

    #[test]
    fn search_empty_query() {
        let apps = fixture();
        assert_eq!(search(&apps, "", 3), vec![0, 1, 2]);
        assert_eq!(search(&apps, "   ", 100).len(), apps.len());
    }

    #[test]
    fn load_apps_smoke() {
        let start = std::time::Instant::now();
        let apps = load_apps();
        let elapsed = start.elapsed();
        println!("load_apps: {} apps in {:?}", apps.len(), elapsed);
        let mut ids = HashSet::new();
        for a in &apps {
            assert!(ids.insert(a.id.clone()), "duplicate id {}", a.id);
        }
        for q in ["fi", "term"] {
            let top: Vec<String> = search(&apps, q, 5).iter().map(|&i| apps[i].name.clone()).collect();
            println!("search {q:?}: {top:?}");
        }
    }
}
