//! The editor window: menus, panels, canvas and every action.

use crate::doc::{over, BlendMode, Document, Layer, Px};
use crate::effects::{self, Fx};
use crate::io;
use crate::raster::{self, Coverage, FillStyle, GradientKind, ShapeKind};
use crate::tools::{self, Tool};
use eframe::egui::{self, pos2, vec2, Color32, Key, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Vec2};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::mpsc;
use crate::clip;

mod freexf;

pub const APP_NAME: &str = "Paint.RS";
pub const DEFAULT_W: usize = 800;
pub const DEFAULT_H: usize = 600;

/// New-image size rule from the original's `AppWorkspace.GetNewDocumentSize` (paintdotnet.dll 0x06001421):
/// 800 x 600 on a landscape screen, 600 x 800 on a portrait one (aspect < 1.0), scaled by the UI scale.
pub fn new_image_size(ctx: &egui::Context) -> (u32, u32) {
    let r = ctx.content_rect();
    let scale = ctx.pixels_per_point().max(0.5);
    let (w, h) = if r.width() / r.height().max(1.0) < 1.0 { (DEFAULT_H, DEFAULT_W) } else { (DEFAULT_W, DEFAULT_H) };
    (((w as f32) * scale).round() as u32, ((h as f32) * scale).round() as u32)
}
/// Zoom steps from the original's `ScaleFactor` static table (paintdotnet.dll 0x060001c2), as fractions.
pub const ZOOM_PRESETS: [f32; 45] = [
    1.0 / 100.0, 1.0 / 88.0, 1.0 / 76.0, 1.0 / 64.0, 1.0 / 56.0, 1.0 / 48.0, 1.0 / 40.0, 1.0 / 32.0, 1.0 / 28.0,
    1.0 / 24.0, 1.0 / 20.0, 1.0 / 16.0, 1.0 / 14.0, 1.0 / 12.0, 1.0 / 10.0, 1.0 / 8.0, 1.0 / 6.0, 1.0 / 5.0,
    1.0 / 4.0, 1.0 / 3.0, 1.0 / 2.0, 2.0 / 3.0, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0,
    20.0, 24.0, 28.0, 32.0, 40.0, 48.0, 56.0, 64.0, 76.0, 88.0, 100.0,
];
/// Mouse wheel on the canvas (CanvasControl.ProcessVerticalPointerWheel, paintdotnet.dll 0x06002299).
pub const WHEEL_ZOOM_FACTOR: f64 = 1.12; // zoom change per notch with Ctrl
pub const WHEEL_ZOOM_SNAP: f64 = 0.05; // snap to a whole-number zoom within 5 %
pub const WHEEL_SCROLL_PIXELS: f32 = 120.0; // screen pixels scrolled per notch (one Windows wheel notch = 120)
/// Touchpads report pixels instead of notches; this many count as one notch (estimated).
pub const WHEEL_PIXELS_PER_NOTCH: f32 = 50.0;
/// Number boxes move by the system's "lines per wheel notch" (3 by default) steps per notch, and ignore
/// the wheel while Shift or Alt is held (PdnNumericSpin.OnMouseWheel, PaintDotNet.Framework.dll 0x0600077f).
pub const NUMBER_STEPS_PER_NOTCH: f64 = 3.0;

/// Where a pasted picture goes, from the original's PasteImageAction.PerformActionImpl (paintdotnet.dll
/// 0x060028f4): at the canvas corner if it fits in the visible part there; otherwise moved so it starts at
/// the visible part's left/top edge, or ends at its right/bottom edge if it would stick out, never above 0.
pub fn paste_origin(pw: usize, ph: usize, visible: Option<(usize, usize, usize, usize)>) -> (usize, usize) {
    let Some((vx0, vy0, vx1, vy1)) = visible else { return (0, 0) };
    let place = |size: usize, lo: usize, hi: usize| -> usize {
        let mut p: i64 = 0;
        if p < lo as i64 { p = lo as i64; }
        if p + size as i64 > hi as i64 { p = hi as i64 - size as i64; }
        p.max(0) as usize
    };
    (place(pw, vx0, vx1), place(ph, vy0, vy1))
}

/// Mouse wheel over a number box: change it by 3 steps per notch, like the original's number boxes.
fn wheel_number<N: egui::emath::Numeric>(ui: &egui::Ui, resp: &egui::Response, value: &mut N, range: std::ops::RangeInclusive<N>, step: f64) -> bool {
    if !resp.hovered() {
        return false;
    }
    let notches: f32 = ui.input(|i| i.events.iter().filter_map(|e| match e {
        egui::Event::MouseWheel { unit, delta, modifiers, .. } if !modifiers.shift && !modifiers.alt => Some(match unit {
            egui::MouseWheelUnit::Point => delta.y / WHEEL_PIXELS_PER_NOTCH,
            egui::MouseWheelUnit::Line => delta.y,
            egui::MouseWheelUnit::Page => delta.y * 3.0,
        }),
        _ => None,
    }).sum());
    let steps = (notches as f64 * NUMBER_STEPS_PER_NOTCH).round();
    if steps == 0.0 {
        return false;
    }
    let v = (value.to_f64() + steps * step).clamp(range.start().to_f64(), range.end().to_f64());
    *value = N::from_f64(v);
    true
}

/// A number box (drag or type) that also follows the mouse wheel.
fn num_box<N: egui::emath::Numeric>(ui: &mut egui::Ui, value: &mut N, range: std::ops::RangeInclusive<N>, step: f64, suffix: &str) -> egui::Response {
    let mut resp = ui.add(egui::DragValue::new(value).range(range.clone()).speed(step * 0.3).max_decimals(if N::INTEGRAL { 0 } else { 2 }).suffix(suffix));
    if wheel_number(ui, &resp, value, range, step) {
        resp.mark_changed();
    }
    resp
}

/// Defaults from the original's settings sections (paintdotnet.dll AppSettings, see MECHANICS.md).
pub const DEFAULT_BRUSH_SIZE: f32 = 2.0; // BrushSection "Size" 2 (1..2000)
pub const DEFAULT_TOLERANCE: f32 = 0.5; // ToolsSection "Tolerance" 0.5 (0..1)
pub const DEFAULT_FONT_SIZE: f32 = 12.0; // TextSection "FontSize" 12 (1..2000), in points
/// Text sizes are points (TextSection "FontSizeMetric" default 0 = Points, paintdotnet.dll 0x06003575),
/// drawn at the picture's resolution (TextTool.GetSizedFontProperties 0x06001cf4): pixels = points × DPI / 72.
/// Pictures here use the original's default resolution of 96 DPI.
pub const TEXT_DPI: f32 = 96.0;
pub fn points_to_pixels(pt: f32) -> f32 { pt * TEXT_DPI / 72.0 }
pub const DEFAULT_RADIUS: f32 = 10.0; // ShapesSection "Radius" 10 dips (0..2000)

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SelMode { Replace, Union, Exclude, Intersect, Xor }
impl SelMode {
    const ALL: [SelMode; 5] = [SelMode::Replace, SelMode::Union, SelMode::Exclude, SelMode::Intersect, SelMode::Xor];
    fn name(self) -> &'static str {
        match self { SelMode::Replace => "Replace", SelMode::Union => "Add (union)", SelMode::Exclude => "Subtract", SelMode::Intersect => "Intersect", SelMode::Xor => "Invert (xor)" }
    }
}

pub struct Opts {
    pub brush: f32,
    pub aa: bool,
    pub fill: FillStyle,
    pub tol: f32,
    pub global: bool,
    pub gradient: GradientKind,
    pub sel_mode: SelMode,
    pub radius: f32,
    pub font_size: f32,
    /// Text tool font: family (None until the system fonts are scanned), weight (100..900) and italic
    pub font_family: Option<String>,
    pub font_weight: u16,
    pub font_italic: bool,
    /// Magic Wand / Paint Bucket compare colours of the whole picture instead of the active layer
    pub sample_image: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum PaintMode { Paint(Px), Erase, Clone((i32, i32)), Recolor(Px, Px) }

struct Drag {
    secondary: bool,
    start: (f32, f32),
    last: (f32, f32),
    snap: Vec<Px>,
    sel_snap: Option<Vec<u8>>,
    cov: Option<Coverage>,
    lasso: Vec<(f32, f32)>,
    mode: Option<PaintMode>,
    float: Option<(Vec<Px>, Vec<u8>)>, // move pixels: base layer and lifted mask
    pan0: (f32, f32),
    moved: bool,
    /// area the last shape preview drew into
    prev_rect: Option<(usize, usize, usize, usize)>,
    /// rectangle/ellipse selection being dragged (image coords), turned into a mask on release
    sel_box: Option<(f32, f32, f32, f32)>,
    /// how far the selection (and pixels) have been dragged
    offset: (i32, i32),
    /// box of the pixels being moved, at drag start
    src_box: Option<(usize, usize, usize, usize)>,
}

struct TextEdit { pos: (f32, f32), text: String, snap: Vec<Px>, color: Px, prev_rect: Option<(usize, usize, usize, usize)> }

enum FileMode { Open, Save, Export, ImportLayer }

/// Where a pasted picture goes (kept while a copied link downloads).
#[derive(Clone, Copy)]
enum PasteTarget { Layer, NewLayer, NewImage }

enum Dialog {
    NewImage { w: u32, h: u32 },
    Resize { w: u32, h: u32, keep: bool },
    CanvasSize { w: u32, h: u32, anchor: (i32, i32) },
    LayerProps { name: String, visible: bool, opacity: u8, blend: BlendMode, orig: (String, bool, u8, BlendMode) },
    Effect { fx: Fx, vals: Vec<f32>, snap: Vec<Px>, applied: Vec<f32> },
    File { mode: FileMode, dir: PathBuf, name: String, ext: usize, err: String, entries: Vec<(String, bool)> },
    Confirm { doc: usize, then: AfterConfirm },
    Message(String),
    /// painting on a smart object: turn it into ordinary pixels first?
    Rasterize,
    /// pasted picture bigger than the canvas: expand the canvas or keep its size
    PasteBig { pic: clip::Picture, new_layer: bool },
    /// files dropped on the window: open them or add them as layers (like the original)
    DropChoice { items: Vec<PathBuf> },
    /// the chosen file already exists
    Overwrite { path: PathBuf, export: bool },
    About,
}

#[derive(Clone, Copy)]
enum AfterConfirm { CloseDoc, Quit }

pub struct App {
    docs: Vec<Document>,
    cur: usize,
    tool: Tool,
    primary: Px,
    secondary: Px,
    opts: Opts,
    palette: Vec<Px>,
    tex: Option<(TextureHandle, usize, u64, bool)>,
    checker: Option<TextureHandle>,
    thumbs: Vec<(u64, TextureHandle)>,
    thumbs_key: (usize, u64),
    outline: (usize, u64, Vec<((f32, f32), (f32, f32))>),
    outline_mask: Option<Vec<u8>>,
    /// when the effect preview was last recomputed
    fx_last: f64,
    drag: Option<Drag>,
    text: Option<TextEdit>,
    clone_anchor: Option<(f32, f32)>,
    clipboard: Option<(usize, usize, Vec<Px>)>,
    /// a copied selection outline (Edit > Copy Selection)
    sel_clipboard: Option<(usize, usize, Vec<u8>)>,
    /// selected pixels lifted for free transform (see freexf.rs)
    floating: Option<freexf::Floating>,
    /// a transform handle being dragged
    xdrag: Option<freexf::XDrag>,
    dialog: Option<Dialog>,
    pixel_grid: bool,
    edit_secondary: bool,
    colors_more: bool,
    /// search text in the font list
    font_filter: String,
    hover: Option<(i32, i32)>,
    title: String,
    last_dir: PathBuf,
    jpeg_quality: u8,
    shot: Option<PathBuf>,
    frame: u32,
    shot_done: Arc<AtomicBool>,
    allow_quit: bool,
    sysclip: clip::SysClipboard,
    download: Option<(mpsc::Receiver<Result<clip::Picture, String>>, PasteTarget)>,
    /// short note shown in the status bar
    status: String,
    /// visible part of the picture (x0, y0, x1, y1), updated each frame
    visible_img: Option<(usize, usize, usize, usize)>,
    dark: bool,
}

/// Colours for the light and dark looks.
#[derive(Clone, Copy)]
struct Theme {
    canvas_bg: Color32,
    tabs_bg: Color32,
    sel_bg: Color32,
    hover_bg: Color32,
    accent: Color32,
    icon: Color32,
    text: Color32,
    muted: Color32,
    border: Color32,
}

const LIGHT: Theme = Theme {
    canvas_bg: Color32::from_rgb(207, 211, 218),
    tabs_bg: Color32::from_rgb(232, 234, 238),
    sel_bg: Color32::from_rgb(204, 228, 247),
    hover_bg: Color32::from_rgb(229, 236, 245),
    accent: Color32::from_rgb(0, 103, 192),
    icon: Color32::from_gray(45),
    text: Color32::from_gray(30),
    muted: Color32::from_gray(150),
    border: Color32::from_gray(120),
};

const DARK: Theme = Theme {
    canvas_bg: Color32::from_rgb(28, 29, 33),
    tabs_bg: Color32::from_rgb(40, 42, 47),
    sel_bg: Color32::from_rgb(36, 74, 114),
    hover_bg: Color32::from_rgb(52, 56, 63),
    accent: Color32::from_rgb(86, 166, 240),
    icon: Color32::from_gray(218),
    text: Color32::from_gray(228),
    muted: Color32::from_gray(115),
    border: Color32::from_gray(85),
};

fn apply_look(ctx: &egui::Context, dark: bool) {
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    if dark {
        v.panel_fill = Color32::from_rgb(36, 38, 42);
        v.window_fill = Color32::from_rgb(44, 46, 51);
        v.selection.bg_fill = Color32::from_rgb(40, 110, 190);
    } else {
        v.panel_fill = Color32::from_rgb(243, 244, 247);
        v.window_fill = Color32::from_rgb(250, 250, 252);
        v.selection.bg_fill = Color32::from_rgb(0, 103, 192);
    }
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    ctx.set_visuals(v);
}

/// Where the dark-mode choice is remembered between runs.
fn settings_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("paint-rs").join("settings.txt"))
}

fn load_dark_setting() -> Option<bool> {
    let text = std::fs::read_to_string(settings_path()?).ok()?;
    text.lines().find_map(|l| l.strip_prefix("dark=")).map(|v| v.trim() == "1")
}

fn save_dark_setting(dark: bool) {
    if let Some(p) = settings_path() {
        if let Some(dir) = p.parent() { let _ = std::fs::create_dir_all(dir); }
        let _ = std::fs::write(p, format!("dark={}\n", if dark { 1 } else { 0 }));
    }
}

fn c32(p: Px) -> Color32 { Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]) }
fn px(c: Color32) -> Px { c.to_srgba_unmultiplied() }

fn default_palette() -> Vec<Px> {
    let mut v = Vec::new();
    for i in 0..8 {
        let g = (i * 255 / 7) as u8;
        v.push([g, g, g, 255]);
    }
    for row in 0..3 {
        for i in 0..8 {
            let hue = i as f32 * 45.0;
            let (s, val) = match row { 0 => (1.0, 1.0), 1 => (0.5, 1.0), _ => (1.0, 0.55) };
            let c = effects::hsv_to_rgb(hue, s, val);
            v.push([c[0], c[1], c[2], 255]);
        }
    }
    v
}

impl App {
    pub fn new(cc: &eframe::CreationContext, shot: Option<PathBuf>, files: Vec<PathBuf>) -> Self {
        // remembered choice first, then the desktop's own light/dark setting
        let dark = load_dark_setting()
            .or_else(|| cc.egui_ctx.system_theme().map(|t| t == egui::Theme::Dark))
            .unwrap_or(false);
        apply_look(&cc.egui_ctx, dark);

        // scan the system fonts for the Text tool while the window opens
        crate::fonts::load_in_background();
        let mut app = App {
            docs: vec![],
            cur: 0,
            tool: Tool::Paintbrush,
            primary: [0, 0, 0, 255],
            secondary: [255, 255, 255, 255],
            opts: Opts { brush: DEFAULT_BRUSH_SIZE, aa: true, fill: FillStyle::Outline, tol: DEFAULT_TOLERANCE, global: false, gradient: GradientKind::Linear, sel_mode: SelMode::Replace, radius: DEFAULT_RADIUS, font_size: DEFAULT_FONT_SIZE, font_family: None, font_weight: 400, font_italic: false, sample_image: false },
            palette: default_palette(),
            tex: None,
            checker: None,
            thumbs: vec![],
            thumbs_key: (usize::MAX, 0),
            outline: (usize::MAX, 0, vec![]),
            outline_mask: None,
            fx_last: 0.0,
            drag: None,
            text: None,
            clone_anchor: None,
            clipboard: None,
            sel_clipboard: None,
            floating: None,
            xdrag: None,
            dialog: None,
            pixel_grid: false,
            edit_secondary: false,
            colors_more: false,
            font_filter: String::new(),
            hover: None,
            title: String::new(),
            last_dir: std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/")),
            jpeg_quality: 95,
            shot,
            frame: 0,
            shot_done: Arc::new(AtomicBool::new(false)),
            allow_quit: false,
            sysclip: clip::SysClipboard::new(),
            download: None,
            status: String::new(),
            visible_img: None,
            dark,
        };
        for f in files {
            match io::open(&f) {
                Ok(d) => app.docs.push(d),
                Err(e) => app.dialog = Some(Dialog::Message(format!("Couldn't open {}: {e}", f.display()))),
            }
        }
        if app.docs.is_empty() {
            app.docs.push(Document::new("Untitled", DEFAULT_W, DEFAULT_H, [255, 255, 255, 255]));
        }
        app.cur = app.docs.len() - 1;
        app
    }

    fn theme(&self) -> Theme { if self.dark { DARK } else { LIGHT } }

    fn set_dark(&mut self, ctx: &egui::Context, dark: bool) {
        self.dark = dark;
        apply_look(ctx, dark);
        save_dark_setting(dark);
    }

    fn doc(&mut self) -> &mut Document { &mut self.docs[self.cur] }

    // ---------------- selection helpers ----------------

    fn combine_sel(&self, old: &Option<Vec<u8>>, new: Vec<u8>, mode: SelMode) -> Option<Vec<u8>> {
        let n = new.len();
        let old_or = |i: usize| -> u8 { match old { Some(m) => m[i], None => 0 } };
        let out: Vec<u8> = match mode {
            SelMode::Replace => new,
            SelMode::Union => (0..n).map(|i| old_or(i).max(new[i])).collect(),
            SelMode::Exclude => (0..n).map(|i| if new[i] >= 128 { 0 } else { old_or(i) }).collect(),
            SelMode::Intersect => (0..n).map(|i| old_or(i).min(new[i])).collect(),
            SelMode::Xor => (0..n).map(|i| if (old_or(i) >= 128) != (new[i] >= 128) { 255 } else { 0 }).collect(),
        };
        if out.iter().all(|v| *v == 0) { None } else { Some(out) }
    }

    /// Selection bounding box (cached in the document until the selection changes).
    fn sel_bounds(d: &Document) -> Option<(usize, usize, usize, usize)> {
        d.sel_bounds()
    }

    // ---------------- painting ----------------

    /// Paint the part of a stroke added since the last call (only that box is recomputed).
    fn apply_cov(&mut self, cov: &mut Coverage, snap: &[Px], mode: PaintMode) {
        let tol = raster::tolerance_byte(self.opts.tol);
        let Some((x0, y0, x1, y1)) = cov.take_recent() else { return };
        let d = &mut self.docs[self.cur];
        let (w, h) = (d.w(), d.h());
        let crate::doc::State { selection, layers, active, .. } = &mut d.s;
        let sel = selection.as_deref();
        let layer = &mut layers[*active].px;
        for y in y0..y1 {
            for x in x0..x1 {
                let i = y * w + x;
                let mut c = cov.get(x, y) as u32;
                if let Some(m) = sel { c = c * m[i] as u32 / 255; }
                let base = snap[i];
                if c == 0 { layer[i] = base; continue; }
                let c = c as u8;
                layer[i] = match mode {
                    PaintMode::Paint(col) => over(base, col, c),
                    PaintMode::Erase => { let mut o = base; o[3] = (base[3] as u32 * (255 - c as u32) / 255) as u8; o }
                    PaintMode::Clone((ox, oy)) => {
                        let sx = x as i32 + ox; let sy = y as i32 + oy;
                        if sx < 0 || sy < 0 || sx >= w as i32 || sy >= h as i32 { base } else { over(base, snap[sy as usize * w + sx as usize], c) }
                    }
                    PaintMode::Recolor(to, from) => {
                        if raster::color_distance(base, from) <= tol { over(base, [to[0], to[1], to[2], base[3]], c) } else { base }
                    }
                };
            }
        }
        d.mark_rect((x0, y0, x1, y1));
    }

    /// Draw shape coverages onto the active layer from `snap`, honouring selection. Only the box the
    /// previous preview used (`prev`) and the new one are touched.
    fn put_cov(&mut self, snap: &[Px], covs: &[(&Coverage, Px)], prev: &mut Option<(usize, usize, usize, usize)>) {
        fn union(a: Option<(usize, usize, usize, usize)>, b: Option<(usize, usize, usize, usize)>) -> Option<(usize, usize, usize, usize)> {
            match (a, b) {
                (Some(a), Some(b)) => Some((a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))),
                (a, None) => a,
                (None, b) => b,
            }
        }
        let mut now = None;
        for (cov, _) in covs { now = union(now, cov.bounds()); }
        let Some(area) = union(*prev, now) else { return };
        let d = &mut self.docs[self.cur];
        let w = d.w();
        let crate::doc::State { selection, layers, active, .. } = &mut d.s;
        let sel = selection.as_deref();
        let layer = &mut layers[*active].px;
        for y in area.1..area.3 {
            let r = y * w + area.0..y * w + area.2;
            layer[r.clone()].copy_from_slice(&snap[r]);
        }
        for (cov, col) in covs {
            if let Some((x0, y0, x1, y1)) = cov.bounds() {
                for y in y0..y1 {
                    for x in x0..x1 {
                        let i = y * w + x;
                        let mut c = cov.get(x, y) as u32;
                        if let Some(m) = sel { c = c * m[i] as u32 / 255; }
                        if c > 0 { layer[i] = over(layer[i], *col, c as u8); }
                    }
                }
            }
        }
        d.mark_rect(area);
        *prev = now;
    }

    fn shift_constrain(a: (f32, f32), b: (f32, f32), square: bool) -> (f32, f32) {
        if !square { return b; }
        let dx = b.0 - a.0;
        let dy = b.1 - a.1;
        let m = dx.abs().max(dy.abs());
        (a.0 + m * dx.signum(), a.1 + m * dy.signum())
    }

    /// Shift-constrained angle: 15-degree steps, rounded away from zero, from the original's
    /// `GradientToolChanges.ConstrainPoints` (paintdotnet.dll 0x06001fd4). With `whole_pixels` (the line tool,
    /// `ShapesToolChanges.ConstrainPoints15Degrees` 0x06001d8f) the length is also rounded at multiples of
    /// 90 degrees, and at 45 degrees rounded in steps of sqrt(2) so the end lands on a whole pixel.
    fn snap_angle_with(a: (f32, f32), b: (f32, f32), whole_pixels: bool) -> (f32, f32) {
        let (dx, dy) = ((b.0 - a.0) as f64, (b.1 - a.1) as f64);
        let mut len = (dx * dx + dy * dy).sqrt();
        let step = std::f64::consts::PI / 12.0;
        let k = (dy.atan2(dx) / step).round(); // f64::round rounds halves away from zero
        let ang = k * step;
        if whole_pixels {
            let ki = k as i64;
            if ki.rem_euclid(6) == 0 {
                len = len.round();
            } else if ki.rem_euclid(3) == 0 {
                len = (len / std::f64::consts::SQRT_2).round() * std::f64::consts::SQRT_2;
            }
        }
        (a.0 + (len * ang.cos()) as f32, a.1 + (len * ang.sin()) as f32)
    }

    fn snap_angle(a: (f32, f32), b: (f32, f32)) -> (f32, f32) { Self::snap_angle_with(a, b, false) }

    fn begin_drag(&mut self, pos: (f32, f32), secondary: bool, mods: egui::Modifiers) {
        let tool = self.tool;
        // tools that change pixels can't paint on a smart object until it's rasterized
        let paints = matches!(tool, Tool::Paintbrush | Tool::Eraser | Tool::Pencil | Tool::CloneStamp | Tool::Recolor
            | Tool::PaintBucket | Tool::Gradient | Tool::Line | Tool::Rectangle | Tool::RoundedRect | Tool::Ellipse | Tool::Text);
        if paints && !(tool == Tool::CloneStamp && mods.ctrl) && self.smart_guard() {
            return;
        }
        let (w, h) = (self.docs[self.cur].w(), self.docs[self.cur].h());
        let inside = pos.0 >= 0.0 && pos.1 >= 0.0 && (pos.0 as usize) < w && (pos.1 as usize) < h;
        let (prim, sec) = if secondary { (self.secondary, self.primary) } else { (self.primary, self.secondary) };
        let d = &self.docs[self.cur];
        let mut drag = Drag {
            secondary, start: pos, last: pos, snap: d.layer().px.clone(), sel_snap: d.s.selection.clone(),
            cov: None, lasso: vec![pos], mode: None, float: None, pan0: d.pan, moved: false, prev_rect: None, sel_box: None, offset: (0, 0), src_box: d.sel_bounds().or(Some((0, 0, w, h))),
        };
        match tool {
            Tool::Paintbrush | Tool::Eraser | Tool::Pencil | Tool::CloneStamp | Tool::Recolor => {
                if tool == Tool::CloneStamp {
                    if mods.ctrl { self.clone_anchor = Some(pos); return; }
                    let Some(an) = self.clone_anchor else {
                        self.dialog = Some(Dialog::Message("Hold Ctrl and click first to choose where the Clone Stamp copies from.".into()));
                        return;
                    };
                    drag.mode = Some(PaintMode::Clone(((an.0 - pos.0).round() as i32, (an.1 - pos.1).round() as i32)));
                } else {
                    drag.mode = Some(match tool {
                        Tool::Eraser => PaintMode::Erase,
                        Tool::Recolor => PaintMode::Recolor(prim, sec),
                        _ => PaintMode::Paint(prim),
                    });
                }
                let mut cov = Coverage::new(w, h);
                if tool == Tool::Pencil { let q = (pos.0.floor() as i32, pos.1.floor() as i32); cov.pixel_line(q, q); }
                else { cov.dab(pos.0, pos.1, self.opts.brush, self.opts.aa); }
                let snap = drag.snap.clone();
                let mode = drag.mode.unwrap();
                self.apply_cov(&mut cov, &snap, mode);
                drag.cov = Some(cov);
            }
            Tool::MovePixels => {
                let mask = d.s.selection.clone().unwrap_or_else(|| vec![255; w * h]);
                let base: Vec<Px> = drag.snap.iter().zip(mask.iter()).map(|(p, m)| { let mut o = *p; o[3] = (p[3] as u32 * (255 - *m as u32) / 255) as u8; o }).collect();
                drag.float = Some((base, mask));
            }
            Tool::PaintBucket => {
                if !inside { return; }
                let sample = if self.opts.sample_image { d.composite() } else { d.layer().px.clone() };
                let mut m = raster::flood_mask(&sample, w, h, pos.0 as usize, pos.1 as usize, self.opts.tol, self.opts.global || mods.shift);
                if let Some(s) = &d.s.selection { for (a, b) in m.iter_mut().zip(s.iter()) { *a = (*a).min(*b); } }
                let d = &mut self.docs[self.cur];
                let a = d.s.active;
                for (p, mv) in d.s.layers[a].px.iter_mut().zip(m.iter()) { if *mv > 0 { *p = over(*p, prim, *mv); } }
                d.commit("Paint Bucket");
                return;
            }
            Tool::MagicWand => {
                if !inside { return; }
                let m = if self.opts.sample_image {
                    raster::flood_mask(&d.composite(), w, h, pos.0 as usize, pos.1 as usize, self.opts.tol, self.opts.global || mods.shift)
                } else {
                    raster::flood_mask(&d.layer().px, w, h, pos.0 as usize, pos.1 as usize, self.opts.tol, self.opts.global || mods.shift)
                };
                let mode = if mods.ctrl { SelMode::Union } else if mods.alt || secondary { SelMode::Exclude } else { self.opts.sel_mode };
                let ns = self.combine_sel(&d.s.selection, m, mode);
                let d = &mut self.docs[self.cur];
                d.s.selection = ns;
                d.commit("Magic Wand");
                return;
            }
            Tool::ColorPicker => {
                if !inside { return; }
                let i = pos.1 as usize * w + pos.0 as usize;
                let c = if mods.ctrl { d.composite()[i] } else { d.layer().px[i] };
                if secondary { self.secondary = c } else { self.primary = c }
                return;
            }
            Tool::Zoom => {
                self.zoom_step(if secondary { -1 } else { 1 }, Some(pos));
                return;
            }
            Tool::Text => {
                self.finish_text();
                let snap = self.docs[self.cur].layer().px.clone();
                self.text = Some(TextEdit { pos, text: String::new(), snap, color: self.primary, prev_rect: None });
                return;
            }
            _ => {}
        }
        self.drag = Some(drag);
    }

    fn update_drag(&mut self, pos: (f32, f32), mods: egui::Modifiers) {
        let Some(mut drag) = self.drag.take() else { return };
        if (pos.0 - drag.start.0).abs() + (pos.1 - drag.start.1).abs() > 1.0 { drag.moved = true; }
        let (w, h) = (self.docs[self.cur].w(), self.docs[self.cur].h());
        let (prim, sec) = if drag.secondary { (self.secondary, self.primary) } else { (self.primary, self.secondary) };
        match self.tool {
            Tool::Paintbrush | Tool::Eraser | Tool::Pencil | Tool::CloneStamp | Tool::Recolor => {
                if let (Some(mut cov), Some(mode)) = (drag.cov.take(), drag.mode) {
                    if self.tool == Tool::Pencil {
                        cov.pixel_line((drag.last.0.floor() as i32, drag.last.1.floor() as i32), (pos.0.floor() as i32, pos.1.floor() as i32));
                    } else {
                        cov.stroke(drag.last, pos, self.opts.brush, self.opts.aa);
                    }
                    self.apply_cov(&mut cov, &drag.snap, mode);
                    drag.cov = Some(cov);
                }
            }
            Tool::Line => {
                let b = if mods.shift { Self::snap_angle_with(drag.start, pos, true) } else { pos };
                let c = raster::line_coverage(w, h, drag.start, b, self.opts.brush, self.opts.aa);
                self.put_cov(&drag.snap, &[(&c, prim)], &mut drag.prev_rect);
            }
            Tool::Rectangle | Tool::RoundedRect | Tool::Ellipse => {
                let kind = match self.tool { Tool::Rectangle => ShapeKind::Rectangle, Tool::RoundedRect => ShapeKind::RoundedRect, _ => ShapeKind::Ellipse };
                let b = Self::shift_constrain(drag.start, pos, mods.shift);
                let (line, fill) = raster::shape_coverage(w, h, kind, drag.start, b, self.opts.brush, self.opts.radius, self.opts.aa);
                match self.opts.fill {
                    FillStyle::Outline => self.put_cov(&drag.snap, &[(&line, prim)], &mut drag.prev_rect),
                    FillStyle::Fill => self.put_cov(&drag.snap, &[(&fill, prim)], &mut drag.prev_rect),
                    FillStyle::Both => self.put_cov(&drag.snap, &[(&fill, sec), (&line, prim)], &mut drag.prev_rect),
                }
            }
            Tool::Gradient => {
                let b = if mods.shift { Self::snap_angle(drag.start, pos) } else { pos };
                let kind = self.opts.gradient;
                let d = &mut self.docs[self.cur];
                // only the selected box changes; rows are shared out across the CPU cores
                let area = d.sel_bounds().unwrap_or((0, 0, w, h));
                let crate::doc::State { selection, layers, active, .. } = &mut d.s;
                let sel = selection.as_deref();
                let layer = &mut layers[*active].px;
                let snap = &drag.snap;
                let start = drag.start;
                use rayon::prelude::*;
                layer.par_chunks_mut(w).enumerate().skip(area.1).take(area.3 - area.1).for_each(|(y, row)| {
                    for x in area.0..area.2 {
                        let i = y * w + x;
                        let m = sel.map(|s| s[i]).unwrap_or(255);
                        if m == 0 { row[x] = snap[i]; continue; }
                        let t = kind.t(start, b, x as f32 + 0.5, y as f32 + 0.5);
                        row[x] = over(snap[i], raster::lerp_px(prim, sec, t), m);
                    }
                });
                d.mark_rect(area);
            }
            Tool::RectSelect | Tool::EllipseSelect => {
                // only remember the box while dragging; the mask is built once on release
                let b = Self::shift_constrain(drag.start, pos, mods.shift);
                drag.sel_box = Some((drag.start.0.min(b.0).round(), drag.start.1.min(b.1).round(), drag.start.0.max(b.0).round(), drag.start.1.max(b.1).round()));
            }
            Tool::LassoSelect => {
                drag.lasso.push(pos);
            }
            Tool::MoveSelection => {
                drag.offset = ((pos.0 - drag.start.0).round() as i32, (pos.1 - drag.start.1).round() as i32);
            }
            Tool::MovePixels => {
                let (dx, dy) = ((pos.0 - drag.start.0).round() as i32, (pos.1 - drag.start.1).round() as i32);
                drag.offset = (dx, dy);
                let Some(src) = drag.src_box else { return };
                // the moved pixels' new box, clipped to the picture
                let clip = |x: i32, lim: usize| x.clamp(0, lim as i32) as usize;
                let now = (clip(src.0 as i32 + dx, w), clip(src.1 as i32 + dy, h), clip(src.2 as i32 + dx, w), clip(src.3 as i32 + dy, h));
                let now = if now.0 < now.2 && now.1 < now.3 { Some(now) } else { None };
                let area = match (drag.prev_rect, now) {
                    (Some(a), Some(b)) => Some((a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))),
                    (a, None) => a,
                    (None, b) => b,
                };
                if let (Some((base, mask)), Some(area)) = (&drag.float, area) {
                    let d = &mut self.docs[self.cur];
                    let a = d.s.active;
                    let layer = &mut d.s.layers[a].px;
                    for y in area.1..area.3 {
                        let r = y * w + area.0..y * w + area.2;
                        layer[r.clone()].copy_from_slice(&base[r]);
                    }
                    if let Some(nb) = now {
                        for y in nb.1..nb.3 {
                            for x in nb.0..nb.2 {
                                let (sx, sy) = (x as i32 - dx, y as i32 - dy);
                                let si = sy as usize * w + sx as usize;
                                if mask[si] == 0 { continue; }
                                let i = y * w + x;
                                layer[i] = over(layer[i], drag.snap[si], mask[si]);
                            }
                        }
                    }
                    d.mark_rect(area);
                }
                drag.prev_rect = now;
            }
            _ => {}
        }
        drag.last = pos;
        self.drag = Some(drag);
    }

    fn end_drag(&mut self) {
        let Some(drag) = self.drag.take() else { return };
        let name = match self.tool {
            Tool::RectSelect | Tool::EllipseSelect if !drag.moved => {
                // a plain click deselects
                let d = self.doc();
                d.s.selection = None;
                if drag.sel_snap.is_some() { d.commit("Deselect"); } else { d.sel_version += 1; }
                return;
            }
            Tool::Pan => return,
            Tool::MovePixels | Tool::MoveSelection if !drag.moved => return,
            Tool::LassoSelect if drag.lasso.len() < 3 => return,
            t => t.name(),
        };
        let (w, h) = (self.docs[self.cur].w(), self.docs[self.cur].h());
        match self.tool {
            Tool::RectSelect | Tool::EllipseSelect => {
                let Some(r) = drag.sel_box else { return };
                let kind = if self.tool == Tool::RectSelect { ShapeKind::Rectangle } else { ShapeKind::Ellipse };
                let mut cov = Coverage::with_window(w, h, r.0 as i32 - 1, r.1 as i32 - 1, r.2 as i32 + 2, r.3 as i32 + 2);
                cov.fill_fn(r.0 as i32, r.1 as i32, r.2 as i32, r.3 as i32, kind == ShapeKind::Ellipse && self.opts.aa, |x, y| raster::shape_inside(kind, x, y, r, 0.0));
                let ns = self.combine_sel(&drag.sel_snap, cov.to_mask(), self.opts.sel_mode);
                self.doc().s.selection = ns;
            }
            Tool::LassoSelect => {
                let m = raster::polygon_mask(&drag.lasso, w, h);
                let ns = self.combine_sel(&drag.sel_snap, m, self.opts.sel_mode);
                self.doc().s.selection = ns;
            }
            Tool::MoveSelection | Tool::MovePixels => {
                if let Some(old) = &drag.sel_snap {
                    let (dx, dy) = drag.offset;
                    let mut nm = vec![0u8; w * h];
                    let (wi, hi) = (w as i32, h as i32);
                    for y in 0..hi {
                        let sy = y - dy;
                        if sy < 0 || sy >= hi { continue; }
                        for x in 0..wi {
                            let sx = x - dx;
                            if sx >= 0 && sx < wi { nm[(y * wi + x) as usize] = old[(sy * wi + sx) as usize]; }
                        }
                    }
                    self.doc().s.selection = if nm.iter().any(|v| *v > 0) { Some(nm) } else { None };
                }
            }
            _ => {}
        }
        self.doc().commit(name);
    }

    fn finish_text(&mut self) {
        if let Some(mut t) = self.text.take() {
            if !t.text.trim().is_empty() {
                self.render_text_preview(&mut t);
                self.doc().commit("Text");
            } else {
                let d = self.doc();
                d.layer_mut().px = t.snap;
                d.mark_full();
            }
        }
    }

    fn render_text_preview(&mut self, t: &mut TextEdit) {
        let (w, h) = (self.docs[self.cur].w(), self.docs[self.cur].h());
        let font = self.text_font();
        let cov = raster::text_coverage(w, h, &t.text, points_to_pixels(self.opts.font_size), t.pos.0, t.pos.1, font.as_ref().map(|(d, i)| (d.as_slice(), *i)));
        let mut prev = t.prev_rect;
        self.put_cov(&t.snap, &[(&cov, t.color)], &mut prev);
        t.prev_rect = prev;
    }

    // ---------------- view ----------------

    fn zoom_step(&mut self, dir: i32, around: Option<(f32, f32)>) {
        let d = self.doc();
        let z = d.zoom;
        let nz = if dir > 0 {
            ZOOM_PRESETS.iter().copied().find(|p| *p > z * 1.001).unwrap_or(z)
        } else {
            ZOOM_PRESETS.iter().rev().copied().find(|p| *p < z * 0.999).unwrap_or(z)
        };
        if let Some(p) = around {
            // keep the clicked pixel under the cursor
            let (cx, cy) = (d.w() as f32 / 2.0, d.h() as f32 / 2.0);
            d.pan.0 += (p.0 - cx) * (z - nz);
            d.pan.1 += (p.1 - cy) * (z - nz);
        }
        d.zoom = nz;
    }

    /// Set the zoom, keeping the picture point `around` (image coords) under the pointer.
    fn zoom_to(&mut self, nz: f32, around: Option<(f32, f32)>) {
        let nz = nz.clamp(ZOOM_PRESETS[0], ZOOM_PRESETS[ZOOM_PRESETS.len() - 1]);
        let d = self.doc();
        let z = d.zoom;
        if let Some(p) = around {
            let (cx, cy) = (d.w() as f32 / 2.0, d.h() as f32 / 2.0);
            d.pan.0 += (p.0 - cx) * (z - nz);
            d.pan.1 += (p.1 - cy) * (z - nz);
        }
        d.zoom = nz;
    }

    /// Ctrl+wheel zoom from the original's CanvasControl.ProcessVerticalPointerWheel (0x06002299):
    /// factor 1.12 per notch; snap to the nearest whole-number zoom when within 5 % of it and closer to it
    /// than the zoom before.
    fn wheel_zoom(&mut self, notches: f32, around: Option<(f32, f32)>) {
        let old = self.docs[self.cur].zoom as f64;
        let factor = WHEEL_ZOOM_FACTOR.powf(notches.abs() as f64);
        let mut new = if notches > 0.0 { old * factor } else { old / factor };
        new = new.clamp(ZOOM_PRESETS[0] as f64, ZOOM_PRESETS[ZOOM_PRESETS.len() - 1] as f64);
        let r = new.round(); // rounds halves away from zero, like the original
        if (r - new).abs() < r * WHEEL_ZOOM_SNAP && (r - new).abs() < (r - old).abs() {
            new = r;
        }
        self.zoom_to(new as f32, around);
    }

    fn zoom_fit(&mut self) { self.doc().fit_pending = true; }
    fn actual_size(&mut self) { let d = self.doc(); d.zoom = 1.0; d.pan = (0.0, 0.0); }

    // ---------------- edit actions ----------------

    /// Edit > Copy Selection: remember just the selection's outline (not its pixels).
    fn copy_selection(&mut self) {
        let d = &self.docs[self.cur];
        match &d.s.selection {
            Some(m) => self.sel_clipboard = Some((d.w(), d.h(), m.clone())),
            None => self.dialog = Some(Dialog::Message("There's no selection to copy.".into())),
        }
    }

    /// Edit > Paste Selection: select the copied outline again, here or in another picture.
    fn paste_selection(&mut self) {
        let Some((sw, sh, m)) = self.sel_clipboard.clone() else {
            self.dialog = Some(Dialog::Message("Copy a selection first (Edit > Copy Selection).".into()));
            return;
        };
        let d = self.doc();
        let (w, h) = (d.w(), d.h());
        let mut nm = vec![0u8; w * h];
        for y in 0..sh.min(h) { for x in 0..sw.min(w) { nm[y * w + x] = m[y * sw + x]; } }
        d.s.selection = if nm.iter().any(|v| *v > 0) { Some(nm) } else { None };
        d.commit("Paste Selection");
    }

    fn select_all(&mut self) { self.finish_floating(); let d = self.doc(); let n = d.w() * d.h(); d.s.selection = Some(vec![255; n]); d.commit("Select All"); }
    fn deselect(&mut self) {
        // floating pixels are only put down when they're deselected
        if self.floating.is_some() { self.finish_floating(); }
        let d = self.doc();
        if d.s.selection.is_some() { d.s.selection = None; d.commit("Deselect"); }
    }
    fn invert_selection(&mut self) {
        self.finish_floating();
        let d = self.doc();
        let Some(m) = &d.s.selection else { return };
        let v: Vec<u8> = m.iter().map(|x| 255 - x).collect();
        d.s.selection = if v.iter().all(|x| *x == 0) { None } else { Some(v) };
        d.commit("Invert Selection");
    }
    fn erase_selection(&mut self) {
        self.finish_floating();
        if self.smart_guard() { return; }
        let d = self.doc();
        let Some(m) = d.s.selection.clone() else { return };
        for (p, mv) in d.layer_mut().px.iter_mut().zip(m.iter()) { p[3] = (p[3] as u32 * (255 - *mv as u32) / 255) as u8; }
        d.commit("Erase Selection");
    }
    fn fill_selection(&mut self) {
        self.finish_floating();
        if self.smart_guard() { return; }
        let col = self.primary;
        let d = self.doc();
        let n = d.w() * d.h();
        let m = d.s.selection.clone().unwrap_or_else(|| vec![255; n]);
        for (p, mv) in d.layer_mut().px.iter_mut().zip(m.iter()) { if *mv > 0 { *p = over(*p, col, *mv); } }
        d.commit("Fill Selection");
    }
    fn copy(&mut self, merged: bool) {
        self.finish_floating();
        let d = &self.docs[self.cur];
        let (w, h) = (d.w(), d.h());
        let (x0, y0, x1, y1) = Self::sel_bounds(d).unwrap_or((0, 0, w, h));
        let src = if merged { d.composite() } else { d.layer().px.clone() };
        let mut out = Vec::with_capacity((x1 - x0) * (y1 - y0));
        for y in y0..y1 { for x in x0..x1 {
            let i = y * w + x;
            let mut p = src[i];
            p[3] = (p[3] as u32 * d.sel_at(i) as u32 / 255) as u8;
            out.push(p);
        } }
        // also on the system clipboard, so it pastes into other pictures, other copies of the app and other programs
        self.sysclip.put(x1 - x0, y1 - y0, &out);
        self.clipboard = Some((x1 - x0, y1 - y0, out));
    }

    /// The picture to paste: the system clipboard first (other programs, other windows), then our own copy.
    /// A copied web link starts a download and pastes when it arrives.
    fn clipboard_picture(&mut self, purpose: PasteTarget) -> Option<clip::Picture> {
        match self.sysclip.offer() {
            clip::Offer::Picture(p) => Some(p),
            clip::Offer::Link(url) => {
                self.download = Some((clip::download(url.clone()), purpose));
                self.status = format!("Downloading the copied picture from {url}…");
                None
            }
            clip::Offer::Nothing => match self.clipboard.clone() {
                Some((w, h, px)) => Some(clip::Picture { w, h, px }),
                None => {
                    self.dialog = Some(Dialog::Message("There's nothing to paste yet. Copy a picture first.".into()));
                    None
                }
            },
        }
    }

    fn paste(&mut self, new_layer: bool) {
        let target = if new_layer { PasteTarget::NewLayer } else { PasteTarget::Layer };
        if let Some(pic) = self.clipboard_picture(target) {
            self.paste_picture(pic, new_layer);
        }
    }

    /// Paste, first asking what to do when the picture is bigger than the canvas.
    fn paste_picture(&mut self, pic: clip::Picture, new_layer: bool) {
        let d = &self.docs[self.cur];
        if pic.w > d.w() || pic.h > d.h() {
            self.dialog = Some(Dialog::PasteBig { pic, new_layer });
        } else {
            self.paste_into(pic, new_layer, false);
        }
    }

    fn paste_into(&mut self, pic: clip::Picture, new_layer: bool, expand: bool) {
        if expand {
            let bg = self.secondary;
            let d = self.doc();
            let (w0, h0) = (d.w(), d.h());
            let (nw, nh) = (w0.max(pic.w), h0.max(pic.h));
            for (i, l) in d.s.layers.iter_mut().enumerate() {
                if l.smart.is_some() { l.rerender(nw, nh); continue; }
                let fill = if i == 0 && l.name == "Background" { bg } else { [0, 0, 0, 0] };
                l.px = raster::recanvas(&l.px, w0, h0, nw, nh, 0, 0, fill);
            }
            d.s.w = nw;
            d.s.h = nh;
            d.s.selection = None;
            d.commit("Expand Canvas");
            self.zoom_fit();
        }
        // The pasted picture floats with the transform handles on, keeping every pixel even where the canvas
        // hides it; it becomes ordinary pixels when deselected (parts past the canvas edge stay a smart object).
        let (ox, oy) = paste_origin(pic.w, pic.h, self.visible_img);
        self.float_picture(pic, if new_layer { "Paste into New Layer" } else { "Paste" }, (ox, oy), new_layer);
    }

    fn paste_new_image(&mut self) {
        if let Some(pic) = self.clipboard_picture(PasteTarget::NewImage) {
            self.new_image_from(pic);
        }
    }

    fn new_image_from(&mut self, pic: clip::Picture) {
        let mut d = Document::new("Untitled", pic.w, pic.h, [0, 0, 0, 0]);
        d.s.layers[0].px = pic.px;
        d.history[0] = crate::doc::HistoryItem::first("New Image", &d.s);
        self.docs.push(d);
        self.cur = self.docs.len() - 1;
    }

    /// File > New: the copied picture's size when there is one, otherwise the original's default size.
    fn open_new_dialog(&mut self, ctx: &egui::Context) {
        let (w, h) = match self.sysclip.offer() {
            clip::Offer::Picture(p) => (p.w as u32, p.h as u32),
            _ => match &self.clipboard {
                Some((w, h, _)) => (*w as u32, *h as u32),
                None => new_image_size(ctx),
            },
        };
        self.dialog = Some(Dialog::NewImage { w, h });
    }

    /// Open dropped items in their own tabs, or add them as layers of the current picture.
    fn handle_drop(&mut self, items: Vec<PathBuf>, as_layers: bool) {
        for p in items {
            let text = p.to_string_lossy().into_owned();
            if text.starts_with("http://") || text.starts_with("https://") {
                self.download = Some((clip::download(text.clone()), if as_layers { PasteTarget::NewLayer } else { PasteTarget::NewImage }));
                self.status = format!("Downloading the dropped picture from {text}…");
            } else if text.starts_with("data:") {
                match clip::decode_data_url(&text) {
                    Ok(pic) if as_layers => self.paste_picture(pic, true),
                    Ok(pic) => self.new_image_from(pic),
                    Err(e) => self.dialog = Some(Dialog::Message(e)),
                }
            } else if as_layers {
                if let Err(e) = self.import_layer(&p) { self.dialog = Some(Dialog::Message(format!("Couldn't open {}: {e}", io::file_name(&p)))); }
            } else {
                self.open_path(p);
            }
        }
    }

    /// Finish a background download of a copied picture link.
    fn poll_download(&mut self) {
        let Some((rx, target)) = &self.download else { return };
        let target = *target;
        match rx.try_recv() {
            Ok(Ok(pic)) => {
                self.download = None;
                self.status.clear();
                match target {
                    PasteTarget::Layer => self.paste_picture(pic, false),
                    PasteTarget::NewLayer => self.paste_picture(pic, true),
                    PasteTarget::NewImage => self.new_image_from(pic),
                }
            }
            Ok(Err(e)) => {
                self.download = None;
                self.status.clear();
                self.dialog = Some(Dialog::Message(e));
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => { self.download = None; self.status.clear(); }
        }
    }
    fn cut(&mut self) {
        self.copy(false);
        if self.docs[self.cur].layer().smart.is_some() {
            // cutting a whole smart object removes the layer; cutting part of it needs ordinary pixels
            if self.docs[self.cur].s.selection.is_none() { self.delete_layer(); } else { self.smart_guard(); }
            return;
        }
        let d = self.doc();
        if d.s.selection.is_none() {
            for p in d.layer_mut().px.iter_mut() { p[3] = 0; }
            d.commit("Cut");
        } else { self.erase_selection(); }
    }

    // ---------------- image & layer actions ----------------

    fn crop_to_selection(&mut self) {
        self.finish_floating();
        let Some((x0, y0, x1, y1)) = Self::sel_bounds(&self.docs[self.cur]) else { return };
        let d = self.doc();
        let w = d.w();
        let (nw, nh) = (x1 - x0, y1 - y0);
        let m = d.s.selection.clone().unwrap();
        for l in d.s.layers.iter_mut() {
            // smart objects keep everything: the canvas just moves over them
            if let Some(s) = l.smart.as_mut() {
                s.xf.cx -= x0 as f32;
                s.xf.cy -= y0 as f32;
                l.rerender(nw, nh);
                continue;
            }
            let mut o = Vec::with_capacity(nw * nh);
            for y in y0..y1 { for x in x0..x1 { let mut p = l.px[y * w + x]; p[3] = (p[3] as u32 * m[y * w + x] as u32 / 255) as u8; o.push(p); } }
            l.px = o;
        }
        d.s.w = nw; d.s.h = nh; d.s.selection = None;
        d.commit("Crop to Selection");
        self.zoom_fit();
    }
    fn image_transform(&mut self, what: &str) {
        self.finish_floating();
        let d = self.doc();
        let (w, h) = (d.w(), d.h());
        for l in d.s.layers.iter_mut() {
            if let Some(s) = l.smart.as_mut() {
                let (nw, nh) = if what.starts_with("Rotate 90") { (h, w) } else { (w, h) };
                match what {
                    "Flip Horizontal" => crate::smart::flip(&mut s.xf, w, h, true),
                    "Flip Vertical" => crate::smart::flip(&mut s.xf, w, h, false),
                    "Rotate 180°" => { crate::smart::flip(&mut s.xf, w, h, true); crate::smart::flip(&mut s.xf, w, h, false); }
                    "Rotate 90° Clockwise" => crate::smart::rot90(&mut s.xf, w, h),
                    _ => { crate::smart::rot90(&mut s.xf, w, h); crate::smart::rot90(&mut s.xf, h, w); crate::smart::rot90(&mut s.xf, w, h); }
                }
                l.rerender(nw, nh);
                continue;
            }
            match what {
                "Flip Horizontal" => raster::flip_h(&mut l.px, w, h),
                "Flip Vertical" => raster::flip_v(&mut l.px, w, h),
                "Rotate 180°" => raster::rot180(&mut l.px),
                "Rotate 90° Clockwise" => l.px = raster::rot90(&l.px, w, h),
                _ => { let a = raster::rot90(&l.px, w, h); let b = raster::rot90(&a, h, w); l.px = raster::rot90(&b, w, h); }
            }
        }
        if what.starts_with("Rotate 90") { d.s.w = h; d.s.h = w; }
        d.s.selection = None;
        d.commit(what);
    }
    fn layer_transform(&mut self, what: &str) {
        self.finish_floating();
        let d = self.doc();
        let (w, h) = (d.w(), d.h());
        let l = d.layer_mut();
        if let Some(s) = l.smart.as_mut() {
            match what {
                "Flip Layer Horizontal" => crate::smart::flip(&mut s.xf, w, h, true),
                "Flip Layer Vertical" => crate::smart::flip(&mut s.xf, w, h, false),
                _ => { crate::smart::flip(&mut s.xf, w, h, true); crate::smart::flip(&mut s.xf, w, h, false); }
            }
            l.rerender(w, h);
            d.commit(what);
            return;
        }
        match what { "Flip Layer Horizontal" => raster::flip_h(&mut l.px, w, h), "Flip Layer Vertical" => raster::flip_v(&mut l.px, w, h), _ => raster::rot180(&mut l.px) }
        d.commit(what);
    }
    fn flatten(&mut self) {
        self.finish_floating();
        let d = self.doc();
        if d.s.layers.len() < 2 { return; }
        let flat = d.composite();
        let mut l = Layer::new("Background", d.w(), d.h(), [0, 0, 0, 0]);
        l.px = flat;
        d.s.layers = vec![l];
        d.s.active = 0;
        d.commit("Flatten");
    }
    fn add_layer(&mut self, commit: bool) {
        self.finish_floating();
        let d = self.doc();
        let n = d.s.layers.len() + 1;
        let l = Layer::new(&format!("Layer {n}"), d.w(), d.h(), [0, 0, 0, 0]);
        let at = d.s.active + 1;
        d.s.layers.insert(at, l);
        d.s.active = at;
        if commit { d.commit("Add New Layer"); }
    }
    fn delete_layer(&mut self) {
        self.finish_floating();
        let d = self.doc();
        if d.s.layers.len() < 2 { return; }
        d.s.layers.remove(d.s.active);
        d.s.active = d.s.active.min(d.s.layers.len() - 1);
        d.commit("Delete Layer");
    }
    fn duplicate_layer(&mut self) {
        self.finish_floating();
        let d = self.doc();
        let mut l = d.layer().clone();
        l.name = format!("{} copy", l.name);
        let at = d.s.active + 1;
        d.s.layers.insert(at, l);
        d.s.active = at;
        d.commit("Duplicate Layer");
    }
    fn merge_down(&mut self) {
        self.finish_floating();
        let d = self.doc();
        let a = d.s.active;
        if a == 0 { return; }
        // merging into a smart object makes it ordinary pixels
        d.s.layers[a - 1].smart = None;
        let top = d.s.layers[a].clone();
        if top.visible {
            for (b, t) in d.s.layers[a - 1].px.iter_mut().zip(top.px.iter()) { *b = top.blend.blend(*b, *t, top.opacity); }
        }
        d.s.layers.remove(a);
        d.s.active = a - 1;
        d.commit("Merge Layer Down");
    }
    fn move_layer(&mut self, up: bool) {
        self.finish_floating();
        let d = self.doc();
        let a = d.s.active;
        if up && a + 1 < d.s.layers.len() { d.s.layers.swap(a, a + 1); d.s.active = a + 1; d.commit("Move Layer Up"); }
        else if !up && a > 0 { d.s.layers.swap(a, a - 1); d.s.active = a - 1; d.commit("Move Layer Down"); }
    }
    fn import_layer(&mut self, path: &std::path::Path) -> Result<(), String> {
        // an imported picture floats on a new layer like a paste: nothing is cut off, even if it's bigger
        let (iw, ih, ipx) = io::load_pixels(path)?;
        self.float_picture(clip::Picture { w: iw, h: ih, px: ipx }, "Import From File", (0, 0), true);
        let name = io::file_name(path);
        let d = self.doc();
        let a = d.s.active;
        d.s.layers[a].name = name;
        Ok(())
    }

    fn start_effect(&mut self, fx: Fx) {
        if self.dialog.is_some() { return; }
        self.finish_floating();
        if self.smart_guard() { return; }
        let vals: Vec<f32> = fx.params().iter().map(|p| p.default).collect();
        let snap = self.docs[self.cur].layer().px.clone();
        self.run_effect(fx, &vals, &snap);
        if vals.is_empty() {
            self.doc().commit(fx.name());
        } else {
            self.dialog = Some(Dialog::Effect { fx, applied: vals.clone(), vals, snap });
        }
    }

    fn run_effect(&mut self, fx: Fx, vals: &[f32], snap: &[Px]) {
        let (prim, sec) = (self.primary, self.secondary);
        let d = self.doc();
        let (w, h) = (d.w(), d.h());
        let out = effects::apply(fx, snap, w, h, vals, prim, sec);
        let sel = d.s.selection.clone();
        let l = d.layer_mut();
        for i in 0..w * h {
            let m = sel.as_ref().map(|s| s[i]).unwrap_or(255);
            l.px[i] = if m == 255 { out[i] } else if m == 0 { snap[i] } else { raster::lerp_px(snap[i], out[i], m as f32 / 255.0) };
        }
        d.mark_full();
    }

    // ---------------- files ----------------

    fn open_file_dialog(&mut self, mode: FileMode) {
        self.finish_floating();
        let d = &self.docs[self.cur];
        let known = d.path.as_ref().or(d.source.as_ref());
        let dir = known.and_then(|p| p.parent().map(|p| p.to_path_buf())).unwrap_or_else(|| self.last_dir.clone());
        let stem = std::path::Path::new(&d.name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Untitled".into());
        let (name, ext) = match mode {
            FileMode::Save => (stem, 0),
            FileMode::Export => {
                // default to the format it came in, as a new file next to it
                let ext = d.source.as_ref().and_then(|p| p.extension()).map(|e| e.to_string_lossy().to_lowercase())
                    .map(|e| if e == "jpeg" { "jpg".to_string() } else if e == "tif" { "tiff".to_string() } else { e })
                    .and_then(|e| io::EXPORT_EXTS.iter().position(|x| *x == e)).unwrap_or(0);
                (Self::free_copy_name(&dir, &stem, io::EXPORT_EXTS[ext]), ext)
            }
            _ => (String::new(), 0),
        };
        let mut dlg = Dialog::File { mode, dir, name, ext, err: String::new(), entries: vec![] };
        Self::refresh_entries(&mut dlg);
        self.dialog = Some(dlg);
    }

    /// "name copy", "name copy 2"... that doesn't exist yet in `dir`.
    fn free_copy_name(dir: &std::path::Path, stem: &str, ext: &str) -> String {
        let mut n = 1;
        loop {
            let name = if n == 1 { format!("{stem} copy") } else { format!("{stem} copy {n}") };
            if !dir.join(format!("{name}.{ext}")).exists() { return name; }
            n += 1;
        }
    }

    fn refresh_entries(dlg: &mut Dialog) {
        if let Dialog::File { dir, entries, mode, .. } = dlg {
            entries.clear();
            if let Ok(rd) = std::fs::read_dir(&dir) {
                let exts: &[&str] = match mode { FileMode::Save => &["pxe"], FileMode::Export => io::EXPORT_EXTS, _ => io::OPEN_EXTS };
                let mut v: Vec<(String, bool)> = rd.flatten().filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') { return None; }
                    let is_dir = e.path().is_dir();
                    let ok = is_dir || std::path::Path::new(&name).extension().map(|x| exts.contains(&x.to_string_lossy().to_lowercase().as_str())).unwrap_or(false);
                    ok.then_some((name, is_dir))
                }).collect();
                v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.to_lowercase().cmp(&b.0.to_lowercase())));
                *entries = v;
            }
        }
    }

    /// Save the layered project (.pxe). This is the only kind of file Save writes.
    fn save_project(&mut self, path: PathBuf) -> Result<(), String> {
        let q = self.jpeg_quality;
        io::save(&self.docs[self.cur], &path, q)?;
        let d = self.doc();
        d.name = io::file_name(&path);
        d.path = Some(path.clone());
        d.dirty = false;
        if let Some(p) = path.parent() { self.last_dir = p.to_path_buf(); }
        Ok(())
    }

    /// Export a flat copy. The picture keeps its project file and stays marked as unsaved if it was.
    fn export_copy(&mut self, path: PathBuf) -> Result<(), String> {
        let q = self.jpeg_quality;
        io::save(&self.docs[self.cur], &path, q)?;
        if let Some(p) = path.parent() { self.last_dir = p.to_path_buf(); }
        self.status.clear();
        Ok(())
    }

    /// Save: back into its own project file, or ask where to keep the project. Never overwrites the
    /// PNG/JPG a picture was opened from (use Export for that, which asks first).
    fn save(&mut self) {
        match self.docs[self.cur].path.clone() {
            Some(p) => { if let Err(e) = self.save_project(p) { self.dialog = Some(Dialog::Message(format!("Couldn't save: {e}"))); } }
            None => self.open_file_dialog(FileMode::Save),
        }
    }

    fn open_path(&mut self, p: PathBuf) {
        match io::open(&p) {
            Ok(d) => {
                if let Some(par) = p.parent() { self.last_dir = par.to_path_buf(); }
                // replace an untouched blank image
                if self.docs.len() == 1 && self.docs[0].path.is_none() && !self.docs[0].dirty && self.docs[0].hist_pos == 0 { self.docs.clear(); }
                self.docs.push(d);
                self.cur = self.docs.len() - 1;
                self.drag = None;
                self.text = None;
            }
            Err(e) => self.dialog = Some(Dialog::Message(format!("Couldn't open {}: {e}", io::file_name(&p)))),
        }
    }

    fn close_doc(&mut self, i: usize, force: bool) {
        if !force && self.docs[i].dirty { self.dialog = Some(Dialog::Confirm { doc: i, then: AfterConfirm::CloseDoc }); return; }
        self.docs.remove(i);
        if self.docs.is_empty() { self.docs.push(Document::new("Untitled", DEFAULT_W, DEFAULT_H, [255, 255, 255, 255])); }
        self.cur = self.cur.min(self.docs.len() - 1);
        self.drag = None;
        self.text = None;
    }

    // ---------------- keyboard ----------------

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some() { return; }
        let typing = ctx.egui_wants_keyboard_input();
        let events = ctx.input(|i| i.events.clone());
        for ev in &events {
            match ev {
                egui::Event::Copy if !typing => self.copy(false),
                egui::Event::Cut if !typing => self.cut(),
                egui::Event::Paste(_) if !typing => self.paste(false),
                // Ctrl+V with only a picture on the clipboard arrives as an image, not as a text paste
                egui::Event::PasteImage(img) if !typing => {
                    let px: Vec<Px> = img.pixels.iter().map(|c| c.to_srgba_unmultiplied()).collect();
                    let pic = clip::Picture { w: img.size[0], h: img.size[1], px };
                    if pic.w > 0 && pic.h > 0 { self.paste_picture(pic, false); } else { self.paste(false); }
                }
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    let (c, s) = (modifiers.command, modifiers.shift);
                    if typing { continue; }
                    match (*key, c, s) {
                        // Edit menu keys from the original's EditMenu..ctor (paintdotnet.dll 0x06000abc)
                        (Key::C, true, true) if modifiers.alt => self.copy_selection(),
                        (Key::V, true, true) if modifiers.alt => self.paste_selection(),
                        (Key::C, true, false) => self.copy(false),
                        (Key::C, true, true) => self.copy(true),
                        (Key::X, true, false) => self.cut(),
                        (Key::V, true, false) if modifiers.alt => self.paste_new_image(),
                        (Key::V, true, false) => self.paste(false),
                        (Key::V, true, true) => self.paste(true),
                        // undo while pixels float puts them back first
                        // undo/redo step through a floating picture's transforms first
                        (Key::Z, true, false) => self.undo(),
                        (Key::Y, true, _) | (Key::Z, true, true) => self.redo(),
                        (Key::Enter, false, _) => self.finish_floating(),
                        (Key::A, true, false) => self.select_all(),
                        (Key::D, true, false) => self.deselect(),
                        (Key::I, true, false) => self.invert_selection(),
                        (Key::I, true, true) => self.start_effect(Fx::InvertColors),
                        (Key::G, true, true) => self.start_effect(Fx::BlackAndWhite),
                        (Key::E, true, true) => self.start_effect(Fx::Sepia),
                        (Key::N, true, false) => self.open_new_dialog(ctx),
                        (Key::N, true, true) => self.add_layer(true),
                        (Key::O, true, false) => self.open_file_dialog(FileMode::Open),
                        (Key::S, true, false) => self.save(),
                        (Key::S, true, true) => self.open_file_dialog(FileMode::Save),
                        (Key::E, true, false) => self.open_file_dialog(FileMode::Export),
                        (Key::W, true, false) => { let c = self.cur; self.close_doc(c, false) }
                        (Key::D, true, true) => self.duplicate_layer(),
                        (Key::M, true, false) => self.merge_down(),
                        (Key::F, true, true) => self.flatten(),
                        (Key::X, true, true) => self.crop_to_selection(),
                        (Key::R, true, false) => { let d = &self.docs[self.cur]; self.dialog = Some(Dialog::Resize { w: d.w() as u32, h: d.h() as u32, keep: true }) }
                        (Key::R, true, true) => { let d = &self.docs[self.cur]; self.dialog = Some(Dialog::CanvasSize { w: d.w() as u32, h: d.h() as u32, anchor: (1, 1) }) }
                        (Key::L, true, true) => { let d = !self.dark; self.set_dark(ctx, d); }
                        (Key::H, true, false) => self.image_transform("Rotate 90° Clockwise"),
                        (Key::B, true, false) => self.zoom_fit(),
                        (Key::A, true, true) | (Key::Num0, true, _) => self.actual_size(),
                        (Key::Plus | Key::Equals, true, _) => self.zoom_step(1, None),
                        (Key::Minus, true, _) => self.zoom_step(-1, None),
                        (Key::Delete, false, _) => self.erase_selection(),
                        (Key::Backspace, false, _) => self.fill_selection(),
                        (Key::X, false, false) => std::mem::swap(&mut self.primary, &mut self.secondary),
                        (Key::Escape, false, _) => {
                            if self.text.is_some() { self.finish_text(); }
                            else if !self.cancel_floating() { self.deselect(); }
                        }
                        (Key::OpenBracket, false, _) => self.opts.brush = (self.opts.brush - 1.0).max(1.0),
                        (Key::CloseBracket, false, _) => self.opts.brush = (self.opts.brush + 1.0).min(2000.0),
                        (k, false, false) => {
                            let letter = k.name();
                            let group: Vec<Tool> = Tool::ALL.iter().copied().filter(|t| t.key() == letter).collect();
                            if !group.is_empty() {
                                let next = match group.iter().position(|t| *t == self.tool) { Some(i) => group[(i + 1) % group.len()], None => group[0] };
                                self.set_tool(next);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    fn set_tool(&mut self, t: Tool) {
        if self.tool == Tool::Text && t != Tool::Text { self.finish_text(); }
        if t != Tool::MovePixels { self.finish_floating(); }
        self.tool = t;
    }
}

#[cfg(test)]
pub fn free_copy_name_for_tests(dir: &std::path::Path, stem: &str, ext: &str) -> String { App::free_copy_name(dir, stem, ext) }

// =====================================================================
// UI
// =====================================================================

impl eframe::App for App {
    /// Ctrl+C/X/V reach the app as keys (clipboard_shortcuts is off). While a text box has focus, turn them
    /// back into text copy/cut/paste so text editing works as usual.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if !ctx.egui_wants_keyboard_input() {
            return;
        }
        let mut out = Vec::with_capacity(raw.events.len());
        for ev in raw.events.drain(..) {
            match &ev {
                egui::Event::Key { key: Key::C, pressed: true, modifiers, .. } if modifiers.command => out.push(egui::Event::Copy),
                egui::Event::Key { key: Key::X, pressed: true, modifiers, .. } if modifiers.command => out.push(egui::Event::Cut),
                egui::Event::Key { key: Key::V, pressed: true, modifiers, .. } if modifiers.command => {
                    if let Some(t) = self.sysclip.text() { out.push(egui::Event::Paste(t)); }
                }
                _ => out.push(ev),
            }
        }
        raw.events = out;
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame += 1;

        // Diagnostics for the system clipboard (PAINT_RS_SELFTEST=copy | paste:<report file>):
        // "copy" puts a 37 x 23 test picture on the clipboard and stays open 15 s; "paste" reads the
        // clipboard after a moment, writes what it found to the report file and quits.
        if let Ok(mode) = std::env::var("PAINT_RS_SELFTEST") {
            ctx.request_repaint();
            if mode == "copy" {
                if self.frame == 20 {
                    let px: Vec<Px> = (0..37 * 23).map(|i| [200, (i % 37 * 6) as u8, 40, 255]).collect();
                    self.sysclip.put(37, 23, &px);
                }
                if self.frame > 900 { ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
            } else if let Some(out) = mode.strip_prefix("fonts:") {
                // font picker check: system fonts found, default chosen, text drawn with it
                if let Some(cat) = crate::fonts::catalog() {
                    if self.frame < 1000 {
                        self.frame = 1000;
                        let fam = cat.default_family();
                        let faces = fam.as_deref().and_then(|f| cat.faces(f)).map(|f| f.len()).unwrap_or(0);
                        let font = self.text_font();
                        let (w, h) = (400, 100);
                        let cov = raster::text_coverage(w, h, "Paint.RS", 40.0, 5.0, 5.0, font.as_ref().map(|(d, i)| (d.as_slice(), *i)));
                        let inked = cov.c.iter().filter(|v| **v > 0).count();
                        let r = format!("families={} default={fam:?} faces={faces} font_loaded={} inked_pixels={inked}\n", cat.families.len(), font.is_some());
                        let _ = std::fs::write(out, r);
                    }
                }
                if self.frame > 1005 || self.frame > 900 && crate::fonts::catalog().is_none() { self.allow_quit = true; ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
            } else if let Some(out) = mode.strip_prefix("transform:") {
                // free transform check: paste, move, deselect; inside → pixels, partly outside → smart object kept
                if self.frame == 10 {
                    let mut r = String::new();
                    let red = || clip::Picture { w: 100, h: 50, px: vec![[220, 30, 30, 255]; 100 * 50] };
                    // 1. paste fully inside, move it, put it down
                    self.float_picture(red(), "Paste", (0, 0), false);
                    r += &format!("handles after paste: {}\n", self.xform_target().is_some());
                    let mut xf = self.xform_target().unwrap().0;
                    xf.cx += 200.0; xf.cy += 100.0;
                    self.apply_xform(xf);
                    self.deselect();
                    let d = &self.docs[self.cur];
                    r += &format!("inside: layers={} pixel(250,125)={:?} smart={}\n", d.s.layers.len(), d.s.layers[0].px[125 * d.w() + 250], d.s.layers.iter().any(|l| l.smart.is_some()));
                    // 2. paste, push half of it past the left edge, put it down
                    self.float_picture(red(), "Paste", (0, 0), false);
                    let mut xf = self.xform_target().unwrap().0;
                    xf.cx -= 50.0; xf.angle = 0.3;
                    self.apply_xform(xf);
                    self.deselect();
                    let d = &self.docs[self.cur];
                    let s = d.s.layers.iter().find_map(|l| l.smart.as_ref());
                    r += &format!("outside: layers={} smart_kept={} source={:?}\n", d.s.layers.len(), s.is_some(), s.map(|s| (s.sw, s.sh)));
                    // 3. moving the smart object back shows the hidden half again
                    if s.is_some() {
                        let mut xf = self.xform_target().unwrap().0;
                        xf.cx += 300.0; xf.angle = 0.0;
                        self.apply_xform(xf);
                        let d = &self.docs[self.cur];
                        let l = d.layer();
                        let shown = l.px.iter().filter(|p| p[3] > 0).count();
                        r += &format!("moved back: visible pixels={shown} (expect 5000)\n");
                    }
                    // 4. undo goes back step by step
                    let before = self.docs[self.cur].hist_pos;
                    self.doc().undo();
                    r += &format!("history steps={} undo ok={}\n", before, self.docs[self.cur].hist_pos + 1 == before);
                    // 5. while a paste floats, undo/redo step through its transforms; the paste stays
                    self.float_picture(red(), "Paste", (0, 0), false);
                    let start = self.xform_target().unwrap();
                    for dx in [100.0f32, 200.0] {
                        let (xf0, sw, sh) = self.xform_target().unwrap();
                        self.xdrag = Some(freexf::XDrag { grip: crate::smart::Grip::Move, start: (0.0, 0.0), xf0, sw, sh });
                        let mut xf = xf0;
                        xf.cx = start.0.cx + dx;
                        self.apply_xform(xf);
                        self.xform_release();
                    }
                    let cx = |s: &Self| s.xform_target().map(|t| t.0.cx - start.0.cx);
                    let after2 = cx(self);
                    self.undo();
                    let after_undo1 = cx(self);
                    self.undo();
                    let after_undo2 = cx(self);
                    self.redo();
                    let after_redo = cx(self);
                    self.undo();
                    self.undo();
                    let after_undo3 = cx(self);
                    r += &format!("float steps: moved={after2:?} undo={after_undo1:?} undo={after_undo2:?} redo={after_redo:?} undo past paste={after_undo3:?} (expect 200, 100, 0, 100, None)\n");
                    let _ = std::fs::write(out, r);
                }
                if self.frame > 15 { self.allow_quit = true; ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
            } else if let Some(out) = mode.strip_prefix("paste:") {
                if self.frame == 90 {
                    let types = winit::platform::wayland::clipboard::types();
                    let report = match self.sysclip.offer() {
                        clip::Offer::Picture(p) => format!("picture {}x{} first={:?} types={types:?}", p.w, p.h, p.px.first()),
                        clip::Offer::Link(u) => format!("link {u} types={types:?}"),
                        clip::Offer::Nothing => format!("nothing types={types:?} wayland={}", winit::platform::wayland::clipboard::available()),
                    };
                    let _ = std::fs::write(out, report);
                }
                if self.frame > 100 { self.allow_quit = true; ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
            }
        }

        // screenshot mode: draw ~120 frames, save the window, quit
        if let Some(path) = self.shot.clone() {
            if self.frame == 120 {
                let done = self.shot_done.clone();
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                    let raw: Vec<u8> = img.pixels.iter().flat_map(|c| { let a = c.to_array(); [a[0], a[1], a[2], 255] }).collect();
                    if let Some(buf) = image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, raw) {
                        if let Some(p) = path.parent() { let _ = std::fs::create_dir_all(p); }
                        if let Err(e) = buf.save(&path) { eprintln!("couldn't save screenshot: {e}"); }
                    }
                    done.store(true, Ordering::SeqCst);
                })));
            }
            if self.shot_done.load(Ordering::SeqCst) || self.frame > 2000 {
                self.allow_quit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
        }

        // closing with unsaved work
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_quit {
            if let Some(i) = self.docs.iter().position(|d| d.dirty) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.cur = i;
                self.dialog = Some(Dialog::Confirm { doc: i, then: AfterConfirm::Quit });
            }
        }

        // files dropped onto the window
        // Anything dropped on the window opens in its own tab: picture files, and pictures dragged from a
        // web browser (an address to download, or a picture embedded in the page as data:).
        // Holding Shift adds them as layers of the current picture instead (the original's other drop
        // choice, "add layers", from MainForm.OnDragDropImpl, paintdotnet.dll 0x06001146).
        let (dropped, as_layers): (Vec<PathBuf>, bool) = ctx.input(|i| (i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect(), i.modifiers.shift));
        // Like the original (MainForm.OnDragDropImpl, 0x06001146): folders are skipped; one picture with no
        // picture open opens straight away; otherwise ask "Open" (each in its own tab) or "Add layers".
        // Our additions: pictures from web browsers, and Shift+drop = add layers without asking.
        let dropped: Vec<PathBuf> = dropped.into_iter().filter(|p| !p.is_dir()).collect();
        if !dropped.is_empty() {
            let nothing_open = self.docs.len() == 1 && self.docs[0].path.is_none() && self.docs[0].source.is_none() && !self.docs[0].dirty && self.docs[0].hist_pos == 0;
            if as_layers {
                self.handle_drop(dropped, true);
            } else if dropped.len() == 1 && nothing_open {
                self.handle_drop(dropped, false);
            } else {
                self.dialog = Some(Dialog::DropChoice { items: dropped });
            }
        }

        self.shortcuts(&ctx);
        self.poll_download();
        if self.download.is_some() { ctx.request_repaint_after(std::time::Duration::from_millis(200)); }

        let title = {
            let d = &self.docs[self.cur];
            format!("{}{} - {}", d.name, if d.dirty { "*" } else { "" }, APP_NAME)
        };
        if title != self.title { ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone())); self.title = title; }

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("toolbar").show(ui, |ui| self.tool_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("tools").resizable(false).exact_size(80.0).show(ui, |ui| self.tools_panel(ui));
        egui::Panel::right("side").resizable(true).default_size(250.0).min_size(200.0).show(ui, |ui| {
            self.history_panel(ui);
            ui.separator();
            self.layers_panel(ui);
        });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(self.theme().canvas_bg)).show(ui, |ui| {
            self.tabs(ui);
            self.canvas(ui);
        });
        self.dialogs(&ctx);
        self.text_box(&ctx);
        self.color_window(&ctx);
    }
}

impl App {
    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            let has_sel = self.docs[self.cur].s.selection.is_some();
            ui.menu_button("File", |ui| {
                if ui.button("New...                Ctrl+N").clicked() { let c = ui.ctx().clone(); self.open_new_dialog(&c); ui.close(); }
                if ui.button("Open...               Ctrl+O").clicked() { self.open_file_dialog(FileMode::Open); ui.close(); }
                if ui.button("Save                  Ctrl+S").clicked() { self.save(); ui.close(); }
                if ui.button("Save As...      Ctrl+Shift+S").clicked() { self.open_file_dialog(FileMode::Save); ui.close(); }
                if ui.button("Export a Copy...       Ctrl+E").on_hover_text("Save a flat picture (PNG, JPG...) as a separate copy").clicked() { self.open_file_dialog(FileMode::Export); ui.close(); }
                if ui.button("Close                 Ctrl+W").clicked() { let c = self.cur; self.close_doc(c, false); ui.close(); }
                ui.separator();
                ui.horizontal(|ui| { ui.label("JPEG quality"); ui.add(egui::Slider::new(&mut self.jpeg_quality, 1..=100)); });
                ui.separator();
                if ui.button("Exit").clicked() { ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close); ui.close(); }
            });
            ui.menu_button("Edit", |ui| {
                let (cu, cr) = (self.docs[self.cur].can_undo(), self.docs[self.cur].can_redo());
                let floating = self.floating_steps();
                let (cu, cr) = match &floating { Some((n, i)) => (true, i + 1 < n.len()), None => (cu, cr) };
                if ui.add_enabled(cu, egui::Button::new("Undo                Ctrl+Z")).clicked() { self.undo(); ui.close(); }
                if ui.add_enabled(cr, egui::Button::new("Redo                Ctrl+Y")).clicked() { self.redo(); ui.close(); }
                ui.separator();
                if ui.button("Cut                 Ctrl+X").clicked() { self.cut(); ui.close(); }
                if ui.button("Copy                Ctrl+C").clicked() { self.copy(false); ui.close(); }
                if ui.button("Copy Merged         Ctrl+Shift+C").clicked() { self.copy(true); ui.close(); }
                if ui.button("Paste               Ctrl+V").clicked() { self.paste(false); ui.close(); }
                if ui.button("Paste into New Layer Ctrl+Shift+V").clicked() { self.paste(true); ui.close(); }
                if ui.button("Paste into New Image Ctrl+Alt+V").clicked() { self.paste_new_image(); ui.close(); }
                ui.separator();
                if ui.add_enabled(has_sel, egui::Button::new("Copy Selection      Ctrl+Shift+Alt+C")).clicked() { self.copy_selection(); ui.close(); }
                if ui.add_enabled(self.sel_clipboard.is_some(), egui::Button::new("Paste Selection     Ctrl+Shift+Alt+V")).clicked() { self.paste_selection(); ui.close(); }
                ui.separator();
                if ui.button("Select All          Ctrl+A").clicked() { self.select_all(); ui.close(); }
                if ui.add_enabled(has_sel, egui::Button::new("Deselect            Ctrl+D")).clicked() { self.deselect(); ui.close(); }
                if ui.add_enabled(has_sel, egui::Button::new("Invert Selection    Ctrl+I")).clicked() { self.invert_selection(); ui.close(); }
                if ui.add_enabled(has_sel, egui::Button::new("Erase Selection     Delete")).clicked() { self.erase_selection(); ui.close(); }
                if ui.button("Fill Selection      Backspace").clicked() { self.fill_selection(); ui.close(); }
            });
            ui.menu_button("View", |ui| {
                if ui.button("Zoom In             Ctrl++").clicked() { self.zoom_step(1, None); ui.close(); }
                if ui.button("Zoom Out            Ctrl+-").clicked() { self.zoom_step(-1, None); ui.close(); }
                if ui.button("Zoom to Window      Ctrl+B").clicked() { self.zoom_fit(); ui.close(); }
                if ui.button("Actual Size         Ctrl+0").clicked() { self.actual_size(); ui.close(); }
                ui.separator();
                ui.checkbox(&mut self.pixel_grid, "Pixel grid");
                let mut dark = self.dark;
                if ui.checkbox(&mut dark, "Dark mode          Ctrl+Shift+L").changed() { let c = ui.ctx().clone(); self.set_dark(&c, dark); }
            });
            ui.menu_button("Image", |ui| {
                if ui.add_enabled(has_sel, egui::Button::new("Crop to Selection   Ctrl+Shift+X")).clicked() { self.crop_to_selection(); ui.close(); }
                if ui.button("Resize...           Ctrl+R").clicked() { let d = &self.docs[self.cur]; self.dialog = Some(Dialog::Resize { w: d.w() as u32, h: d.h() as u32, keep: true }); ui.close(); }
                if ui.button("Canvas Size...      Ctrl+Shift+R").clicked() { let d = &self.docs[self.cur]; self.dialog = Some(Dialog::CanvasSize { w: d.w() as u32, h: d.h() as u32, anchor: (1, 1) }); ui.close(); }
                ui.separator();
                for t in ["Flip Horizontal", "Flip Vertical", "Rotate 90° Clockwise", "Rotate 90° Counter-Clockwise", "Rotate 180°"] {
                    if ui.button(t).clicked() { self.image_transform(t); ui.close(); }
                }
                ui.separator();
                if ui.add_enabled(self.docs[self.cur].s.layers.len() > 1, egui::Button::new("Flatten             Ctrl+Shift+F")).clicked() { self.flatten(); ui.close(); }
            });
            ui.menu_button("Layers", |ui| self.layer_menu_items(ui));
            ui.menu_button("Adjustments", |ui| {
                for fx in Fx::ADJUSTMENTS {
                    if ui.button(fx.name()).clicked() { self.start_effect(fx); ui.close(); }
                }
            });
            ui.menu_button("Effects", |ui| {
                for (group, list) in Fx::EFFECTS {
                    ui.menu_button(group, |ui| {
                        for fx in list.iter() {
                            if ui.button(fx.name()).clicked() { self.start_effect(*fx); ui.close(); }
                        }
                    });
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("About").clicked() { self.dialog = Some(Dialog::About); ui.close(); }
            });
        });
    }

    fn layer_menu_items(&mut self, ui: &mut egui::Ui) {
        let n = self.docs[self.cur].s.layers.len();
        let a = self.docs[self.cur].s.active;
        if ui.button("Add New Layer       Ctrl+Shift+N").clicked() { self.add_layer(true); ui.close(); }
        if ui.add_enabled(n > 1, egui::Button::new("Delete Layer")).clicked() { self.delete_layer(); ui.close(); }
        if ui.button("Duplicate Layer     Ctrl+Shift+D").clicked() { self.duplicate_layer(); ui.close(); }
        if ui.add_enabled(a > 0, egui::Button::new("Merge Layer Down    Ctrl+M")).clicked() { self.merge_down(); ui.close(); }
        if ui.button("Import From File...").clicked() { self.open_file_dialog(FileMode::ImportLayer); ui.close(); }
        ui.separator();
        for t in ["Flip Layer Horizontal", "Flip Layer Vertical", "Rotate Layer 180°"] {
            if ui.button(t).clicked() { self.layer_transform(t); ui.close(); }
        }
        ui.separator();
        if ui.add_enabled(a + 1 < n, egui::Button::new("Move Layer Up")).clicked() { self.move_layer(true); ui.close(); }
        if ui.add_enabled(a > 0, egui::Button::new("Move Layer Down")).clicked() { self.move_layer(false); ui.close(); }
        if ui.button("Layer Properties...").clicked() { self.open_layer_props(); ui.close(); }
        let is_smart = self.docs[self.cur].layer().smart.is_some();
        if ui.add_enabled(is_smart, egui::Button::new("Rasterize Layer")).on_hover_text("Turn a smart object into ordinary pixels").clicked() { self.rasterize_layer(); ui.close(); }
    }

    fn open_layer_props(&mut self) {
        let l = self.docs[self.cur].layer();
        self.dialog = Some(Dialog::LayerProps { name: l.name.clone(), visible: l.visible, opacity: l.opacity, blend: l.blend, orig: (l.name.clone(), l.visible, l.opacity, l.blend) });
    }

    /// The chosen text font's file and face index (None: the built-in font, e.g. while fonts are scanned).
    fn text_font(&mut self) -> Option<(Arc<Vec<u8>>, u32)> {
        let cat = crate::fonts::catalog()?;
        if self.opts.font_family.is_none() {
            self.opts.font_family = cat.default_family();
        }
        let fam = self.opts.font_family.clone()?;
        let face = cat.pick(&fam, self.opts.font_weight, self.opts.font_italic)?;
        cat.data(face.id)
    }

    /// Font family, weight and italic pickers for the Text tool.
    fn font_controls(&mut self, ui: &mut egui::Ui) {
        let Some(cat) = crate::fonts::catalog() else {
            ui.weak("Loading fonts…");
            return;
        };
        if self.opts.font_family.is_none() {
            self.opts.font_family = cat.default_family();
        }
        let current = self.opts.font_family.clone().unwrap_or_default();
        ui.label("Font:");
        egui::ComboBox::from_id_salt("font-family").selected_text(&current).width(170.0).height(320.0).show_ui(ui, |ui| {
            ui.add(egui::TextEdit::singleline(&mut self.font_filter).hint_text("Search fonts").desired_width(160.0));
            let f = self.font_filter.to_lowercase();
            for (name, _) in cat.families.iter().filter(|(n, _)| f.is_empty() || n.to_lowercase().contains(&f)) {
                if ui.selectable_label(*name == current, name).clicked() {
                    self.opts.font_family = Some(name.clone());
                    ui.close();
                }
            }
        });
        if let Some(faces) = cat.faces(&current) {
            let chosen = cat.pick(&current, self.opts.font_weight, self.opts.font_italic);
            let label = |f: &crate::fonts::Face| format!("{}{}", crate::fonts::weight_name(f.weight), if f.italic { " Italic" } else { "" });
            egui::ComboBox::from_id_salt("font-weight").selected_text(chosen.map(|c| label(&c)).unwrap_or_default()).show_ui(ui, |ui| {
                for face in faces {
                    if ui.selectable_label(Some(*face) == chosen, label(face)).clicked() {
                        self.opts.font_weight = face.weight;
                        self.opts.font_italic = face.italic;
                    }
                }
            });
        }
    }

    fn tool_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let r = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover()).0;
            tools::icon(ui.painter(), r, self.tool, self.theme().icon);
            ui.strong(self.tool.name());
            ui.separator();
            let t = self.tool;
            if t.uses_brush_width() {
                ui.label("Brush width:");
                num_box(ui, &mut self.opts.brush, 1.0..=2000.0, 1.0, "");
            }
            if t.is_shape() {
                ui.label("Draw:");
                egui::ComboBox::from_id_salt("fill").selected_text(match self.opts.fill { FillStyle::Outline => "Outline", FillStyle::Fill => "Filled", FillStyle::Both => "Filled with outline" }).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.opts.fill, FillStyle::Outline, "Outline");
                    ui.selectable_value(&mut self.opts.fill, FillStyle::Fill, "Filled");
                    ui.selectable_value(&mut self.opts.fill, FillStyle::Both, "Filled with outline");
                });
            }
            if t == Tool::RoundedRect {
                ui.label("Radius:");
                num_box(ui, &mut self.opts.radius, 0.0..=500.0, 1.0, "");
            }
            if matches!(t, Tool::Paintbrush | Tool::Eraser | Tool::CloneStamp | Tool::Recolor | Tool::Line | Tool::Rectangle | Tool::RoundedRect | Tool::Ellipse | Tool::EllipseSelect) {
                ui.checkbox(&mut self.opts.aa, "Antialiasing");
            }
            if matches!(t, Tool::MagicWand | Tool::PaintBucket | Tool::Recolor) {
                if t != Tool::Recolor {
                    ui.label("Flood mode:");
                    egui::ComboBox::from_id_salt("flood").selected_text(if self.opts.global { "Global" } else { "Contiguous" }).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.opts.global, false, "Contiguous");
                        ui.selectable_value(&mut self.opts.global, true, "Global");
                    });
                }
                ui.label("Tolerance:");
                let mut pct = (self.opts.tol * 100.0).round();
                if ui.add(egui::Slider::new(&mut pct, 0.0..=100.0).suffix("%")).changed() { self.opts.tol = pct / 100.0; }
                if t != Tool::Recolor {
                    // the original's "SampleAllLayers" setting (ToolsSection, default off)
                    ui.label("Sampling:");
                    egui::ComboBox::from_id_salt("sample").selected_text(if self.opts.sample_image { "Image" } else { "Layer" }).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.opts.sample_image, false, "Layer").on_hover_text("Compare colours on the active layer");
                        ui.selectable_value(&mut self.opts.sample_image, true, "Image").on_hover_text("Compare colours of the whole picture, all layers together");
                    });
                }
            }
            if t.is_select() {
                ui.label("Selection mode:");
                egui::ComboBox::from_id_salt("selmode").selected_text(self.opts.sel_mode.name()).show_ui(ui, |ui| {
                    for m in SelMode::ALL { ui.selectable_value(&mut self.opts.sel_mode, m, m.name()); }
                });
            }
            if t == Tool::Gradient {
                ui.label("Type:");
                egui::ComboBox::from_id_salt("grad").selected_text(self.opts.gradient.name()).show_ui(ui, |ui| {
                    for g in GradientKind::ALL { ui.selectable_value(&mut self.opts.gradient, g, g.name()); }
                });
            }
            if t == Tool::Text {
                self.font_controls(ui);
                ui.label("Size:");
                num_box(ui, &mut self.opts.font_size, 1.0..=2000.0, 1.0, " pt");
            }
            if t == Tool::CloneStamp {
                ui.label(match self.clone_anchor { Some(a) => format!("Copying from {}, {}", a.0 as i32, a.1 as i32), None => "Ctrl+click to choose a source".into() });
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if self.status.is_empty() { ui.label(self.tool.help()); } else { ui.spinner(); ui.label(&self.status); }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let hover = self.hover;
                let d = &mut self.docs[self.cur];
                let mut pct = d.zoom * 100.0;
                if num_box(ui, &mut pct, 1.0..=10000.0, 1.0, "%").changed() { d.zoom = pct / 100.0; }
                ui.separator();
                ui.label(format!("{} × {}", d.w(), d.h()));
                ui.separator();
                if let Some((x0, y0, x1, y1)) = Self::sel_bounds(d) { ui.label(format!("Selection {} × {}", x1 - x0, y1 - y0)); ui.separator(); }
                if let Some((x, y)) = hover { ui.label(format!("{x}, {y}")); }
            });
        });
    }

    fn tools_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Tools").small().strong());
        egui::Grid::new("toolgrid").spacing(vec2(4.0, 4.0)).min_col_width(30.0).show(ui, |ui| {
            for (i, t) in Tool::ALL.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
                let sel = *t == self.tool;
                let th = self.theme();
                let bg = if sel { th.sel_bg } else if resp.hovered() { th.hover_bg } else { Color32::TRANSPARENT };
                ui.painter().rect_filled(r, 4.0, bg);
                if sel { ui.painter().rect_stroke(r, 4.0, Stroke::new(1.0, th.accent), egui::StrokeKind::Inside); }
                tools::icon(ui.painter(), r.shrink(4.0), *t, th.icon);
                if resp.clicked() { self.set_tool(*t); }
                resp.on_hover_text(format!("{} ({})", t.name(), t.key()));
                if i % 2 == 1 { ui.end_row(); }
            }
        });
        ui.add_space(8.0);
        ui.separator();
        self.colors_panel(ui);
    }

    fn colors_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Colors").small().strong());
        let (r, _) = ui.allocate_exact_size(vec2(64.0, 54.0), Sense::hover());
        let sec_r = Rect::from_min_size(r.min + vec2(24.0, 22.0), vec2(30.0, 30.0));
        let pri_r = Rect::from_min_size(r.min + vec2(4.0, 2.0), vec2(30.0, 30.0));
        for (rr, c, is_sec) in [(sec_r, self.secondary, true), (pri_r, self.primary, false)] {
            let resp = ui.interact(rr, ui.scope_id().with(("swatch", is_sec)), Sense::click());
            let p = ui.painter();
            p.rect_filled(rr, 0.0, Color32::WHITE);
            for y in 0..3 { for x in 0..3 { if (x + y) % 2 == 0 { p.rect_filled(Rect::from_min_size(rr.min + vec2(x as f32 * 10.0, y as f32 * 10.0), vec2(10.0, 10.0)), 0.0, Color32::from_gray(200)); } } }
            p.rect_filled(rr, 0.0, c32(c));
            let th = self.theme();
            let outline = if self.edit_secondary == is_sec && self.colors_more { th.accent } else { th.border };
            p.rect_stroke(rr, 0.0, Stroke::new(1.5, outline), egui::StrokeKind::Outside);
            if resp.clicked() { self.edit_secondary = is_sec; self.colors_more = true; }
            resp.on_hover_text(if is_sec { "Secondary colour (click to edit)" } else { "Primary colour (click to edit)" });
        }
        ui.horizontal(|ui| {
            if ui.small_button("⇄").on_hover_text("Swap colours (X)").clicked() { std::mem::swap(&mut self.primary, &mut self.secondary); }
            if ui.small_button("◪").on_hover_text("Reset to black and white").clicked() { self.primary = [0, 0, 0, 255]; self.secondary = [255, 255, 255, 255]; }
        });
        if ui.small_button(if self.colors_more { "Less «" } else { "More »" }).clicked() { self.colors_more = !self.colors_more; }
        ui.add_space(4.0);
        egui::Grid::new("palette").spacing(vec2(1.0, 1.0)).min_col_width(15.0).show(ui, |ui| {
            let pal = self.palette.clone();
            for (i, c) in pal.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(15.0, 12.0), Sense::click());
                ui.painter().rect_filled(r, 0.0, c32(*c));
                if resp.clicked() { self.primary = *c; }
                if resp.secondary_clicked() { self.secondary = *c; }
                resp.on_hover_text("Left click: primary, right click: secondary");
                if i % 4 == 3 { ui.end_row(); }
            }
        });
    }

    fn color_window(&mut self, ctx: &egui::Context) {
        if !self.colors_more { return; }
        let target = if self.edit_secondary { self.secondary } else { self.primary };
        let mut c = c32(target);
        let mut open = true;
        let mut add = false;
        egui::Window::new(if self.edit_secondary { "Secondary colour" } else { "Primary colour" })
            .open(&mut open).resizable(false).collapsible(false).default_pos(pos2(96.0, 300.0))
            .show(ctx, |ui| {
                egui::color_picker::color_picker_color32(ui, &mut c, egui::color_picker::Alpha::OnlyBlend);
                let mut hex = format!("{:02X}{:02X}{:02X}{:02X}", target[0], target[1], target[2], target[3]);
                ui.horizontal(|ui| {
                    ui.label("Hex:");
                    if ui.add(egui::TextEdit::singleline(&mut hex).desired_width(80.0)).changed() {
                        let h = hex.trim_start_matches('#');
                        if h.len() >= 6 {
                            let b = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("FF"), 16).unwrap_or(255);
                            c = Color32::from_rgba_unmultiplied(b(0), b(2), b(4), if h.len() >= 8 { b(6) } else { 255 });
                        }
                    }
                });
                if ui.button("Add to palette").clicked() { add = true; }
            });
        let nc = px(c);
        if nc != target { if self.edit_secondary { self.secondary = nc } else { self.primary = nc } }
        if add { self.palette.push(nc); }
        if !open { self.colors_more = false; }
    }

    fn history_panel(&mut self, ui: &mut egui::Ui) {
        // a floating picture's own steps (paste, transforms) come after the picture's history
        let floating = self.floating_steps();
        let (can_undo, can_redo) = match &floating {
            Some((n, i)) => (true, i + 1 < n.len()),
            None => (self.docs[self.cur].can_undo(), self.docs[self.cur].can_redo()),
        };
        let mut action = 0;
        ui.horizontal(|ui| {
            ui.strong("History");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(can_redo, egui::Button::new("⏭").small()).on_hover_text("Redo all").clicked() { action = 4; }
                if ui.add_enabled(can_redo, egui::Button::new("↷").small()).on_hover_text("Redo (Ctrl+Y)").clicked() { action = 3; }
                if ui.add_enabled(can_undo, egui::Button::new("↶").small()).on_hover_text("Undo (Ctrl+Z)").clicked() { action = 2; }
                if ui.add_enabled(can_undo, egui::Button::new("⏮").small()).on_hover_text("Undo all").clicked() { action = 1; }
            });
        });
        let mut jump = None;
        let mut float_jump = None;
        egui::ScrollArea::vertical().id_salt("hist").max_height(180.0).stick_to_bottom(true).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let muted = self.theme().muted;
            let icon_col = self.theme().icon;
            // one row: the step's icon, then its name
            let row = |ui: &mut egui::Ui, name: &str, selected: bool, text: egui::RichText, future: bool| -> egui::Response {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    tools::history_icon(ui.painter(), r, name, if future { muted } else { icon_col });
                    ui.selectable_label(selected, text)
                }).inner
            };
            let d = &self.docs[self.cur];
            for (i, item) in d.history.iter().enumerate() {
                let future = i > d.hist_pos;
                let text = if future { egui::RichText::new(&item.name).color(muted) } else { egui::RichText::new(&item.name) };
                if row(ui, &item.name, i == d.hist_pos && floating.is_none(), text, future).clicked() { jump = Some(i); }
            }
            if let Some((names, cur)) = &floating {
                for (i, name) in names.iter().enumerate() {
                    let future = i > *cur;
                    let text = if future { egui::RichText::new(name).color(muted) } else { egui::RichText::new(name).italics() };
                    if row(ui, name, i == *cur, text, future).on_hover_text("Still floating: deselect or press Enter to put it down").clicked() { float_jump = Some(i); }
                }
            }
        });
        match action {
            1 => { if floating.is_some() { self.floating_goto(0); self.undo(); } self.finish_floating(); self.doc().goto(0); }
            2 => self.undo(),
            3 => self.redo(),
            4 => { if let Some((n, _)) = &floating { self.floating_goto(n.len() - 1); } else { let d = self.doc(); let n = d.history.len() - 1; d.goto(n); } }
            _ => {}
        }
        if let Some(i) = float_jump { self.floating_goto(i); }
        if let Some(i) = jump {
            // jumping back past a floating picture takes it away first
            self.cancel_floating();
            self.doc().goto(i);
        }
    }

    fn layers_panel(&mut self, ui: &mut egui::Ui) {
        ui.strong("Layers");
        let key = (self.cur, self.docs[self.cur].version);
        if self.thumbs_key != key && self.drag.is_none() {
            self.thumbs_key = key;
            let d = &self.docs[self.cur];
            let (w, h) = (d.w(), d.h());
            let s = 40.0 / w.max(h) as f32;
            let (tw, th) = (((w as f32 * s) as usize).max(1), ((h as f32 * s) as usize).max(1));
            self.thumbs = d.s.layers.iter().enumerate().map(|(i, l)| {
                let mut img = egui::ColorImage::filled([tw, th], Color32::WHITE);
                for y in 0..th { for x in 0..tw {
                    let sx = ((x as f32 + 0.5) / s) as usize; let sy = ((y as f32 + 0.5) / s) as usize;
                    let p = l.px[sy.min(h - 1) * w + sx.min(w - 1)];
                    let bgc = if ((x / 4) + (y / 4)) % 2 == 0 { [255u8; 4] } else { [210, 210, 210, 255] };
                    img.pixels[y * tw + x] = c32(over(bgc, p, 255));
                } }
                (d.version, ui.ctx().load_texture(format!("thumb{i}"), img, TextureOptions::LINEAR))
            }).collect();
        }
        let mut action: Option<(usize, u8)> = None;
        egui::ScrollArea::vertical().id_salt("layers").max_height((ui.available_height() - 40.0).max(80.0)).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let th = self.theme();
            let d = &self.docs[self.cur];
            for i in (0..d.s.layers.len()).rev() {
                let l = &d.s.layers[i];
                let active = i == d.s.active;
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click());
                if active { ui.painter().rect_filled(r, 3.0, th.sel_bg); } else if resp.hovered() { ui.painter().rect_filled(r, 3.0, th.hover_bg); }
                if let Some((_, t)) = self.thumbs.get(i) {
                    let tr = Rect::from_center_size(pos2(r.left() + 26.0, r.center().y), t.size_vec2());
                    ui.painter().image(t.id(), tr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    ui.painter().rect_stroke(tr, 0.0, Stroke::new(1.0, th.border), egui::StrokeKind::Outside);
                }
                let name_rect = ui.painter().text(pos2(r.left() + 52.0, r.center().y), egui::Align2::LEFT_CENTER, &l.name, egui::FontId::proportional(13.0), th.text);
                if l.smart.is_some() {
                    // a small mark for smart objects
                    ui.painter().text(pos2(name_rect.right() + 6.0, r.center().y), egui::Align2::LEFT_CENTER, "◆ smart", egui::FontId::proportional(10.0), th.accent);
                }
                let cb = Rect::from_center_size(pos2(r.right() - 14.0, r.center().y), vec2(16.0, 16.0));
                let cresp = ui.interact(cb, ui.scope_id().with(("vis", i)), Sense::click());
                ui.painter().rect_filled(cb, 2.0, Color32::WHITE);
                ui.painter().rect_stroke(cb, 2.0, Stroke::new(1.0, th.border), egui::StrokeKind::Inside);
                if l.visible { ui.painter().text(cb.center(), egui::Align2::CENTER_CENTER, "✔", egui::FontId::proportional(12.0), Color32::from_rgb(0, 90, 170)); }
                if cresp.clicked() { action = Some((i, 0)); }
                else if resp.double_clicked() { action = Some((i, 1)); }
                else if resp.clicked() { action = Some((i, 2)); }
            }
        });
        match action {
            Some((i, 0)) => { self.finish_floating(); let d = self.doc(); d.s.layers[i].visible = !d.s.layers[i].visible; d.commit("Layer Visibility"); }
            Some((i, 1)) => { self.finish_floating(); self.doc().s.active = i; self.open_layer_props(); }
            Some((i, _)) => { self.finish_floating(); let d = self.doc(); d.s.active = i; d.touch(); }
            None => {}
        }
        ui.horizontal(|ui| {
            let n = self.docs[self.cur].s.layers.len();
            let a = self.docs[self.cur].s.active;
            if ui.small_button("+").on_hover_text("Add New Layer").clicked() { self.add_layer(true); }
            if ui.add_enabled(n > 1, egui::Button::new("🗑").small()).on_hover_text("Delete Layer").clicked() { self.delete_layer(); }
            if ui.small_button("⧉").on_hover_text("Duplicate Layer").clicked() { self.duplicate_layer(); }
            if ui.add_enabled(a > 0, egui::Button::new("⤓").small()).on_hover_text("Merge Layer Down").clicked() { self.merge_down(); }
            if ui.add_enabled(a + 1 < n, egui::Button::new("⏶").small()).on_hover_text("Move Layer Up").clicked() { self.move_layer(true); }
            if ui.add_enabled(a > 0, egui::Button::new("⏷").small()).on_hover_text("Move Layer Down").clicked() { self.move_layer(false); }
            if ui.small_button("⚙").on_hover_text("Layer Properties").clicked() { self.open_layer_props(); }
        });
    }

    fn tabs(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().fill(self.theme().tabs_bg).inner_margin(egui::Margin::symmetric(6, 3)).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                let mut close = None;
                let mut pick = None;
                for i in 0..self.docs.len() {
                    let d = &self.docs[i];
                    let label = format!("{}{}", d.name, if d.dirty { " *" } else { "" });
                    if ui.selectable_label(i == self.cur, label).clicked() && i != self.cur { pick = Some(i); }
                    if ui.small_button("×").on_hover_text("Close").clicked() { close = Some(i); }
                    ui.add_space(6.0);
                }
                if let Some(i) = pick { self.finish_text(); self.finish_floating(); self.cur = i; self.drag = None; }
                if let Some(i) = close { self.close_doc(i, false); }
            });
        });
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let avail = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(avail, Sense::click_and_drag());
        let ctx = ui.ctx().clone();
        let (w, h) = (self.docs[self.cur].w(), self.docs[self.cur].h());

        if self.docs[self.cur].fit_pending {
            let d = self.doc();
            let z = ((avail.width() - 40.0) / w as f32).min((avail.height() - 40.0) / h as f32).clamp(0.01, 1.0);
            d.zoom = z;
            d.pan = (0.0, 0.0);
            d.fit_pending = false;
        }

        // texture of the composited picture
        let ver = self.docs[self.cur].version;
        let nearest = self.docs[self.cur].zoom >= 2.0;
        let need = match &self.tex { Some((_, c, v, n)) => *c != self.cur || *v != ver || *n != nearest, None => true };
        if need {
            let opts = if nearest { TextureOptions::NEAREST } else { TextureOptions::LINEAR };
            let cur = self.cur;
            // Only re-mix and re-upload the changed box when the same picture is already on screen.
            let same_picture = matches!(&self.tex, Some((t, c, _, n)) if *c == cur && *n == nearest && t.size() == [w, h]);
            let d = &mut self.docs[cur];
            let partial = if same_picture && !d.full_dirty { d.damage } else { None };
            let full = !same_picture || d.full_dirty;
            d.damage = None;
            d.full_dirty = false;
            if full {
                let flat = d.composite();
                let raw: Vec<u8> = flat.iter().flatten().copied().collect();
                let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &raw);
                match &mut self.tex {
                    Some((t, c, v, n)) => { t.set(img, opts); *c = cur; *v = ver; *n = nearest; }
                    None => self.tex = Some((ctx.load_texture("canvas", img, opts), cur, ver, nearest)),
                }
            } else if let Some(r) = partial {
                let part = d.composite_rect(r);
                let raw: Vec<u8> = part.iter().flatten().copied().collect();
                let img = egui::ColorImage::from_rgba_unmultiplied([r.2 - r.0, r.3 - r.1], &raw);
                if let Some((t, _, v, _)) = &mut self.tex { t.set_partial([r.0, r.1], img, opts); *v = ver; }
            } else if let Some((_, _, v, _)) = &mut self.tex {
                *v = ver; // nothing on the picture changed (e.g. a history step was recorded)
            }
        }
        if self.checker.is_none() {
            let mut img = egui::ColorImage::filled([16, 16], Color32::WHITE);
            for y in 0..16 { for x in 0..16 { if (x / 8 + y / 8) % 2 == 1 { img.pixels[y * 16 + x] = Color32::from_gray(204); } } }
            self.checker = Some(ctx.load_texture("checker", img, TextureOptions { wrap_mode: egui::TextureWrapMode::Repeat, ..TextureOptions::NEAREST }));
        }

        let d = &self.docs[self.cur];
        let z = d.zoom;
        let size = vec2(w as f32 * z, h as f32 * z);
        let center = avail.center() + vec2(d.pan.0, d.pan.1);
        let img_rect = Rect::from_center_size(center, size);
        let to_img = |p: Pos2| ((p.x - img_rect.min.x) / z, (p.y - img_rect.min.y) / z);
        let to_scr = |x: f32, y: f32| pos2(img_rect.min.x + x * z, img_rect.min.y + y * z);
        // the part of the picture on screen, for placing pastes like the original
        {
            let vis = img_rect.intersect(avail);
            self.visible_img = (vis.width() > 0.0 && vis.height() > 0.0).then(|| {
                let (a, b) = (to_img(vis.min), to_img(vis.max));
                (a.0.ceil().max(0.0) as usize, a.1.ceil().max(0.0) as usize, (b.0.floor().max(0.0) as usize).min(w), (b.1.floor().max(0.0) as usize).min(h))
            });
        }

        let painter = ui.painter_at(avail);
        painter.rect_filled(img_rect.translate(vec2(3.0, 3.0)).expand(1.0), 0.0, Color32::from_black_alpha(50));
        if let Some(ch) = &self.checker {
            painter.image(ch.id(), img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(size.x / 16.0, size.y / 16.0)), Color32::WHITE);
        }
        if let Some((t, ..)) = &self.tex {
            painter.image(t.id(), img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        painter.rect_stroke(img_rect, 0.0, Stroke::new(1.0, self.theme().border), egui::StrokeKind::Outside);

        // pixel grid
        if self.pixel_grid && z >= 6.0 {
            let vis = img_rect.intersect(avail);
            let (x0, y0) = to_img(vis.min);
            let (x1, y1) = to_img(vis.max);
            let gs = Stroke::new(1.0, Color32::from_black_alpha(60));
            for x in (x0.floor() as i32).max(0)..=(x1.ceil() as i32).min(w as i32) { painter.line_segment([to_scr(x as f32, y0.max(0.0)), to_scr(x as f32, y1.min(h as f32))], gs); }
            for y in (y0.floor() as i32).max(0)..=(y1.ceil() as i32).min(h as i32) { painter.line_segment([to_scr(x0.max(0.0), y as f32), to_scr(x1.min(w as f32), y as f32)], gs); }
        }

        // selection outline ("marching ants")
        if self.outline.0 != self.cur || self.outline.1 != d.sel_version {
            // history steps bump the version without changing the selection: only rebuild when it really changed
            let changed = self.outline.0 != self.cur || self.outline_mask.as_deref() != d.s.selection.as_deref();
            if changed {
                let segs = match &d.s.selection { Some(m) => raster::mask_outline(m, w, h), None => vec![] };
                self.outline.2 = segs;
                self.outline_mask = d.s.selection.clone();
            }
            self.outline.0 = self.cur;
            self.outline.1 = d.sel_version;
        }
        // while a selection is being dragged, show the moved/new outline instead of the old one
        let drag_offset = match (&self.drag, self.tool) {
            (Some(dr), Tool::MoveSelection | Tool::MovePixels) => vec2(dr.offset.0 as f32, dr.offset.1 as f32) * z,
            _ => Vec2::ZERO,
        };
        let selecting = matches!((&self.drag, self.tool), (Some(_), Tool::RectSelect | Tool::EllipseSelect | Tool::LassoSelect));
        if !self.outline.2.is_empty() && !(selecting && self.opts.sel_mode == SelMode::Replace) {
            // only the edges that are on screen, with dashes and animation only when that stays cheap
            let view = avail.expand(2.0);
            let visible: Vec<(Pos2, Pos2)> = self.outline.2.iter()
                .map(|(a, b)| (to_scr(a.0, a.1) + drag_offset, to_scr(b.0, b.1) + drag_offset))
                .filter(|(pa, pb)| view.intersects(Rect::from_two_pos(*pa, *pb)))
                .collect();
            if visible.len() <= 4000 {
                let phase = (ctx.input(|i| i.time) * 8.0) as f32 % 8.0;
                for (pa, pb) in &visible {
                    painter.line_segment([*pa, *pb], Stroke::new(1.0, Color32::WHITE));
                    painter.extend(egui::Shape::dashed_line_with_offset(&[*pa, *pb], Stroke::new(1.0, Color32::BLACK), &[4.0], &[4.0], phase));
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            } else if visible.len() <= 60000 {
                for (pa, pb) in &visible { painter.line_segment([*pa, *pb], Stroke::new(1.0, Color32::BLACK)); }
            } else if let Some((x0, y0, x1, y1)) = Self::sel_bounds(d) {
                painter.rect_stroke(Rect::from_min_max(to_scr(x0 as f32, y0 as f32), to_scr(x1 as f32, y1 as f32)).translate(drag_offset), 0.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Middle);
            }
        }
        // the selection being drawn right now
        if let (Some(dr), true) = (&self.drag, selecting) {
            let ants = |p: &egui::Painter, pts: &[Pos2]| {
                p.add(egui::Shape::line(pts.to_vec(), Stroke::new(1.0, Color32::WHITE)));
                p.extend(egui::Shape::dashed_line(pts, Stroke::new(1.0, Color32::BLACK), 4.0, 4.0));
            };
            match (self.tool, dr.sel_box) {
                (Tool::RectSelect, Some(r)) => {
                    let (a, b, c, e) = (to_scr(r.0, r.1), to_scr(r.2, r.1), to_scr(r.2, r.3), to_scr(r.0, r.3));
                    ants(&painter, &[a, b, c, e, a]);
                }
                (Tool::EllipseSelect, Some(r)) => {
                    let (cx, cy, rx, ry) = ((r.0 + r.2) / 2.0, (r.1 + r.3) / 2.0, (r.2 - r.0) / 2.0, (r.3 - r.1) / 2.0);
                    let pts: Vec<Pos2> = (0..=64).map(|i| { let t = i as f32 / 64.0 * std::f32::consts::TAU; to_scr(cx + rx * t.cos(), cy + ry * t.sin()) }).collect();
                    ants(&painter, &pts);
                }
                (Tool::LassoSelect, _) if dr.lasso.len() > 1 => {
                    let mut pts: Vec<Pos2> = dr.lasso.iter().map(|q| to_scr(q.0, q.1)).collect();
                    pts.push(pts[0]);
                    ants(&painter, &pts);
                }
                _ => {}
            }
        }

        // pointer handling
        let pointer = ctx.input(|i| i.pointer.clone());
        let mods = ctx.input(|i| i.modifiers);
        let hover_pos = pointer.hover_pos().filter(|p| avail.contains(*p));
        self.hover = hover_pos.map(|p| { let q = to_img(p); (q.0.floor() as i32, q.1.floor() as i32) });

        if let Some(p) = hover_pos {
            if matches!(self.tool, Tool::Paintbrush | Tool::Eraser | Tool::CloneStamp | Tool::Recolor) {
                painter.circle_stroke(p, (self.opts.brush * z / 2.0).max(1.5), Stroke::new(1.0, Color32::from_black_alpha(150)));
            }
            if let (Tool::CloneStamp, Some(a)) = (self.tool, self.clone_anchor) {
                painter.circle_stroke(to_scr(a.0, a.1), 6.0, Stroke::new(1.0, Color32::from_rgb(220, 40, 40)));
            }
        }

        // Mouse wheel, as the original's CanvasControl.ProcessVerticalPointerWheel (paintdotnet.dll 0x06002299):
        // no key = scroll up/down, Shift = scroll sideways (120 screen pixels per notch), Ctrl = zoom by
        // 1.12 per notch around the pointer, snapping to a whole-number zoom when within 5 %.
        if resp.hovered() {
            let (wheels, zoom_delta) = ctx.input(|i| {
                let w: Vec<(Vec2, bool, egui::Modifiers)> = i.events.iter().filter_map(|e| match e {
                    egui::Event::MouseWheel { unit, delta, modifiers, .. } => Some(match unit {
                        egui::MouseWheelUnit::Point => (*delta / WHEEL_PIXELS_PER_NOTCH, true, *modifiers),
                        egui::MouseWheelUnit::Line => (*delta, false, *modifiers),
                        egui::MouseWheelUnit::Page => (*delta * 3.0, false, *modifiers),
                    }),
                    _ => None,
                }).collect();
                (w, i.zoom_delta())
            });
            let around = hover_pos.map(to_img);
            for (notches, _fine, m) in &wheels {
                let n = if notches.y != 0.0 { notches.y } else { notches.x };
                if m.ctrl || m.command {
                    if n != 0.0 { self.wheel_zoom(n, around); }
                } else if m.shift {
                    self.doc().pan.0 += n * WHEEL_SCROLL_PIXELS;
                } else if !m.alt {
                    let d = self.doc();
                    d.pan.0 += notches.x * WHEEL_SCROLL_PIXELS;
                    d.pan.1 += notches.y * WHEEL_SCROLL_PIXELS;
                }
            }
            // touchpad pinch (no wheel events): same smooth zoom
            if wheels.is_empty() && zoom_delta != 1.0 {
                self.zoom_to(self.docs[self.cur].zoom * zoom_delta, around);
            }
        }

        // middle-drag pans with any tool
        if resp.dragged_by(egui::PointerButton::Middle) {
            let dd = resp.drag_delta();
            let d = self.doc();
            d.pan.0 += dd.x;
            d.pan.1 += dd.y;
        }

        let pressed_p = pointer.button_pressed(egui::PointerButton::Primary);
        let pressed_s = pointer.button_pressed(egui::PointerButton::Secondary);
        // free transform (Move Selected Pixels tool): handles move, resize and rotate
        let scr = |x: f32, y: f32| { let s = to_scr(x, y); (s.x, s.y) };
        if self.tool == Tool::MovePixels {
            if (pressed_p || pressed_s) && resp.hovered() && self.xdrag.is_none() && self.dialog.is_none() {
                if let Some(p) = hover_pos {
                    self.xform_press((p.x, p.y), to_img(p), pressed_s, &scr);
                }
            } else if self.xdrag.is_some() {
                if let Some(p) = pointer.latest_pos() { self.xform_update(to_img(p), mods.shift); }
                if !(pointer.primary_down() || pointer.secondary_down()) { self.xform_release(); }
                ctx.request_repaint();
            }
            self.draw_xform_box(&painter, &scr);
            if resp.hovered() {
                if let Some(p) = hover_pos { ctx.set_cursor_icon(self.xform_cursor((p.x, p.y), &scr)); }
            }
            return;
        }
        if (pressed_p || pressed_s) && resp.hovered() && self.drag.is_none() && self.dialog.is_none() {
            if let Some(p) = hover_pos {
                if self.tool == Tool::Pan {
                    let pan0 = self.docs[self.cur].pan;
                    self.drag = Some(Drag { secondary: pressed_s, start: (p.x, p.y), last: (p.x, p.y), snap: vec![], sel_snap: None, cov: None, lasso: vec![], mode: None, float: None, pan0, moved: false, prev_rect: None, sel_box: None, offset: (0, 0), src_box: None });
                } else {
                    self.begin_drag(to_img(p), pressed_s, mods);
                }
            }
        } else if self.drag.is_some() {
            let down = pointer.primary_down() || pointer.secondary_down();
            if let Some(p) = pointer.latest_pos() {
                if self.tool == Tool::Pan {
                    if let Some(dr) = &self.drag {
                        let (sx, sy, p0) = (dr.start.0, dr.start.1, dr.pan0);
                        self.doc().pan = (p0.0 + p.x - sx, p0.1 + p.y - sy);
                    }
                } else {
                    let ip = to_img(p);
                    if self.drag.as_ref().map(|d| d.last != ip).unwrap_or(false) { self.update_drag(ip, mods); }
                }
            }
            if !down { self.end_drag(); }
        }

        if resp.hovered() {
            ctx.set_cursor_icon(match self.tool {
                Tool::Pan => if self.drag.is_some() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab },
                Tool::Zoom => egui::CursorIcon::ZoomIn,
                Tool::MovePixels | Tool::MoveSelection => egui::CursorIcon::Move,
                Tool::Text => egui::CursorIcon::Text,
                _ => egui::CursorIcon::Crosshair,
            });
        }
        if self.text.is_some() || self.drag.is_some() { ctx.request_repaint(); }
    }

    fn text_box(&mut self, ctx: &egui::Context) {
        let Some(mut t) = self.text.take() else { return };
        let mut finish = false;
        let mut cancel = false;
        let before = t.text.clone();
        let font_key = |o: &Opts| (o.font_size, o.font_family.clone(), o.font_weight, o.font_italic);
        let font_before = (font_key(&self.opts), t.color);
        egui::Window::new("Text").collapsible(false).resizable(false).default_pos(pos2(420.0, 140.0)).show(ctx, |ui| {
            ui.label("Type your text. It is drawn in the primary colour.");
            let r = ui.add(egui::TextEdit::multiline(&mut t.text).desired_rows(3).desired_width(260.0));
            if before.is_empty() && !r.has_focus() { r.request_focus(); }
            ui.horizontal(|ui| {
                self.font_controls(ui);
            });
            ui.horizontal(|ui| {
                ui.label("Size:");
                num_box(ui, &mut self.opts.font_size, 1.0..=2000.0, 1.0, " pt");
            });
            ui.horizontal(|ui| {
                if ui.button("Finish").clicked() { finish = true; }
                if ui.button("Cancel").clicked() { cancel = true; }
            });
        });
        t.color = self.primary;
        if cancel {
            let d = self.doc();
            d.layer_mut().px = t.snap;
            d.mark_full();
            return;
        }
        if t.text != before || font_before != (font_key(&self.opts), t.color) { self.render_text_preview(&mut t); }
        self.text = Some(t);
        if finish { self.finish_text(); }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.dialog.take() else { return };
        let mut keep = true;
        let win = |title: &str| egui::Window::new(title.to_string()).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0)).order(egui::Order::Foreground);
        match &mut dlg {
            Dialog::Message(m) => {
                win("Notice").show(ctx, |ui| { ui.label(m.as_str()); if ui.button("OK").clicked() { keep = false; } });
            }
            Dialog::About => {
                win("About").show(ctx, |ui| {
                    ui.heading(APP_NAME);
                    ui.label("A layered image editor for Linux, written in Rust.");
                    ui.label("Tools, layers, history, adjustments and effects.");
                    if ui.button("OK").clicked() { keep = false; }
                });
            }
            Dialog::PasteBig { pic, new_layer } => {
                let (pw, ph, nl) = (pic.w, pic.h, *new_layer);
                let (cw, ch) = (self.docs[self.cur].w(), self.docs[self.cur].h());
                let mut choice = 0;
                win("Paste a bigger picture").show(ctx, |ui| {
                    ui.label(format!("The picture you're pasting is {pw} × {ph}, bigger than the canvas ({cw} × {ch})."));
                    ui.horizontal(|ui| {
                        if ui.button("Expand canvas").clicked() { choice = 1; }
                        if ui.button("Keep canvas size").clicked() { choice = 2; }
                        if ui.button("Cancel").clicked() { choice = 3; }
                    });
                });
                if choice != 0 {
                    keep = false;
                    if choice < 3 {
                        let Dialog::PasteBig { pic, .. } = std::mem::replace(&mut dlg, Dialog::About) else { unreachable!() };
                        self.paste_into(pic, nl, choice == 1);
                    }
                }
            }
            Dialog::Rasterize => {
                let mut choice = 0;
                win("Smart object").show(ctx, |ui| {
                    ui.label("This layer is a smart object: it keeps its whole picture, even the parts outside the canvas.");
                    ui.label("To paint or apply effects on it, it has to become ordinary pixels first. Parts outside the canvas will then be cut off.");
                    ui.horizontal(|ui| {
                        if ui.button("Rasterize layer").clicked() { choice = 1; }
                        if ui.button("Cancel").clicked() { choice = 2; }
                    });
                });
                if choice != 0 {
                    keep = false;
                    if choice == 1 { self.rasterize_layer(); }
                }
            }
            Dialog::DropChoice { items } => {
                let n = items.len();
                let mut choice = 0;
                win(if n == 1 { "Open or add as a layer?" } else { "Open or add as layers?" }).show(ctx, |ui| {
                    ui.label(if n == 1 { "What do you want to do with the picture you dropped?".to_string() } else { format!("What do you want to do with the {n} pictures you dropped?") });
                    ui.horizontal(|ui| {
                        if ui.button(if n == 1 { "Open" } else { "Open each" }).on_hover_text("Open in its own tab").clicked() { choice = 1; }
                        if ui.button(if n == 1 { "Add layer" } else { "Add layers" }).on_hover_text("Add to the current picture as new layers").clicked() { choice = 2; }
                        if ui.button("Cancel").clicked() { choice = 3; }
                    });
                });
                if choice != 0 {
                    keep = false;
                    if choice < 3 {
                        let items = std::mem::take(items);
                        self.handle_drop(items, choice == 2);
                    }
                }
            }
            Dialog::Overwrite { path, export } => {
                let (p, export) = (path.clone(), *export);
                let is_original = export && self.docs[self.cur].source.as_ref() == Some(&p);
                let mut choice = 0;
                win("Replace file?").show(ctx, |ui| {
                    if is_original {
                        ui.label(format!("\"{}\" is the original picture you opened. Replace it with this version?", io::file_name(&p)));
                    } else {
                        ui.label(format!("\"{}\" already exists. Replace it?", io::file_name(&p)));
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Replace").clicked() { choice = 1; }
                        if ui.button("Cancel").clicked() { choice = 2; }
                    });
                });
                if choice != 0 {
                    keep = false;
                    if choice == 1 {
                        let r = if export { self.export_copy(p) } else { self.save_project(p) };
                        if let Err(e) = r { self.dialog = Some(Dialog::Message(format!("Couldn't save: {e}"))); }
                    }
                }
            }
            Dialog::NewImage { w, h } => {
                win("New Image").show(ctx, |ui| {
                    egui::Grid::new("ng").show(ui, |ui| {
                        ui.label("Width:"); num_box(ui, w, 1..=20000, 1.0, " px"); ui.end_row();
                        ui.label("Height:"); num_box(ui, h, 1..=20000, 1.0, " px"); ui.end_row();
                    });
                    ui.label("The background uses the secondary colour.");
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            let n = self.docs.len() + 1;
                            let bg = self.secondary;
                            self.docs.push(Document::new(&format!("Untitled {n}"), *w as usize, *h as usize, bg));
                            self.cur = self.docs.len() - 1;
                            self.drag = None;
                            keep = false;
                        }
                        if ui.button("Cancel").clicked() { keep = false; }
                    });
                });
            }
            Dialog::Resize { w, h, keep: k } => {
                let (ow, oh) = (self.docs[self.cur].w() as f32, self.docs[self.cur].h() as f32);
                let mut ok = false;
                win("Resize Image").show(ctx, |ui| {
                    egui::Grid::new("rg").show(ui, |ui| {
                        ui.label("Width:"); if num_box(ui, w, 1..=20000, 1.0, " px").changed() && *k { *h = ((*w as f32) * oh / ow).round().max(1.0) as u32; } ui.end_row();
                        ui.label("Height:"); if num_box(ui, h, 1..=20000, 1.0, " px").changed() && *k { *w = ((*h as f32) * ow / oh).round().max(1.0) as u32; } ui.end_row();
                    });
                    ui.checkbox(k, "Keep aspect ratio");
                    ui.horizontal(|ui| {
                        for pct in [25, 50, 200] { if ui.small_button(format!("{pct}%")).clicked() { *w = ((ow * pct as f32 / 100.0) as u32).max(1); *h = ((oh * pct as f32 / 100.0) as u32).max(1); } }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() { ok = true; keep = false; }
                        if ui.button("Cancel").clicked() { keep = false; }
                    });
                });
                if ok {
                    let (nw, nh) = (*w as usize, *h as usize);
                    let d = self.doc();
                    let (w0, h0) = (d.w(), d.h());
                    let (fx, fy) = (nw as f32 / w0 as f32, nh as f32 / h0 as f32);
                    for l in d.s.layers.iter_mut() {
                        if let Some(s) = l.smart.as_mut() {
                            // smart objects are resized by their placement, from their full original
                            s.xf.cx *= fx; s.xf.cy *= fy; s.xf.sx *= fx; s.xf.sy *= fy;
                            l.rerender(nw, nh);
                        } else {
                            l.px = raster::resize(&l.px, w0, h0, nw, nh);
                        }
                    }
                    d.s.w = nw; d.s.h = nh; d.s.selection = None;
                    d.commit("Resize Image");
                    self.zoom_fit();
                }
            }
            Dialog::CanvasSize { w, h, anchor } => {
                let mut ok = false;
                win("Canvas Size").show(ctx, |ui| {
                    egui::Grid::new("cg").show(ui, |ui| {
                        ui.label("Width:"); num_box(ui, w, 1..=20000, 1.0, " px"); ui.end_row();
                        ui.label("Height:"); num_box(ui, h, 1..=20000, 1.0, " px"); ui.end_row();
                    });
                    ui.label("Anchor:");
                    egui::Grid::new("anchor").spacing(vec2(2.0, 2.0)).show(ui, |ui| {
                        for ay in 0..3 { for ax in 0..3 { if ui.selectable_label(*anchor == (ax, ay), if *anchor == (ax, ay) { "●" } else { "○" }).clicked() { *anchor = (ax, ay); } } ui.end_row(); }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() { ok = true; keep = false; }
                        if ui.button("Cancel").clicked() { keep = false; }
                    });
                });
                if ok {
                    let (nw, nh) = (*w as usize, *h as usize);
                    let an = *anchor;
                    let bg = self.secondary;
                    let d = self.doc();
                    let (w0, h0) = (d.w(), d.h());
                    let ox = (nw as i32 - w0 as i32) * an.0 / 2;
                    let oy = (nh as i32 - h0 as i32) * an.1 / 2;
                    for (i, l) in d.s.layers.iter_mut().enumerate() {
                        if let Some(s) = l.smart.as_mut() {
                            // the canvas moves over the smart object; hidden parts come back if it grows
                            s.xf.cx += ox as f32; s.xf.cy += oy as f32;
                            l.rerender(nw, nh);
                            continue;
                        }
                        let fill = if i == 0 && l.name == "Background" { bg } else { [0, 0, 0, 0] };
                        l.px = raster::recanvas(&l.px, w0, h0, nw, nh, ox, oy, fill);
                    }
                    d.s.w = nw; d.s.h = nh; d.s.selection = None;
                    d.commit("Canvas Size");
                    self.zoom_fit();
                }
            }
            Dialog::LayerProps { name, visible, opacity, blend, orig } => {
                let mut result = 0;
                win("Layer Properties").show(ctx, |ui| {
                    egui::Grid::new("lp").show(ui, |ui| {
                        ui.label("Name:"); ui.text_edit_singleline(name); ui.end_row();
                        ui.label("Visible:"); ui.checkbox(visible, ""); ui.end_row();
                        ui.label("Blend mode:");
                        egui::ComboBox::from_id_salt("bm").selected_text(blend.name()).show_ui(ui, |ui| { for m in BlendMode::ALL { ui.selectable_value(blend, m, m.name()); } });
                        ui.end_row();
                        ui.label("Opacity:"); ui.add(egui::Slider::new(opacity, 0..=255)); ui.end_row();
                    });
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() { result = 1; }
                        if ui.button("Cancel").clicked() { result = 2; }
                    });
                });
                let (n2, v2, o2, b2) = (name.clone(), *visible, *opacity, *blend);
                let orig = orig.clone();
                let d = self.doc();
                {
                    let l = d.layer_mut();
                    if l.visible != v2 || l.opacity != o2 || l.blend != b2 { l.visible = v2; l.opacity = o2; l.blend = b2; d.mark_full(); }
                }
                if result == 1 {
                    d.layer_mut().name = n2.clone();
                    if (n2, v2, o2, b2) != orig { d.commit("Layer Properties"); }
                    keep = false;
                } else if result == 2 {
                    let l = d.layer_mut();
                    l.visible = orig.1; l.opacity = orig.2; l.blend = orig.3;
                    d.mark_full();
                    keep = false;
                }
            }
            Dialog::Effect { fx, vals, snap, applied } => {
                let fx = *fx;
                let mut result = 0;
                win(fx.name()).show(ctx, |ui| {
                    egui::Grid::new("fxg").show(ui, |ui| {
                        for (i, p) in fx.params().iter().enumerate() {
                            ui.label(p.name);
                            let mut s = egui::Slider::new(&mut vals[i], p.min..=p.max);
                            if p.integer { s = s.integer(); }
                            ui.add(s);
                            ui.end_row();
                        }
                    });
                    if ui.small_button("Reset").clicked() { for (i, p) in fx.params().iter().enumerate() { vals[i] = p.default; } }
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() { result = 1; }
                        if ui.button("Cancel").clicked() { result = 2; }
                    });
                });
                if result != 2 && vals != applied {
                    // while a slider is held, refresh the preview at most every 150 ms; always on release or OK
                    let (now, held) = ctx.input(|i| (i.time, i.pointer.any_down()));
                    if result == 1 || !held || now - self.fx_last >= 0.15 {
                        let v = vals.clone();
                        self.run_effect(fx, &v, snap);
                        *applied = v;
                        self.fx_last = now;
                    } else {
                        ctx.request_repaint_after(std::time::Duration::from_millis(150));
                    }
                }
                if result == 1 { self.doc().commit(fx.name()); keep = false; }
                if result == 2 {
                    let s = snap.clone();
                    let d = self.doc();
                    d.layer_mut().px = s;
                    d.mark_full();
                    keep = false;
                }
            }
            Dialog::Confirm { doc, then } => {
                let name = self.docs.get(*doc).map(|d| d.name.clone()).unwrap_or_default();
                let (doc, then) = (*doc, *then);
                let mut result = 0;
                win("Unsaved changes").show(ctx, |ui| {
                    ui.label(format!("Save changes to \"{name}\" before closing?"));
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() { result = 1; }
                        if ui.button("Don't Save").clicked() { result = 2; }
                        if ui.button("Cancel").clicked() { result = 3; }
                    });
                });
                match result {
                    1 => { keep = false; self.cur = doc; self.save(); }
                    2 => {
                        keep = false;
                        match then {
                            AfterConfirm::CloseDoc => self.close_doc(doc, true),
                            AfterConfirm::Quit => { for d in self.docs.iter_mut() { d.dirty = false; } self.allow_quit = true; ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
                        }
                    }
                    3 => keep = false,
                    _ => {}
                }
            }
            Dialog::File { .. } => {
                keep = self.file_dialog(ctx, &mut dlg);
            }
        }
        if keep && self.dialog.is_none() { self.dialog = Some(dlg); }
    }

    /// Our own simple file browser. Returns false when it closes.
    fn file_dialog(&mut self, ctx: &egui::Context, dlg: &mut Dialog) -> bool {
        let mut keep = true;
        let mut refresh = false;
        let mut accept: Option<PathBuf> = None;
        let Dialog::File { mode, dir, name, ext, err, entries } = dlg else { return false };
        let title = match mode { FileMode::Open => "Open", FileMode::Save => "Save Project (.pxe)", FileMode::Export => "Export a Copy", FileMode::ImportLayer => "Import From File" };
        let is_save = matches!(mode, FileMode::Save | FileMode::Export);
        let is_export = matches!(mode, FileMode::Export);
        egui::Window::new(title).collapsible(false).resizable(true).default_size(vec2(520.0, 420.0)).anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("⬆ Up").clicked() { if let Some(p) = dir.parent() { *dir = p.to_path_buf(); refresh = true; } }
                if ui.button("Home").clicked() { if let Some(hm) = std::env::var_os("HOME") { *dir = PathBuf::from(hm); refresh = true; } }
                let mut s = dir.to_string_lossy().into_owned();
                if ui.add(egui::TextEdit::singleline(&mut s).desired_width(f32::INFINITY)).lost_focus() { let p = PathBuf::from(&s); if p.is_dir() { *dir = p; refresh = true; } }
            });
            ui.separator();
            egui::ScrollArea::vertical().max_height(260.0).auto_shrink([false, false]).show(ui, |ui| {
                for (n, is_dir) in entries.iter() {
                    let label = if *is_dir { format!("📁 {n}") } else { format!("🖼 {n}") };
                    let r = ui.selectable_label(!*is_dir && name == n, label);
                    if r.double_clicked() {
                        if *is_dir { *dir = dir.join(n); refresh = true; } else { accept = Some(dir.join(n)); }
                    } else if r.clicked() {
                        if *is_dir { *dir = dir.join(n); refresh = true; } else { *name = n.clone(); }
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("File name:");
                let r = ui.add(egui::TextEdit::singleline(name).desired_width(240.0));
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !name.is_empty() { accept = Some(dir.join(name.as_str())); }
                if is_export {
                    egui::ComboBox::from_id_salt("ext").selected_text(io::EXPORT_EXTS[*ext].to_uppercase()).show_ui(ui, |ui| {
                        for (i, e) in io::EXPORT_EXTS.iter().enumerate() {
                            ui.selectable_value(ext, i, e.to_uppercase());
                        }
                    });
                } else if is_save {
                    ui.label(".pxe (keeps layers)");
                }
            });
            if is_export { ui.label("Writes a separate flat picture. Your project and the original stay as they are."); }
            if !err.is_empty() { ui.colored_label(Color32::from_rgb(190, 30, 30), err.as_str()); }
            ui.horizontal(|ui| {
                if ui.button(if is_save { "Save" } else { "Open" }).clicked() && !name.is_empty() { accept = Some(dir.join(name.as_str())); }
                if ui.button("Cancel").clicked() { keep = false; }
            });
        });
        if let Some(mut p) = accept {
            match mode {
                FileMode::Save | FileMode::Export => {
                    let export = matches!(mode, FileMode::Export);
                    let want = if export { io::EXPORT_EXTS[*ext] } else { "pxe" };
                    let has = p.extension().map(|e| e.to_string_lossy().to_lowercase());
                    if has.as_deref() != Some(want) && !(want == "jpg" && has.as_deref() == Some("jpeg")) { let mut s = p.into_os_string(); s.push(format!(".{want}")); p = PathBuf::from(s); }
                    let own_project = !export && self.docs[self.cur].path.as_ref() == Some(&p);
                    if p.exists() && !own_project {
                        // never replace an existing file without asking
                        keep = false;
                        self.dialog = Some(Dialog::Overwrite { path: p, export });
                    } else {
                        let r = if export { self.export_copy(p) } else { self.save_project(p) };
                        match r { Ok(()) => keep = false, Err(e) => *err = format!("Couldn't save: {e}") }
                    }
                }
                FileMode::Open => {
                    if p.is_dir() { *dir = p; refresh = true; } else { keep = false; self.open_path(p); }
                }
                FileMode::ImportLayer => {
                    match self.import_layer(&p) { Ok(()) => keep = false, Err(e) => *err = format!("Couldn't open: {e}") }
                }
            }
        }
        if refresh { Self::refresh_entries(dlg); }
        keep
    }
}
