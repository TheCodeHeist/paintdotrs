//! Paint.RS addition: drag and drop and picture clipboard for Wayland.
//!
//! Upstream winit 0.30 has no drag and drop on Wayland, and egui's Wayland clipboard only handles text.
//! This binds `wl_data_device` for each seat and:
//! - turns drops into `WindowEvent::DroppedFile` (file paths; web addresses as-is; dropped picture data
//!   saved to a temporary file first),
//! - keeps the current clipboard offer so the app can read any type from it (`clipboard_read`),
//! - lets the app put data on the clipboard (`clipboard_write`).

use std::io::Read;
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sctk::data_device_manager::data_device::DataDeviceHandler;
use sctk::data_device_manager::data_offer::{DataOfferHandler, DragOffer, SelectionOffer};
use sctk::data_device_manager::data_source::{DataSourceData, DataSourceHandler};
use sctk::data_device_manager::{ReadPipe, WritePipe};
use sctk::reexports::client::protocol::wl_data_device::WlDataDevice;
use sctk::reexports::client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use sctk::reexports::client::protocol::wl_data_source::WlDataSource;
use sctk::reexports::client::protocol::wl_seat::WlSeat;
use sctk::reexports::client::protocol::wl_surface::WlSurface;
use sctk::reexports::client::{Connection, QueueHandle};

use super::state::WinitState;
use crate::event::WindowEvent;

/// Serial of the latest keyboard or pointer input, needed to set the clipboard.
pub(crate) static LAST_SERIAL: AtomicU32 = AtomicU32::new(0);

pub(crate) fn note_serial(serial: u32) {
    LAST_SERIAL.store(serial, Ordering::Relaxed);
}

/// What the app-facing clipboard functions need, shared outside the event loop.
struct Shared {
    conn: Connection,
    qh: QueueHandle<WinitState>,
    manager: WlDataDeviceManager,
    devices: Vec<WlDataDevice>,
    selection: Option<SelectionOffer>,
    /// our own clipboard source and the data it serves, per type
    source: Option<(WlDataSource, Vec<(String, Vec<u8>)>)>,
}

static SHARED: Mutex<Option<Shared>> = Mutex::new(None);

/// Drag types we can use, best first.
const DROP_TYPES: &[&str] = &["text/uri-list", "text/x-moz-url", "image/png", "image/jpeg", "image/webp", "image/gif", "image/bmp", "text/plain;charset=utf-8", "text/plain", "UTF8_STRING"];

pub(crate) fn init(conn: &Connection, qh: &QueueHandle<WinitState>, state: &mut WinitState) {
    let Some(manager) = state.data_device_manager.as_ref() else { return };
    let mut devices = Vec::new();
    for seat in state.seat_state.seats() {
        let dev = manager.get_data_device(qh, &seat);
        devices.push(dev.inner().clone());
        state.data_devices.push(dev);
    }
    *SHARED.lock().unwrap() = Some(Shared {
        conn: conn.clone(),
        qh: qh.clone(),
        manager: manager.data_device_manager().clone(),
        devices,
        selection: None,
        source: None,
    });
}

pub(crate) fn new_seat(qh: &QueueHandle<WinitState>, state: &mut WinitState, seat: &WlSeat) {
    let Some(manager) = state.data_device_manager.as_ref() else { return };
    let dev = manager.get_data_device(qh, seat);
    if let Some(shared) = SHARED.lock().unwrap().as_mut() {
        shared.devices.push(dev.inner().clone());
    }
    state.data_devices.push(dev);
}

/// Read everything from a pipe, giving up after `timeout`.
fn read_all(mut pipe: ReadPipe, timeout: Duration) -> Option<Vec<u8>> {
    use rustix::event::{poll, PollFd, PollFlags};
    let start = Instant::now();
    let mut out = Vec::new();
    let mut buf = [0u8; 65536];
    loop {
        let left = timeout.checked_sub(start.elapsed())?;
        let ready = {
            let fd = pipe.as_fd();
            let mut fds = [PollFd::new(&fd, PollFlags::IN)];
            poll(&mut fds, left.as_millis().min(i32::MAX as u128) as i32).ok()?
        };
        if ready == 0 {
            return None;
        }
        match pipe.read(&mut buf) {
            Ok(0) => return Some(out),
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {},
            Err(_) => return None,
        }
    }
}

/// The types the current clipboard offers (empty when unknown).
pub fn clipboard_types() -> Vec<String> {
    let guard = SHARED.lock().unwrap();
    match guard.as_ref().and_then(|s| s.selection.as_ref()) {
        Some(offer) => offer.with_mime_types(|m| m.to_vec()),
        None => Vec::new(),
    }
}

/// Whether this backend is active (a Wayland window exists).
pub fn clipboard_available() -> bool {
    SHARED.lock().unwrap().is_some()
}

/// Read one type from the clipboard. `None` if it isn't offered or doesn't arrive in time.
pub fn clipboard_read(mime: &str, timeout: Duration) -> Option<Vec<u8>> {
    let (pipe, conn) = {
        let guard = SHARED.lock().unwrap();
        let shared = guard.as_ref()?;
        // our own data: answer directly (the compositor would ask us, and we're the ones waiting)
        if let Some((_, items)) = &shared.source {
            if let Some((_, data)) = items.iter().find(|(m, _)| m == mime) {
                return Some(data.clone());
            }
        }
        let offer = shared.selection.as_ref()?;
        if !offer.with_mime_types(|m| m.iter().any(|x| x == mime)) {
            return None;
        }
        (offer.receive(mime.to_string()).ok()?, shared.conn.clone())
    };
    let _ = conn.flush();
    read_all(pipe, timeout)
}

/// Put data on the clipboard, one entry per type. Returns false if there's no Wayland clipboard.
pub fn clipboard_write(items: Vec<(String, Vec<u8>)>) -> bool {
    let mut guard = SHARED.lock().unwrap();
    let Some(shared) = guard.as_mut() else { return false };
    let serial = LAST_SERIAL.load(Ordering::Relaxed);
    let source = shared.manager.create_data_source(&shared.qh, DataSourceData::default());
    for (mime, _) in &items {
        source.offer(mime.clone());
    }
    for dev in &shared.devices {
        dev.set_selection(Some(&source), serial);
    }
    if let Some((old, _)) = shared.source.replace((source, items)) {
        old.destroy();
    }
    let _ = shared.conn.flush();
    true
}

fn best_type(offer: &DragOffer) -> Option<String> {
    offer.with_mime_types(|m| DROP_TYPES.iter().find(|t| m.iter().any(|x| x == **t)).map(|t| t.to_string()))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Turn dropped data into "paths": local files, web/data addresses as-is, picture data saved to a temp file.
fn dropped_paths(mime: &str, data: &[u8]) -> Vec<PathBuf> {
    if let Some(ext) = mime.strip_prefix("image/") {
        let path = std::env::temp_dir().join(format!(
            "paint-rs-drop-{}-{}.{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0),
            if ext == "jpeg" { "jpg" } else { ext }
        ));
        return if std::fs::write(&path, data).is_ok() { vec![path] } else { vec![] };
    }
    let text = if mime == "text/x-moz-url" {
        // UTF-16: address, newline, title
        let u: Vec<u16> = data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&u)
    } else {
        String::from_utf8_lossy(data).into_owned()
    };
    let mut out = Vec::new();
    for line in text.split(['\r', '\n']).map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        if let Some(rest) = line.strip_prefix("file://") {
            // file:///path or file://host/path
            let path = if rest.starts_with('/') { rest.to_string() } else { rest.find('/').map(|i| rest[i..].to_string()).unwrap_or_default() };
            if !path.is_empty() {
                out.push(PathBuf::from(percent_decode(&path)));
            }
        } else if line.starts_with("http://") || line.starts_with("https://") || line.starts_with("data:") {
            out.push(PathBuf::from(line));
        } else if line.starts_with('/') {
            out.push(PathBuf::from(line));
        }
        if mime == "text/x-moz-url" {
            break; // only the address, not the title
        }
    }
    out
}

impl WinitState {
    fn current_drag_offer(&self, device: &WlDataDevice) -> Option<DragOffer> {
        self.data_devices.iter().find(|d| d.inner() == device).and_then(|d| d.data().drag_offer())
    }
}

impl DataDeviceHandler for WinitState {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, device: &WlDataDevice, _x: f64, _y: f64, surface: &WlSurface) {
        self.dnd_window = Some(super::make_wid(surface));
        if let Some(offer) = self.current_drag_offer(device) {
            let mime = best_type(&offer);
            offer.accept_mime_type(offer.serial, mime.clone());
            if mime.is_some() {
                offer.set_actions(DndAction::Copy | DndAction::Move, DndAction::Copy);
            }
        }
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, device: &WlDataDevice, _x: f64, _y: f64) {
        // some compositors want the type accepted again as the pointer moves
        if let Some(offer) = self.current_drag_offer(device) {
            let mime = best_type(&offer);
            offer.accept_mime_type(offer.serial, mime);
        }
    }

    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, device: &WlDataDevice) {
        let offer = self.data_devices.iter().find(|d| d.inner() == device).and_then(|d| d.data().selection_offer());
        if let Some(shared) = SHARED.lock().unwrap().as_mut() {
            shared.selection = offer;
        }
    }

    fn drop_performed(&mut self, conn: &Connection, _: &QueueHandle<Self>, device: &WlDataDevice) {
        let Some(offer) = self.current_drag_offer(device) else { return };
        let Some(window_id) = self.dnd_window.or_else(|| Some(super::make_wid(&offer.surface))) else { return };
        let Some(mime) = best_type(&offer) else {
            offer.destroy();
            return;
        };
        let data = match offer.receive(mime.clone()) {
            Ok(pipe) => {
                let _ = conn.flush();
                read_all(pipe, Duration::from_secs(5))
            },
            Err(_) => None,
        };
        offer.finish();
        offer.destroy();
        if let Some(data) = data {
            for path in dropped_paths(&mime, &data) {
                self.events_sink.push_window_event(WindowEvent::DroppedFile(path), window_id);
            }
        }
    }
}

impl DataOfferHandler for WinitState {
    fn source_actions(&mut self, _: &Connection, _: &QueueHandle<Self>, offer: &mut DragOffer, actions: DndAction) {
        if actions.contains(DndAction::Copy) {
            offer.set_actions(DndAction::Copy, DndAction::Copy);
        } else if actions.contains(DndAction::Move) {
            offer.set_actions(DndAction::Move, DndAction::Move);
        }
    }

    fn selected_action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &mut DragOffer, _: DndAction) {}
}

impl DataSourceHandler for WinitState {
    fn accept_mime(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: Option<String>) {}

    fn send_request(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource, mime: String, fd: WritePipe) {
        let data = {
            let guard = SHARED.lock().unwrap();
            guard.as_ref().and_then(|s| s.source.as_ref()).filter(|(src, _)| src == source).and_then(|(_, items)| items.iter().find(|(m, _)| *m == mime).map(|(_, d)| d.clone()))
        };
        if let Some(data) = data {
            // write from a thread so a slow reader never blocks the window
            std::thread::spawn(move || {
                use std::io::Write;
                let mut fd = fd;
                let _ = fd.write_all(&data);
            });
        }
    }

    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        let mut guard = SHARED.lock().unwrap();
        if let Some(shared) = guard.as_mut() {
            if shared.source.as_ref().map(|(s, _)| s == source).unwrap_or(false) {
                shared.source = None;
            }
        }
        source.destroy();
    }

    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
}

sctk::delegate_data_device!(WinitState);
