//! `tomboy ext …`: list, install, update and remove extensions from a terminal (the same code
//! as the store), and the contributor tools: `check`, `index`, `new`.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use super::index;
use super::install::{self, Installed, Source, Status};
use super::manifest::{ENTRY, MANIFEST, Manifest, Permissions, valid_id};

const USAGE: &str = "\
usage: tomboy ext <command>

  list                        installed extensions
  available                   extensions in the registry
  install <id> [--yes]        install one (asks to accept its permissions)
  update [<id>] [--yes]       update one or all
  uninstall <id>              remove one
  enable <id> | disable <id>  turn one on or off
  dev <dir>                   run a local folder as an extension
  check <dir>                 validate an extension folder
  index [--check] [<dir>]     write (or verify) <dir>/index.json; <dir> defaults to extensions
  new <id> [<dir>]            scaffold <dir>/<id>/ (default: the current folder)
";

/// Runs `tomboy ext <args>` against the registry at `registry`; returns the exit code.
pub fn run(args: &[String], registry: &str) -> i32 {
    let yes = args.iter().any(|a| a == "--yes" || a == "-y");
    let check = args.iter().any(|a| a == "--check");
    let rest: Vec<&str> = args.iter().map(String::as_str).filter(|a| !a.starts_with('-')).collect();
    let result = match rest.as_slice() {
        ["list"] => list(),
        ["available"] => available(registry),
        ["install", id] => install_one(registry, id, yes),
        ["update"] => update(registry, None, yes),
        ["update", id] => update(registry, Some(id), yes),
        ["uninstall", id] => install::uninstall(id).map(|_| println!("Uninstalled {id}")),
        ["enable", id] => install::set_enabled(id, true).map(|_| println!("Enabled {id}")),
        ["disable", id] => install::set_enabled(id, false).map(|_| println!("Disabled {id}")),
        ["check", dir] => check_dir(Path::new(dir)),
        ["index"] => write_index(Path::new("extensions"), check),
        ["index", dir] => write_index(Path::new(dir), check),
        ["new", id] => new(id, Path::new(".")),
        ["new", id, parent] => new(id, Path::new(parent)),
        _ => {
            eprint!("{USAGE}");
            return 2;
        }
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("tomboy ext: {e}");
            1
        }
    }
}

fn list() -> Result<(), String> {
    let installed = Installed::load();
    if installed.0.is_empty() {
        println!("No extensions installed. See `tomboy ext available`.");
    }
    for (id, e) in &installed.0 {
        let state = if e.enabled { "enabled" } else { "disabled" };
        let source = if e.source == Source::Local { "  (local)" } else { "" };
        println!("{id}  {}  {state}{source}  {}", e.version, permissions_hint(&e.permissions));
    }
    Ok(())
}

fn available(registry: &str) -> Result<(), String> {
    let index = install::fetch_index(registry, install::INDEX_MAX_AGE)?;
    let installed = Installed::load();
    for e in &index.extensions {
        let m = &e.manifest;
        let status = install::status(Some(e), installed.0.get(&m.id)).label();
        println!("{}  {}  {}  {status}\n    {}", m.id, m.version, m.name, m.description);
    }
    Ok(())
}

fn permissions_hint(p: &Permissions) -> String {
    if p.is_empty() { "no permissions".into() } else { p.describe().join(", ") }
}

fn install_one(registry: &str, id: &str, yes: bool) -> Result<(), String> {
    let index = install::fetch_index(registry, install::INDEX_MAX_AGE)?;
    let entry = index.get(id).ok_or_else(|| format!("no extension \"{id}\" in the registry"))?;
    let m = &entry.manifest;
    m.supported().map_err(|e| format!("{}: {e}", m.name))?;
    println!("{} {} by {}\n{}", m.name, m.version, m.author, m.description);
    if !yes && !consent(&m.permissions, "Install?") {
        return Err("cancelled".into());
    }
    let done = install::install_fresh(registry, id)?;
    println!("Installed {} {}", done.manifest.name, done.manifest.version);
    Ok(())
}

fn update(registry: &str, only: Option<&str>, yes: bool) -> Result<(), String> {
    let installed = Installed::load();
    if let Some(id) = only
        && !installed.0.contains_key(id)
    {
        return Err(format!("{id} is not installed"));
    }
    let index = install::fetch_index(registry, install::INDEX_MAX_AGE)?;
    let mut failed = 0;
    let mut updated = 0;
    for (id, rec) in installed.0.iter().filter(|(id, _)| only.is_none_or(|o| o == *id)) {
        let entry = index.get(id);
        match install::status(entry, Some(rec)) {
            Status::UpdateAvailable(v) => {
                let entry = entry.expect("an update has an entry");
                let added = install::needs_consent(entry, Some(rec));
                println!("{id}: {} → {v}", rec.version);
                if !yes && !added.is_empty() && !consent(&added, "Update?") {
                    println!("{id}: skipped");
                    continue;
                }
                match install::install_fresh(registry, id) {
                    Ok(_) => updated += 1,
                    Err(e) => {
                        eprintln!("{id}: {e}");
                        failed += 1;
                    }
                }
            }
            Status::RemovedFromRegistry => println!("{id}: removed from the registry (`tomboy ext uninstall {id}`)"),
            _ if only.is_some() => println!("{id}: up to date"),
            _ => {}
        }
    }
    if only.is_none() {
        println!("{updated} updated");
    }
    if failed > 0 { Err(format!("{failed} update(s) failed")) } else { Ok(()) }
}

/// Shows the permissions and asks `question [y/N]` on stdin.
fn consent(p: &Permissions, question: &str) -> bool {
    if p.is_empty() {
        println!("It asks for no permissions.");
    } else {
        println!("It asks for these permissions:");
        for line in p.describe() {
            println!("  - {line}");
        }
    }
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    let _ = std::io::stdin().lock().read_line(&mut answer);
    matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
}

/// The folder's name, even for `.` or a trailing slash.
fn folder_name(dir: &Path) -> String {
    let abs = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    abs.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string()
}

fn check_dir(dir: &Path) -> Result<(), String> {
    let files = index::check_files(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let m = Manifest::load(dir)?;
    let folder = folder_name(dir);
    if m.id != folder {
        return Err(format!("id \"{}\" must match its folder name \"{folder}\"", m.id));
    }
    if !files.contains_key(index::README) {
        return Err(format!("{}: add a README.md; the store shows it on the extension's page", dir.display()));
    }
    if let Err(e) = m.supported() {
        println!("note: {e}");
    }
    for note in super::runtime::check(dir)? {
        println!("note: {note}");
    }
    println!("ok: {} {} ({} files)", m.id, m.version, files.len());
    Ok(())
}

fn write_index(dir: &Path, check: bool) -> Result<(), String> {
    let rendered = index::render(&index::build(dir)?);
    let path = dir.join("index.json");
    if check {
        let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
        if on_disk != rendered {
            return Err(format!("{} is stale: run `tomboy ext index`", path.display()));
        }
        println!("{} is up to date", path.display());
        return Ok(());
    }
    std::fs::write(&path, rendered).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
}

fn new(id: &str, parent: &Path) -> Result<(), String> {
    if !valid_id(id) {
        return Err(format!("\"{id}\": use lowercase letters, digits and dashes"));
    }
    let dir: PathBuf = parent.join(id);
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = title_case(id);
    for (file, body) in [(MANIFEST, manifest_template(id, &name)), (ENTRY, entry_template(id)), (index::README, readme_template(id, &name))] {
        std::fs::write(dir.join(file), body).map_err(|e| format!("{}: {e}", dir.join(file).display()))?;
    }
    println!("Created {}\nTry it: tomboy ext dev {0}", dir.display());
    Ok(())
}

/// `my-ext` → `My Ext`.
fn title_case(id: &str) -> String {
    id.split('-').filter(|w| !w.is_empty()).map(|w| {
        let mut c = w.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
    }).collect::<Vec<_>>().join(" ")
}

fn manifest_template(id: &str, name: &str) -> String {
    format!(r#"id = "{id}"
name = "{name}"
version = "0.1.0"
api = {api}
description = "What {name} does, in one line"
author = "your-github-login"
license = "MIT"

[[commands]]
id = "{id}"
title = "{name}"
keywords = []

# Hosts api.http.get may reach (https only), e.g. net = ["hn.algolia.com"].
[permissions]
net = []
"#, api = super::manifest::API_VERSION)
}

fn entry_template(id: &str) -> String {
    format!(r#"local ui = require("@tomboy/ui")

-- One function per command id in tomboy.toml; each returns what tomboy shows.
return {{
    ["{id}"] = function(ctx)
        return ui.list({{
            items = {{
                {{
                    id = "tomboy",
                    title = "tomboy on GitHub",
                    subtitle = "The launcher this extension runs in",
                    accessories = {{ "github.com" }},
                    actions = {{
                        ui.open("https://github.com/dzhibas/tomboy", "Open"),
                        ui.copy("https://github.com/dzhibas/tomboy", "Copy URL"),
                    }},
                }},
                {{
                    id = "hello",
                    title = "Hello from {id}",
                    subtitle = "Edit init.luau to change this list",
                    accessories = {{ "Luau" }},
                    actions = {{
                        ui.copy("Hello from {id}", "Copy"),
                        ui.open("https://luau.org", "Open luau.org"),
                    }},
                }},
            }},
        }})
    end,
}}
"#)
}

fn readme_template(id: &str, name: &str) -> String {
    format!(r#"# {name}

What it does and how to use it.

## Develop

    tomboy ext dev extensions/{id}
    tomboy ext check extensions/{id}
"#)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ext::index::tests::temp;

    #[test]
    fn scaffold_is_a_valid_extension() {
        let parent = temp("cli-new");
        new("my-ext", &parent).unwrap();
        let dir = parent.join("my-ext");
        let m = Manifest::load(&dir).unwrap();
        assert_eq!((m.id.as_str(), m.name.as_str(), m.commands[0].id.as_str()), ("my-ext", "My Ext", "my-ext"));
        assert!(index::check_files(&dir).is_ok());
        assert!(std::fs::read_to_string(dir.join(ENTRY)).unwrap().contains("[\"my-ext\"] = function(ctx)"));
        assert!(new("my-ext", &parent).unwrap_err().contains("exists"));
        assert!(new("My Ext", &parent).is_err());

        assert!(write_index(&parent, true).unwrap_err().contains("stale"));
        write_index(&parent, false).unwrap();
        write_index(&parent, true).unwrap();
        std::fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn usage_on_unknown() {
        assert_eq!(run(&["nope".into()], install::DEFAULT_REGISTRY), 2);
        assert_eq!(run(&[], install::DEFAULT_REGISTRY), 2);
    }
}
