//! Linux application discovery and launching.
//!
//! Implements the relevant parts of the freedesktop.org Desktop Entry
//! specification (directory precedence, desktop file ids, value escapes,
//! Exec quoting and field codes, TryExec, OnlyShowIn/NotShowIn) using only
//! the standard library.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::AppEntry;

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Iterate `(key, raw_value)` pairs of the `[Desktop Entry]` group,
/// skipping comments, blank lines and localized keys (`Name[de]`).
fn desktop_group_pairs(contents: &str) -> impl Iterator<Item = (&str, &str)> {
    let mut in_group = false;
    contents.lines().filter_map(move |line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
            return None;
        }
        if !in_group {
            return None;
        }
        let (key, value) = line.split_once('=')?;
        let key = key.trim();
        if key.is_empty() || key.contains('[') {
            return None;
        }
        Some((key, value.trim()))
    })
}

/// Decode desktop-entry string escapes: `\s \n \t \r \\`.
/// Unknown escapes are kept verbatim (backslash included).
pub fn unescape_value(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Split a `;`-separated list value (honouring `\;`), decoding escapes and
/// dropping empty items.
pub fn parse_list(raw: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut cur = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(';') => cur.push(';'),
                Some(other) => {
                    cur.push('\\');
                    cur.push(other);
                }
                None => cur.push('\\'),
            },
            ';' => {
                let item = unescape_value(cur.trim());
                if !item.is_empty() {
                    items.push(item);
                }
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    let item = unescape_value(cur.trim());
    if !item.is_empty() {
        items.push(item);
    }
    items
}

fn parse_bool(raw: &str) -> bool {
    raw.trim().eq_ignore_ascii_case("true") || raw.trim() == "1"
}

fn non_empty(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

/// Parse a desktop file's `[Desktop Entry]` group into an [`AppEntry`].
///
/// Returns `None` for non-applications, `NoDisplay=true`, `Hidden=true`, or
/// entries missing `Name`/`Exec`. Does not check `TryExec` or
/// `OnlyShowIn`/`NotShowIn` (that is done by [`load_apps`]).
pub fn parse_entry(id: &str, path: &Path, contents: &str) -> Option<AppEntry> {
    let mut ty = None;
    let mut name = None;
    let mut generic_name = None;
    let mut comment = None;
    let mut exec = None;
    let mut icon = None;
    let mut keywords = Vec::new();
    let mut terminal = false;
    let mut no_display = false;
    let mut hidden = false;

    for (key, value) in desktop_group_pairs(contents) {
        match key {
            "Type" => ty = Some(unescape_value(value)),
            "Name" => name = Some(unescape_value(value)),
            "GenericName" => generic_name = Some(unescape_value(value)),
            "Comment" => comment = Some(unescape_value(value)),
            "Exec" => exec = Some(unescape_value(value)),
            "Icon" => icon = Some(unescape_value(value)),
            "Keywords" => keywords = parse_list(value),
            "Terminal" => terminal = parse_bool(value),
            "NoDisplay" => no_display = parse_bool(value),
            "Hidden" => hidden = parse_bool(value),
            _ => {}
        }
    }

    if ty.as_deref() != Some("Application") || no_display || hidden {
        return None;
    }
    let exec = exec.filter(|e| !e.trim().is_empty())?;
    let name = name.filter(|n| !n.trim().is_empty())?;

    Some(AppEntry {
        id: id.to_string(),
        name,
        generic_name: generic_name.and_then(non_empty),
        comment: comment.and_then(non_empty),
        exec,
        icon: icon.and_then(non_empty),
        keywords,
        terminal,
        path: path.to_path_buf(),
    })
}

// ---------------------------------------------------------------------------
// Exec parsing
// ---------------------------------------------------------------------------

fn is_field_code(c: char) -> bool {
    matches!(
        c,
        'f' | 'F' | 'u' | 'U' | 'd' | 'D' | 'n' | 'N' | 'i' | 'c' | 'k' | 'v' | 'm'
    )
}

/// Parse an Exec= value into argv following the Desktop Entry quoting rules.
///
/// Double-quoted arguments support `\" \` \$ \\` escapes. Field codes
/// (`%f %F %u %U %d %D %n %N %i %c %k %v %m`) are removed; an argument that
/// consisted solely of field codes is dropped. `%%` becomes `%`.
pub fn exec_argv(exec: &str) -> Vec<String> {
    let mut argv = Vec::new();
    let mut cur = String::new();
    let mut in_token = false;
    let mut quoted = false; // token contained a quoted section
    let mut had_code = false;
    let mut in_quotes = false;

    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' => in_quotes = false,
                '\\' => match chars.peek() {
                    Some(&n @ ('"' | '`' | '$' | '\\')) => {
                        cur.push(n);
                        chars.next();
                    }
                    _ => cur.push('\\'),
                },
                '%' => handle_percent(&mut chars, &mut cur, &mut had_code),
                _ => cur.push(c),
            }
            continue;
        }
        match c {
            ' ' | '\t' | '\n' | '\r' => {
                if in_token {
                    finish_token(&mut argv, &mut cur, quoted, had_code);
                    in_token = false;
                    quoted = false;
                    had_code = false;
                }
            }
            '"' => {
                in_token = true;
                quoted = true;
                in_quotes = true;
            }
            '\\' => {
                in_token = true;
                match chars.next() {
                    Some(n) => cur.push(n),
                    None => cur.push('\\'),
                }
            }
            '%' => {
                in_token = true;
                handle_percent(&mut chars, &mut cur, &mut had_code);
            }
            _ => {
                in_token = true;
                cur.push(c);
            }
        }
    }
    if in_token {
        finish_token(&mut argv, &mut cur, quoted, had_code);
    }
    argv
}

fn handle_percent(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    cur: &mut String,
    had_code: &mut bool,
) {
    match chars.peek() {
        Some('%') => {
            cur.push('%');
            chars.next();
        }
        Some(&n) if is_field_code(n) => {
            *had_code = true;
            chars.next();
        }
        _ => cur.push('%'),
    }
}

fn finish_token(argv: &mut Vec<String>, cur: &mut String, quoted: bool, had_code: bool) {
    if cur.is_empty() && had_code && !quoted {
        return;
    }
    argv.push(std::mem::take(cur));
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

fn env_nonempty(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.is_empty())
}

fn home_dir() -> Option<PathBuf> {
    env_nonempty("HOME").map(PathBuf::from)
}

/// Data directories in precedence order (highest first).
pub fn data_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let home = home_dir();

    if let Some(d) = env_nonempty("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(d));
    } else if let Some(h) = &home {
        dirs.push(h.join(".local/share"));
    }

    let sys = env_nonempty("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.extend(sys.split(':').filter(|s| !s.is_empty()).map(PathBuf::from));

    let mut flatpak = Vec::new();
    if let Some(h) = &home {
        flatpak.push(h.join(".local/share/flatpak/exports/share"));
    }
    flatpak.push(PathBuf::from("/var/lib/flatpak/exports/share"));

    let norm = |p: &Path| p.to_string_lossy().trim_end_matches('/').to_string();
    for f in flatpak {
        if !dirs.iter().any(|d| norm(d) == norm(&f)) {
            dirs.push(f);
        }
    }

    // Drop duplicates while preserving order.
    let mut seen = HashSet::new();
    dirs.retain(|d| seen.insert(norm(d)));
    dirs
}

fn collect_desktop_files(base: &Path, dir: &Path, depth: usize, out: &mut Vec<(String, PathBuf)>) {
    if depth > 8 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for ent in rd.flatten() {
        let path = ent.path();
        let Ok(ft) = ent.file_type() else { continue };
        let is_dir = if ft.is_symlink() {
            fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false)
        } else {
            ft.is_dir()
        };
        if is_dir {
            collect_desktop_files(base, &path, depth + 1, out);
        } else if path.extension().is_some_and(|e| e == "desktop") {
            let Ok(rel) = path.strip_prefix(base) else { continue };
            let id = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("-");
            out.push((id, path));
        }
    }
}

struct Env {
    path_dirs: Vec<PathBuf>,
    desktops: Vec<String>,
}

impl Env {
    fn current() -> Self {
        let path_dirs = env::var_os("PATH")
            .map(|p| env::split_paths(&p).filter(|d| !d.as_os_str().is_empty()).collect())
            .unwrap_or_default();
        let desktops = env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .split(':')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase())
            .collect();
        Env { path_dirs, desktops }
    }
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn find_program(prog: &str, path_dirs: &[PathBuf]) -> Option<PathBuf> {
    if prog.is_empty() {
        return None;
    }
    if prog.contains('/') {
        let p = PathBuf::from(prog);
        return is_executable(&p).then_some(p);
    }
    path_dirs.iter().map(|d| d.join(prog)).find(|p| is_executable(p))
}

/// Checks TryExec and OnlyShowIn/NotShowIn for an entry's contents.
fn passes_env_filters(contents: &str, env: &Env) -> bool {
    for (key, value) in desktop_group_pairs(contents) {
        match key {
            "TryExec" => {
                let prog = unescape_value(value);
                if !prog.is_empty() && find_program(&prog, &env.path_dirs).is_none() {
                    return false;
                }
            }
            "OnlyShowIn" => {
                let list = parse_list(value);
                if !list.is_empty()
                    && !list.iter().any(|d| env.desktops.contains(&d.to_ascii_lowercase()))
                {
                    return false;
                }
            }
            "NotShowIn"
                if parse_list(value)
                    .iter()
                    .any(|d| env.desktops.contains(&d.to_ascii_lowercase()))
                => {
                    return false;
                }
            _ => {}
        }
    }
    true
}

/// Discover all launchable applications, sorted by name (case-insensitive).
pub fn load_apps() -> Vec<AppEntry> {
    let env = Env::current();
    let mut seen: HashSet<String> = HashSet::new();
    let mut apps = Vec::new();

    for data_dir in data_dirs() {
        let base = data_dir.join("applications");
        let mut files = Vec::new();
        collect_desktop_files(&base, &base, 0, &mut files);
        files.sort();
        for (id, path) in files {
            // The first file with a given id shadows all later ones, even if
            // it ends up hidden.
            if !seen.insert(id.clone()) {
                continue;
            }
            let Ok(contents) = fs::read_to_string(&path) else { continue };
            let Some(entry) = parse_entry(&id, &path, &contents) else { continue };
            if !passes_env_filters(&contents, &env) {
                continue;
            }
            apps.push(entry);
        }
    }

    apps.sort_by_cached_key(|a| (a.name.to_lowercase(), a.id.clone()));
    apps
}

/// The program an Exec= line runs, lowercased (`/usr/bin/code %F` → `code`), for search.
pub fn program_name(exec: &str) -> String {
    let argv = exec_argv(exec);
    let prog = argv
        .iter()
        .find(|a| a.as_str() != "env" && !a.contains('=') && !a.starts_with('-'))
        .map(String::as_str)
        .unwrap_or("");
    prog.rsplit('/').next().unwrap_or("").to_lowercase()
}

// ---------------------------------------------------------------------------
// Launching
// ---------------------------------------------------------------------------

/// Build the full command line used to launch `app` (terminal and uwsm
/// wrapping included).
pub fn launch_argv(app: &AppEntry) -> io::Result<Vec<String>> {
    let env = Env::current();
    let mut argv = exec_argv(&app.exec);
    if argv.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("empty Exec for {} ({})", app.id, app.path.display()),
        ));
    }
    if app.terminal {
        let mut term: Vec<String> = if find_program("xdg-terminal-exec", &env.path_dirs).is_some() {
            vec!["xdg-terminal-exec".into()]
        } else if let Some(t) = env_nonempty("TERMINAL") {
            vec![t, "-e".into()]
        } else {
            vec!["alacritty".into(), "-e".into()]
        };
        term.append(&mut argv);
        argv = term;
    }
    if find_program("uwsm", &env.path_dirs).is_some() {
        let mut wrapped: Vec<String> = vec!["uwsm".into(), "app".into(), "--".into()];
        wrapped.append(&mut argv);
        argv = wrapped;
    }
    Ok(argv)
}

/// Spawn `app` detached from the launcher (own process group, null stdio,
/// working directory $HOME). Does not wait for the child.
pub fn launch(app: &AppEntry) -> io::Result<()> {
    let argv = launch_argv(app)?;
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    if let Some(home) = home_dir() {
        cmd.current_dir(home);
    }
    crate::sys::spawn_detached(&mut cmd)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn exec_simple_and_field_codes() {
        assert_eq!(exec_argv("firefox %u"), v(&["firefox"]));
        assert_eq!(exec_argv("nautilus --new-window %U"), v(&["nautilus", "--new-window"]));
        assert_eq!(exec_argv("app %f %F %d %D %n %N %i %c %k %v %m"), v(&["app"]));
        assert_eq!(exec_argv("app --file=%f"), v(&["app", "--file="]));
        assert_eq!(exec_argv("  app   -a\t-b  "), v(&["app", "-a", "-b"]));
    }

    #[test]
    fn exec_percent_escape() {
        assert_eq!(exec_argv("printf 100%%"), v(&["printf", "100%"]));
        assert_eq!(exec_argv("app %%u"), v(&["app", "%u"]));
        assert_eq!(exec_argv("app %x"), v(&["app", "%x"]));
    }

    #[test]
    fn exec_quoting() {
        assert_eq!(
            exec_argv(r#""/opt/My App/bin/app" --flag"#),
            v(&["/opt/My App/bin/app", "--flag"])
        );
        assert_eq!(
            exec_argv(r#"sh -c "echo \"hi\" \$HOME \`x\` \\ done""#),
            v(&["sh", "-c", r#"echo "hi" $HOME `x` \ done"#])
        );
        assert_eq!(exec_argv(r#"app """#), v(&["app", ""]));
        assert_eq!(exec_argv(r#"app pre"mid dle"post"#), v(&["app", "premid dlepost"]));
        assert_eq!(exec_argv(r#"sh -c "code %U""#), v(&["sh", "-c", "code "]));
    }

    #[test]
    fn exec_after_value_unescape() {
        // In the file, `\\"` decodes to `\"` before Exec quoting is applied.
        let entry = "[Desktop Entry]\nType=Application\nName=X\nExec=sh -c \"echo \\\\\"a\\\\\"\"\n";
        let e = parse_entry("x.desktop", Path::new("/x.desktop"), entry).unwrap();
        assert_eq!(exec_argv(&e.exec), v(&["sh", "-c", "echo \"a\""]));
    }

    const SAMPLE: &str = r#"
# comment
[Desktop Entry]
Type=Application
Name=Files
Name[de]=Dateien
GenericName=File Manager
Comment=Access and organize files\sand\tfolders
Exec=nautilus --new-window %U
Icon=org.gnome.Nautilus
Keywords=folder;manager;explore;disk\;stuff;
Keywords[de]=ordner;
Terminal=false
TryExec=definitely-not-installed-xyz

[Desktop Action new-window]
Name=New Window
Exec=other-binary
Icon=wrong
"#;

    #[test]
    fn parse_basic_entry() {
        let e = parse_entry("org.gnome.Nautilus.desktop", Path::new("/a/b.desktop"), SAMPLE)
            .expect("parsed");
        assert_eq!(e.id, "org.gnome.Nautilus.desktop");
        assert_eq!(e.name, "Files");
        assert_eq!(e.generic_name.as_deref(), Some("File Manager"));
        assert_eq!(e.comment.as_deref(), Some("Access and organize files and\tfolders"));
        assert_eq!(e.exec, "nautilus --new-window %U");
        assert_eq!(e.icon.as_deref(), Some("org.gnome.Nautilus"));
        assert_eq!(e.keywords, v(&["folder", "manager", "explore", "disk;stuff"]));
        assert!(!e.terminal);
        assert_eq!(e.path, PathBuf::from("/a/b.desktop"));
    }

    #[test]
    fn parse_rejections() {
        let p = Path::new("/x.desktop");
        let base = "[Desktop Entry]\nName=X\nExec=x\n";
        assert!(parse_entry("x", p, &format!("{base}Type=Application\n")).is_some());
        assert!(parse_entry("x", p, &format!("{base}Type=Link\n")).is_none());
        assert!(parse_entry("x", p, base).is_none());
        assert!(parse_entry("x", p, &format!("{base}Type=Application\nNoDisplay=true\n")).is_none());
        assert!(parse_entry("x", p, &format!("{base}Type=Application\nHidden=true\n")).is_none());
        assert!(parse_entry("x", p, "[Desktop Entry]\nType=Application\nName=X\n").is_none());
        assert!(parse_entry("x", p, "[Other]\nType=Application\nName=X\nExec=x\n").is_none());
        let t = parse_entry("x", p, &format!("{base}Type=Application\nTerminal=true\n")).unwrap();
        assert!(t.terminal);
    }

    #[test]
    fn env_filters() {
        let env = Env {
            path_dirs: vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")],
            desktops: vec!["hyprland".into()],
        };
        assert!(!passes_env_filters(SAMPLE, &env)); // TryExec missing
        let ok = "[Desktop Entry]\nTryExec=sh\nOnlyShowIn=GNOME;Hyprland;\n";
        assert!(passes_env_filters(ok, &env));
        assert!(!passes_env_filters("[Desktop Entry]\nOnlyShowIn=GNOME;KDE;\n", &env));
        assert!(!passes_env_filters("[Desktop Entry]\nNotShowIn=HYPRLAND;\n", &env));
        assert!(passes_env_filters("[Desktop Entry]\nNotShowIn=KDE;\n", &env));
        assert!(passes_env_filters("[Desktop Entry]\nTryExec=/bin/sh\n", &env));
        assert!(!passes_env_filters("[Desktop Entry]\nTryExec=/nonexistent/x\n", &env));
    }

}
