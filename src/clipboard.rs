//! Clipboard history: the list `tomboy clipboard` (or "Clipboard History" in the launcher)
//! shows, and the watcher that records it where no one else does.
//!
//! On Omarchy the shell already records every copy in
//! `~/.local/state/omarchy/clipboard-history.json` (`wl-paste --watch …/capture.sh`), so tomboy
//! reads that. Elsewhere `tomboy clipboard-watch` (run it at login) keeps tomboy's own history
//! in `~/.local/state/tomboy/clipboard-history.json`, in the same format: a JSON array, newest
//! first, of `{"type":"text","text":…}` and `{"type":"image","mime":…,"path":…}`.

use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::platform::Platform;
use crate::sys;

/// Entries the watcher keeps (Omarchy's default too).
const LIMIT: usize = 100;

#[derive(Clone, Debug, PartialEq)]
pub enum Clip {
    Text(String),
    /// `captured` is when it was copied, as Omarchy writes it ("Monday 14:30").
    Image { mime: String, path: PathBuf, captured: Option<String> },
}

impl Clip {
    fn from_json(v: &serde_json::Value) -> Option<Clip> {
        match v.get("type")?.as_str()? {
            "text" => {
                let t = v.get("text")?.as_str()?;
                (!t.is_empty()).then(|| Clip::Text(t.to_string()))
            }
            "image" => Some(Clip::Image {
                mime: v.get("mime").and_then(|m| m.as_str()).unwrap_or("image/png").to_string(),
                path: PathBuf::from(v.get("path")?.as_str()?),
                captured: v.get("capturedAt").and_then(|c| c.as_str()).map(str::to_string),
            }),
            _ => None,
        }
    }

    fn to_json(&self) -> serde_json::Value {
        match self {
            Clip::Text(t) => serde_json::json!({ "type": "text", "text": t }),
            Clip::Image { mime, path, captured } =>
                serde_json::json!({ "type": "image", "mime": mime, "path": path, "capturedAt": captured }),
        }
    }

    /// The row title: the first non-blank line, squeezed; an image's file name.
    pub fn title(&self) -> String {
        match self {
            Clip::Text(t) => {
                if let [path] = file_paths(t).as_slice() { return file_name(path); }
                let n = file_paths(t).len();
                if n > 1 { return format!("{n} files"); }
                let line = t.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("(whitespace)");
                let squeezed = line.split_whitespace().collect::<Vec<_>>().join(" ");
                truncate(&squeezed, 120)
            }
            Clip::Image { captured: Some(at), .. } => format!("Image · {at}"),
            Clip::Image { .. } => "Image".into(),
        }
    }

    /// "Text · 3 lines · 120 characters", "Image · PNG", "File".
    pub fn kind(&self) -> String {
        match self {
            Clip::Text(t) if !file_paths(t).is_empty() => {
                let n = file_paths(t).len();
                if n == 1 { "File".into() } else { format!("{n} Files") }
            }
            Clip::Text(t) => {
                let lines = t.lines().count().max(1);
                let chars = t.chars().count();
                let plural = |n: usize, w: &str| if n == 1 { format!("1 {w}") } else { format!("{n} {w}s") };
                if lines > 1 { format!("Text · {} · {}", plural(lines, "line"), plural(chars, "character")) }
                else { format!("Text · {}", plural(chars, "character")) }
            }
            Clip::Image { mime, .. } => format!("Image · {}", mime.trim_start_matches("image/").to_uppercase()),
        }
    }

    /// Is this a link? (One line that looks like a URL.)
    pub fn is_link(&self) -> bool {
        matches!(self, Clip::Text(t) if !t.trim().contains(char::is_whitespace)
            && (t.trim().starts_with("http://") || t.trim().starts_with("https://")))
    }

    /// The path to preview as a picture: an image clip, or one copied image file.
    pub fn image_path(&self) -> Option<PathBuf> {
        match self {
            Clip::Image { path, .. } => Some(path.clone()),
            Clip::Text(t) => match file_paths(t).as_slice() {
                [p] if is_image_path(p) => Some(p.clone()),
                _ => None,
            },
        }
    }

    fn haystack(&self) -> String {
        match self {
            Clip::Text(t) => t.to_lowercase(),
            Clip::Image { mime, captured, .. } =>
                format!("image {mime} {}", captured.as_deref().unwrap_or("")).to_lowercase(),
        }
    }

    /// Put this clip back on the clipboard. Copied files go back as `text/uri-list`, so a
    /// file manager pastes the files; images go back as their own type.
    pub fn copy(&self) -> io::Result<bool> {
        match self {
            Clip::Text(t) if !file_paths(t).is_empty() => sys::copy_bytes(t.as_bytes(), Some("text/uri-list")),
            Clip::Text(t) => sys::copy_to_clipboard(t),
            Clip::Image { mime, path, .. } => sys::copy_bytes(&std::fs::read(path)?, Some(mime)),
        }
    }
}

/// `file:///home/me/a.png` lines of a copied-files clip (every non-blank line must be one).
fn file_paths(text: &str) -> Vec<PathBuf> {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let paths: Vec<PathBuf> = lines.iter().filter_map(|l| {
        let rest = l.strip_prefix("file://")?;
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        rest.starts_with('/').then(|| PathBuf::from(percent_decode(rest)))
    }).collect();
    if !lines.is_empty() && paths.len() == lines.len() { paths } else { Vec::new() }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_image_path(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp")
    })
}

fn file_name(p: &Path) -> String {
    p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// Text for the preview pane, cut short so a huge copy doesn't stall layout.
pub fn preview_text(text: &str) -> String {
    const MAX: usize = 20_000;
    truncate(text, MAX)
}

/// Omarchy's history file on Omarchy, tomboy's own elsewhere.
pub fn history_path(platform: &Platform) -> Option<PathBuf> {
    let dir = sys::state_dir()?;
    Some(match platform {
        Platform::Omarchy { .. } => dir.join("omarchy/clipboard-history.json"),
        Platform::Linux | Platform::MacOS => own_history_path(&dir),
    })
}

fn own_history_path(state: &Path) -> PathBuf {
    state.join("tomboy/clipboard-history.json")
}

/// The history, newest first; empty when there is none yet.
pub fn load(path: &Path) -> Vec<Clip> {
    let Ok(raw) = std::fs::read_to_string(path) else { return Vec::new() };
    parse(&raw)
}

fn parse(raw: &str) -> Vec<Clip> {
    let Ok(serde_json::Value::Array(rows)) = serde_json::from_str(raw) else { return Vec::new() };
    rows.iter().filter_map(Clip::from_json).collect()
}

/// Indices of the clips matching every word of `query` (case-insensitive), newest first.
pub fn search(clips: &[Clip], query: &str) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    clips.iter().enumerate()
        .filter(|(_, c)| { let h = c.haystack(); words.iter().all(|w| h.contains(w.as_str())) })
        .map(|(i, _)| i)
        .collect()
}

/// Does the launcher query ask for the clipboard history? (`clip`, `clipboard`, `paste`, `history`.)
pub fn matches_command(query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.chars().count() >= 3 && ["clipboard history", "paste history"].iter().any(|name| {
        name.starts_with(q.as_str()) || name.split(' ').any(|w| w.starts_with(q.as_str()))
    })
}

/// `clip` on top of `history`, dropping an older copy of the same clip, keeping `LIMIT`.
fn push(history: Vec<Clip>, clip: Clip) -> Vec<Clip> {
    let same = |a: &Clip, b: &Clip| match (a, b) {
        (Clip::Image { path: p, .. }, Clip::Image { path: q, .. }) => p == q,
        _ => a == b,
    };
    let rest: Vec<Clip> = history.into_iter().filter(|c| !same(c, &clip)).collect();
    std::iter::once(clip).chain(rest).take(LIMIT).collect()
}

fn save(path: &Path, history: &[Clip]) -> io::Result<()> {
    let json = serde_json::Value::Array(history.iter().map(Clip::to_json).collect());
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&json)? + "\n")?;
    std::fs::rename(tmp, path)
}

// ---- recording (`tomboy clipboard-watch`) ----

/// Record copies into tomboy's history until killed. On Linux this runs `wl-paste --watch`,
/// which calls `tomboy clipboard-capture <mime>` with each copy on stdin; on macOS it polls
/// `pbpaste`.
pub fn watch() -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return Err(io::Error::other("clipboard-watch needs a Wayland session (wl-paste)"));
        }
        let exe = std::env::current_exe()?;
        let mut kids = Vec::new();
        for mime in ["text", "image/png"] {
            kids.push(std::process::Command::new("wl-paste")
                .args(["--type", mime, "--watch"]).arg(&exe).args(["clipboard-capture", mime])
                .spawn()?);
        }
        for mut k in kids { k.wait()?; }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        let mut last = None;
        loop {
            let out = std::process::Command::new("pbpaste").output()?;
            if out.status.success() && !out.stdout.is_empty() && last.as_ref() != Some(&out.stdout) {
                if last.is_some() { record(&out.stdout, "text")?; }
                last = Some(out.stdout);
            }
            std::thread::sleep(std::time::Duration::from_millis(700));
        }
    }
}

/// One copy, on stdin, from `wl-paste --watch`.
pub fn capture(mime: &str) -> io::Result<()> {
    // wl-paste marks copies from password managers.
    if std::env::var("CLIPBOARD_STATE").is_ok_and(|s| s == "sensitive") { return Ok(()); }
    let mut bytes = Vec::new();
    io::stdin().read_to_end(&mut bytes)?;
    record(&bytes, mime)
}

fn record(bytes: &[u8], mime: &str) -> io::Result<()> {
    if bytes.is_empty() { return Ok(()); }
    let state = sys::state_dir().ok_or_else(|| io::Error::other("no $HOME"))?;
    let path = own_history_path(&state);
    let dir = path.parent().expect("history has a parent");
    std::fs::create_dir_all(dir)?;
    let clip = if mime.starts_with("image/") {
        use sha2::Digest;
        let hash: String = sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
        let ext = match mime.trim_start_matches("image/") { "jpeg" => "jpg", e => e };
        let images = dir.join("clipboard-images");
        std::fs::create_dir_all(&images)?;
        let file = images.join(format!("{hash}.{ext}"));
        if !file.exists() { std::fs::write(&file, bytes)?; }
        let captured = jiff::Zoned::now().strftime("%A %H:%M").to_string();
        Clip::Image { mime: mime.to_string(), path: file, captured: Some(captured) }
    } else {
        let text = String::from_utf8_lossy(bytes).into_owned();
        if text.trim().is_empty() { return Ok(()); }
        Clip::Text(text)
    };
    save(&path, &push(load(&path), clip))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_omarchy_history() {
        let clips = parse(r#"[
            {"type":"text","text":"hello\nworld"},
            {"type":"image","mime":"image/png","path":"/tmp/a.png","capturedAt":"Monday 10:00"},
            {"type":"text","text":""},
            {"type":"weird"}
        ]"#);
        assert_eq!(clips, vec![
            Clip::Text("hello\nworld".into()),
            Clip::Image { mime: "image/png".into(), path: "/tmp/a.png".into(), captured: Some("Monday 10:00".into()) },
        ]);
        assert_eq!(clips[1].title(), "Image · Monday 10:00");
        assert!(parse("not json").is_empty());
    }

    #[test]
    fn titles_and_kinds() {
        let t = Clip::Text("\n   first   line  \nsecond".into());
        assert_eq!(t.title(), "first line");
        assert_eq!(t.kind(), "Text · 3 lines · 25 characters");
        assert_eq!(Clip::Text("x".into()).kind(), "Text · 1 character");
        let f = Clip::Text("file:///home/me/My%20Shot.png\n".into());
        assert_eq!(f.title(), "My Shot.png");
        assert_eq!(f.kind(), "File");
        assert_eq!(f.image_path(), Some(PathBuf::from("/home/me/My Shot.png")));
        assert_eq!(Clip::Text("file:///a\nfile:///b".into()).title(), "2 files");
        assert!(Clip::Text("https://example.com/x".into()).is_link());
        assert!(!Clip::Text("see https://example.com".into()).is_link());
    }

    #[test]
    fn searches_every_word() {
        let clips = vec![Clip::Text("Hello World".into()), Clip::Text("hello there".into())];
        assert_eq!(search(&clips, "hello"), vec![0, 1]);
        assert_eq!(search(&clips, "WORLD hel"), vec![0]);
        assert_eq!(search(&clips, ""), vec![0, 1]);
    }

    #[test]
    fn command_matches() {
        for q in ["clip", "Clipboard", "clipboard hist", "history", "paste"] { assert!(matches_command(q), "{q}"); }
        for q in ["cl", "firefox", "clipboards"] { assert!(!matches_command(q), "{q}"); }
    }

    #[test]
    fn push_dedups_and_limits() {
        let h: Vec<Clip> = (0..LIMIT).map(|i| Clip::Text(i.to_string())).collect();
        let h = push(h, Clip::Text("5".into()));
        assert_eq!(h.len(), LIMIT);
        assert_eq!(h[0], Clip::Text("5".into()));
        assert_eq!(h.iter().filter(|c| **c == Clip::Text("5".into())).count(), 1);
        let h = push(h, Clip::Text("new".into()));
        assert_eq!(h.len(), LIMIT);
        assert_eq!(h[0], Clip::Text("new".into()));
    }
}
