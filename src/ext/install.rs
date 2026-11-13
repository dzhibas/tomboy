//! Installing extensions from the registry: fetch and cache the index, download an
//! extension's files, check each against the index's SHA-256, move the folder into place,
//! and remember what was installed (and which permissions the user accepted) in
//! `installed.json`.
//!
//! The registry is a base URL (`https://…`) or, for forks and testing, a local folder
//! (`file://…` or a plain path); the index is `<base>/extensions/index.json` and an
//! extension's files are under `<base>/extensions/<id>/`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::index::{self, Entry, Index};
use super::manifest::{Manifest, Permissions, is_newer, valid_id};

pub const DEFAULT_REGISTRY: &str = "https://raw.githubusercontent.com/dzhibas/tomboy/main";

/// How long the cached index is used without asking the registry again.
pub const INDEX_MAX_AGE: u64 = 60 * 60;

const INSTALLED: &str = "installed.json";
const INDEX_CACHE: &str = "extensions-index.json";
const TIMEOUT: Duration = Duration::from_secs(10);
/// Most bytes one download may take.
const MAX_DOWNLOAD: u64 = 5 * 1024 * 1024;
/// In a download error: the file differs from the index (they come from different commits).
const MISMATCH: &str = "does not match the extension index";

/// Gets the bytes at a URL; [`fetch`] in the app, a stub in tests.
pub type Fetch<'a> = &'a dyn Fn(&str) -> Result<Vec<u8>, String>;

// ---- installed.json ----

/// `installed.json`: id → what was installed.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(transparent)]
pub struct Installed(pub BTreeMap<String, InstalledEntry>);

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct InstalledEntry {
    pub version: String,
    /// What the user accepted; an update asking for more asks again.
    #[serde(default)]
    pub permissions: Permissions,
    pub source: Source,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Registry,
    Local,
}

impl Installed {
    /// What is installed; empty when nothing is or the file doesn't parse.
    pub fn load() -> Installed {
        super::data_dir().map(|d| Installed::load_from(&d)).unwrap_or_default()
    }

    pub fn load_from(data: &Path) -> Installed {
        std::fs::read_to_string(data.join(INSTALLED)).ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save_to(&self, data: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())? + "\n";
        write_atomic(&data.join(INSTALLED), json.as_bytes())
    }
}

fn data() -> Result<PathBuf, String> {
    super::data_dir().ok_or_else(|| "no home directory".to_string())
}

/// Writes through a temp file and a rename, so a reader never sees half a file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("{}: {e}", path.display());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let result = std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(fail)
}

// ---- the registry ----

pub fn index_url(base: &str) -> String {
    format!("{}/extensions/index.json", base.trim_end_matches('/'))
}

pub fn file_url(base: &str, id: &str, path: &str) -> String {
    format!("{}/extensions/{id}/{path}", base.trim_end_matches('/'))
}

/// The bytes at `url`: over https, or from disk for a `file://` URL or a plain path. Any other
/// scheme (plain http included) is refused.
pub fn fetch(url: &str) -> Result<Vec<u8>, String> {
    if url.starts_with("https://") {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .https_only(true)
            .build()
            .into();
        let mut response = agent.get(url).call().map_err(|e| format!("{url}: {e}"))?;
        return response.body_mut().with_config().limit(MAX_DOWNLOAD).read_to_vec()
            .map_err(|e| format!("{url}: {e}"));
    }
    if url.contains("://") && !url.starts_with("file://") {
        return Err(format!("{url}: the registry must be an https URL or a local folder"));
    }
    let path = Path::new(url.strip_prefix("file://").unwrap_or(url));
    let len = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?.len();
    if len > MAX_DOWNLOAD {
        return Err(format!("{}: over {} MB", path.display(), MAX_DOWNLOAD / 1024 / 1024));
    }
    std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

#[derive(Deserialize, Serialize)]
struct CachedIndex {
    base: String,
    /// Unix seconds.
    fetched_at: u64,
    index: Index,
}

fn index_cache() -> Option<PathBuf> {
    super::cache_root().map(|d| d.join(INDEX_CACHE))
}

fn read_cache(path: &Path, base: &str) -> Option<CachedIndex> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<CachedIndex>(&raw).ok().filter(|c| c.base == base)
}

/// The registry's index, from the cache when it is younger than `max_age_secs` (0 always
/// asks the registry) and came from `base`. When the registry can't be reached, a stale cache
/// is better than nothing.
pub fn fetch_index(base: &str, max_age_secs: u64) -> Result<Index, String> {
    fetch_index_with(index_cache().as_deref(), &fetch, base, max_age_secs)
}

/// The cached index of `base`, however old.
pub fn cached_index(base: &str) -> Option<Index> {
    read_cache(&index_cache()?, base).map(|c| c.index)
}

fn fetch_index_with(cache: Option<&Path>, fetch: Fetch, base: &str, max_age_secs: u64) -> Result<Index, String> {
    let cached = cache.and_then(|p| read_cache(p, base));
    if let Some(c) = &cached
        && crate::frecency::now().saturating_sub(c.fetched_at) < max_age_secs
    {
        return Ok(c.index.clone());
    }
    let fresh = fetch(&index_url(base)).and_then(|bytes| {
        index::parse(&String::from_utf8(bytes).map_err(|_| "extension index: not UTF-8".to_string())?)
    });
    match fresh {
        Ok(index) => {
            if let Some(path) = cache {
                let c = CachedIndex { base: base.into(), fetched_at: crate::frecency::now(), index };
                if let Ok(json) = serde_json::to_string(&c) {
                    let _ = write_atomic(path, json.as_bytes());
                }
                return Ok(c.index);
            }
            Ok(index)
        }
        Err(e) => cached.map(|c| c.index).ok_or(e),
    }
}

// ---- install, update, uninstall ----

/// Installs (or updates) extension `id` using the (cached) index; when a file doesn't match
/// it, the index is fetched again and the install retried once. Returns the entry installed.
pub fn install_fresh(base: &str, id: &str) -> Result<Entry, String> {
    install_fresh_with(&data()?, index_cache().as_deref(), &fetch, base, id)
}

fn install_fresh_with(data: &Path, cache: Option<&Path>, fetch: Fetch, base: &str, id: &str) -> Result<Entry, String> {
    let find = |index: Index| {
        index.extensions.into_iter().find(|e| e.manifest.id == id)
            .ok_or_else(|| format!("no extension \"{id}\" in the registry"))
    };
    let entry = find(fetch_index_with(cache, fetch, base, INDEX_MAX_AGE)?)?;
    match install_into(data, fetch, base, &entry) {
        Err(e) if e.contains(MISMATCH) => {
            let fresh = find(fetch_index_with(cache, fetch, base, 0)?)?;
            // Consent was given for the first entry's permissions, not the new one's.
            if !fresh.manifest.permissions.added_since(&entry.manifest.permissions).is_empty() {
                return Err(format!("{} changed in the registry and now asks for more permissions; try again", entry.manifest.name));
            }
            install_into(data, fetch, base, &fresh)?;
            Ok(fresh)
        }
        Err(e) => Err(e),
        Ok(()) => Ok(entry),
    }
}

/// Downloads `entry`'s files into a temp folder in `data`, checks them, then swaps the folder
/// into `data/<id>` and records it in `installed.json`. On failure nothing is left behind and
/// an installed older version stays as it was.
pub fn install_into(data: &Path, fetch: Fetch, base: &str, entry: &Entry) -> Result<(), String> {
    let m = &entry.manifest;
    m.supported().map_err(|e| format!("{}: {e}", m.name))?;
    check_entry(entry)?;
    std::fs::create_dir_all(data).map_err(|e| format!("{}: {e}", data.display()))?;
    let tmp = data.join(format!(".tmp-{}-{}", m.id, std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let done = download(&tmp, fetch, base, entry).and_then(|_| replace(data, &m.id, &tmp));
    if done.is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    done?;
    let mut installed = Installed::load_from(data);
    // An update keeps a disabled extension disabled.
    let enabled = installed.0.get(&m.id).is_none_or(|i| i.enabled);
    let record = InstalledEntry { version: m.version.clone(), permissions: m.permissions.clone(), source: Source::Registry, enabled };
    installed.0.insert(m.id.clone(), record);
    installed.save_to(data)
}

/// What can be checked before downloading anything.
fn check_entry(entry: &Entry) -> Result<(), String> {
    let m = &entry.manifest;
    m.validate()?;
    for required in [super::manifest::MANIFEST, super::manifest::ENTRY] {
        if !entry.files.contains_key(required) {
            return Err(format!("{}: {required} is missing", m.id));
        }
    }
    entry.files.keys().try_for_each(|p| index::check_path(p)).map_err(|e| format!("{}: {e}", m.id))
}

fn download(tmp: &Path, fetch: Fetch, base: &str, entry: &Entry) -> Result<(), String> {
    let id = &entry.manifest.id;
    let mut total = 0;
    for (rel, hash) in &entry.files {
        let bytes = fetch(&file_url(base, id, rel))?;
        index::check_size(rel, bytes.len() as u64, &mut total).map_err(|e| format!("{id}: {e}"))?;
        if !index::sha256_hex(&bytes).eq_ignore_ascii_case(hash) {
            return Err(format!("{id}: {rel} {MISMATCH}"));
        }
        let path = tmp.join(rel);
        let parent = path.parent().unwrap_or(tmp);
        std::fs::create_dir_all(parent).and_then(|_| std::fs::write(&path, &bytes))
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    // The manifest on disk is what runs; it must say what the store showed.
    let disk = Manifest::load(tmp)?;
    let m = &entry.manifest;
    if disk.id != m.id || disk.version != m.version || disk.permissions != m.permissions {
        return Err(format!("{id}: tomboy.toml {MISMATCH}"));
    }
    Ok(())
}

/// Moves `tmp` to `data/<id>`, keeping the old folder until the new one is in place.
fn replace(data: &Path, id: &str, tmp: &Path) -> Result<(), String> {
    let dest = data.join(id);
    let backup = data.join(format!(".old-{id}-{}", std::process::id()));
    let fail = |e: std::io::Error| format!("{}: {e}", dest.display());
    let had = dest.exists();
    if had {
        let _ = std::fs::remove_dir_all(&backup);
        std::fs::rename(&dest, &backup).map_err(fail)?;
    }
    if let Err(e) = std::fs::rename(tmp, &dest) {
        if had {
            let _ = std::fs::rename(&backup, &dest);
        }
        return Err(fail(e));
    }
    if had {
        let _ = std::fs::remove_dir_all(&backup);
    }
    Ok(())
}

pub fn uninstall(id: &str) -> Result<(), String> {
    uninstall_from(&data()?, id)
}

fn uninstall_from(data: &Path, id: &str) -> Result<(), String> {
    if !valid_id(id) {
        return Err(format!("\"{id}\" is not an extension id"));
    }
    let mut installed = Installed::load_from(data);
    let dir = data.join(id);
    if installed.0.remove(id).is_none() && !dir.exists() {
        return Err(format!("{id} is not installed"));
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    installed.save_to(data)
}

pub fn set_enabled(id: &str, enabled: bool) -> Result<(), String> {
    set_enabled_in(&data()?, id, enabled)
}

fn set_enabled_in(data: &Path, id: &str, enabled: bool) -> Result<(), String> {
    let mut installed = Installed::load_from(data);
    let entry = installed.0.get_mut(id).ok_or_else(|| format!("{id} is not installed"))?;
    entry.enabled = enabled;
    installed.save_to(data)
}

/// Every enabled installed extension whose manifest loads and this tomboy can run, with its
/// folder.
pub fn installed_dirs() -> Vec<(Manifest, PathBuf)> {
    super::data_dir().map(|d| installed_dirs_in(&d)).unwrap_or_default()
}

fn installed_dirs_in(data: &Path) -> Vec<(Manifest, PathBuf)> {
    Installed::load_from(data).0.into_iter()
        .filter(|(_, e)| e.enabled)
        .filter_map(|(id, _)| {
            let dir = data.join(&id);
            let m = Manifest::load(&dir).ok().filter(|m| m.id == id && m.supported().is_ok())?;
            Some((m, dir))
        })
        .collect()
}

// ---- what the store shows ----

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    NotInstalled,
    Installed,
    /// The registry has this newer version.
    UpdateAvailable(String),
    /// Not installed and this tomboy can't run it: "Needs a newer tomboy", …
    Unsupported(String),
    /// Installed from the registry, which no longer lists it.
    RemovedFromRegistry,
}

impl Status {
    /// The store's accessory text: "Installed", "Update 1.1.0", "Needs a newer tomboy", ….
    pub fn label(&self) -> String {
        match self {
            Status::NotInstalled => String::new(),
            Status::Installed => "Installed".into(),
            Status::UpdateAvailable(v) => format!("Update {v}"),
            Status::Unsupported(why) => why.clone(),
            Status::RemovedFromRegistry => "Removed from registry".into(),
        }
    }
}

/// An extension's status from its registry entry (if listed) and install record (if any).
/// An update this tomboy can't run is not offered.
pub fn status(entry: Option<&Entry>, installed: Option<&InstalledEntry>) -> Status {
    match (entry, installed) {
        (None, None) => Status::NotInstalled,
        (None, Some(i)) if i.source == Source::Registry => Status::RemovedFromRegistry,
        (None, Some(_)) => Status::Installed,
        (Some(e), None) => match e.manifest.supported() {
            Ok(()) => Status::NotInstalled,
            Err(why) => Status::Unsupported(why),
        },
        (Some(e), Some(i)) => {
            let m = &e.manifest;
            if i.source == Source::Registry && is_newer(&m.version, &i.version) && m.supported().is_ok() {
                Status::UpdateAvailable(m.version.clone())
            } else {
                Status::Installed
            }
        }
    }
}

/// The permissions installing `entry` would newly grant: all of them on a fresh install, those
/// added since the user last accepted on an update. Empty means no consent screen.
pub fn needs_consent(entry: &Entry, installed: Option<&InstalledEntry>) -> Permissions {
    let none = Permissions::default();
    entry.manifest.permissions.added_since(installed.map_or(&none, |i| &i.permissions))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ext::index::tests::{temp, write_ext};
    use std::cell::Cell;

    /// A registry folder with `demo` at `version`, and its index written.
    fn registry(name: &str, version: &str) -> (PathBuf, Index) {
        let reg = temp(name);
        let exts = reg.join("extensions");
        std::fs::create_dir_all(&exts).unwrap();
        write_ext(&exts, "demo", version);
        let index = index::build(&exts).unwrap();
        std::fs::write(exts.join("index.json"), index::render(&index)).unwrap();
        (reg, index)
    }

    fn leftovers(data: &Path) -> Vec<String> {
        std::fs::read_dir(data).map(|d| d.filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".tmp-") || n.starts_with(".old-"))
            .collect()).unwrap_or_default()
    }

    #[test]
    fn installs_updates_and_uninstalls_from_a_local_registry() {
        let (reg, index) = registry("reg-ok", "1.0.0");
        let data = temp("data-ok");
        let base = reg.to_str().unwrap();
        install_into(&data, &fetch, base, index.get("demo").unwrap()).unwrap();
        assert!(data.join("demo/init.luau").is_file());
        let installed = Installed::load_from(&data);
        let rec = &installed.0["demo"];
        assert_eq!((rec.version.as_str(), rec.source, rec.enabled), ("1.0.0", Source::Registry, true));
        assert_eq!(rec.permissions.net, vec!["example.com"]);
        assert_eq!(installed_dirs_in(&data).len(), 1);

        set_enabled_in(&data, "demo", false).unwrap();
        assert!(installed_dirs_in(&data).is_empty());

        // An update through file:// replaces the folder and keeps it disabled.
        let (reg2, index2) = registry("reg-ok2", "1.1.0");
        let base2 = format!("file://{}", reg2.display());
        install_into(&data, &fetch, &base2, index2.get("demo").unwrap()).unwrap();
        let rec = &Installed::load_from(&data).0["demo"];
        assert_eq!((rec.version.as_str(), rec.enabled), ("1.1.0", false));
        assert!(Manifest::load(&data.join("demo")).unwrap().version == "1.1.0");
        assert!(leftovers(&data).is_empty());

        uninstall_from(&data, "demo").unwrap();
        assert!(!data.join("demo").exists());
        assert!(Installed::load_from(&data).0.is_empty());
        assert!(uninstall_from(&data, "demo").is_err());
        for d in [reg, reg2, data] {
            std::fs::remove_dir_all(d).unwrap();
        }
    }

    #[test]
    fn hash_mismatch_leaves_nothing() {
        let (reg, index) = registry("reg-bad", "1.0.0");
        std::fs::write(reg.join("extensions/demo/init.luau"), "return { evil = true }").unwrap();
        let data = temp("data-bad");
        let err = install_into(&data, &fetch, reg.to_str().unwrap(), index.get("demo").unwrap()).unwrap_err();
        assert!(err.contains("init.luau") && err.contains(MISMATCH), "{err}");
        assert!(!data.join("demo").exists());
        assert!(leftovers(&data).is_empty());
        assert!(Installed::load_from(&data).0.is_empty());
        std::fs::remove_dir_all(reg).unwrap();
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn refuses_path_traversal_and_bad_types() {
        let (reg, index) = registry("reg-path", "1.0.0");
        let data = temp("data-path");
        let fetched = Cell::new(0);
        let counting = |url: &str| { fetched.set(fetched.get() + 1); fetch(url) };
        for bad in ["../evil.luau", "/abs.luau", "lib/../../x.luau", "run.sh"] {
            let mut entry = index.get("demo").unwrap().clone();
            entry.files.insert(bad.into(), "00".into());
            assert!(install_into(&data, &counting, reg.to_str().unwrap(), &entry).is_err(), "{bad}");
        }
        assert_eq!(fetched.get(), 0, "refused before downloading");
        assert!(!data.join("demo").exists());
        assert!(!data.parent().unwrap().join("evil.luau").exists());
        std::fs::remove_dir_all(reg).unwrap();
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn refuses_unsupported_and_http() {
        let (reg, index) = registry("reg-api", "1.0.0");
        let mut entry = index.get("demo").unwrap().clone();
        entry.manifest.api = 99;
        let data = temp("data-api");
        let fail = |_: &str| -> Result<Vec<u8>, String> { panic!("no download") };
        assert!(install_into(&data, &fail, "x", &entry).unwrap_err().contains("newer tomboy"));
        assert!(fetch("http://example.com/x").unwrap_err().contains("https"));
        assert!(fetch("ftp://example.com/x").is_err());
        std::fs::remove_dir_all(reg).unwrap();
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn mismatch_refetches_the_index_once() {
        let (reg, _) = registry("reg-retry", "1.0.0");
        let base = reg.to_str().unwrap();
        let cache = temp("cache-retry").join(INDEX_CACHE);
        let data = temp("data-retry");
        // Cache the 1.0.0 index, then the registry moves on to 1.1.0.
        fetch_index_with(Some(&cache), &fetch, base, INDEX_MAX_AGE).unwrap();
        let exts = reg.join("extensions");
        write_ext(&exts, "demo", "1.1.0");
        std::fs::write(exts.join("index.json"), index::render(&index::build(&exts).unwrap())).unwrap();
        assert_eq!(fetch_index_with(Some(&cache), &fetch, base, INDEX_MAX_AGE).unwrap().get("demo").unwrap().manifest.version, "1.0.0");
        let entry = install_fresh_with(&data, Some(&cache), &fetch, base, "demo").unwrap();
        assert_eq!(entry.manifest.version, "1.1.0");
        assert_eq!(Installed::load_from(&data).0["demo"].version, "1.1.0");
        assert!(install_fresh_with(&data, Some(&cache), &fetch, base, "nope").unwrap_err().contains("no extension"));
        std::fs::remove_dir_all(reg).unwrap();
        std::fs::remove_dir_all(cache.parent().unwrap()).unwrap();
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn stale_cache_when_offline() {
        let (reg, _) = registry("reg-offline", "1.0.0");
        let base = reg.to_str().unwrap();
        let cache = temp("cache-offline").join(INDEX_CACHE);
        let offline = |_: &str| -> Result<Vec<u8>, String> { Err("offline".into()) };
        assert_eq!(fetch_index_with(Some(&cache), &offline, base, 0).unwrap_err(), "offline");
        fetch_index_with(Some(&cache), &fetch, base, 0).unwrap();
        assert!(fetch_index_with(Some(&cache), &offline, base, 0).unwrap().get("demo").is_some());
        // Another registry's cache is not used.
        assert!(fetch_index_with(Some(&cache), &offline, "elsewhere", 0).is_err());
        std::fs::remove_dir_all(reg).unwrap();
        std::fs::remove_dir_all(cache.parent().unwrap()).unwrap();
    }

    fn rec(version: &str, net: &[&str], source: Source) -> InstalledEntry {
        let permissions = Permissions { net: net.iter().map(|s| s.to_string()).collect() };
        InstalledEntry { version: version.into(), permissions, source, enabled: true }
    }

    #[test]
    fn statuses() {
        let (reg, index) = registry("reg-status", "1.1.0");
        let e = index.get("demo").unwrap();
        assert_eq!(status(None, None), Status::NotInstalled);
        assert_eq!(status(Some(e), None), Status::NotInstalled);
        assert_eq!(status(Some(e), Some(&rec("1.0.0", &[], Source::Registry))), Status::UpdateAvailable("1.1.0".into()));
        assert_eq!(status(Some(e), Some(&rec("1.1.0", &[], Source::Registry))), Status::Installed);
        assert_eq!(status(Some(e), Some(&rec("1.0.0", &[], Source::Local))), Status::Installed);
        assert_eq!(status(None, Some(&rec("1.0.0", &[], Source::Registry))), Status::RemovedFromRegistry);
        assert_eq!(status(None, Some(&rec("1.0.0", &[], Source::Local))), Status::Installed);
        let mut newer = e.clone();
        newer.manifest.api = 99;
        assert_eq!(status(Some(&newer), None), Status::Unsupported("Needs a newer tomboy".into()));
        assert_eq!(status(Some(&newer), Some(&rec("1.0.0", &[], Source::Registry))), Status::Installed);
        let installed = Installed([("demo".to_string(), rec("1.0.0", &[], Source::Registry))].into());
        std::fs::remove_dir_all(reg).unwrap();
    }

    #[test]
    fn consent() {
        let (reg, index) = registry("reg-consent", "1.0.0");
        let e = index.get("demo").unwrap();
        assert_eq!(needs_consent(e, None).net, vec!["example.com"]);
        assert!(needs_consent(e, Some(&rec("0.9.0", &["example.com"], Source::Registry))).is_empty());
        assert_eq!(needs_consent(e, Some(&rec("0.9.0", &["other.com"], Source::Registry))).net, vec!["example.com"]);
        std::fs::remove_dir_all(reg).unwrap();
    }
}
