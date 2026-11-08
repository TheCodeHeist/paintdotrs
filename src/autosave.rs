//! Autosave cache: pictures with unsaved changes are written to a cache folder in the background every
//! minute, so the work survives a crash or a power cut. The next start offers to bring them back.
//! Each file is named after the process and the picture (`<pid>-<id>.pxe`, with a `.txt` beside it holding
//! the picture's name and where it was saved), so two copies of the app running at once don't mix up.

use crate::doc::Document;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How often unsaved pictures are written to the cache.
pub const INTERVAL: Duration = Duration::from_secs(60);

pub fn dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("paint-rs").join("autosave"))
}

fn files_for(dir: &Path, pid: u32, id: u64) -> (PathBuf, PathBuf) {
    (dir.join(format!("{pid}-{id}.pxe")), dir.join(format!("{pid}-{id}.txt")))
}

/// A picture left behind by a run that didn't end normally.
#[derive(Clone)]
pub struct Found {
    pub pxe: PathBuf,
    pub meta: PathBuf,
    pub name: String,
    pub path: Option<PathBuf>,
    pub age_secs: u64,
}

/// Autosaves from runs that are no longer running.
pub fn find_left_behind() -> Vec<Found> {
    let Some(dir) = dir() else { return vec![] };
    let Ok(rd) = std::fs::read_dir(&dir) else { return vec![] };
    let me = std::process::id();
    let mut out = vec![];
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().map(|x| x != "pxe").unwrap_or(true) { continue; }
        let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(pid) = stem.split('-').next().and_then(|v| v.parse::<u32>().ok()) else { continue };
        if pid == me || Path::new(&format!("/proc/{pid}")).exists() { continue; }
        let meta = p.with_extension("txt");
        let text = std::fs::read_to_string(&meta).unwrap_or_default();
        let mut lines = text.lines();
        let name = lines.next().unwrap_or("Untitled").to_string();
        let path = lines.next().filter(|l| !l.is_empty()).map(PathBuf::from);
        let age_secs = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).map(|d| d.as_secs()).unwrap_or(0);
        out.push(Found { pxe: p, meta, name, path, age_secs });
    }
    out.sort_by_key(|f| f.age_secs);
    out
}

pub fn forget(f: &Found) {
    let _ = std::fs::remove_file(&f.pxe);
    let _ = std::fs::remove_file(&f.meta);
}

/// Open a left-behind picture as an unsaved document.
pub fn recover(f: &Found) -> Result<Document, String> {
    let (w, h, layers) = crate::io::read_pxe(&f.pxe)?;
    let mut d = Document::from_layers(&format!("{} (recovered)", f.name), w, h, layers, "Recovered");
    d.path = f.path.clone();
    d.dirty = true;
    Ok(d)
}

pub struct Autosaver {
    last: Instant,
    /// what was last written for each picture (by its history step), to skip unchanged ones
    written: HashMap<u64, usize>,
    busy: Option<std::sync::mpsc::Receiver<()>>,
}

impl Default for Autosaver {
    fn default() -> Self {
        Autosaver { last: Instant::now(), written: HashMap::new(), busy: None }
    }
}

/// A fingerprint of a picture's current history step: its layers are shared (Arc) between steps, so the
/// pointers change exactly when the content does.
fn step_key(d: &Document) -> usize {
    let st = &d.history[d.hist_pos].state;
    let mut k = d.hist_pos.wrapping_mul(31) ^ st.w.wrapping_mul(7) ^ st.h;
    for l in &st.layers { k = k.wrapping_mul(1_000_003) ^ (Arc::as_ptr(l) as usize); }
    k
}

impl Autosaver {
    /// Call every frame; writes in a background thread at most once per `INTERVAL`.
    pub fn tick(&mut self, docs: &[Document]) {
        if let Some(rx) = &self.busy {
            match rx.try_recv() {
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                _ => self.busy = None,
            }
        }
        if self.last.elapsed() < INTERVAL { return; }
        self.last = Instant::now();
        let Some(dir) = dir() else { return };
        let pid = std::process::id();
        // pictures that are saved (or gone) don't need a backup any more
        let live: Vec<u64> = docs.iter().filter(|d| d.dirty).map(|d| d.id).collect();
        let stale: Vec<u64> = self.written.keys().copied().filter(|id| !live.contains(id)).collect();
        for id in stale {
            let (a, b) = files_for(&dir, pid, id);
            let _ = std::fs::remove_file(a);
            let _ = std::fs::remove_file(b);
            self.written.remove(&id);
        }
        let mut jobs = vec![];
        for d in docs.iter().filter(|d| d.dirty) {
            let key = step_key(d);
            if self.written.get(&d.id) == Some(&key) { continue; }
            self.written.insert(d.id, key);
            let st = &d.history[d.hist_pos].state;
            let meta = format!("{}\n{}\n", d.name.trim_end_matches(" (recovered)"), d.path.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default());
            jobs.push((d.id, st.w, st.h, st.layers.clone(), meta));
        }
        if jobs.is_empty() { return; }
        let (tx, rx) = std::sync::mpsc::channel();
        self.busy = Some(rx);
        std::thread::spawn(move || {
            let _ = std::fs::create_dir_all(&dir);
            for (id, w, h, layers, meta) in jobs {
                let (pxe, txt) = files_for(&dir, pid, id);
                // write next to it first, then swap in, so a cut in the middle never leaves a broken file
                let tmp = pxe.with_extension("pxe.part");
                let ok = crate::io::write_layers(w, h, layers.iter().map(|l| l.as_ref()), &tmp, flate2::Compression::fast()).is_ok();
                if ok && std::fs::rename(&tmp, &pxe).is_ok() {
                    let _ = std::fs::write(&txt, meta);
                } else {
                    let _ = std::fs::remove_file(&tmp);
                }
            }
            let _ = tx.send(());
        });
    }

    /// The app is closing normally (unsaved work was confirmed): remove this run's backups.
    pub fn clear(&mut self) {
        if let Some(rx) = self.busy.take() { let _ = rx.recv_timeout(Duration::from_secs(10)); }
        let Some(dir) = dir() else { return };
        let pid = std::process::id();
        for id in self.written.keys() {
            let (a, b) = files_for(&dir, pid, *id);
            let _ = std::fs::remove_file(a);
            let _ = std::fs::remove_file(b);
        }
        self.written.clear();
    }
}
