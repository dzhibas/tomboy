//! Launch profiling: `TOMBOY_TRACE=1 tomboy` prints, to stderr, what each part of a launch costs
//! and when it happened, from `main` (or from the keypress, see below) to the process ending.
//!
//! - `TOMBOY_TRACE=1` (or `stderr`) writes to stderr; any other value is a file to append to,
//!   for a launch from a compositor key binding, whose stderr goes nowhere.
//! - `TOMBOY_T0` is the time the launch was asked for, in Unix nanoseconds (`date +%s%N`) or
//!   milliseconds. With it, every time is measured from the keypress and the first line says
//!   how long exec, the dynamic loader and Rust's runtime took before `main`.
//!
//! Each line: `[tomboy +<ms since t0>] <label>` and, for timed steps, their duration. Besides
//! one-off steps it reports the first frames, any later slow view, `results()` call or frame,
//! and a summary of counts at exit (views, searches and background wakes per drawn frame).
//!
//! Unset, every call is one `OnceLock` read and a branch.

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// A view that takes longer than this is reported after the first frame.
const SLOW_VIEW_MS: f64 = 4.0;
/// A `results()` call that takes longer than this is reported.
const SLOW_RESULTS_MS: f64 = 1.0;
/// A frame whose update plus encode takes longer than this is reported after the first frames.
const SLOW_FRAME_MS: f64 = 8.0;
/// Frames reported whatever they cost, from the first.
const FIRST_FRAMES: u64 = 3;

struct Tracer {
    t0: Instant,
    out: Mutex<Box<dyn Write + Send>>,
    seen: Mutex<Vec<&'static str>>,
}

static TRACER: OnceLock<Option<Tracer>> = OnceLock::new();

static VIEWS: AtomicU64 = AtomicU64::new(0);
static RESULTS: AtomicU64 = AtomicU64::new(0);
static WAKES: AtomicU64 = AtomicU64::new(0);
static FRAMES: AtomicU64 = AtomicU64::new(0);
static DRAWN: AtomicU64 = AtomicU64::new(0);
/// Counts at the first drawn frame, for the summary: what it took to get there.
static AT_FIRST: Mutex<Option<(u64, u64, u64)>> = Mutex::new(None);

/// Reads the environment; call first thing in `main`, so times start there.
pub fn start() {
    let now = Instant::now();
    TRACER.get_or_init(|| {
        let dest = std::env::var_os("TOMBOY_TRACE").filter(|v| !v.is_empty() && v != "0")?;
        let out: Box<dyn Write + Send> = if dest == "1" || dest == "stderr" {
            Box::new(std::io::stderr())
        } else {
            match std::fs::OpenOptions::new().create(true).append(true).open(&dest) {
                Ok(f) => Box::new(f),
                Err(e) => { eprintln!("tomboy: TOMBOY_TRACE {}: {e}", dest.to_string_lossy()); Box::new(std::io::stderr()) }
            }
        };
        let pre_main = requested_ago();
        let t0 = pre_main.and_then(|d| now.checked_sub(d)).unwrap_or(now);
        let t = Tracer { t0, out: Mutex::new(out), seen: Mutex::new(Vec::new()) };
        let args: Vec<String> = std::env::args().skip(1).collect();
        t.line(now, &format!("---- launch pid {} args {args:?}", std::process::id()));
        if let Some(d) = pre_main { t.line(now, &format!("main (exec → main {:.2} ms)", ms(d))); } else { t.line(now, "main"); }
        Some(t)
    });
}

/// How long ago `TOMBOY_T0` was, if it is set and in the past.
fn requested_ago() -> Option<Duration> {
    let raw: u128 = std::env::var("TOMBOY_T0").ok()?.trim().parse().ok()?;
    // Nanoseconds have 19 digits today, milliseconds 13.
    let at = if raw > 10u128.pow(15) { Duration::from_nanos(raw as u64) } else { Duration::from_millis(raw as u64) };
    SystemTime::now().duration_since(UNIX_EPOCH).ok()?.checked_sub(at)
}

impl Tracer {
    fn line(&self, at: Instant, msg: &str) {
        let since = at.saturating_duration_since(self.t0);
        let mut out = self.out.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writeln!(out, "[tomboy +{:8.2}ms] {msg}", ms(since));
    }
}

fn ms(d: Duration) -> f64 { d.as_secs_f64() * 1000.0 }

fn tracer() -> Option<&'static Tracer> { TRACER.get().and_then(Option::as_ref) }

pub fn on() -> bool { tracer().is_some() }

/// A point in time.
pub fn mark(label: &str) {
    if let Some(t) = tracer() { t.line(Instant::now(), label); }
}

/// A point in time, reported only the first time `label` is seen.
pub fn first(label: &'static str) {
    let Some(t) = tracer() else { return };
    {
        let mut seen = t.seen.lock().unwrap_or_else(|e| e.into_inner());
        if seen.contains(&label) { return; }
        seen.push(label);
    }
    t.line(Instant::now(), label);
}

/// Runs `f` and reports how long it took.
pub fn span<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let Some(t) = tracer() else { return f() };
    let start = Instant::now();
    let r = f();
    let end = Instant::now();
    t.line(end, &format!("{label:<34} {:7.2} ms", ms(end - start)));
    r
}

/// A step that began at `start` and ended now.
pub fn since(label: &str, start: Instant) {
    if let Some(t) = tracer() {
        let end = Instant::now();
        t.line(end, &format!("{label:<34} {:7.2} ms", ms(end - start)));
    }
}

/// A view built since `start`: the first is always reported, later ones when slow.
pub fn view(start: Instant) {
    let Some(t) = tracer() else { return };
    let n = VIEWS.fetch_add(1, Ordering::Relaxed);
    let took = ms(start.elapsed());
    if n == 0 { t.line(Instant::now(), &format!("{:<34} {took:7.2} ms", "first view()")); }
    else if took > SLOW_VIEW_MS { t.line(Instant::now(), &format!("{:<34} {took:7.2} ms", format!("slow view() #{n}"))); }
}

/// A `results()` call that began at `start`, reported when slow.
pub fn results(start: Instant, rows: usize) {
    let Some(t) = tracer() else { return };
    let n = RESULTS.fetch_add(1, Ordering::Relaxed);
    let took = ms(start.elapsed());
    if took > SLOW_RESULTS_MS { t.line(Instant::now(), &format!("{:<34} {took:7.2} ms ({rows} rows)", format!("slow results() #{n}"))); }
}

/// A background wake (an icon decoded, a menu condition checked, …).
pub fn wake() {
    if on() { WAKES.fetch_add(1, Ordering::Relaxed); }
}

/// For `App::on_frame_encoded`: the first frames, then slow ones.
pub fn frame(f: &rune::FrameEncoded) {
    let Some(t) = tracer() else { return };
    let n = FRAMES.fetch_add(1, Ordering::Relaxed);
    let drawn = f.outcome() == rune::FrameOutcome::Drawn;
    let d = if drawn { DRAWN.fetch_add(1, Ordering::Relaxed) } else { DRAWN.load(Ordering::Relaxed) };
    if drawn && d == 0 {
        let counts = (VIEWS.load(Ordering::Relaxed), RESULTS.load(Ordering::Relaxed), WAKES.load(Ordering::Relaxed));
        *AT_FIRST.lock().unwrap_or_else(|e| e.into_inner()) = Some(counts);
        // `TOMBOY_TRACE_QUIT=<ms>`: end the process that long after the first frame, so a script
        // can time real windowed launches one after another.
        if let Some(after) = std::env::var("TOMBOY_TRACE_QUIT").ok().and_then(|v| v.parse::<u64>().ok()) {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(after));
                mark("TOMBOY_TRACE_QUIT");
                summary();
                std::process::exit(0);
            });
        }
    }
    let (update, encode) = (f.update_ms().unwrap_or(0.0), f.encode_ms().unwrap_or(0.0));
    if n >= FIRST_FRAMES && !(drawn && update + encode > SLOW_FRAME_MS) { return; }
    let label = if drawn && d == 0 { "first frame drawn".to_string() }
        else if n < FIRST_FRAMES { format!("frame #{n} {:?}", f.outcome()) }
        else { format!("slow frame #{n}") };
    let presents = f.presents_at().map(|p| format!(", presents +{:.2} ms", ms(p.saturating_duration_since(f.at())))).unwrap_or_default();
    t.line(f.at(), &format!("{label:<34} update {update:.2} ms, encode {encode:.2} ms{presents}"));
}

/// The counts so far: what the launch did per frame, and Rune's own launch steps
/// (`rune::marks`: event loop, window, GPU, renderer, fonts, first frame) on the same clock,
/// each with the gap since the step before. Call at exit.
pub fn summary() {
    let Some(t) = tracer() else { return };
    let mut prev = t.t0;
    let mut lines = vec!["rune launch steps (gap since previous):".to_string()];
    for (name, at) in rune::marks::marks() {
        lines.push(format!("  +{:8.2}ms  {name:<26} {:+7.2} ms", ms(at.saturating_duration_since(t.t0)), ms(at.saturating_duration_since(prev))));
        prev = at;
    }
    for (name, v) in rune::marks::metrics() {
        let v = if name.ends_with("_ns") { format!("{:.2} ms", v as f64 / 1e6) } else { v.to_string() };
        lines.push(format!("  metric {name:<26} {v}"));
    }
    for (name, v) in rune::marks::notes() { lines.push(format!("  note   {name:<26} {v}")); }
    t.line(Instant::now(), &lines.join("\n"));
    let get = |c: &AtomicU64| c.load(Ordering::Relaxed);
    let first = match *AT_FIRST.lock().unwrap_or_else(|e| e.into_inner()) {
        Some((v, r, w)) => format!("; before the first frame: {v} views, {r} results(), {w} wakes"),
        None => String::new(),
    };
    t.line(Instant::now(), &format!(
        "summary: {} frames ({} drawn), {} views, {} results(), {} wakes{first}",
        get(&FRAMES), get(&DRAWN), get(&VIEWS), get(&RESULTS), get(&WAKES),
    ));
}
