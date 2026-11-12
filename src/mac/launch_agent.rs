//! `tomboy install-launch-agent`: run `tomboy daemon` at login (and again if it crashes)
//! through a LaunchAgent in `~/Library/LaunchAgents`, the way a Mac app's login item would.

use std::io;
use std::path::PathBuf;
use std::process::Command;

pub const LABEL: &str = "io.github.dzhibas.tomboy";

fn plist_path() -> io::Result<PathBuf> {
    let home = crate::sys::home_dir().ok_or_else(|| io::Error::other("no home directory"))?;
    Ok(home.join(format!("Library/LaunchAgents/{LABEL}.plist")))
}

fn domain() -> String {
    let uid = Command::new("id").arg("-u").output().ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    format!("gui/{uid}")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The plist: this binary with `daemon`, kept alive unless it quits cleanly ("Quit tomboy"),
/// with this shell's PATH so the agent CLI (`claude` in /opt/homebrew/bin) is found.
fn plist(exe: &str, path: &str, log: &str) -> String {
    let (exe, path, log) = (xml_escape(exe), xml_escape(path), xml_escape(log));
    format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>{exe}</string><string>daemon</string></array>
  <key>EnvironmentVariables</key>
  <dict><key>PATH</key><string>{path}</string></dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key>
  <dict><key>SuccessfulExit</key><false/></dict>
  <key>ProcessType</key><string>Interactive</string>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#)
}

/// Write the plist and (re)load it, so the daemon starts now and at every login. Running it
/// again after `cargo install` or a `[hotkeys]` change restarts the daemon on the new build.
pub fn install() -> io::Result<()> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let home = crate::sys::home_dir().ok_or_else(|| io::Error::other("no home directory"))?;
    let log = home.join("Library/Logs/tomboy.log");
    let file = plist_path()?;
    std::fs::create_dir_all(file.parent().expect("LaunchAgents has a parent"))?;
    let path = std::env::var("PATH").unwrap_or_else(|_| "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".into());
    std::fs::write(&file, plist(&exe.to_string_lossy(), &path, &log.to_string_lossy()))?;
    let _ = Command::new("launchctl").args(["bootout", &format!("{}/{LABEL}", domain())]).output();
    // bootout returns before the old daemon is gone; until then bootstrap fails (error 5).
    let mut tries = 0;
    loop {
        let out = Command::new("launchctl").args(["bootstrap", &domain()]).arg(&file).output()?;
        if out.status.success() { break; }
        tries += 1;
        if tries == 20 {
            return Err(io::Error::other(format!("launchctl bootstrap: {}", String::from_utf8_lossy(&out.stderr).trim())));
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    println!("tomboy daemon runs now and at login ({})\nlog: {}", file.display(), log.display());
    Ok(())
}

/// Stop the daemon and remove the plist.
pub fn uninstall() -> io::Result<()> {
    let file = plist_path()?;
    let _ = Command::new("launchctl").args(["bootout", &format!("{}/{LABEL}", domain())]).output();
    match std::fs::remove_file(&file) {
        Ok(()) => println!("removed {}", file.display()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => println!("no launch agent installed"),
        Err(e) => return Err(e),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn plist_escapes_paths() {
        let p = super::plist("/Users/a&b/tomboy", "/bin", "/tmp/log");
        assert!(p.contains("<string>/Users/a&amp;b/tomboy</string><string>daemon</string>"));
        assert!(p.contains("<key>SuccessfulExit</key><false/>"));
    }
}
