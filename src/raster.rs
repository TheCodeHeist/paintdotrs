//! Pixel drawing: coverage masks for brushes and shapes, flood fill, gradients, text, transforms.

use crate::doc::Px;

/// Distance between brush dabs as a fraction of the brush size: BrushSection "Spacing" 0.15
/// (paintdotnet.dll 0x06003574).
pub const BRUSH_SPACING: f32 = 0.15;
/// Brush hardness: BrushSection "Hardness" 0.75 (paintdotnet.dll 0x06003574). Solid inside this share of
/// the radius, then fading to the edge (the fade shape is estimated).
pub const BRUSH_HARDNESS: f32 = 0.75;

/// A per-pixel coverage buffer (0..255) over a window of the image, with a dirty box.
/// Brushes use a window the size of the image; shapes and lines only cover their own box,
/// so a preview frame never allocates a whole-picture buffer.
pub struct Coverage {
    pub w: usize,
    pub h: usize,
    /// window origin and size inside the image
    ox: usize,
    oy: usize,
    cw: usize,
    ch: usize,
    pub c: Vec<u8>,
    pub min: (i32, i32),
    pub max: (i32, i32),
    /// box touched since `take_recent` was last called
    rmin: (i32, i32),
    rmax: (i32, i32),
}

impl Coverage {
    pub fn new(w: usize, h: usize) -> Self {
        Self::with_window(w, h, 0, 0, w as i32, h as i32)
    }
    /// Coverage limited to the box (x0, y0)..(x1, y1) (exclusive end, clipped to the image).
    pub fn with_window(w: usize, h: usize, x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        let ox = x0.clamp(0, w as i32) as usize;
        let oy = y0.clamp(0, h as i32) as usize;
        let ex = x1.clamp(0, w as i32) as usize;
        let ey = y1.clamp(0, h as i32) as usize;
        let (cw, ch) = (ex.saturating_sub(ox), ey.saturating_sub(oy));
        Coverage { w, h, ox, oy, cw, ch, c: vec![0; cw * ch], min: (i32::MAX, i32::MAX), max: (i32::MIN, i32::MIN), rmin: (i32::MAX, i32::MAX), rmax: (i32::MIN, i32::MIN) }
    }
    fn grow(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        self.min.0 = self.min.0.min(x0);
        self.min.1 = self.min.1.min(y0);
        self.max.0 = self.max.0.max(x1);
        self.max.1 = self.max.1.max(y1);
        self.rmin.0 = self.rmin.0.min(x0);
        self.rmin.1 = self.rmin.1.min(y0);
        self.rmax.0 = self.rmax.0.max(x1);
        self.rmax.1 = self.rmax.1.max(y1);
    }

    /// The box touched since the last call (clipped), then forget it.
    pub fn take_recent(&mut self) -> Option<(usize, usize, usize, usize)> {
        let (a, b) = (self.rmin, self.rmax);
        self.rmin = (i32::MAX, i32::MAX);
        self.rmax = (i32::MIN, i32::MIN);
        if a.0 > b.0 {
            return None;
        }
        let x0 = (a.0.max(0) as usize).max(self.ox);
        let y0 = (a.1.max(0) as usize).max(self.oy);
        let x1 = ((b.0 + 1).max(0) as usize).min(self.ox + self.cw);
        let y1 = ((b.1 + 1).max(0) as usize).min(self.oy + self.ch);
        if x0 >= x1 || y0 >= y1 { None } else { Some((x0, y0, x1, y1)) }
    }
    #[inline]
    fn put(&mut self, x: i32, y: i32, v: u8) {
        let (lx, ly) = (x - self.ox as i32, y - self.oy as i32);
        if lx >= 0 && ly >= 0 && (lx as usize) < self.cw && (ly as usize) < self.ch {
            let i = ly as usize * self.cw + lx as usize;
            if v > self.c[i] {
                self.c[i] = v;
            }
        }
    }
    /// Coverage at an image pixel (0 outside the window).
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> u8 {
        if x < self.ox || y < self.oy { return 0; }
        let (lx, ly) = (x - self.ox, y - self.oy);
        if lx < self.cw && ly < self.ch { self.c[ly * self.cw + lx] } else { 0 }
    }
    /// The coverage as a whole-image mask.
    pub fn to_mask(&self) -> Vec<u8> {
        let mut m = vec![0u8; self.w * self.h];
        for ly in 0..self.ch {
            let row = (self.oy + ly) * self.w + self.ox;
            m[row..row + self.cw].copy_from_slice(&self.c[ly * self.cw..(ly + 1) * self.cw]);
        }
        m
    }
    /// Bounding box clipped to the image: (x0, y0, x1, y1) exclusive end.
    pub fn bounds(&self) -> Option<(usize, usize, usize, usize)> {
        if self.min.0 > self.max.0 {
            return None;
        }
        let x0 = self.min.0.max(0) as usize;
        let y0 = self.min.1.max(0) as usize;
        let x0 = x0.max(self.ox);
        let y0 = y0.max(self.oy);
        let x1 = ((self.max.0 + 1).max(0) as usize).min(self.w).min(self.ox + self.cw);
        let y1 = ((self.max.1 + 1).max(0) as usize).min(self.h).min(self.oy + self.ch);
        if x0 >= x1 || y0 >= y1 { None } else { Some((x0, y0, x1, y1)) }
    }

    /// A round dab of diameter `size` centred on (cx, cy).
    pub fn dab(&mut self, cx: f32, cy: f32, size: f32, aa: bool) {
        let r = (size / 2.0).max(0.5);
        let x0 = (cx - r - 1.0).floor() as i32;
        let x1 = (cx + r + 1.0).ceil() as i32;
        let y0 = (cy - r - 1.0).floor() as i32;
        let y1 = (cy + r + 1.0).ceil() as i32;
        self.grow(x0, y0, x1, y1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let d = (dx * dx + dy * dy).sqrt();
                let v = if aa {
                    let edge = (r - d + 0.5).clamp(0.0, 1.0);
                    let inner = r * BRUSH_HARDNESS;
                    let soft = if r >= 2.0 && d > inner { (1.0 - (d - inner) / (r - inner)).clamp(0.0, 1.0) } else { 1.0 };
                    (edge * soft * 255.0) as u8
                } else if d <= r { 255 } else { 0 };
                if v > 0 {
                    self.put(x, y, v);
                }
            }
        }
    }

    /// Dabs along a segment, spaced so the stroke looks continuous.
    pub fn stroke(&mut self, a: (f32, f32), b: (f32, f32), size: f32, aa: bool) {
        let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let step = (size * BRUSH_SPACING).max(0.5);
        let n = (len / step).ceil().max(1.0) as i32;
        for i in 0..=n {
            let t = i as f32 / n as f32;
            self.dab(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, size, aa);
        }
    }

    /// One-pixel hard line (pencil).
    pub fn pixel_line(&mut self, a: (i32, i32), b: (i32, i32)) {
        let (mut x, mut y) = a;
        let dx = (b.0 - a.0).abs();
        let dy = -(b.1 - a.1).abs();
        let sx = if a.0 < b.0 { 1 } else { -1 };
        let sy = if a.1 < b.1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.grow(x, y, x, y);
            self.put(x, y, 255);
            if x == b.0 && y == b.1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy { err += dy; x += sx; }
            if e2 <= dx { err += dx; y += sy; }
        }
    }

    /// Fill this coverage from an inside-test sampled on a grid (antialiased when `aa`).
    pub fn fill_fn(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, aa: bool, inside: impl Fn(f32, f32) -> bool) {
        self.grow(x0, y0, x1, y1);
        let n = if aa { 4 } else { 1 };
        let wy1 = (self.oy + self.ch) as i32 - 1;
        let wx1 = (self.ox + self.cw) as i32 - 1;
        for y in y0.max(self.oy as i32)..=y1.min(wy1) {
            for x in x0.max(self.ox as i32)..=x1.min(wx1) {
                let mut hits = 0;
                for sy in 0..n {
                    for sx in 0..n {
                        let px = x as f32 + (sx as f32 + 0.5) / n as f32;
                        let py = y as f32 + (sy as f32 + 0.5) / n as f32;
                        if inside(px, py) {
                            hits += 1;
                        }
                    }
                }
                if hits > 0 {
                    self.put(x, y, (hits * 255 / (n * n)) as u8);
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ShapeKind { Rectangle, RoundedRect, Ellipse }

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FillStyle { Outline, Fill, Both }

fn rounded_inside(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> bool {
    if px < x0 || px > x1 || py < y0 || py > y1 {
        return false;
    }
    let r = r.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0).max(0.0);
    let cx = px.clamp(x0 + r, x1 - r);
    let cy = py.clamp(y0 + r, y1 - r);
    (px - cx).powi(2) + (py - cy).powi(2) <= r * r
}

fn ellipse_inside(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32) -> bool {
    let rx = (x1 - x0) / 2.0;
    let ry = (y1 - y0) / 2.0;
    if rx <= 0.0 || ry <= 0.0 {
        return false;
    }
    let dx = (px - (x0 + rx)) / rx;
    let dy = (py - (y0 + ry)) / ry;
    dx * dx + dy * dy <= 1.0
}

pub fn shape_inside(kind: ShapeKind, px: f32, py: f32, r: (f32, f32, f32, f32), radius: f32) -> bool {
    match kind {
        ShapeKind::Rectangle => px >= r.0 && px <= r.2 && py >= r.1 && py <= r.3,
        ShapeKind::RoundedRect => rounded_inside(px, py, r.0, r.1, r.2, r.3, radius),
        ShapeKind::Ellipse => ellipse_inside(px, py, r.0, r.1, r.2, r.3),
    }
}

/// Outline and fill coverages for a shape in the box `r` (image coords, may be unordered).
pub fn shape_coverage(
    w: usize, h: usize, kind: ShapeKind, a: (f32, f32), b: (f32, f32), width: f32, radius: f32, aa: bool,
) -> (Coverage, Coverage) {
    let r = (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1));
    let half = width / 2.0;
    let outer = (r.0 - half, r.1 - half, r.2 + half, r.3 + half);
    let inner = (r.0 + half, r.1 + half, r.2 - half, r.3 - half);
    let bx = (outer.0.floor() as i32 - 1, outer.1.floor() as i32 - 1, outer.2.ceil() as i32 + 1, outer.3.ceil() as i32 + 1);
    let mut line = Coverage::with_window(w, h, bx.0, bx.1, bx.2 + 1, bx.3 + 1);
    line.fill_fn(bx.0, bx.1, bx.2, bx.3, aa, |x, y| {
        shape_inside(kind, x, y, outer, radius + half)
            && !(inner.2 > inner.0 && inner.3 > inner.1 && shape_inside(kind, x, y, inner, (radius - half).max(0.0)))
    });
    let mut fill = Coverage::with_window(w, h, bx.0, bx.1, bx.2 + 1, bx.3 + 1);
    fill.fill_fn(bx.0, bx.1, bx.2, bx.3, aa, |x, y| shape_inside(kind, x, y, r, radius));
    (line, fill)
}

/// Thick straight line with round caps.
pub fn line_coverage(w: usize, h: usize, a: (f32, f32), b: (f32, f32), width: f32, aa: bool) -> Coverage {
    let r = (width / 2.0).max(0.5);
    let x0 = (a.0.min(b.0) - r - 1.0).floor() as i32;
    let y0 = (a.1.min(b.1) - r - 1.0).floor() as i32;
    let x1 = (a.0.max(b.0) + r + 1.0).ceil() as i32;
    let y1 = (a.1.max(b.1) + r + 1.0).ceil() as i32;
    let mut c = Coverage::with_window(w, h, x0, y0, x1 + 1, y1 + 1);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = (dx * dx + dy * dy).max(1e-6);
    c.fill_fn(x0, y0, x1, y1, aa, |px, py| {
        let t = (((px - a.0) * dx + (py - a.1) * dy) / l2).clamp(0.0, 1.0);
        let qx = a.0 + dx * t - px;
        let qy = a.1 + dy * t - py;
        qx * qx + qy * qy <= r * r
    });
    c
}

/// Squared tolerance curve from the original: t = round(v*255); t*t/255.
pub fn tolerance_byte(v: f32) -> u32 {
    let t = (v * 255.0).round().clamp(0.0, 255.0) as u32;
    t * t / 255
}

/// The original's `FloodFillAlgorithm.GetDistanceStraight` (default "Straight" alpha mode):
/// 0 for identical colours, otherwise round(0.5 * 255 * |a - b|) over RGBA in 0..1, at least 1.
pub fn color_distance(a: Px, b: Px) -> u32 {
    if a == b {
        return 0;
    }
    let mut s = 0f32;
    for c in 0..4 {
        let d = (a[c] as f32 - b[c] as f32) / 255.0;
        s += d * d;
    }
    ((s.sqrt() * 0.5 * 255.0).round() as u32).clamp(1, 255)
}

/// Region of colours similar to the colour at (x, y); `global` ignores connectivity.
pub fn flood_mask(px: &[Px], w: usize, h: usize, x: usize, y: usize, tol: f32, global: bool) -> Vec<u8> {
    let seed = px[y * w + x];
    let tb = tolerance_byte(tol);
    let ok = |p: Px| color_distance(p, seed) <= tb;
    let mut m = vec![0u8; w * h];
    if global {
        for (i, p) in px.iter().enumerate() {
            if ok(*p) {
                m[i] = 255;
            }
        }
        return m;
    }
    let mut stack = vec![(x, y)];
    while let Some((sx, sy)) = stack.pop() {
        let row = sy * w;
        if m[row + sx] != 0 || !ok(px[row + sx]) {
            continue;
        }
        let mut l = sx;
        while l > 0 && m[row + l - 1] == 0 && ok(px[row + l - 1]) {
            l -= 1;
        }
        let mut r = sx;
        while r + 1 < w && m[row + r + 1] == 0 && ok(px[row + r + 1]) {
            r += 1;
        }
        for i in l..=r {
            m[row + i] = 255;
            if sy > 0 && m[row - w + i] == 0 && ok(px[row - w + i]) {
                stack.push((i, sy - 1));
            }
            if sy + 1 < h && m[row + w + i] == 0 && ok(px[row + w + i]) {
                stack.push((i, sy + 1));
            }
        }
    }
    m
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GradientKind { Linear, Reflected, Diamond, Radial, Conical }

impl GradientKind {
    pub const ALL: [GradientKind; 5] = [GradientKind::Linear, GradientKind::Reflected, GradientKind::Diamond, GradientKind::Radial, GradientKind::Conical];
    pub fn name(self) -> &'static str {
        match self {
            GradientKind::Linear => "Linear",
            GradientKind::Reflected => "Linear (Reflected)",
            GradientKind::Diamond => "Linear (Diamond)",
            GradientKind::Radial => "Radial",
            GradientKind::Conical => "Conical",
        }
    }
    /// Position 0..1 of pixel (x, y) for a drag from a to b.
    pub fn t(self, a: (f32, f32), b: (f32, f32), x: f32, y: f32) -> f32 {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt().max(1e-3);
        let (px, py) = (x - a.0, y - a.1);
        match self {
            GradientKind::Linear => ((px * dx + py * dy) / (len * len)).clamp(0.0, 1.0),
            GradientKind::Reflected => ((px * dx + py * dy) / (len * len)).abs().clamp(0.0, 1.0),
            GradientKind::Diamond => {
                let (ux, uy) = (dx / len, dy / len);
                let along = (px * ux + py * uy).abs();
                let across = (-px * uy + py * ux).abs();
                ((along + across) / len).clamp(0.0, 1.0)
            }
            GradientKind::Radial => ((px * px + py * py).sqrt() / len).clamp(0.0, 1.0),
            GradientKind::Conical => {
                let a0 = dy.atan2(dx);
                let a1 = py.atan2(px);
                let mut d = (a1 - a0).abs() / std::f32::consts::PI;
                if d > 1.0 { d = 2.0 - d; }
                d.clamp(0.0, 1.0)
            }
        }
    }
}

pub fn lerp_px(a: Px, b: Px, t: f32) -> Px {
    let mut o = [0u8; 4];
    for c in 0..4 {
        o[c] = (a[c] as f32 + (b[c] as f32 - a[c] as f32) * t).round().clamp(0.0, 255.0) as u8;
    }
    o
}

/// Text rendered as a coverage mask at (x, y) top-left, in the given font file (bytes, face index), or the
/// built-in font when there's none. `size` is in pixels.
pub fn text_coverage(w: usize, h: usize, text: &str, size: f32, x: f32, y: f32, font: Option<(&[u8], u32)>) -> Coverage {
    use ab_glyph::FontRef;
    let chosen = font.and_then(|(data, index)| FontRef::try_from_slice_and_index(data, index).ok());
    let font = chosen.unwrap_or_else(|| FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT).expect("font"));
    draw_text(&font, w, h, text, size, x, y)
}

fn draw_text(font: &ab_glyph::FontRef, w: usize, h: usize, text: &str, size: f32, x: f32, y: f32) -> Coverage {
    use ab_glyph::{Font, ScaleFont};
    let mut cov = Coverage::new(w, h);
    let sf = font.as_scaled(size.max(1.0));
    let mut caret_y = y + sf.ascent();
    for line in text.split('\n') {
        let mut caret_x = x;
        let mut prev = None;
        for ch in line.chars() {
            let gid = font.glyph_id(ch);
            if let Some(p) = prev {
                caret_x += sf.kern(p, gid);
            }
            let g = gid.with_scale_and_position(size, ab_glyph::point(caret_x, caret_y));
            caret_x += sf.h_advance(gid);
            prev = Some(gid);
            if let Some(og) = font.outline_glyph(g) {
                let bb = og.px_bounds();
                og.draw(|gx, gy, v| {
                    let px = bb.min.x as i32 + gx as i32;
                    let py = bb.min.y as i32 + gy as i32;
                    cov.grow(px, py, px, py);
                    cov.put(px, py, (v.clamp(0.0, 1.0) * 255.0) as u8);
                });
            }
        }
        caret_y += sf.height() + sf.line_gap();
    }
    cov
}

// ---- whole-image transforms ----

pub fn flip_h(px: &mut [Px], w: usize, h: usize) {
    for y in 0..h {
        px[y * w..(y + 1) * w].reverse();
    }
}
pub fn flip_v(px: &mut [Px], w: usize, h: usize) {
    for y in 0..h / 2 {
        for x in 0..w {
            px.swap(y * w + x, (h - 1 - y) * w + x);
        }
    }
}
/// Rotate 90° clockwise; returns new pixels (new size is h x w).
pub fn rot90(px: &[Px], w: usize, h: usize) -> Vec<Px> {
    let mut o = vec![[0u8; 4]; w * h];
    for y in 0..h {
        for x in 0..w {
            o[x * h + (h - 1 - y)] = px[y * w + x];
        }
    }
    o
}
pub fn rot180(px: &mut [Px]) { px.reverse(); }

pub fn resize(px: &[Px], w: usize, h: usize, nw: usize, nh: usize) -> Vec<Px> {
    let flat: Vec<u8> = px.iter().flatten().copied().collect();
    let img = image::RgbaImage::from_raw(w as u32, h as u32, flat).expect("size");
    let filter = if nw < w || nh < h { image::imageops::FilterType::Triangle } else { image::imageops::FilterType::CatmullRom };
    let r = image::imageops::resize(&img, nw as u32, nh as u32, filter);
    r.pixels().map(|p| p.0).collect()
}

/// Place `px` (w x h) into a new nw x nh buffer at offset (ox, oy), filling with `fill`.
pub fn recanvas(px: &[Px], w: usize, h: usize, nw: usize, nh: usize, ox: i32, oy: i32, fill: Px) -> Vec<Px> {
    let mut o = vec![fill; nw * nh];
    for y in 0..h as i32 {
        let ny = y + oy;
        if ny < 0 || ny >= nh as i32 { continue; }
        for x in 0..w as i32 {
            let nx = x + ox;
            if nx < 0 || nx >= nw as i32 { continue; }
            o[ny as usize * nw + nx as usize] = px[y as usize * w + x as usize];
        }
    }
    o
}

/// Boundary segments of a mask in image coordinates, merged into runs.
pub fn mask_outline(m: &[u8], w: usize, h: usize) -> Vec<((f32, f32), (f32, f32))> {
    let on = |x: i32, y: i32| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && m[y as usize * w + x as usize] >= 128;
    let mut segs = Vec::new();
    // horizontal edges
    for y in 0..=h as i32 {
        let mut start: Option<i32> = None;
        for x in 0..=w as i32 {
            let edge = x < w as i32 && on(x, y) != on(x, y - 1);
            match (edge, start) {
                (true, None) => start = Some(x),
                (false, Some(s)) => { segs.push(((s as f32, y as f32), (x as f32, y as f32))); start = None; }
                _ => {}
            }
        }
    }
    for x in 0..=w as i32 {
        let mut start: Option<i32> = None;
        for y in 0..=h as i32 {
            let edge = y < h as i32 && on(x, y) != on(x - 1, y);
            match (edge, start) {
                (true, None) => start = Some(y),
                (false, Some(s)) => { segs.push(((x as f32, s as f32), (x as f32, y as f32))); start = None; }
                _ => {}
            }
        }
    }
    segs
}

/// Polygon fill (even-odd) into a 0/255 mask, for the lasso.
pub fn polygon_mask(pts: &[(f32, f32)], w: usize, h: usize) -> Vec<u8> {
    let mut m = vec![0u8; w * h];
    if pts.len() < 3 {
        return m;
    }
    for y in 0..h {
        let fy = y as f32 + 0.5;
        let mut xs = Vec::new();
        for i in 0..pts.len() {
            let a = pts[i];
            let b = pts[(i + 1) % pts.len()];
            if (a.1 <= fy && b.1 > fy) || (b.1 <= fy && a.1 > fy) {
                xs.push(a.0 + (fy - a.1) / (b.1 - a.1) * (b.0 - a.0));
            }
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for pair in xs.chunks(2) {
            if pair.len() < 2 { break; }
            let x0 = (pair[0] - 0.5).ceil().max(0.0) as usize;
            let x1 = ((pair[1] - 0.5).floor() as i64).min(w as i64 - 1);
            if x1 < 0 { continue; }
            for x in x0..=(x1 as usize) {
                if x < w { m[y * w + x] = 255; }
            }
        }
    }
    m
}
