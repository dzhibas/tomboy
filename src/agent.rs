//! Running a coding agent on a prompt.
//!
//! The agent is `[agent] name` from tomboy's config when set, else Omarchy's default
//! (`omarchy-default-agent`) on Omarchy, else `$TOMBOY_AGENT` (default `claude`). [`Run::start`] runs it headless and streams its answer into a shared
//! transcript that tomboy draws; agents without a headless mode go to a terminal via [`ask`].
//!
//! Claude streams `--output-format stream-json` events (text deltas, tool calls, a final
//! result with cost); opencode, codex and crush print plain text, shown as it arrives.
//!
//! Every run carries tomboy's skill (`skill.md`: its config file and options), so the user can
//! say "change the working directory to ~/sites" and the agent edits the config: claude gets
//! it as an appended system prompt, the others ahead of the prompt.

use std::io::{self, BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use serde_json::Value;

use crate::config;
use crate::platform::Platform;
use crate::sys;

/// What the agent knows about tomboy: the config file and every option in it.
pub const SKILL: &str = include_str!("skill.md");

/// The agent CLI to use: `forced` (the config's `[agent] name`) wins over the system's choice.
pub fn name(platform: &Platform, forced: Option<&str>) -> String {
    if let Some(agent) = forced.map(str::trim).filter(|a| !a.is_empty()) { return agent.to_string(); }
    if let Platform::Omarchy { root } = platform {
        let out = Command::new("omarchy-default-agent").env("PATH", omarchy_path(root))
            .stdin(Stdio::null()).stderr(Stdio::null()).output();
        if let Ok(out) = out {
            let agent = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !agent.is_empty() { return agent; }
        }
    }
    std::env::var("TOMBOY_AGENT").ok().filter(|a| !a.trim().is_empty()).unwrap_or_else(|| "claude".into())
}

/// Open the agent interactively in a terminal with `prompt`. A `forced` agent opens itself;
/// otherwise Omarchy launches its default.
pub fn ask(prompt: &str, platform: &Platform, forced: Option<&str>) -> io::Result<()> {
    let prompt = prompt.trim();
    match platform {
        _ if forced.is_some_and(|a| !a.trim().is_empty()) => sys::run_in_terminal(&[&name(platform, forced), prompt]),
        Platform::Omarchy { root } => {
            let mut cmd = Command::new("omarchy-agent-prompt");
            cmd.arg(prompt).env("PATH", omarchy_path(root));
            if let Some(home) = sys::home_dir() { cmd.current_dir(home); }
            sys::spawn_detached(&mut cmd)
        }
        Platform::Linux | Platform::MacOS => sys::run_in_terminal(&[&name(platform, None), prompt]),
    }
}

fn omarchy_path(root: &std::path::Path) -> String {
    std::env::var("PATH").map(|p| format!("{}:{p}", root.join("bin").display()))
        .unwrap_or_else(|_| root.join("bin").display().to_string())
}

/// Whether [`Run::start`] can run this agent headless.
pub fn supports_headless(agent: &str) -> bool {
    matches!(agent, "claude" | "opencode" | "codex" | "crush")
}

/// The headless command for `agent`. Like Omarchy's own launcher, agents run unattended.
/// Claude may also edit `extra_dirs` (tomboy's config and state), outside its working directory.
/// `model` (`[agent.models]` in the config) is passed as the agent's model flag.
fn headless_argv(agent: &str, prompt: &str, resume: Option<&str>, extra_dirs: &[PathBuf], model: Option<&str>)
    -> Option<Vec<String>> {
    let s = |v: &[&str]| v.iter().map(|a| a.to_string()).collect::<Vec<_>>();
    let model = model.map(str::trim).filter(|m| !m.is_empty());
    let mut argv = match agent {
        "claude" => {
            let mut a = s(&["claude", "-p", "--output-format", "stream-json", "--include-partial-messages",
                "--verbose", "--permission-mode", "auto", "--append-system-prompt", SKILL]);
            for dir in extra_dirs { a.extend(["--add-dir".into(), dir.display().to_string()]); }
            if let Some(m) = model { a.extend(s(&["--model", m])); }
            if let Some(id) = resume { a.extend(s(&["--resume", id])); }
            a.push("--".into());
            a.push(prompt.to_string());
            return Some(a);
        }
        "opencode" => s(&["opencode", "run", "--auto"]),
        "codex" => s(&["codex", "exec"]),
        "crush" => s(&["crush", "run"]),
        _ => return None,
    };
    if let Some(m) = model { argv.extend(s(&["-m", m])); }
    if agent != "crush" { argv.push("--".into()); }
    // No system prompt flag shared by these; each turn is a fresh run, so the skill leads it.
    argv.push(format!("{SKILL}\n---\n\nThe user's request:\n\n{prompt}"));
    Some(argv)
}

// ---- Transcript ----------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    /// The user's prompt (one per turn).
    Prompt(String),
    /// The agent's answer, growing as it streams.
    Text(String),
    /// A tool the agent called, summarized (`Bash: ls -la`).
    Tool(String),
    Error(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Running,
    Done { secs: f32, cost_usd: Option<f64> },
    Failed(String),
}

#[derive(Debug)]
pub struct Transcript {
    pub agent: String,
    pub blocks: Vec<Block>,
    pub status: Status,
    /// Claude's session id, so a follow-up continues the conversation.
    pub session: Option<String>,
    /// Process group of the running agent, to stop it.
    pgid: Option<u32>,
    /// Bumped by every turn, so a finished turn's reader can't touch the next turn's state.
    turn: u64,
    started: Instant,
    /// Where the agent runs.
    dir: Option<PathBuf>,
    /// The model the agent is told to use (`[agent.models]`); `None` is its own default.
    pub model: Option<String>,
}

impl Transcript {
    fn push_text(&mut self, delta: &str) {
        match self.blocks.last_mut() {
            Some(Block::Text(t)) => t.push_str(delta),
            _ => self.blocks.push(Block::Text(delta.to_string())),
        }
    }

    /// The answer of the latest turn: its text blocks, joined.
    pub fn last_answer(&self) -> String {
        let start = self.blocks.iter().rposition(|b| matches!(b, Block::Prompt(_))).map_or(0, |i| i + 1);
        self.blocks[start..].iter()
            .filter_map(|b| if let Block::Text(t) = b { Some(t.trim()) } else { None })
            .collect::<Vec<_>>().join("\n\n")
    }

    pub fn is_running(&self) -> bool {
        self.status == Status::Running
    }
}

/// A transcript shared between tomboy's UI and the threads reading the agent.
#[derive(Clone)]
pub struct Run(Arc<Mutex<Transcript>>);

impl Run {
    /// `dir` is where the agent runs (`[agent] working_directory`, else ~/Work, else ~); it
    /// stays for the whole conversation, since a resumed session belongs to its directory.
    pub fn new(agent: String, dir: Option<PathBuf>, model: Option<String>) -> Self {
        Run(Arc::new(Mutex::new(Transcript {
            agent, dir, model, blocks: Vec::new(), status: Status::Done { secs: 0.0, cost_usd: None },
            session: None, pgid: None, turn: 0, started: Instant::now(),
        })))
    }

    /// A finished transcript, for headless snapshots.
    pub fn demo(agent: &str, blocks: &[Block], status: Status) -> Self {
        let run = Run::new(agent.into(), None, None);
        { let mut t = run.lock(); t.blocks = blocks.to_vec(); t.status = status; }
        run
    }

    pub fn lock(&self) -> MutexGuard<'_, Transcript> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Start a turn: run the agent on `prompt` (continuing the session when there is one) and
    /// stream its output into the transcript, calling `wake` whenever it changes.
    pub fn start(&self, prompt: &str, wake: impl Fn() + Send + Sync + 'static) -> io::Result<()> {
        let (agent, session, dir, model) = {
            let t = self.lock();
            (t.agent.clone(), t.session.clone(), t.dir.clone(), t.model.clone())
        };
        // tomboy's config, state and todo directories (config.toml, frecency.json, the daily
        // lists), created so the agent may write into them.
        let extra_dirs: Vec<PathBuf> = [config::path().and_then(|p| p.parent().map(PathBuf::from)),
                                        sys::state_dir().map(|d| d.join("tomboy")),
                                        config::load().0.todo_dir()]
            .into_iter().flatten().filter(|d| std::fs::create_dir_all(d).is_ok()).collect();
        let argv = headless_argv(&agent, prompt, session.as_deref(), &extra_dirs, model.as_deref())
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, format!("{agent} has no headless mode")))?;
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        if let Some(dir) = dir { cmd.current_dir(dir); }
        let mut child = cmd.spawn()?;
        let turn = {
            let mut t = self.lock();
            t.blocks.push(Block::Prompt(prompt.trim().to_string()));
            t.status = Status::Running;
            t.pgid = Some(child.id());
            t.started = Instant::now();
            t.turn += 1;
            t.turn
        };
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let stderr = child.stderr.take().map(|e| {
            std::thread::spawn(move || {
                let mut err = String::new();
                let _ = io::Read::read_to_string(&mut BufReader::new(e), &mut err);
                err
            })
        });
        let stdout = child.stdout.take();
        let run = self.clone();
        std::thread::spawn(move || {
            if let Some(out) = stdout {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    let mut t = run.lock();
                    if t.turn != turn { break; }
                    if agent == "claude" { apply_claude_event(&mut t, &line); } else { t.push_text(&format!("{line}\n")); }
                    drop(t);
                    wake();
                }
            }
            let ok = child.wait().is_ok_and(|s| s.success());
            let err = stderr.and_then(|h| h.join().ok()).unwrap_or_default();
            let mut t = run.lock();
            if t.turn != turn { return; }
            t.pgid = None;
            if t.status == Status::Running {
                t.status = if ok {
                    Status::Done { secs: t.started.elapsed().as_secs_f32(), cost_usd: None }
                } else {
                    let msg = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("agent exited with an error");
                    Status::Failed(msg.trim().to_string())
                };
            }
            drop(t);
            wake();
        });
        Ok(())
    }

    /// Stop a running turn (the whole process group, so the agent's tools stop too).
    pub fn stop(&self) {
        let mut t = self.lock();
        if let Some(pgid) = t.pgid.take() {
            let _ = Command::new("kill").args(["-TERM", &format!("-{pgid}")])
                .stdout(Stdio::null()).stderr(Stdio::null()).status();
            t.status = Status::Failed("Stopped".into());
        }
    }
}

/// Fold one `claude --output-format stream-json` line into the transcript.
fn apply_claude_event(t: &mut Transcript, line: &str) {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return };
    let get = |v: &Value, path: &[&str]| path.iter().try_fold(v.clone(), |v, k| v.get(k).cloned());
    match v.get("type").and_then(Value::as_str) {
        Some("system") => {
            if let Some(id) = v.get("session_id").and_then(Value::as_str) { t.session = Some(id.to_string()); }
        }
        // Text arrives as deltas; the full `assistant` message repeats it, so only tools come from there.
        Some("stream_event") => {
            if get(&v, &["event", "delta", "type"]).as_ref().and_then(Value::as_str) == Some("text_delta")
                && let Some(text) = get(&v, &["event", "delta", "text"]).as_ref().and_then(Value::as_str)
                && v.get("parent_tool_use_id").is_none_or(Value::is_null)
            {
                t.push_text(text);
            }
        }
        Some("assistant") if v.get("parent_tool_use_id").is_none_or(Value::is_null) => {
            let content = get(&v, &["message", "content"]);
            for c in content.as_ref().and_then(Value::as_array).into_iter().flatten() {
                if c.get("type").and_then(Value::as_str) == Some("tool_use") {
                    let name = c.get("name").and_then(Value::as_str).unwrap_or("tool");
                    t.blocks.push(Block::Tool(tool_summary(name, c.get("input"))));
                }
            }
        }
        Some("result") => {
            let secs = v.get("duration_ms").and_then(Value::as_f64).map_or(t.started.elapsed().as_secs_f32(), |ms| (ms / 1000.0) as f32);
            let cost_usd = v.get("total_cost_usd").and_then(Value::as_f64);
            if v.get("is_error").and_then(Value::as_bool) == Some(true) {
                let msg = v.get("result").and_then(Value::as_str).unwrap_or("the agent reported an error");
                t.blocks.push(Block::Error(msg.to_string()));
                t.status = Status::Failed(msg.lines().next().unwrap_or("error").to_string());
            } else {
                t.status = Status::Done { secs, cost_usd };
            }
        }
        _ => {}
    }
}

/// `Bash` + `{"command": "ls -la"}` → `Bash: ls -la`, one line, at most 90 characters.
fn tool_summary(name: &str, input: Option<&Value>) -> String {
    let detail = input.and_then(|i| {
        ["command", "file_path", "pattern", "url", "query", "description", "path"]
            .iter().find_map(|k| i.get(k).and_then(Value::as_str))
    }).unwrap_or("");
    let line = detail.lines().next().unwrap_or("");
    let mut s = if line.is_empty() { name.to_string() } else { format!("{name}: {line}") };
    if s.chars().count() > 90 { s = s.chars().take(89).collect::<String>() + "…"; }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transcript() -> Transcript {
        Transcript { agent: "claude".into(), blocks: vec![Block::Prompt("hi".into())], status: Status::Running,
            session: None, pgid: None, turn: 1, started: Instant::now(), dir: None, model: None }
    }

    #[test]
    fn claude_stream_builds_the_transcript() {
        let mut t = transcript();
        for line in [
            r#"{"type":"system","subtype":"init","session_id":"abc"}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}},"parent_tool_use_id":null}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo"}},"parent_tool_use_id":null}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello"},{"type":"tool_use","name":"Bash","input":{"command":"ls -la\nmore"}}]},"parent_tool_use_id":null}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Done."}},"parent_tool_use_id":null}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"(subagent)"}},"parent_tool_use_id":"tool_1"}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":2500,"total_cost_usd":0.0123,"result":"Done."}"#,
            "not json",
        ] {
            apply_claude_event(&mut t, line);
        }
        assert_eq!(t.session.as_deref(), Some("abc"));
        assert_eq!(t.blocks, vec![
            Block::Prompt("hi".into()),
            Block::Text("Hello".into()),
            Block::Tool("Bash: ls -la".into()),
            Block::Text("Done.".into()),
        ]);
        assert_eq!(t.status, Status::Done { secs: 2.5, cost_usd: Some(0.0123) });
        assert_eq!(t.last_answer(), "Hello\n\nDone.");
    }

    #[test]
    fn claude_errors_fail_the_turn() {
        let mut t = transcript();
        apply_claude_event(&mut t, r#"{"type":"result","is_error":true,"result":"Credit balance too low"}"#);
        assert_eq!(t.status, Status::Failed("Credit balance too low".into()));
    }

    #[test]
    fn configured_agent_wins() {
        let omarchy = Platform::Omarchy { root: "/nonexistent".into() };
        assert_eq!(name(&omarchy, Some(" codex ")), "codex");
        assert_eq!(name(&Platform::Linux, Some("opencode")), "opencode");
        // Blank means unset: fall back to the system's choice.
        assert_ne!(name(&Platform::Linux, Some("  ")), "  ");
    }

    #[test]
    fn headless_commands() {
        let claude = headless_argv("claude", "hi", Some("s1"), &["/c/tomboy".into(), "/s/tomboy".into()], None).unwrap();
        assert_eq!(claude[8..], ["--append-system-prompt", SKILL, "--add-dir", "/c/tomboy", "--add-dir", "/s/tomboy", "--resume", "s1", "--", "hi"]);
        let codex = headless_argv("codex", "-x", None, &[], None).unwrap();
        assert_eq!(codex[..3], ["codex", "exec", "--"]);
        assert!(codex[3].starts_with(SKILL) && codex[3].ends_with("\n\n-x"));
        assert!(headless_argv("hermes", "hi", None, &[], None).is_none());
        // A configured model goes in as the agent's model flag.
        let claude = headless_argv("claude", "hi", None, &[], Some("haiku")).unwrap();
        assert_eq!(claude[claude.len() - 4..], ["--model", "haiku", "--", "hi"]);
        let codex = headless_argv("codex", "hi", None, &[], Some(" gpt-5 ")).unwrap();
        assert_eq!(codex[..5], ["codex", "exec", "-m", "gpt-5", "--"]);
        let crush = headless_argv("crush", "hi", None, &[], Some("sonnet")).unwrap();
        assert_eq!(crush[..4], ["crush", "run", "-m", "sonnet"]);
        assert!(!headless_argv("claude", "hi", None, &[], Some("  ")).unwrap().contains(&"--model".to_string()));
        assert!(supports_headless("opencode") && !supports_headless("pi"));
    }

    /// Runs the real `claude` CLI (a tiny paid request): `cargo test live_claude -- --ignored`.
    #[test]
    #[ignore]
    fn live_claude_streams_an_answer_and_a_follow_up() {
        let run = Run::new("claude".into(), None, None);
        let wait = |run: &Run| {
            let deadline = Instant::now() + std::time::Duration::from_secs(120);
            while run.lock().is_running() && Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        };
        run.start("Reply with exactly the word: pineapple", || {}).unwrap();
        wait(&run);
        {
            let t = run.lock();
            println!("{:?}\n{:?}", t.blocks, t.status);
            assert!(matches!(t.status, Status::Done { .. }), "{:?}", t.status);
            assert!(t.last_answer().to_lowercase().contains("pineapple"));
            assert!(t.session.is_some());
        }
        run.start("What word did you just say? Reply with only that word.", || {}).unwrap();
        wait(&run);
        let t = run.lock();
        println!("{:?}\n{:?}", t.blocks, t.status);
        assert!(t.last_answer().to_lowercase().contains("pineapple"), "the follow-up resumed the session");
    }

    #[test]
    fn plain_text_lines_join_into_one_answer() {
        let run = Run::new("crush".into(), None, None);
        let mut t = run.lock();
        t.push_text("a\n");
        t.push_text("b\n");
        assert_eq!(t.last_answer(), "a\nb");
    }
}
