//! The daily todo list: one Markdown file per day in `[todo] directory` (default
//! `~/Documents/tomboy/todo/`), `2026-11-12.md`, readable by any editor or Obsidian:
//!
//! ```markdown
//! # 2026-11-12
//!
//! - [x] Reply to Jonas
//! - [ ] Review PR #42
//! ```
//!
//! Tasks are `- [ ]` / `- [x]` lines (`*` works as the bullet too); every other line is kept as
//! it is. Opening today's list moves the latest earlier day's open tasks into it, marked
//! `- [>]` (moved) where they were. Every change re-reads the file first, so
//! an edit made meanwhile (the agent, an editor) isn't written over.

use std::io;
use std::path::{Path, PathBuf};

use jiff::civil::Date;

/// One task line.
#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    /// Index of its line in the file.
    pub line: usize,
    pub done: bool,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Day {
    pub date: Date,
    pub path: PathBuf,
    lines: Vec<String>,
}

/// Today's date, local time.
pub fn today_date() -> Date {
    jiff::Zoned::now().date()
}

/// `(done, text)` for a task line; `None` for anything else (including `- [>]` moved tasks).
fn parse(line: &str) -> Option<(bool, &str)> {
    let t = line.trim_start();
    let t = t.strip_prefix("- ").or_else(|| t.strip_prefix("* "))?;
    let (done, rest) = if let Some(r) = t.strip_prefix("[ ]") { (false, r) }
        else if let Some(r) = t.strip_prefix("[x]").or_else(|| t.strip_prefix("[X]")) { (true, r) }
        else { return None };
    if !(rest.is_empty() || rest.starts_with(' ')) { return None; }
    Some((done, rest.trim()))
}

fn task_line(text: &str, done: bool) -> String {
    format!("- [{}] {}", if done { "x" } else { " " }, clean(text))
}

/// One line, no surrounding space.
fn clean(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Day {
    /// An in-memory day (nothing on disk yet): just its heading.
    fn empty(date: Date, path: PathBuf) -> Day {
        Day { date, path, lines: vec![format!("# {date}"), String::new()] }
    }

    /// The day's file in `dir`, or an empty day when it doesn't exist.
    pub fn load(dir: &Path, date: Date) -> io::Result<Day> {
        let path = dir.join(format!("{date}.md"));
        match std::fs::read_to_string(&path) {
            Ok(raw) => Ok(Day { date, lines: raw.lines().map(str::to_string).collect(), path }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Day::empty(date, path)),
            Err(e) => Err(e),
        }
    }

    /// A day held only in memory, for demos.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn demo(date: Date, tasks: &[(bool, &str)]) -> Day {
        let mut d = Day::empty(date, PathBuf::from("/nonexistent/tomboy-demo.md"));
        d.lines.extend(tasks.iter().map(|(done, t)| task_line(t, *done)));
        d
    }

    pub fn tasks(&self) -> Vec<Task> {
        self.lines.iter().enumerate()
            .filter_map(|(i, l)| parse(l).map(|(done, text)| Task { line: i, done, text: text.to_string() }))
            .collect()
    }

    /// (done, total).
    pub fn counts(&self) -> (usize, usize) {
        let t = self.tasks();
        (t.iter().filter(|t| t.done).count(), t.len())
    }

    /// Add an open task after the last task (at the end when there are none).
    pub fn add(&mut self, text: &str) -> io::Result<bool> {
        let text = clean(text);
        if text.is_empty() { return Ok(false); }
        self.change(|lines| { insert_tasks(lines, &[text]); true })
    }

    /// Check or uncheck `task` (as listed by [`Day::tasks`]). `Ok(false)` when it's no longer
    /// in the file.
    pub fn toggle(&mut self, task: &Task) -> io::Result<bool> {
        self.change(|lines| match find(lines, task) {
            Some(i) => {
                let (done, _) = parse(&lines[i]).expect("found a task");
                lines[i] = set_mark(&lines[i], if done { ' ' } else { 'x' });
                true
            }
            None => false,
        })
    }

    /// Delete `task`'s line. `Ok(false)` when it's no longer in the file.
    pub fn remove(&mut self, task: &Task) -> io::Result<bool> {
        self.change(|lines| match find(lines, task) {
            Some(i) => { lines.remove(i); true }
            None => false,
        })
    }

    /// Re-read the file, apply `f`, and save when it changed something.
    fn change(&mut self, f: impl FnOnce(&mut Vec<String>) -> bool) -> io::Result<bool> {
        let dir = self.path.parent().map(Path::to_path_buf).unwrap_or_default();
        let fresh = Day::load(&dir, self.date)?;
        self.lines = fresh.lines;
        if !f(&mut self.lines) { return Ok(false); }
        self.save()?;
        Ok(true)
    }

    fn save(&self) -> io::Result<()> {
        if let Some(dir) = self.path.parent() { std::fs::create_dir_all(dir)?; }
        let tmp = self.path.with_extension("md.tmp");
        std::fs::write(&tmp, self.lines.join("\n") + "\n")?;
        std::fs::rename(tmp, &self.path)
    }
}

/// Add open tasks after the last task, or at the end (after a blank line) when there are none.
fn insert_tasks(lines: &mut Vec<String>, texts: &[String]) {
    let new = texts.iter().map(|t| task_line(t, false));
    match lines.iter().rposition(|l| parse(l).is_some()) {
        Some(i) => { lines.splice(i + 1..i + 1, new); }
        None => {
            while lines.last().is_some_and(|l| l.trim().is_empty()) { lines.pop(); }
            if !lines.is_empty() { lines.push(String::new()); }
            lines.extend(new);
        }
    }
}

/// The line holding `task`: where it was if it's still there, else the first with its text.
fn find(lines: &[String], task: &Task) -> Option<usize> {
    let same = |i: usize| lines.get(i).and_then(|l| parse(l)).is_some_and(|(_, t)| t == task.text);
    if same(task.line) { return Some(task.line); }
    (0..lines.len()).find(|&i| same(i))
}

/// `- [ ] x` with its mark replaced, keeping indentation and bullet.
fn set_mark(line: &str, mark: char) -> String {
    match line.find('[') {
        Some(i) => format!("{}[{mark}]{}", &line[..i], &line[i + 3..]),
        None => line.to_string(),
    }
}

/// Today's list in `dir`, with the latest earlier day's open tasks moved in (marked `- [>]`
/// there, so doing it again moves nothing). This runs on every open, not just a day's first,
/// so a today file made early (by the agent, an editor) still gets yesterday's leftovers.
pub fn today(dir: &Path, date: Date) -> io::Result<Day> {
    let mut day = Day::load(dir, date)?;
    let Some(prev) = latest_before(dir, date) else { return Ok(day) };
    let mut old = Day::load(dir, prev)?;
    let open: Vec<Task> = old.tasks().into_iter().filter(|t| !t.done).collect();
    if open.is_empty() { return Ok(day); }
    let texts: Vec<String> = open.iter().map(|t| t.text.clone()).collect();
    insert_tasks(&mut day.lines, &texts);
    // Today first: if the second write fails, a task shows twice rather than getting lost.
    day.save()?;
    for t in &open { old.lines[t.line] = set_mark(&old.lines[t.line], '>'); }
    old.save()?;
    Ok(day)
}

/// (done, total) for today without writing anything: today's file plus what opening it would
/// carry over from the latest earlier day.
pub fn peek(dir: &Path, date: Date) -> (usize, usize) {
    let (done, total) = Day::load(dir, date).map_or((0, 0), |d| d.counts());
    let carried = latest_before(dir, date).and_then(|prev| Day::load(dir, prev).ok())
        .map_or(0, |old| old.tasks().iter().filter(|t| !t.done).count());
    (done, total + carried)
}

/// The latest `YYYY-MM-DD.md` in `dir` before `date`.
fn latest_before(dir: &Path, date: Date) -> Option<Date> {
    std::fs::read_dir(dir).ok()?
        .filter_map(|e| e.ok()?.file_name().to_str()?.strip_suffix(".md")?.parse::<Date>().ok())
        .filter(|d| *d < date)
        .max()
}

/// `tomboy todo add <text>`: add to today's list and print the count.
pub fn add_cli(dir: &Path, text: &str) -> io::Result<()> {
    if text.trim().is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "usage: tomboy todo add <text>"));
    }
    let mut day = today(dir, today_date())?;
    day.add(text)?;
    let (done, total) = day.counts();
    println!("{done}/{total} done");
    Ok(())
}

/// Does the launcher query ask for the list? (`todo`, `tasks`, `gtd`, …, 3+ letters.)
pub fn matches_command(query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.chars().count() >= 3 && ["todo", "todos", "tasks", "gtd", "today"].iter().any(|w| w.starts_with(q.as_str()))
}

/// `todo buy milk` → "buy milk": a task to add from the launcher.
pub fn quick_add(query: &str) -> Option<&str> {
    let q = query.trim_start();
    let (word, rest) = q.split_once(char::is_whitespace)?;
    let rest = rest.trim();
    (["todo", "task"].contains(&word.to_lowercase().as_str()) && !rest.is_empty()).then_some(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Date { s.parse().unwrap() }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tomboy-todo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn parses_lines() {
        assert_eq!(parse("- [ ] buy milk"), Some((false, "buy milk")));
        assert_eq!(parse("  * [X] done "), Some((true, "done")));
        assert_eq!(parse("- [x]"), Some((true, "")));
        assert_eq!(parse("- [>] moved"), None);
        assert_eq!(parse("- [ ]x"), None);
        assert_eq!(parse("# 2026-11-12"), None);
        assert_eq!(parse("- plain bullet"), None);
        assert_eq!(set_mark("  - [ ] a", 'x'), "  - [x] a");
    }

    #[test]
    fn adds_toggles_removes_keeping_other_lines() {
        let dir = tmp("edit");
        let date = d("2026-11-12");
        let mut day = today(&dir, date).unwrap();
        assert!(!day.path.exists()); // nothing written until something is added
        assert_eq!(day.counts(), (0, 0));
        assert!(day.add("  buy   milk ").unwrap());
        assert!(!day.add("   ").unwrap());
        day.add("review PR #42").unwrap();
        assert_eq!(std::fs::read_to_string(&day.path).unwrap(), "# 2026-11-12\n\n- [ ] buy milk\n- [ ] review PR #42\n");
        // A note added by hand stays put.
        std::fs::write(&day.path, "# 2026-11-12\n\n- [ ] buy milk\n- [ ] review PR #42\n\n## Notes\nsomething\n").unwrap();
        let tasks = day.tasks();
        assert!(day.toggle(&tasks[0]).unwrap());
        day.add("call mum").unwrap();
        assert_eq!(day.counts(), (1, 3));
        let t = day.tasks();
        assert!(day.remove(&t[1]).unwrap());
        assert_eq!(std::fs::read_to_string(&day.path).unwrap(),
            "# 2026-11-12\n\n- [x] buy milk\n- [ ] call mum\n\n## Notes\nsomething\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn edits_made_meanwhile_survive() {
        let dir = tmp("race");
        let mut day = today(&dir, d("2026-11-12")).unwrap();
        day.add("a").unwrap();
        day.add("b").unwrap();
        let shown = day.tasks();
        // Someone else inserts a task above and deletes "a" while tomboy is open.
        std::fs::write(&day.path, "# 2026-11-12\n\n- [ ] new\n- [ ] b\n").unwrap();
        assert!(!day.toggle(&shown[0]).unwrap()); // "a" is gone: nothing happens
        assert!(day.toggle(&shown[1]).unwrap()); // "b" moved down a line: still found
        assert_eq!(std::fs::read_to_string(&day.path).unwrap(), "# 2026-11-12\n\n- [ ] new\n- [x] b\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn carries_over_open_tasks() {
        let dir = tmp("carry");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("2026-11-09.md"), "# 2026-11-09\n\n- [ ] ancient\n").unwrap();
        std::fs::write(dir.join("2026-11-10.md"), "# 2026-11-10\n\n- [x] done\n- [ ] dentist\n  - [ ] nested\n").unwrap();
        std::fs::write(dir.join("notes.md"), "not a day").unwrap();
        assert_eq!(peek(&dir, d("2026-11-12")), (0, 2));
        assert!(!dir.join("2026-11-12.md").exists());
        let day = today(&dir, d("2026-11-12")).unwrap();
        assert_eq!(peek(&dir, d("2026-11-12")), (0, 2));
        assert_eq!(day.tasks().iter().map(|t| t.text.as_str()).collect::<Vec<_>>(), ["dentist", "nested"]);
        assert_eq!(std::fs::read_to_string(dir.join("2026-11-10.md")).unwrap(),
            "# 2026-11-10\n\n- [x] done\n- [>] dentist\n  - [>] nested\n");
        // The older day is left alone, and opening today again carries nothing more.
        assert!(std::fs::read_to_string(dir.join("2026-11-09.md")).unwrap().contains("- [ ] ancient"));
        assert_eq!(today(&dir, d("2026-11-12")).unwrap().counts(), (0, 2));
        // Tomorrow's file made early (the agent moving a task) still gets today's leftovers.
        std::fs::write(dir.join("2026-11-13.md"), "# 2026-11-13\n\n- [ ] moved early\n").unwrap();
        assert_eq!(peek(&dir, d("2026-11-13")), (0, 3));
        let next = today(&dir, d("2026-11-13")).unwrap();
        assert_eq!(next.tasks().iter().map(|t| t.text.as_str()).collect::<Vec<_>>(), ["moved early", "dentist", "nested"]);
        assert_eq!(peek(&dir, d("2026-11-13")), (0, 3));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn launcher_queries() {
        for q in ["todo", "Tod", "tasks", "gtd", "today"] { assert!(matches_command(q), "{q}"); }
        for q in ["to", "tomboy", "terminal"] { assert!(!matches_command(q), "{q}"); }
        assert_eq!(quick_add("todo buy milk "), Some("buy milk"));
        assert_eq!(quick_add("Task call mum"), Some("call mum"));
        assert_eq!(quick_add("todo "), None);
        assert_eq!(quick_add("todos"), None);
    }
}
