//! Past agent conversations, kept so they can be read again and continued.
//!
//! Each conversation is one JSON file in `~/.local/state/tomboy/agent-sessions/`, written when
//! a turn starts and again when it ends. Opening one makes it the live conversation: Claude
//! resumes its own session (`--resume`), other agents get the earlier turns with the prompt.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::agent::Block;
use crate::sys;

/// One saved conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    /// The file name (without `.json`).
    pub id: String,
    pub agent: String,
    #[serde(default)]
    pub model: Option<String>,
    /// Where the agent ran; a resumed Claude session belongs to this directory.
    #[serde(default)]
    pub dir: Option<PathBuf>,
    /// The agent's own session id (Claude's), to resume it.
    #[serde(default)]
    pub session: Option<String>,
    /// Unix seconds.
    pub created: i64,
    pub updated: i64,
    pub blocks: Vec<Block>,
}

impl Saved {
    /// The first prompt, on one line.
    pub fn title(&self) -> String {
        let first = self.blocks.iter().find_map(|b| if let Block::Prompt(p) = b { Some(p.as_str()) } else { None });
        let line = first.unwrap_or("(empty)").split_whitespace().collect::<Vec<_>>().join(" ");
        if line.chars().count() > 120 { line.chars().take(119).collect::<String>() + "…" } else { line }
    }

    /// How many prompts the conversation has.
    pub fn turns(&self) -> usize {
        self.blocks.iter().filter(|b| matches!(b, Block::Prompt(_))).count()
    }

    /// Whether every word of `q` appears in a prompt or an answer (case-insensitive).
    fn matches(&self, q: &str) -> bool {
        let words: Vec<String> = q.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() { return true; }
        let hay = self.blocks.iter().filter_map(|b| match b {
            Block::Prompt(t) | Block::Text(t) => Some(t.to_lowercase()),
            _ => None,
        }).collect::<Vec<_>>().join("\n");
        words.iter().all(|w| hay.contains(w.as_str()))
    }
}

/// `~/.local/state/tomboy/agent-sessions`.
pub fn dir() -> Option<PathBuf> {
    sys::state_dir().map(|d| d.join("tomboy").join("agent-sessions"))
}

/// A fresh id: the time and the process, so two tomboys never collide.
pub fn new_id() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    format!("{nanos:x}-{:x}", std::process::id())
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Write `saved` into `dir` (through a temporary file, so a reader never sees half of it).
pub fn save_in(dir: &Path, saved: &Saved) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_vec_pretty(saved).map_err(io::Error::other)?;
    let tmp = dir.join(format!(".{}.tmp", saved.id));
    std::fs::write(&tmp, json)?;
    std::fs::rename(tmp, dir.join(format!("{}.json", saved.id)))
}

pub fn save(saved: &Saved) -> io::Result<()> {
    let dir = dir().ok_or_else(|| io::Error::other("no state directory"))?;
    save_in(&dir, saved)
}

/// Every saved conversation in `dir`, the most recently used first. Unreadable files are skipped.
pub fn load_in(dir: &Path) -> Vec<Saved> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut all: Vec<Saved> = entries.flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
        .filter(|s: &Saved| s.turns() > 0)
        .collect();
    all.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| b.id.cmp(&a.id)));
    all
}

pub fn load() -> Vec<Saved> {
    dir().map(|d| load_in(&d)).unwrap_or_default()
}

pub fn remove_in(dir: &Path, id: &str) -> io::Result<()> {
    std::fs::remove_file(dir.join(format!("{id}.json")))
}

pub fn remove(id: &str) -> io::Result<()> {
    let dir = dir().ok_or_else(|| io::Error::other("no state directory"))?;
    remove_in(&dir, id)
}

/// The conversations matching `q`, by their place in `all`.
pub fn search(all: &[Saved], q: &str) -> Vec<usize> {
    (0..all.len()).filter(|&i| all[i].matches(q)).collect()
}

/// "Agent History" in the launcher: `agent history`, `agents`, `sessions` or a prefix of a word.
pub fn matches_command(query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.chars().count() >= 3 && ["agent history", "agents", "agent sessions"].iter().any(|name| {
        name.starts_with(q.as_str()) || name.split(' ').any(|w| w.starts_with(q.as_str()))
    })
}

/// `5m ago`, `3h ago`, `2d ago`, or the date for anything older than a week.
pub fn ago(then: i64, now: i64) -> String {
    let secs = (now - then).max(0);
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86_400 => format!("{}h ago", secs / 3600),
        86_400..604_800 => format!("{}d ago", secs / 86_400),
        _ => jiff::Timestamp::from_second(then).map(|t| t.strftime("%-d %b %Y").to_string()).unwrap_or_default(),
    }
}

/// The earlier turns as plain text, for an agent with no session of its own to resume: the
/// latest ones, at most `limit` characters.
pub fn context(blocks: &[Block], limit: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut used = 0;
    for b in blocks.iter().rev() {
        let part = match b {
            Block::Prompt(p) => format!("User: {}", p.trim()),
            Block::Text(t) if !t.trim().is_empty() => format!("Assistant: {}", t.trim()),
            _ => continue,
        };
        if used + part.len() > limit && !parts.is_empty() { break; }
        used += part.len();
        parts.push(part);
    }
    parts.reverse();
    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(id: &str, updated: i64, prompt: &str, answer: &str) -> Saved {
        Saved { id: id.into(), agent: "claude".into(), model: None, dir: Some("/w".into()), session: Some("s".into()),
            created: updated, updated, blocks: vec![Block::Prompt(prompt.into()), Block::Tool("Bash: ls".into()), Block::Text(answer.into())] }
    }

    #[test]
    fn saves_loads_newest_first_and_removes() {
        let dir = std::env::temp_dir().join(format!("tomboy-history-{}", new_id()));
        save_in(&dir, &saved("a", 10, "first", "one")).unwrap();
        save_in(&dir, &saved("b", 20, "second", "two")).unwrap();
        // An empty conversation (no prompt yet) and junk are skipped.
        save_in(&dir, &Saved { blocks: vec![], ..saved("c", 30, "", "") }).unwrap();
        std::fs::write(dir.join("junk.json"), "{").unwrap();
        let all = load_in(&dir);
        assert_eq!(all.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(all[1], saved("a", 10, "first", "one"));
        remove_in(&dir, "b").unwrap();
        assert_eq!(load_in(&dir).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_titles_and_commands() {
        let all = [saved("a", 1, "What's on port 3000?", "node is"), saved("b", 2, "fix   the\nbuild", "Done")];
        assert_eq!(search(&all, "PORT node"), [0]);
        assert_eq!(search(&all, ""), [0, 1]);
        assert_eq!(all[1].title(), "fix the build");
        assert_eq!(all[0].turns(), 1);
        assert!(matches_command("agent hist") && matches_command("sessions") && matches_command("agents"));
        assert!(!matches_command("ag") && !matches_command("clipboard"));
    }

    #[test]
    fn context_keeps_the_latest_turns() {
        let blocks = [Block::Prompt("q1".into()), Block::Text("a1".into()), Block::Tool("Bash".into()),
            Block::Prompt("q2".into()), Block::Text("a2".into())];
        assert_eq!(context(&blocks, 10_000), "User: q1\n\nAssistant: a1\n\nUser: q2\n\nAssistant: a2");
        assert_eq!(context(&blocks, 25), "User: q2\n\nAssistant: a2");
    }

    #[test]
    fn ages() {
        assert_eq!(ago(100, 130), "just now");
        assert_eq!(ago(0, 3 * 3600), "3h ago");
        assert_eq!(ago(0, 2 * 86_400), "2d ago");
    }
}
