//! `kill <name>`: find your own processes by name or command line and SIGTERM one.
//!
//! Nothing lists processes until the query starts with `kill`, and a listing is reused
//! for [`CACHE_TTL`] rather than re-taken on every keystroke.
//!
//! CPU figures:
//! - Linux reads `/proc` twice, [`SAMPLE_GAP`] apart, and reports the CPU time each process
//!   used in between as a percentage of one core (like `top`). A process that appears
//!   only in the second sample shows 0%. The gap is paid once per listing, not per keystroke.
//! - macOS uses `ps`'s own `%cpu`, a decaying recent average.

use super::{Action, Answer, Kind};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a process listing is reused.
const CACHE_TTL: Duration = Duration::from_secs(3);
/// Gap between the two `/proc` samples that give instantaneous CPU on Linux.
#[cfg(target_os = "linux")]
const SAMPLE_GAP: Duration = Duration::from_millis(150);
/// Longest command line shown in a subtitle, in characters.
const CMD_MAX: usize = 60;

/// One process as listed.
#[derive(Clone, Debug, PartialEq)]
struct Proc {
    pid: u32,
    uid: u32,
    name: String,
    cmd: String,
    /// Percent of one core.
    cpu: f64,
    rss_bytes: u64,
}

static CACHE: Mutex<Option<(Instant, Vec<Proc>)>> = Mutex::new(None);

/// Rows for `kill <name>`: matching processes, best first, at most `limit`.
pub fn answers(query: &str, limit: usize) -> Vec<Answer> {
    let Some(needle) = parse_query(query) else {
        return Vec::new();
    };
    if limit == 0 {
        return Vec::new();
    }
    let procs = cached_procs();
    rank(&procs, &needle)
        .into_iter()
        .take(limit)
        .map(to_answer)
        .collect()
}

/// Send SIGTERM to `pid`.
pub fn terminate(pid: u32) -> std::io::Result<()> {
    let status = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    invalidate();
    let status = status?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "kill -TERM {pid} failed ({status})"
        )))
    }
}

/// Forget the cached listing.
fn invalidate() {
    if let Ok(mut cache) = CACHE.lock() {
        *cache = None;
    }
}

/// `Some(lowercased search text)` when `query` starts with the word `kill`; the text is
/// empty for a bare `kill`.
fn parse_query(query: &str) -> Option<String> {
    let q = query.trim_start();
    let head = q.get(..4)?;
    if !head.eq_ignore_ascii_case("kill") {
        return None;
    }
    let rest = &q[4..];
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None; // `killall`, `killer`, ...
    }
    Some(rest.trim().to_lowercase())
}

fn cached_procs() -> Vec<Proc> {
    let mut cache = match CACHE.lock() {
        Ok(c) => c,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some((at, procs)) = cache.as_ref()
        && at.elapsed() < CACHE_TTL
    {
        return procs.clone();
    }
    let procs = filter_own(list_procs(), std::process::id());
    *cache = Some((Instant::now(), procs.clone()));
    procs
}

/// Keep processes the current user owns, minus tomboy itself and pid 0/1. The current
/// uid is taken from tomboy's own entry in the listing.
fn filter_own(procs: Vec<Proc>, me: u32) -> Vec<Proc> {
    let Some(uid) = procs
        .iter()
        .find(|p| p.pid == me)
        .map(|p| p.uid)
        .or_else(current_uid)
    else {
        return Vec::new();
    };
    procs
        .into_iter()
        .filter(|p| p.uid == uid && p.pid != me && p.pid > 1)
        .collect()
}

/// Matching processes, best first: name-prefix matches, then other matches; within
/// each, CPU then memory descending. An empty needle matches everything.
fn rank<'a>(procs: &'a [Proc], needle: &str) -> Vec<&'a Proc> {
    let mut hits: Vec<(u8, &Proc)> = procs
        .iter()
        .filter_map(|p| {
            if needle.is_empty() {
                return Some((0, p));
            }
            let name = p.name.to_lowercase();
            if name.starts_with(needle) {
                Some((0, p))
            } else if name.contains(needle) || p.cmd.to_lowercase().contains(needle) {
                Some((1, p))
            } else {
                None
            }
        })
        .collect();
    hits.sort_by(|(ra, a), (rb, b)| {
        ra.cmp(rb)
            .then(b.cpu.total_cmp(&a.cpu))
            .then(b.rss_bytes.cmp(&a.rss_bytes))
            .then(a.pid.cmp(&b.pid))
    });
    hits.into_iter().map(|(_, p)| p).collect()
}

fn to_answer(p: &Proc) -> Answer {
    let mut subtitle = format!(
        "pid {} · {:.1}% CPU · {}",
        p.pid,
        p.cpu,
        format_mem(p.rss_bytes)
    );
    if !p.cmd.is_empty() {
        subtitle.push_str(" · ");
        subtitle.push_str(&shorten(&p.cmd, CMD_MAX));
    }
    Answer {
        kind: Kind::Process,
        title: p.name.clone(),
        subtitle,
        action: Action::Kill {
            pid: p.pid,
            name: p.name.clone(),
        },
    }
}

fn format_mem(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else {
        format!("{mb:.0} MB")
    }
}

/// `s` cut to at most `max` characters, ending in `…` when cut.
fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.truncate(out.trim_end().len());
    out.push('…');
    out
}

/// Last path component of `path`.
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Run `cmd args` and return its trimmed stdout, if it succeeded.
fn command_output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Fallback when tomboy's own pid is missing from the listing.
fn current_uid() -> Option<u32> {
    command_output("id", &["-u"])?.parse().ok()
}

// ---------------------------------------------------------------- Linux

#[cfg(any(target_os = "linux", test))]
/// Fields of `/proc/<pid>/stat` that we use.
#[derive(Debug, PartialEq)]
struct Stat {
    comm: String,
    /// utime + stime, in clock ticks.
    ticks: u64,
}

#[cfg(any(target_os = "linux", test))]
/// Parse a `/proc/<pid>/stat` line. The name sits in parentheses and may itself contain
/// spaces and parentheses, so it runs from the first `(` to the *last* `)`.
fn parse_stat(line: &str) -> Option<Stat> {
    let open = line.find('(')?;
    let close = line.rfind(')')?;
    if close < open {
        return None;
    }
    let comm = line[open + 1..close].to_string();
    // After `)`: field 3 (state) is index 0, so utime (14) is 11 and stime (15) is 12.
    let rest: Vec<&str> = line[close + 1..].split_whitespace().collect();
    let utime: u64 = rest.get(11)?.parse().ok()?;
    let stime: u64 = rest.get(12)?.parse().ok()?;
    Some(Stat {
        comm,
        ticks: utime + stime,
    })
}

#[cfg(any(target_os = "linux", test))]
/// Real uid from the text of `/proc/<pid>/status`.
fn parse_status_uid(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|uid| uid.parse().ok())
}

#[cfg(any(target_os = "linux", test))]
/// Process name: `comm`, unless the kernel truncated it to 15 bytes and argv[0]'s
/// basename is the longer original.
fn proc_name(comm: &str, argv0: &str) -> String {
    let base = basename(argv0);
    if comm.len() >= 15 && base.len() > comm.len() && base.starts_with(comm) {
        base.to_string()
    } else {
        comm.to_string()
    }
}

#[cfg(target_os = "linux")]
fn sysconf(name: &str, default: u64) -> u64 {
    command_output("getconf", &[name])
        .and_then(|s| s.parse().ok())
        .filter(|&v| v > 0)
        .unwrap_or(default)
}

#[cfg(target_os = "linux")]
fn page_size() -> u64 {
    static V: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *V.get_or_init(|| sysconf("PAGESIZE", 4096))
}

#[cfg(target_os = "linux")]
fn clk_tck() -> u64 {
    static V: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *V.get_or_init(|| sysconf("CLK_TCK", 100))
}

/// Numeric entries of `/proc`.
#[cfg(target_os = "linux")]
fn proc_pids() -> Vec<u32> {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
        .collect()
}

#[cfg(target_os = "linux")]
fn read_ticks(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    Some(parse_stat(&stat)?.ticks)
}

/// One full `/proc` read of `pid`, with `cpu` still zero. `None` for kernel threads
/// (empty cmdline) and processes that vanished.
#[cfg(target_os = "linux")]
fn read_proc(pid: u32) -> Option<(Proc, u64)> {
    let base = format!("/proc/{pid}");
    let raw = std::fs::read(format!("{base}/cmdline")).ok()?;
    if raw.is_empty() {
        return None;
    }
    let args: Vec<String> = raw
        .split(|&b| b == 0)
        .filter(|a| !a.is_empty())
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    let stat = parse_stat(&std::fs::read_to_string(format!("{base}/stat")).ok()?)?;
    let uid = parse_status_uid(&std::fs::read_to_string(format!("{base}/status")).ok()?)?;
    let rss_pages: u64 = std::fs::read_to_string(format!("{base}/statm"))
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse().ok())
        .unwrap_or(0);
    let name = proc_name(&stat.comm, args.first().map(String::as_str).unwrap_or(""));
    let proc = Proc {
        pid,
        uid,
        name,
        cmd: args.join(" "),
        cpu: 0.0,
        rss_bytes: rss_pages * page_size(),
    };
    Some((proc, stat.ticks))
}

#[cfg(target_os = "linux")]
fn list_procs() -> Vec<Proc> {
    use std::collections::HashMap;
    let start = Instant::now();
    let before: HashMap<u32, u64> = proc_pids()
        .into_iter()
        .filter_map(|pid| Some((pid, read_ticks(pid)?)))
        .collect();
    std::thread::sleep(SAMPLE_GAP);
    let secs = start.elapsed().as_secs_f64().max(0.001);
    let tck = clk_tck() as f64;
    proc_pids()
        .into_iter()
        .filter_map(read_proc)
        .map(|(mut p, ticks)| {
            if let Some(&t0) = before.get(&p.pid) {
                p.cpu = ticks.saturating_sub(t0) as f64 / tck / secs * 100.0;
            }
            p
        })
        .collect()
}

// ---------------------------------------------------------------- macOS

#[cfg(any(target_os = "macos", test))]
/// Parse one line of `ps -axo pid=,uid=,pcpu=,rss=,comm=,args=` (rss in KiB).
///
/// `comm` and `args` may both contain spaces, so the split between them is where the
/// remainder's args begin with comm (or with a path ending in comm); failing that, comm
/// is the first word.
fn parse_ps_line(line: &str) -> Option<Proc> {
    let mut rest = line.trim_start();
    let mut field = || {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (f, r) = rest.split_at(end);
        rest = r.trim_start();
        f
    };
    let pid: u32 = field().parse().ok()?;
    let uid: u32 = field().parse().ok()?;
    let cpu: f64 = field().replace(',', ".").parse().ok()?;
    let rss_kib: u64 = field().parse().ok()?;
    let tail = rest.trim_end();
    if tail.is_empty() {
        return None;
    }
    let splits: Vec<(&str, &str)> = tail
        .match_indices(' ')
        .map(|(i, _)| (tail[..i].trim_end(), tail[i..].trim_start()))
        .filter(|(c, a)| !c.is_empty() && !a.is_empty())
        .collect();
    let (comm, args) = splits
        .iter()
        .find(|(c, a)| a.starts_with(c))
        .or_else(|| {
            splits.iter().find(|(c, a)| {
                let argv0_end = a.find(" -").unwrap_or(a.len());
                a[..argv0_end].ends_with(&format!("/{}", basename(c)))
            })
        })
        .copied()
        .unwrap_or_else(|| splits.first().copied().unwrap_or((tail, tail)));
    Some(Proc {
        pid,
        uid,
        name: basename(comm).to_string(),
        cmd: args.to_string(),
        cpu,
        rss_bytes: rss_kib * 1024,
    })
}

#[cfg(target_os = "macos")]
fn list_procs() -> Vec<Proc> {
    let Some(out) = command_output("ps", &["-axo", "pid=,uid=,pcpu=,rss=,comm=,args="]) else {
        return Vec::new();
    };
    out.lines().filter_map(parse_ps_line).collect()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn list_procs() -> Vec<Proc> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, name: &str, cmd: &str, cpu: f64, rss_mb: u64) -> Proc {
        Proc {
            pid,
            uid: 1000,
            name: name.into(),
            cmd: cmd.into(),
            cpu,
            rss_bytes: rss_mb << 20,
        }
    }

    #[test]
    fn stat_with_spaces_and_parens_in_name() {
        let line = "4242 (Web (Content) x) S 1 4242 4242 0 -1 4194560 100 0 0 0 \
                    250 75 0 0 20 0 30 0 123456 1000000 2000 18446744073709551615";
        assert_eq!(
            parse_stat(line),
            Some(Stat {
                comm: "Web (Content) x".into(),
                ticks: 325
            })
        );
        assert_eq!(parse_stat("12 (sh) S 1 2"), None);
        assert_eq!(parse_stat("garbage"), None);
    }

    #[test]
    fn status_uid() {
        let s = "Name:\tbash\nState:\tS\nUid:\t1000\t1000\t1000\t1000\nGid:\t1000\n";
        assert_eq!(parse_status_uid(s), Some(1000));
        assert_eq!(parse_status_uid("Name:\tx\n"), None);
    }

    #[test]
    fn truncated_comm_uses_argv0() {
        assert_eq!(
            proc_name("Isolated Web Co", "/usr/lib/firefox/Isolated Web Content"),
            "Isolated Web Content"
        );
        assert_eq!(proc_name("bash", "-bash"), "bash");
        assert_eq!(
            proc_name("chromium", "/usr/lib/chromium/chromium"),
            "chromium"
        );
    }

    #[test]
    fn ps_lines() {
        let simple = "  4242   501   3.1  422000 /usr/bin/vim vim notes.txt";
        let got = parse_ps_line(simple).unwrap();
        assert_eq!(
            (got.pid, got.uid, got.cpu, got.rss_bytes),
            (4242, 501, 3.1, 422000 * 1024)
        );
        assert_eq!(got.name, "vim");
        assert_eq!(got.cmd, "vim notes.txt");

        let spaced = "  777 501  12.5 102400 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome \
                      /Applications/Google Chrome.app/Contents/MacOS/Google Chrome --flag";
        let got = parse_ps_line(spaced).unwrap();
        assert_eq!(got.name, "Google Chrome");
        assert_eq!(
            got.cmd,
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome --flag"
        );

        let helper = "900 501 0.0 2048 Google Chrome Helper (Renderer) \
                      /Applications/X.app/Google Chrome Helper (Renderer) --type=renderer";
        let got = parse_ps_line(helper).unwrap();
        assert_eq!(got.name, "Google Chrome Helper (Renderer)");
        assert!(got.cmd.starts_with("/Applications/X.app/"));

        let bare = "1 0 0.0 4000 /sbin/launchd";
        let got = parse_ps_line(bare).unwrap();
        assert_eq!(
            (got.name.as_str(), got.cmd.as_str()),
            ("launchd", "/sbin/launchd")
        );

        assert!(parse_ps_line("").is_none());
        assert!(parse_ps_line("abc 1 2 3 x").is_none());
    }

    #[test]
    fn query_trigger() {
        assert_eq!(parse_query("kill"), Some(String::new()));
        assert_eq!(parse_query("kill "), Some(String::new()));
        assert_eq!(parse_query("  KiLL  Chromium "), Some("chromium".into()));
        assert_eq!(parse_query("killall"), None);
        assert_eq!(parse_query("kil"), None);
        assert_eq!(parse_query("skill x"), None);
        assert_eq!(parse_query("ki€"), None);
    }

    #[test]
    fn non_kill_is_empty_and_lists_nothing() {
        invalidate();
        assert!(answers("chromium", 10).is_empty());
        assert!(answers("killall foo", 10).is_empty());
    }

    #[test]
    fn ranking() {
        let procs = vec![
            p(
                10,
                "electron",
                "/usr/bin/electron --app chromium-ish",
                50.0,
                100,
            ),
            p(11, "chromium", "/usr/lib/chromium/chromium", 1.0, 900),
            p(
                12,
                "chromium",
                "/usr/lib/chromium/chromium --type=renderer",
                1.0,
                400,
            ),
            p(
                13,
                "chromium",
                "/usr/lib/chromium/chromium --type=gpu",
                5.0,
                10,
            ),
            p(14, "vim", "vim", 99.0, 1),
        ];
        let order: Vec<u32> = rank(&procs, "chrom").iter().map(|p| p.pid).collect();
        assert_eq!(order, vec![13, 11, 12, 10]);
        let all: Vec<u32> = rank(&procs, "").iter().map(|p| p.pid).collect();
        assert_eq!(all, vec![14, 10, 13, 11, 12]);
        assert!(rank(&procs, "nope").is_empty());
    }

    #[test]
    fn filtering() {
        let mut other = p(20, "root-thing", "x", 0.0, 1);
        other.uid = 0;
        let procs = vec![
            p(1, "init", "init", 0.0, 1),
            p(99, "tomboy", "tomboy", 0.0, 1),
            other,
            p(30, "a", "a", 0.0, 1),
        ];
        let kept: Vec<u32> = filter_own(procs, 99).iter().map(|p| p.pid).collect();
        assert_eq!(kept, vec![30]);
    }

    #[test]
    fn answer_shape() {
        let long = format!("/usr/lib/chromium/chromium {}", "--flag ".repeat(20));
        let a = to_answer(&p(4242, "chromium", &long, 3.12, 412));
        assert_eq!(a.kind, Kind::Process);
        assert_eq!(a.title, "chromium");
        assert!(
            a.subtitle
                .starts_with("pid 4242 · 3.1% CPU · 412 MB · /usr/lib/chromium"),
            "{}",
            a.subtitle
        );
        assert!(a.subtitle.ends_with('…'));
        assert!(a.subtitle.split(" · ").nth(3).unwrap().chars().count() <= CMD_MAX);
        assert_eq!(
            a.action,
            Action::Kill {
                pid: 4242,
                name: "chromium".into()
            }
        );
        assert_eq!(format_mem(3 << 30), "3.0 GB");
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn live_find_and_terminate_own_sleep() {
        let mut child = std::process::Command::new("sleep")
            .arg("300")
            .spawn()
            .expect("spawn sleep");
        let pid = child.id();
        invalidate();
        let rows = answers("kill sleep", 50);
        let found = rows.iter().any(|a| {
            a.action
                == Action::Kill {
                    pid,
                    name: "sleep".into(),
                }
        });
        if !found {
            let _ = child.kill();
            panic!("sleep {pid} not found in {rows:?}");
        }
        assert!(
            answers("kill", 1000)
                .iter()
                .all(|a| !matches!(a.action, Action::Kill { pid: 1, .. }))
        );
        terminate(pid).expect("terminate");
        let status = child.wait().expect("wait");
        assert!(!status.success());
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(status.signal(), Some(15));
        }
    }
}
