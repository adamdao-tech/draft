//! The browser side of [`crate::platform`].
//!
//! The app was written against a file system, so the browser build keeps one in memory:
//! [`fs`] mirrors the parts of `std::fs` the app uses. Everything under `/config` and
//! `/saves` is written through to IndexedDB and read back at startup, so settings, themes
//! and autosaves survive a reload. Two folders are special:
//!
//! - a file written to [`DOWNLOADS`] is handed to the browser as a download and not kept;
//! - files the user picks are copied to [`UPLOADS`], and the dialog returns that path.
//!
//! Background work runs on the page's only thread, after the current event has finished
//! (see [`spawn`]).

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use js_sys::{Array, Uint8Array};
use wasm_bindgen::prelude::*;

pub use web_time::Instant;

pub const DOWNLOADS: &str = "/downloads";
pub const UPLOADS: &str = "/uploads";
const ICONS: &str = "/champion_icons";
const PERSISTED: [&str; 2] = ["/config/", "/saves/"];

#[wasm_bindgen(inline_js = r#"
const DB = 'recvrve', STORE = 'files';
let db = null;
function open() {
  return new Promise((ok, no) => {
    const r = indexedDB.open(DB, 1);
    r.onupgradeneeded = () => r.result.createObjectStore(STORE);
    r.onsuccess = () => ok(r.result);
    r.onerror = () => no(r.error);
  });
}
export async function idb_load() {
  try { db = await open(); } catch (e) { console.warn('IndexedDB is unavailable', e); return []; }
  return new Promise((ok) => {
    const out = [];
    const c = db.transaction(STORE).objectStore(STORE).openCursor();
    c.onsuccess = () => {
      const cur = c.result;
      if (!cur) return ok(out);
      out.push([cur.key, cur.value.bytes, cur.value.mtime]);
      cur.continue();
    };
    c.onerror = () => ok(out);
  });
}
export function idb_put(path, bytes, mtime) {
  if (db) db.transaction(STORE, 'readwrite').objectStore(STORE).put({ bytes, mtime }, path);
}
export function idb_delete(path) {
  if (db) db.transaction(STORE, 'readwrite').objectStore(STORE).delete(path);
}
export function pick_files(accept, multiple) {
  return new Promise((ok) => {
    const i = document.createElement('input');
    i.type = 'file';
    if (accept) i.accept = accept;
    i.multiple = multiple;
    i.style.display = 'none';
    document.body.appendChild(i);
    const done = (v) => { i.remove(); ok(v); };
    i.addEventListener('cancel', () => done([]));
    i.addEventListener('change', async () => {
      const out = [];
      for (const f of i.files) out.push([f.name, new Uint8Array(await f.arrayBuffer())]);
      done(out);
    });
    i.click();
  });
}
export function download(name, bytes) {
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([bytes]));
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 60000);
}
export function blob_url(bytes, mime) {
  return URL.createObjectURL(new Blob([bytes], { type: mime }));
}
export async function fetch_bytes(url) {
  const r = await fetch(url);
  if (!r.ok) return null;
  return new Uint8Array(await r.arrayBuffer());
}
export function fullscreen(on) {
  if (on) document.documentElement.requestFullscreen?.().catch(() => {});
  else if (document.fullscreenElement) document.exitFullscreen();
}
export function set_doc_title(t) { document.title = t; }
"#)]
extern "C" {
    #[wasm_bindgen(catch)]
    async fn idb_load() -> Result<JsValue, JsValue>;
    fn idb_put(path: &str, bytes: Uint8Array, mtime: f64);
    fn idb_delete(path: &str);
    #[wasm_bindgen(catch)]
    async fn pick_files(accept: &str, multiple: bool) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = download)]
    fn browser_download(name: &str, bytes: Uint8Array);
    fn blob_url(bytes: Uint8Array, mime: &str) -> String;
    #[wasm_bindgen(catch)]
    async fn fetch_bytes(url: &str) -> Result<JsValue, JsValue>;
    fn fullscreen(on: bool);
    fn set_doc_title(title: &str);
}

/// The shipped config, placed in the file system at startup. Defaults are always refreshed;
/// the others only appear if the user has no such file yet.
const SHIPPED: [(&str, &str); 6] = [
    ("/config/reprgroups.json", include_str!("../../config/reprgroups.json")),
    ("/config/selectiongroups.json", include_str!("../../config/selectiongroups.json")),
    ("/config/defaults/reprgroups.json", include_str!("../../config/defaults/reprgroups.json")),
    ("/config/defaults/selectiongroups.json", include_str!("../../config/defaults/selectiongroups.json")),
    ("/config/themes/dark/theme.json", include_str!("../../config/themes/dark/theme.json")),
    ("/config/themes/light/theme.json", include_str!("../../config/themes/light/theme.json")),
];

/// Reads IndexedDB into the file system and adds the shipped config. Call before anything
/// reads a file.
pub async fn init() {
    if let Ok(entries) = idb_load().await {
        for entry in Array::from(&entries).iter() {
            let entry = Array::from(&entry);
            let Some(path) = entry.get(0).as_string() else {
                continue;
            };
            let bytes = Uint8Array::new(&entry.get(1)).to_vec();
            let mtime = entry.get(2).as_f64().unwrap_or(0.0);
            fs::insert(&path, bytes, mtime);
        }
    }
    for (path, text) in SHIPPED {
        if path.contains("/defaults/") || !fs::contains(path) {
            fs::insert(path, text.as_bytes().to_vec(), 0.0);
        }
    }
}

/// Loads the champion portraits exports need; the page itself uses them as plain images.
pub async fn load_icons() {
    for champion in crate::champions::CHAMPIONS.iter() {
        let path = format!("{ICONS}/{}.png", champion.key);
        if fs::contains(&path) {
            continue;
        }
        let url = format!("champion_icons/{}.png", champion.key);
        if let Ok(value) = fetch_bytes(&url).await
            && !value.is_null()
        {
            fs::insert(&path, Uint8Array::new(&value).to_vec(), 0.0);
        }
    }
}

/// The folder [`load_icons`] fills.
pub fn icon_dir() -> PathBuf {
    PathBuf::from(ICONS)
}

/// A URL for the file at `path`, for pictures that only exist in the browser's storage.
pub fn file_url(path: &Path, mime: &str) -> Option<String> {
    thread_local! {
        static URLS: RefCell<BTreeMap<(PathBuf, u64), String>> = const { RefCell::new(BTreeMap::new()) };
    }
    let stamp = fs::metadata(path).ok()?.mtime as u64;
    let key = (path.to_path_buf(), stamp);
    if let Some(url) = URLS.with_borrow(|u| u.get(&key).cloned()) {
        return Some(url);
    }
    let bytes = fs::read(path).ok()?;
    let url = blob_url(Uint8Array::from(bytes.as_slice()), mime);
    URLS.with_borrow_mut(|u| u.insert(key, url.clone()));
    Some(url)
}

/// Runs `work` once the current event is over, so the page can show what led to it first.
/// It still has the page's thread to itself while it runs.
pub fn spawn<F: FnOnce() + 'static>(work: F) {
    wasm_bindgen_futures::spawn_local(async move {
        gloo_timers::future::sleep(Duration::from_millis(30)).await;
        work();
    });
}

/// [`spawn`] for exports, which draw the champion portraits.
pub fn spawn_with_icons<F: FnOnce() + 'static>(work: F) {
    wasm_bindgen_futures::spawn_local(async move {
        load_icons().await;
        gloo_timers::future::sleep(Duration::from_millis(30)).await;
        work();
    });
}

pub async fn sleep(duration: Duration) {
    gloo_timers::future::sleep(duration).await;
}

pub fn process_id() -> u32 {
    (js_sys::Date::now() % 1e9) as u32
}

pub type WindowId = u64;

pub fn window_id() -> WindowId {
    0
}

pub fn set_title(title: &str) {
    set_doc_title(title);
}

pub fn set_fullscreen(on: bool) {
    fullscreen(on);
}

pub fn focus_window() {}

pub fn show_window() {}

pub fn close_window() {}

pub mod dialog {
    use super::*;

    pub struct FileHandle(PathBuf);

    impl FileHandle {
        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    /// The parts of `rfd`'s dialog the app uses. Opening copies the chosen files into
    /// the file system; saving just names a file in the downloads folder.
    #[derive(Default)]
    pub struct AsyncFileDialog {
        file_name: Option<String>,
        extensions: Vec<String>,
        any: bool,
    }

    impl AsyncFileDialog {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn set_title(self, _title: impl Into<String>) -> Self {
            self
        }

        pub fn set_directory(self, _dir: impl AsRef<Path>) -> Self {
            self
        }

        pub fn set_file_name(mut self, name: impl Into<String>) -> Self {
            self.file_name = Some(name.into());
            self
        }

        pub fn add_filter(mut self, _name: impl Into<String>, extensions: &[impl ToString]) -> Self {
            for e in extensions {
                let e = e.to_string();
                if e == "*" {
                    self.any = true;
                } else {
                    self.extensions.push(format!(".{e}"));
                }
            }
            self
        }

        fn accept(&self) -> String {
            if self.any { String::new() } else { self.extensions.join(",") }
        }

        async fn pick(self, multiple: bool) -> Vec<FileHandle> {
            let Ok(picked) = pick_files(&self.accept(), multiple).await else {
                return Vec::new();
            };
            Array::from(&picked)
                .iter()
                .filter_map(|entry| {
                    let entry = Array::from(&entry);
                    let name = entry.get(0).as_string()?;
                    let path = Path::new(UPLOADS).join(&name);
                    fs::write(&path, Uint8Array::new(&entry.get(1)).to_vec()).ok()?;
                    Some(FileHandle(path))
                })
                .collect()
        }

        pub async fn pick_file(self) -> Option<FileHandle> {
            self.pick(false).await.into_iter().next()
        }

        pub async fn pick_files(self) -> Option<Vec<FileHandle>> {
            let picked = self.pick(true).await;
            (!picked.is_empty()).then_some(picked)
        }

        /// Browsers can't hand a page a folder to write in; exports arrive as a .zip.
        pub async fn pick_folder(self) -> Option<FileHandle> {
            None
        }

        pub async fn save_file(self) -> Option<FileHandle> {
            let name = self.file_name.unwrap_or_else(|| "recvrve-file".to_string());
            Some(FileHandle(Path::new(DOWNLOADS).join(name)))
        }
    }
}

pub mod fs {
    use super::*;
    use std::ffi::OsString;
    use std::io::{self, Error, ErrorKind, Seek, SeekFrom, Write};

    struct Entry {
        bytes: Vec<u8>,
        mtime: f64,
    }

    #[derive(Default)]
    struct Vfs {
        files: BTreeMap<String, Entry>,
        dirs: BTreeSet<String>,
    }

    thread_local! {
        static VFS: RefCell<Vfs> = RefCell::new(Vfs::default());
    }

    /// `/`-separated with a leading slash and no `.` or empty parts.
    fn key(path: impl AsRef<Path>) -> String {
        let text = path.as_ref().to_string_lossy().replace('\\', "/");
        let mut parts: Vec<&str> = Vec::new();
        for part in text.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                other => parts.push(other),
            }
        }
        format!("/{}", parts.join("/"))
    }

    fn parent_of(key: &str) -> Option<&str> {
        match key.rfind('/') {
            Some(0) if key.len() > 1 => Some("/"),
            Some(0) | None => None,
            Some(i) => Some(&key[..i]),
        }
    }

    fn not_found(path: &str) -> Error {
        Error::new(ErrorKind::NotFound, format!("{path}: no such file"))
    }

    fn persisted(key: &str) -> bool {
        PERSISTED.iter().any(|p| key.starts_with(p))
    }

    /// A temporary name of a write that finishes with a rename.
    fn is_partial(key: &str) -> bool {
        key.ends_with(".part") || key.ends_with(".tmp")
    }

    fn add_parents(v: &mut Vfs, key: &str) {
        let mut at = key;
        while let Some(parent) = parent_of(at) {
            v.dirs.insert(parent.to_string());
            at = parent;
        }
    }

    pub(super) fn insert(path: &str, bytes: Vec<u8>, mtime: f64) {
        let k = key(path);
        VFS.with_borrow_mut(|v| {
            add_parents(v, &k);
            v.files.insert(k, Entry { bytes, mtime });
        });
    }

    pub(super) fn contains(path: &str) -> bool {
        VFS.with_borrow(|v| v.files.contains_key(&key(path)))
    }

    fn store(k: String, bytes: Vec<u8>) {
        let downloads = format!("{DOWNLOADS}/");
        if k.starts_with(&downloads) && !is_partial(&k) {
            let name = k.rsplit('/').next().unwrap_or("download");
            browser_download(name, Uint8Array::from(bytes.as_slice()));
            return;
        }
        let mtime = js_sys::Date::now();
        if persisted(&k) {
            idb_put(&k, Uint8Array::from(bytes.as_slice()), mtime);
        }
        VFS.with_borrow_mut(|v| {
            add_parents(v, &k);
            v.files.insert(k, Entry { bytes, mtime });
        });
    }

    fn drop_file(k: &str) -> bool {
        let existed = VFS.with_borrow_mut(|v| v.files.remove(k).is_some());
        if existed && persisted(k) {
            idb_delete(k);
        }
        existed
    }

    pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        let k = key(path);
        VFS.with_borrow(|v| v.files.get(&k).map(|e| e.bytes.clone()))
            .ok_or_else(|| not_found(&k))
    }

    pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
        String::from_utf8(read(path)?).map_err(|e| Error::new(ErrorKind::InvalidData, e))
    }

    pub fn write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
        store(key(path), contents.as_ref().to_vec());
        Ok(())
    }

    pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
        let k = key(path);
        VFS.with_borrow_mut(|v| {
            add_parents(v, &k);
            v.dirs.insert(k);
        });
        Ok(())
    }

    pub fn create_dir(path: impl AsRef<Path>) -> io::Result<()> {
        let k = key(path);
        if VFS.with_borrow(|v| v.dirs.contains(&k)) {
            return Err(Error::new(ErrorKind::AlreadyExists, k));
        }
        create_dir_all(k)
    }

    pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
        let k = key(path);
        if drop_file(&k) { Ok(()) } else { Err(not_found(&k)) }
    }

    pub fn remove_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
        let k = key(path);
        let prefix = format!("{k}/");
        let inside: Vec<String> = VFS.with_borrow_mut(|v| {
            v.dirs.retain(|d| d != &k && !d.starts_with(&prefix));
            v.files.keys().filter(|f| f.starts_with(&prefix)).cloned().collect()
        });
        for file in inside {
            drop_file(&file);
        }
        Ok(())
    }

    pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
        let (from, to) = (key(from), key(to));
        if contains(&from) {
            let bytes = read(&from)?;
            drop_file(&from);
            store(to, bytes);
            return Ok(());
        }
        let prefix = format!("{from}/");
        let is_dir = VFS.with_borrow(|v| v.dirs.contains(&from));
        if !is_dir {
            return Err(not_found(&from));
        }
        let inside: Vec<String> = VFS.with_borrow(|v| {
            v.files.keys().filter(|f| f.starts_with(&prefix)).cloned().collect()
        });
        for file in inside {
            let bytes = read(&file)?;
            drop_file(&file);
            store(format!("{to}/{}", &file[prefix.len()..]), bytes);
        }
        VFS.with_borrow_mut(|v| {
            let moved: Vec<String> = v
                .dirs
                .iter()
                .filter(|d| d.starts_with(&prefix))
                .cloned()
                .collect();
            for d in moved {
                v.dirs.remove(&d);
                v.dirs.insert(format!("{to}/{}", &d[prefix.len()..]));
            }
            v.dirs.remove(&from);
            add_parents(v, &to);
            v.dirs.insert(to.clone());
        });
        Ok(())
    }

    pub fn copy(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<u64> {
        let bytes = read(from)?;
        let len = bytes.len() as u64;
        store(key(to), bytes);
        Ok(len)
    }

    #[derive(Clone, Copy)]
    pub struct Metadata {
        dir: bool,
        len: u64,
        pub(super) mtime: f64,
    }

    impl Metadata {
        pub fn is_file(&self) -> bool {
            !self.dir
        }
        pub fn is_dir(&self) -> bool {
            self.dir
        }
        pub fn len(&self) -> u64 {
            self.len
        }
        pub fn modified(&self) -> io::Result<SystemTime> {
            Ok(SystemTime::UNIX_EPOCH + Duration::from_millis(self.mtime as u64))
        }
    }

    pub fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
        let k = key(path);
        VFS.with_borrow(|v| {
            if let Some(e) = v.files.get(&k) {
                Some(Metadata { dir: false, len: e.bytes.len() as u64, mtime: e.mtime })
            } else if v.dirs.contains(&k) || k == "/" {
                Some(Metadata { dir: true, len: 0, mtime: 0.0 })
            } else {
                None
            }
        })
        .ok_or_else(|| not_found(&k))
    }

    #[derive(Clone, Copy)]
    pub struct FileType(bool);

    impl FileType {
        pub fn is_dir(&self) -> bool {
            self.0
        }
        pub fn is_file(&self) -> bool {
            !self.0
        }
    }

    pub struct DirEntry {
        path: PathBuf,
    }

    impl DirEntry {
        pub fn path(&self) -> PathBuf {
            self.path.clone()
        }
        pub fn file_name(&self) -> OsString {
            self.path.file_name().unwrap_or_default().to_os_string()
        }
        pub fn file_type(&self) -> io::Result<FileType> {
            Ok(FileType(metadata(&self.path)?.is_dir()))
        }
        pub fn metadata(&self) -> io::Result<Metadata> {
            metadata(&self.path)
        }
    }

    pub fn read_dir(path: impl AsRef<Path>) -> io::Result<impl Iterator<Item = io::Result<DirEntry>>> {
        let k = key(path);
        if !metadata(&k)?.is_dir() {
            return Err(Error::new(ErrorKind::NotADirectory, k));
        }
        let names: Vec<String> = VFS.with_borrow(|v| {
            let of = |p: &String| parent_of(p) == Some(k.as_str());
            v.dirs.iter().filter(|d| of(d)).chain(v.files.keys().filter(|f| of(f))).cloned().collect()
        });
        Ok(names.into_iter().map(|p| Ok(DirEntry { path: PathBuf::from(p) })))
    }

    /// A file being written; it lands in the file system on flush and on drop.
    pub struct File {
        path: String,
        buffer: Vec<u8>,
        at: usize,
        dirty: bool,
    }

    /// Only ever used to open a session's lock file (see `autosave.rs`).
    pub struct OpenOptions;

    impl OpenOptions {
        pub fn write(self, _on: bool) -> Self {
            self
        }
        pub fn open(self, path: impl AsRef<Path>) -> io::Result<File> {
            let k = key(path);
            if !contains(&k) {
                return Err(not_found(&k));
            }
            Ok(File { path: k, buffer: Vec::new(), at: 0, dirty: false })
        }
    }

    impl File {
        pub fn create(path: impl AsRef<Path>) -> io::Result<File> {
            let k = key(path);
            // A download happens once, with the finished contents.
            if !k.starts_with(DOWNLOADS) {
                store(k.clone(), Vec::new());
            }
            Ok(File { path: k, buffer: Vec::new(), at: 0, dirty: false })
        }

        pub fn options() -> OpenOptions {
            OpenOptions
        }

        /// A page has no other process to share files with. Every earlier session counts
        /// as ended, so its tabs are offered back after a reload.
        pub fn try_lock(&self) -> Result<(), Error> {
            Ok(())
        }
    }

    impl Write for File {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            let end = self.at + data.len();
            if self.buffer.len() < end {
                self.buffer.resize(end, 0);
            }
            self.buffer[self.at..end].copy_from_slice(data);
            self.at = end;
            self.dirty = true;
            Ok(data.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            if self.dirty {
                self.dirty = false;
                store(self.path.clone(), self.buffer.clone());
            }
            Ok(())
        }
    }

    impl Seek for File {
        fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
            let at = match to {
                SeekFrom::Start(n) => n as i64,
                SeekFrom::Current(n) => self.at as i64 + n,
                SeekFrom::End(n) => self.buffer.len() as i64 + n,
            };
            self.at = usize::try_from(at).map_err(|_| Error::from(ErrorKind::InvalidInput))?;
            Ok(self.at as u64)
        }
    }

    impl Drop for File {
        fn drop(&mut self) {
            let _ = self.flush();
        }
    }
}
