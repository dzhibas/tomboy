//! One launched window at a time: the launch hotkey toggles it. The first `tomboy` listens on
//! a Unix socket; a later launch finds it there, asks it to close, and exits without opening
//! a window. The next launch then finds no one listening and opens a fresh one.

use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use rune::Waker;

/// Another launch asked this window to close (stays set).
static CLOSE: AtomicBool = AtomicBool::new(false);
/// That request, until `poll` acts on it.
static PENDING: AtomicBool = AtomicBool::new(false);
/// The window's waker, once `setup` has one, so a request redraws (and so closes) it.
static WAKER: OnceLock<Mutex<Option<Waker>>> = OnceLock::new();

fn socket_path() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()).map(PathBuf::from)
        .or_else(|| crate::sys::state_dir().map(|d| d.join("tomboy")))?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("tomboy.sock"))
}

/// Before the window opens: if a tomboy window is already up, close it and return `false`
/// (this launch should exit). Otherwise start listening for later launches and return `true`.
pub fn claim() -> bool {
    let Some(path) = socket_path() else { return true };
    if UnixStream::connect(&path).is_ok() { return false; } // it closes on the connection
    // No one listening: a socket left by a crash (or none). Take it over.
    let _ = std::fs::remove_file(&path);
    let Ok(listener) = UnixListener::bind(&path) else { return true };
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            if conn.is_err() { continue; }
            CLOSE.store(true, Ordering::Release);
            PENDING.store(true, Ordering::Release);
            if let Some(wake) = WAKER.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner()).as_ref() { wake(); }
            // This window is closing: stop answering now, so a quick next press opens a new
            // one rather than reaching this one on its way out.
            let _ = std::fs::remove_file(&path);
            return;
        }
    });
    true
}

/// `setup`: remember the waker, and wake now if a request came in before there was one.
pub fn set_waker(waker: Waker) {
    *WAKER.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner()) = Some(waker.clone());
    if PENDING.load(Ordering::Acquire) { waker(); }
}

/// `poll`: another launch asked this window to close.
pub fn close_requested() -> bool {
    PENDING.swap(false, Ordering::AcqRel)
}

/// After the event loop (closed by Esc or Super+W): stop answering, so the next launch opens a
/// window straight away. Not if another launch's close already did, since by then a newer
/// window may own the path.
pub fn release() {
    if CLOSE.load(Ordering::Acquire) { return; }
    if let Some(path) = socket_path() { let _ = std::fs::remove_file(path); }
}
