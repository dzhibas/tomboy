//! Every OS command tomboy shells out to, behind one function per job, so the rest of the
//! app never names `xdg-open`, `open`, `wl-copy` or `pbcopy` itself.

use std::io::{self, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("tomboy supports Linux and macOS");

/// Start `cmd` in its own process group with no stdio, so it outlives tomboy; a thread
/// reaps it so no zombie is left while tomboy runs.
pub fn spawn_detached(cmd: &mut Command) -> io::Result<()> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .process_group(0).spawn()?;
    std::thread::spawn(move || { let _ = child.wait(); });
    Ok(())
}

/// Open a URL (or file) in the default handler: `xdg-open` on Linux, `open` on macOS.
pub fn open_url(url: &str) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    let program = "xdg-open";
    #[cfg(target_os = "macos")]
    let program = "open";
    spawn_detached(Command::new(program).arg(url))
}

/// Put text on the system clipboard through a helper process that keeps serving it after
/// tomboy quits (on Wayland the clipboard dies with its owner). `Ok(false)` when no helper
/// is installed, so the caller can copy in-process instead.
pub fn copy_to_clipboard(text: &str) -> io::Result<bool> {
    for argv in clipboard_helpers() {
        let spawned = Command::new(argv[0]).args(&argv[1..])
            .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        let Ok(mut child) = spawned else { continue };
        if let Some(mut stdin) = child.stdin.take() { stdin.write_all(text.as_bytes())?; }
        return child.wait().map(|s| s.success());
    }
    Ok(false)
}

#[cfg(target_os = "linux")]
fn clipboard_helpers() -> Vec<&'static [&'static str]> {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let x11: [&'static [&'static str]; 2] = [&["xclip", "-selection", "clipboard"], &["xsel", "--clipboard", "--input"]];
    if wayland { [&["wl-copy"][..]].into_iter().chain(x11).collect() } else { x11.into() }
}

#[cfg(target_os = "macos")]
fn clipboard_helpers() -> Vec<&'static [&'static str]> {
    vec![&["pbcopy"]]
}

/// The user's home directory.
pub fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(Into::into)
}
