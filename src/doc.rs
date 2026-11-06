//! The picture being edited: layers, selection, blend modes and undo history.

pub type Px = [u8; 4]; // straight (non-premultiplied) RGBA

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlendMode {
    Normal,
    Multiply,
    ColorBurn,
    ColorDodge,
    Reflect,
    Glow,
    Overlay,
    Difference,
    Negation,
    Lighten,
    Darken,
    Screen,
    Xor,
    Additive,
}

impl BlendMode {
    pub const ALL: [BlendMode; 14] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Additive,
        BlendMode::ColorBurn,
        BlendMode::ColorDodge,
        BlendMode::Reflect,
        BlendMode::Glow,
        BlendMode::Overlay,
        BlendMode::Difference,
        BlendMode::Negation,
        BlendMode::Lighten,
        BlendMode::Darken,
        BlendMode::Screen,
        BlendMode::Xor,
    ];
    pub fn name(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Multiply => "Multiply",
            BlendMode::Additive => "Additive",
            BlendMode::ColorBurn => "Color Burn",
            BlendMode::ColorDodge => "Color Dodge",
            BlendMode::Reflect => "Reflect",
            BlendMode::Glow => "Glow",
            BlendMode::Overlay => "Overlay",
            BlendMode::Difference => "Difference",
            BlendMode::Negation => "Negation",
            BlendMode::Lighten => "Lighten",
            BlendMode::Darken => "Darken",
            BlendMode::Screen => "Screen",
            BlendMode::Xor => "Xor",
        }
    }
    pub fn to_u8(self) -> u8 {
        Self::ALL.iter().position(|m| *m == self).unwrap_or(0) as u8
    }
    pub fn from_u8(v: u8) -> Self {
        *Self::ALL.get(v as usize).unwrap_or(&BlendMode::Normal)
    }

    /// Blend one colour channel (0..255). `b` = below, `t` = top.
    /// One colour channel, `b` = below (dst), `t` = top (src). Each formula is the original's
    /// `CompositionOps/<Mode>.IBlend.BlendComponent` (PaintDotNet.Core.dll, see MECHANICS.md).
    fn channel(self, b: i32, t: i32) -> i32 {
        let r = match self {
            BlendMode::Normal => t,
            BlendMode::Multiply => fast_scale(b, t),
            BlendMode::Additive => (b + t).min(255),
            BlendMode::ColorBurn => {
                if t == 0 { 0 } else { (255 - (((255 - b) * 255) & 0xffff) / t).max(0) }
            }
            BlendMode::ColorDodge => {
                if t == 255 { 255 } else { (((b * 255) & 0xffff) / (255 - t)).min(255) }
            }
            BlendMode::Reflect => reflect(b, t),
            BlendMode::Glow => reflect(t, b),
            BlendMode::Overlay => {
                if b < 128 { fast_scale((2 * b) & 0xff, t) } else { 255 - fast_scale((2 * (255 - b)) & 0xff, 255 - t) }
            }
            BlendMode::Difference => (t - b).abs(),
            BlendMode::Negation => 255 - (255 - b - t).abs(),
            BlendMode::Lighten => b.max(t),
            BlendMode::Darken => b.min(t),
            BlendMode::Screen => t + b - fast_scale(t, b),
            BlendMode::Xor => b ^ t,
        };
        r.clamp(0, 255)
    }

    /// Composite `top` over `bot` exactly like the original's `CompositionOps.OpImpl<T>.Apply`:
    /// below-only, top-only and overlap parts weighted by alpha, divided by the total alpha.
    /// Layer opacity first scales the top's alpha.
    pub fn blend(self, bot: Px, top: Px, opacity: u8) -> Px {
        let sa = if opacity == 255 { top[3] as i32 } else { fast_scale(top[3] as i32, opacity as i32) };
        if sa == 0 {
            return bot;
        }
        if sa == 255 && self == BlendMode::Normal {
            return top; // opaque top hides what is below (same result as the full formula)
        }
        let da = bot[3] as i32;
        let below_only = fast_scale(da, 255 - sa);
        let total = below_only + sa;
        if total == 0 {
            return [0, 0, 0, 0];
        }
        let overlap = fast_scale(da, sa);
        let top_only = sa - overlap;
        let mut out = [0u8; 4];
        for c in 0..3 {
            let d = bot[c] as i32;
            let s = top[c] as i32;
            let blended = self.channel(d, s);
            out[c] = ((d * below_only + s * top_only + blended * overlap) / total).clamp(0, 255) as u8;
        }
        out[3] = total.min(255) as u8;
        out
    }
}

/// The original's `ByteUtil.FastScale`: a*b/255 with rounding.
#[inline]
pub fn fast_scale(a: i32, b: i32) -> i32 {
    let t = a * b + 128;
    ((t >> 8) + t) >> 8
}

#[inline]
fn reflect(d: i32, s: i32) -> i32 {
    if s == 255 { 255 } else { ((d * d) / (255 - s)).min(255) }
}

#[derive(Clone)]
pub struct Layer {
    pub name: String,
    pub visible: bool,
    pub opacity: u8,
    pub blend: BlendMode,
    pub px: Vec<Px>,
}

impl Layer {
    pub fn new(name: &str, w: usize, h: usize, fill: Px) -> Self {
        Layer { name: name.into(), visible: true, opacity: 255, blend: BlendMode::Normal, px: vec![fill; w * h] }
    }
}

/// Everything undo needs to restore.
#[derive(Clone)]
pub struct State {
    pub w: usize,
    pub h: usize,
    pub layers: Vec<Layer>,
    pub active: usize,
    pub selection: Option<Vec<u8>>,
}

/// A saved step. Layers and the selection are shared with the neighbouring step when they did not
/// change, so a brush stroke on one layer only stores that one layer.
#[derive(Clone)]
pub struct Snapshot {
    pub w: usize,
    pub h: usize,
    pub layers: Vec<std::sync::Arc<Layer>>,
    pub active: usize,
    pub selection: Option<std::sync::Arc<Vec<u8>>>,
}

impl Snapshot {
    /// Store `s`, reusing every layer/selection that equals the one in `prev`.
    fn of(s: &State, prev: Option<&Snapshot>) -> (Snapshot, usize) {
        use std::sync::Arc;
        let mut new_bytes = 0;
        let same_size = prev.map(|p| p.w == s.w && p.h == s.h).unwrap_or(false);
        let layers = s.layers.iter().enumerate().map(|(i, l)| {
            if same_size {
                if let Some(old) = prev.and_then(|p| p.layers.get(i)) {
                    if old.px == l.px && old.name == l.name && old.visible == l.visible && old.opacity == l.opacity && old.blend == l.blend {
                        return Arc::clone(old);
                    }
                }
            }
            new_bytes += l.px.len() * 4;
            Arc::new(l.clone())
        }).collect();
        let selection = match (&s.selection, prev.and_then(|p| p.selection.as_ref())) {
            (Some(m), Some(old)) if same_size && **old == *m => Some(Arc::clone(old)),
            (Some(m), _) => { new_bytes += m.len(); Some(Arc::new(m.clone())) }
            (None, _) => None,
        };
        (Snapshot { w: s.w, h: s.h, layers, active: s.active, selection }, new_bytes)
    }

    fn to_state(&self) -> State {
        State {
            w: self.w,
            h: self.h,
            layers: self.layers.iter().map(|l| (**l).clone()).collect(),
            active: self.active,
            selection: self.selection.as_ref().map(|m| (**m).clone()),
        }
    }
}

pub struct HistoryItem {
    pub name: String,
    pub state: Snapshot,
    /// memory this step added on top of the step before it
    bytes: usize,
}

impl HistoryItem {
    pub fn first(name: &str, s: &State) -> Self {
        let (state, bytes) = Snapshot::of(s, None);
        HistoryItem { name: name.into(), state, bytes }
    }
}

/// The original keeps every step (HistoryStack.PushNewMemento, paintdotnet.dll 0x0600010e, adds without a
/// count limit and moves old steps out to disk). This rebuild keeps every step too, in memory, and only drops
/// the oldest ones if the steps' own data would pass this budget (estimated, to protect small PCs).
pub const HISTORY_MEMORY_BUDGET: usize = 1536 * 1024 * 1024;

pub struct Document {
    pub name: String,
    pub path: Option<std::path::PathBuf>,
    pub s: State,
    pub history: Vec<HistoryItem>,
    pub hist_pos: usize,
    pub dirty: bool,
    pub version: u64, // bumps when pixels change, to refresh the texture
    pub sel_version: u64,
    pub zoom: f32,
    pub pan: (f32, f32),
    pub fit_pending: bool,
    /// Area changed since the screen picture was last refreshed (None + `full_dirty` = everything).
    pub damage: Option<(usize, usize, usize, usize)>,
    pub full_dirty: bool,
    sel_bounds_cache: std::cell::Cell<(u64, Option<(usize, usize, usize, usize)>)>,
}

impl Document {
    pub fn new(name: &str, w: usize, h: usize, bg: Px) -> Self {
        let s = State { w, h, layers: vec![Layer::new("Background", w, h, bg)], active: 0, selection: None };
        let mut d = Document {
            name: name.into(),
            path: None,
            s,
            history: vec![],
            hist_pos: 0,
            dirty: false,
            version: 1,
            sel_version: 1,
            zoom: 1.0,
            pan: (0.0, 0.0),
            fit_pending: true,
            damage: None,
            full_dirty: true,
            sel_bounds_cache: std::cell::Cell::new((0, None)),
        };
        d.history.push(HistoryItem::first("New Image", &d.s));
        d
    }

    pub fn from_layers(name: &str, w: usize, h: usize, layers: Vec<Layer>, first: &str) -> Self {
        let s = State { w, h, layers, active: 0, selection: None };
        let mut d = Document { name: name.into(), path: None, s, history: vec![], hist_pos: 0, dirty: false, version: 1, sel_version: 1, zoom: 1.0, pan: (0.0, 0.0), fit_pending: true, damage: None, full_dirty: true, sel_bounds_cache: std::cell::Cell::new((0, None)) };
        d.history.push(HistoryItem::first(first, &d.s));
        d
    }

    pub fn w(&self) -> usize { self.s.w }
    pub fn h(&self) -> usize { self.s.h }
    pub fn layer(&self) -> &Layer { &self.s.layers[self.s.active] }
    pub fn layer_mut(&mut self) -> &mut Layer { let a = self.s.active; &mut self.s.layers[a] }

    pub fn touch(&mut self) { self.version += 1; self.sel_version += 1; self.full_dirty = true; }

    /// Everything on screen must be redrawn.
    pub fn mark_full(&mut self) { self.version += 1; self.full_dirty = true; }

    /// Only this box (exclusive end) changed.
    pub fn mark_rect(&mut self, r: (usize, usize, usize, usize)) {
        self.version += 1;
        self.damage = Some(match self.damage {
            Some(d) => (d.0.min(r.0), d.1.min(r.1), d.2.max(r.2), d.3.max(r.3)),
            None => r,
        });
    }

    /// Bounding box of the selection, cached until the selection changes.
    pub fn sel_bounds(&self) -> Option<(usize, usize, usize, usize)> {
        let (v, b) = self.sel_bounds_cache.get();
        if v == self.sel_version && v != 0 {
            return b;
        }
        let b = self.s.selection.as_ref().and_then(|m| {
            let (w, h) = (self.s.w, self.s.h);
            let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
            for y in 0..h {
                let row = &m[y * w..(y + 1) * w];
                if let Some(first) = row.iter().position(|v| *v > 0) {
                    let last = row.iter().rposition(|v| *v > 0).unwrap();
                    x0 = x0.min(first); x1 = x1.max(last + 1); y0 = y0.min(y); y1 = y1.max(y + 1);
                }
            }
            (x0 < x1).then_some((x0, y0, x1, y1))
        });
        self.sel_bounds_cache.set((self.sel_version, b));
        b
    }

    /// Composite only a box of the picture (row-major, (x1-x0)*(y1-y0) pixels).
    pub fn composite_rect(&self, r: (usize, usize, usize, usize)) -> Vec<Px> {
        let (x0, y0, x1, y1) = r;
        let (rw, w) = (x1 - x0, self.s.w);
        let mut out = vec![[0u8; 4]; rw * (y1 - y0)];
        use rayon::prelude::*;
        let layers = &self.s.layers;
        out.par_chunks_mut(rw.max(1)).enumerate().for_each(|(ry, row)| {
            let y = y0 + ry;
            for l in layers.iter().filter(|l| l.visible && l.opacity > 0) {
                let src = &l.px[y * w + x0..y * w + x1];
                for (o, p) in row.iter_mut().zip(src) { *o = l.blend.blend(*o, *p, l.opacity); }
            }
        });
        out
    }

    /// Record the current state as a finished step.
    pub fn commit(&mut self, name: &str) {
        self.history.truncate(self.hist_pos + 1);
        let (state, bytes) = Snapshot::of(&self.s, self.history.last().map(|h| &h.state));
        self.history.push(HistoryItem { name: name.into(), state, bytes });
        let mut total: usize = self.history.iter().map(|h| h.bytes).sum();
        while total > HISTORY_MEMORY_BUDGET && self.history.len() > 2 {
            self.history.remove(0);
            // the new first step now owns everything it shares
            let first = &mut self.history[0];
            first.bytes = first.state.layers.iter().map(|l| l.px.len() * 4).sum::<usize>() + first.state.selection.as_ref().map(|m| m.len()).unwrap_or(0);
            total = self.history.iter().map(|h| h.bytes).sum();
        }
        self.hist_pos = self.history.len() - 1;
        self.dirty = true;
        self.touch();
    }

    pub fn goto(&mut self, i: usize) {
        if i < self.history.len() {
            self.hist_pos = i;
            self.s = self.history[i].state.to_state();
            self.dirty = true;
            self.touch();
        }
    }
    pub fn can_undo(&self) -> bool { self.hist_pos > 0 }
    pub fn can_redo(&self) -> bool { self.hist_pos + 1 < self.history.len() }
    pub fn undo(&mut self) { if self.can_undo() { self.goto(self.hist_pos - 1) } }
    pub fn redo(&mut self) { if self.can_redo() { self.goto(self.hist_pos + 1) } }

    pub fn composite(&self) -> Vec<Px> {
        composite_layers(&self.s.layers, self.s.w * self.s.h)
    }

    pub fn sel_at(&self, i: usize) -> u8 {
        match &self.s.selection { Some(m) => m[i], None => 255 }
    }
}

pub fn composite_layers(layers: &[Layer], n: usize) -> Vec<Px> {
    use rayon::prelude::*;
    let mut out = vec![[0u8, 0, 0, 0]; n];
    const CHUNK: usize = 16 * 1024;
    out.par_chunks_mut(CHUNK).enumerate().for_each(|(ci, chunk)| {
        let start = ci * CHUNK;
        let len = chunk.len();
        for l in layers.iter().filter(|l| l.visible && l.opacity > 0) {
            for (o, p) in chunk.iter_mut().zip(l.px[start..start + len].iter()) {
                *o = l.blend.blend(*o, *p, l.opacity);
            }
        }
    });
    out
}

/// Normal "over" with a coverage (0..255) used by every painting tool.
#[inline]
pub fn over(dst: Px, src: Px, cov: u8) -> Px {
    BlendMode::Normal.blend(dst, src, cov)
}

pub fn intensity(p: Px) -> u8 {
    ((7471 * p[2] as u32 + 38470 * p[1] as u32 + 19595 * p[0] as u32) >> 16) as u8
}
