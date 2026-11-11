//! Telling dictated text from typed text.
//!
//! Superwhisper delivers a dictation as one insert (an fcitx5 commit or a paste), and its
//! CLI reports the last dictation (`superwhisper read`: `inserted_text`, `createdAt`). A
//! multi-character insert that equals a dictation from the last minute came from voice.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How old the last dictation may be and still count as the text that just arrived.
const MAX_AGE: Duration = Duration::from_secs(60);
/// An insert this long or longer in one change can't be typing.
pub const MIN_BURST: usize = 4;

/// The text a single edit inserted: `after` minus the prefix and suffix it shares with `before`.
pub fn inserted<'a>(before: &str, after: &'a str) -> &'a str {
    let prefix = before.chars().zip(after.chars()).take_while(|(a, b)| a == b).map(|(c, _)| c.len_utf8()).sum::<usize>();
    let (b, a) = (&before[prefix..], &after[prefix..]);
    let suffix = b.chars().rev().zip(a.chars().rev()).take_while(|(x, y)| x == y).map(|(c, _)| c.len_utf8()).sum::<usize>();
    &a[..a.len() - suffix.min(a.len())]
}

/// Whether `text` is Superwhisper's most recent dictation, made within the last minute.
pub fn is_from_superwhisper(text: &str) -> bool {
    let want = normalize(text);
    if want.is_empty() { return false; }
    let Some((said, at)) = last_superwhisper() else { return false };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    now - at <= MAX_AGE.as_secs() as i64 && normalize(&said) == want
}

fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `(inserted text, unix seconds)` of the last dictation, from `superwhisper read`.
fn last_superwhisper() -> Option<(String, i64)> {
    let out = Command::new(superwhisper_bin()?).arg("read")
        .stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let text = ["inserted_text", "text"].iter().find_map(|k| v.get(k)?.as_str())?.to_string();
    let at = v.get("createdAt")?.as_f64()? as i64;
    Some((text, at))
}

fn superwhisper_bin() -> Option<PathBuf> {
    let on_path = std::env::var_os("PATH").into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|d| d.join("superwhisper"))
        .find(|p| p.is_file());
    on_path.or_else(|| crate::sys::home_dir().map(|h| h.join(".local/bin/superwhisper")).filter(|p| p.is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserted_is_the_new_middle() {
        assert_eq!(inserted("", "hello world"), "hello world");
        assert_eq!(inserted("ab", "abc"), "c");
        assert_eq!(inserted("open ", "open the pod bay doors"), "the pod bay doors");
        assert_eq!(inserted("ac", "abc"), "b");
        assert_eq!(inserted("abc", "ab"), "");
        assert_eq!(inserted("žą", "žaą"), "a");
    }

    #[test]
    fn reads_the_last_dictation_when_superwhisper_is_installed() {
        if superwhisper_bin().is_none() { return; }
        let (text, at) = last_superwhisper().expect("superwhisper read parses");
        println!("last dictation at {at}: {text:?}");
        assert!(at > 1_600_000_000, "createdAt is unix seconds");
        assert!(!is_from_superwhisper("definitely not what was dictated 12345"));
    }

    #[test]
    fn whitespace_is_normalized() {
        assert_eq!(normalize("  hi \n there "), "hi there");
    }
}
