//! Smart objects and free transform.
//!
//! A smart object keeps its whole original picture plus a placement (centre, scale, rotation). The canvas
//! only masks what is shown: parts outside it are kept and come back when the object moves or the canvas
//! grows. Smart objects exist only inside this app (saved in .pxe files); exports are flattened.

use crate::doc::Px;
use std::sync::Arc;

/// Where a picture sits on the canvas: its centre, its scale (negative = mirrored) and its rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xform {
    pub cx: f32,
    pub cy: f32,
    pub sx: f32,
    pub sy: f32,
    pub angle: f32, // radians, clockwise on screen
}

impl Xform {
    /// Unscaled, unrotated, with the picture's top-left at (x, y).
    pub fn at(x: f32, y: f32, w: usize, h: usize) -> Self {
        Xform { cx: x + w as f32 / 2.0, cy: y + h as f32 / 2.0, sx: 1.0, sy: 1.0, angle: 0.0 }
    }

    /// Canvas position of a point given relative to the picture's centre, in picture pixels.
    pub fn apply(&self, u: f32, v: f32) -> (f32, f32) {
        let (s, c) = self.angle.sin_cos();
        let (x, y) = (u * self.sx, v * self.sy);
        (self.cx + x * c - y * s, self.cy + x * s + y * c)
    }

    /// Picture point (relative to its centre) under a canvas point.
    pub fn invert(&self, x: f32, y: f32) -> (f32, f32) {
        let (s, c) = self.angle.sin_cos();
        let (dx, dy) = (x - self.cx, y - self.cy);
        let (rx, ry) = (dx * c + dy * s, -dx * s + dy * c);
        (rx / self.sx, ry / self.sy)
    }

    /// The four corners on the canvas: top-left, top-right, bottom-right, bottom-left.
    pub fn corners(&self, w: usize, h: usize) -> [(f32, f32); 4] {
        let (hw, hh) = (w as f32 / 2.0, h as f32 / 2.0);
        [self.apply(-hw, -hh), self.apply(hw, -hh), self.apply(hw, hh), self.apply(-hw, hh)]
    }

    /// No scaling or rotation, and on whole pixels: the picture can be copied exactly.
    fn is_plain(&self, w: usize, h: usize) -> Option<(i64, i64)> {
        if self.angle != 0.0 || self.sx != 1.0 || self.sy != 1.0 {
            return None;
        }
        let x0 = self.cx - w as f32 / 2.0;
        let y0 = self.cy - h as f32 / 2.0;
        ((x0 - x0.round()).abs() < 1e-4 && (y0 - y0.round()).abs() < 1e-4).then(|| (x0.round() as i64, y0.round() as i64))
    }
}

/// A smart object: the untouched source picture and where it is placed.
#[derive(Clone, Debug)]
pub struct Smart {
    pub src: Arc<Vec<Px>>,
    pub sw: usize,
    pub sh: usize,
    pub xf: Xform,
}

impl PartialEq for Smart {
    fn eq(&self, o: &Self) -> bool {
        Arc::ptr_eq(&self.src, &o.src) && self.sw == o.sw && self.sh == o.sh && self.xf == o.xf
    }
}

impl Smart {
    pub fn new(src: Vec<Px>, sw: usize, sh: usize, xf: Xform) -> Self {
        Smart { src: Arc::new(src), sw, sh, xf }
    }

    /// Draw it on an empty canvas of w x h.
    pub fn render(&self, w: usize, h: usize) -> Vec<Px> {
        let mut out = vec![[0u8; 4]; w * h];
        draw(&mut out, w, h, &self.src, self.sw, self.sh, &self.xf, None);
        out
    }

    /// Whether any visible part lies outside the canvas.
    #[cfg(test)]
    pub fn sticks_out(&self, w: usize, h: usize) -> bool {
        sticks_out(&self.src, self.sw, self.sh, &self.xf, w, h)
    }
}

/// Pixel of a straight-alpha picture, premultiplied, as floats; transparent outside it.
#[inline]
fn fetch(src: &[Px], sw: usize, sh: usize, x: i64, y: i64) -> [f32; 4] {
    if x < 0 || y < 0 || x >= sw as i64 || y >= sh as i64 {
        return [0.0; 4];
    }
    let p = src[y as usize * sw + x as usize];
    let a = p[3] as f32 / 255.0;
    [p[0] as f32 * a, p[1] as f32 * a, p[2] as f32 * a, p[3] as f32]
}

/// Bilinear sample (in premultiplied space, so edges don't darken) at picture coords (u, v) = pixel centres
/// at +0.5. When `smooth` is false, the nearest pixel is taken (for shrinking, area averaging is used).
fn sample(src: &[Px], sw: usize, sh: usize, u: f32, v: f32, footprint: f32) -> Px {
    let mut acc = [0f32; 4];
    if footprint > 1.5 {
        // shrinking: average a small grid of bilinear samples over the footprint
        let n = (footprint.ceil() as i32).clamp(2, 6);
        for j in 0..n {
            for i in 0..n {
                let ou = ((i as f32 + 0.5) / n as f32 - 0.5) * footprint;
                let ov = ((j as f32 + 0.5) / n as f32 - 0.5) * footprint;
                let s = bilinear(src, sw, sh, u + ou, v + ov);
                for c in 0..4 { acc[c] += s[c]; }
            }
        }
        let k = 1.0 / (n * n) as f32;
        for c in acc.iter_mut() { *c *= k; }
    } else {
        // the original's default Move tool quality is High Quality Cubic (see MECHANICS.md)
        acc = bicubic(src, sw, sh, u, v);
    }
    let a = acc[3];
    if a <= 0.5 {
        return [0, 0, 0, 0];
    }
    let k = 255.0 / a;
    [(acc[0] * k).round().clamp(0.0, 255.0) as u8, (acc[1] * k).round().clamp(0.0, 255.0) as u8, (acc[2] * k).round().clamp(0.0, 255.0) as u8, a.round().clamp(0.0, 255.0) as u8]
}

/// Catmull-Rom cubic sample (premultiplied), clamped so it never rings past the neighbouring values.
fn bicubic(src: &[Px], sw: usize, sh: usize, u: f32, v: f32) -> [f32; 4] {
    let (fu, fv) = (u - 0.5, v - 0.5);
    let (x0, y0) = (fu.floor(), fv.floor());
    let (tx, ty) = (fu - x0, fv - y0);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let w = |t: f32| -> [f32; 4] {
        let (t2, t3) = (t * t, t * t * t);
        [(-t3 + 2.0 * t2 - t) * 0.5, (3.0 * t3 - 5.0 * t2 + 2.0) * 0.5, (-3.0 * t3 + 4.0 * t2 + t) * 0.5, (t3 - t2) * 0.5]
    };
    let (wx, wy) = (w(tx), w(ty));
    let mut o = [0f32; 4];
    let (mut lo, mut hi) = ([f32::MAX; 4], [f32::MIN; 4]);
    for (j, wyj) in wy.iter().enumerate() {
        for (i, wxi) in wx.iter().enumerate() {
            let p = fetch(src, sw, sh, x0 - 1 + i as i64, y0 - 1 + j as i64);
            let k = wxi * wyj;
            for c in 0..4 { o[c] += p[c] * k; }
            if (i == 1 || i == 2) && (j == 1 || j == 2) {
                for c in 0..4 { lo[c] = lo[c].min(p[c]); hi[c] = hi[c].max(p[c]); }
            }
        }
    }
    for c in 0..4 { o[c] = o[c].clamp(lo[c], hi[c]); }
    // colour can't exceed alpha in premultiplied form
    for c in 0..3 { o[c] = o[c].min(o[3]); }
    o
}

fn bilinear(src: &[Px], sw: usize, sh: usize, u: f32, v: f32) -> [f32; 4] {
    let (fu, fv) = (u - 0.5, v - 0.5);
    let (x0, y0) = (fu.floor(), fv.floor());
    let (tx, ty) = (fu - x0, fv - y0);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let a = fetch(src, sw, sh, x0, y0);
    let b = fetch(src, sw, sh, x0 + 1, y0);
    let c = fetch(src, sw, sh, x0, y0 + 1);
    let d = fetch(src, sw, sh, x0 + 1, y0 + 1);
    let mut o = [0f32; 4];
    for i in 0..4 {
        let top = a[i] + (b[i] - a[i]) * tx;
        let bot = c[i] + (d[i] - c[i]) * tx;
        o[i] = top + (bot - top) * ty;
    }
    o
}

/// Canvas box (x0, y0, x1, y1) the placed picture can touch, clipped to the canvas.
pub fn canvas_box(sw: usize, sh: usize, xf: &Xform, w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let cs = xf.corners(sw, sh);
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in cs { x0 = x0.min(x); y0 = y0.min(y); x1 = x1.max(x); y1 = y1.max(y); }
    let x0 = (x0.floor() - 1.0).max(0.0) as usize;
    let y0 = (y0.floor() - 1.0).max(0.0) as usize;
    let x1 = ((x1.ceil() + 1.0).max(0.0) as usize).min(w);
    let y1 = ((y1.ceil() + 1.0).max(0.0) as usize).min(h);
    (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
}

/// Composite the placed picture over `out` (w x h). With `mask_out`, also write its coverage there.
pub fn draw(out: &mut [Px], w: usize, h: usize, src: &[Px], sw: usize, sh: usize, xf: &Xform, mask_out: Option<&mut [u8]>) {
    use rayon::prelude::*;
    if let Some((ox, oy)) = xf.is_plain(sw, sh) {
        // exact copy, no resampling
        if mask_out.is_none() {
            // rows in parallel: only the rows and columns that land on the canvas
            let (tx0, tx1) = (ox.max(0), (ox + sw as i64).min(w as i64));
            if tx0 >= tx1 { return; }
            out.par_chunks_mut(w).enumerate().for_each(|(ty, row)| {
                let y = ty as i64 - oy;
                if y < 0 || y >= sh as i64 { return; }
                let srow = &src[y as usize * sw..(y as usize + 1) * sw];
                for tx in tx0..tx1 {
                    let p = srow[(tx - ox) as usize];
                    if p[3] == 0 { continue; }
                    let o = &mut row[tx as usize];
                    *o = crate::doc::over(*o, p, 255);
                }
            });
            return;
        }
        let mut mask_out = mask_out;
        for y in 0..sh as i64 {
            let ty = y + oy;
            if ty < 0 || ty >= h as i64 { continue; }
            for x in 0..sw as i64 {
                let tx = x + ox;
                if tx < 0 || tx >= w as i64 { continue; }
                let p = src[y as usize * sw + x as usize];
                let i = ty as usize * w + tx as usize;
                out[i] = crate::doc::over(out[i], p, 255);
                if p[3] > 0 { if let Some(m) = mask_out.as_deref_mut() { m[i] = 255; } }
            }
        }
        return;
    }
    let Some((x0, y0, x1, y1)) = canvas_box(sw, sh, xf, w, h) else { return };
    let footprint = 1.0 / xf.sx.abs().min(xf.sy.abs()).max(1e-3);
    let (hw, hh) = (sw as f32 / 2.0, sh as f32 / 2.0);
    let rows: Vec<(usize, Vec<Px>)> = (y0..y1).into_par_iter().map(|y| {
        let mut row = Vec::with_capacity(x1 - x0);
        for x in x0..x1 {
            let (u, v) = xf.invert(x as f32 + 0.5, y as f32 + 0.5);
            let (u, v) = (u + hw, v + hh);
            if u < -1.0 || v < -1.0 || u > sw as f32 + 1.0 || v > sh as f32 + 1.0 {
                row.push([0, 0, 0, 0]);
            } else {
                row.push(sample(src, sw, sh, u, v, footprint));
            }
        }
        (y, row)
    }).collect();
    let mut mask_out = mask_out;
    for (y, row) in rows {
        for (k, p) in row.into_iter().enumerate() {
            if p[3] == 0 { continue; }
            let i = y * w + x0 + k;
            out[i] = crate::doc::over(out[i], p, 255);
            if let Some(m) = mask_out.as_deref_mut() { m[i] = m[i].max(255); }
        }
    }
}

/// Whether any non-transparent part of the placed picture falls outside a w x h canvas.
pub fn sticks_out(src: &[Px], sw: usize, sh: usize, xf: &Xform, w: usize, h: usize) -> bool {
    let cs = xf.corners(sw, sh);
    let inside = cs.iter().all(|(x, y)| *x >= -0.01 && *y >= -0.01 && *x <= w as f32 + 0.01 && *y <= h as f32 + 0.01);
    if inside {
        return false;
    }
    // the box sticks out; check whether anything visible is out there
    let (hw, hh) = (sw as f32 / 2.0, sh as f32 / 2.0);
    for y in 0..sh {
        for x in 0..sw {
            if src[y * sw + x][3] == 0 { continue; }
            let (cx, cy) = xf.apply(x as f32 + 0.5 - hw, y as f32 + 0.5 - hh);
            if cx < 0.0 || cy < 0.0 || cx >= w as f32 || cy >= h as f32 {
                return true;
            }
        }
    }
    false
}

/// Which part of the transform box the pointer is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grip {
    Move,
    /// corner: index 0..4 (TL, TR, BR, BL)
    Corner(usize),
    /// edge: 0 top, 1 right, 2 bottom, 3 left
    Edge(usize),
    Rotate,
}

/// Find the grip under a screen point. `to_scr` maps canvas to screen; `r` is the grab radius in pixels.
pub fn hit(xf: &Xform, sw: usize, sh: usize, p: (f32, f32), to_scr: impl Fn(f32, f32) -> (f32, f32), r: f32) -> Option<Grip> {
    let cs = xf.corners(sw, sh).map(|(x, y)| to_scr(x, y));
    let d = |a: (f32, f32)| ((a.0 - p.0).powi(2) + (a.1 - p.1).powi(2)).sqrt();
    let rh = rotate_handle(&cs);
    if d(rh) <= r + 2.0 {
        return Some(Grip::Rotate);
    }
    for (i, c) in cs.iter().enumerate() {
        if d(*c) <= r { return Some(Grip::Corner(i)); }
    }
    for i in 0..4 {
        let (a, b) = (cs[i], cs[(i + 1) % 4]);
        if d(((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)) <= r { return Some(Grip::Edge(i)); }
    }
    if point_in_quad(p, &cs) {
        return Some(Grip::Move);
    }
    None
}

/// Screen position of the rotate handle: above the middle of the top edge.
pub fn rotate_handle(cs: &[(f32, f32); 4]) -> (f32, f32) {
    let top = ((cs[0].0 + cs[1].0) / 2.0, (cs[0].1 + cs[1].1) / 2.0);
    let bot = ((cs[3].0 + cs[2].0) / 2.0, (cs[3].1 + cs[2].1) / 2.0);
    let (dx, dy) = (top.0 - bot.0, top.1 - bot.1);
    let len = (dx * dx + dy * dy).sqrt().max(1e-3);
    (top.0 + dx / len * 24.0, top.1 + dy / len * 24.0)
}

fn point_in_quad(p: (f32, f32), q: &[(f32, f32); 4]) -> bool {
    let mut sign = 0.0f32;
    for i in 0..4 {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        let cross = (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
        if cross.abs() < 1e-6 { continue; }
        if sign == 0.0 { sign = cross.signum(); } else if cross.signum() != sign { return false; }
    }
    true
}

/// New placement while dragging `grip` from `start` (canvas coords) to `now`, starting from `xf0`.
/// `keep_aspect`: corners keep the proportions; rotation snaps to 15 degrees.
pub fn drag(grip: Grip, xf0: &Xform, sw: usize, sh: usize, start: (f32, f32), now: (f32, f32), keep_aspect: bool) -> Xform {
    let mut xf = *xf0;
    match grip {
        Grip::Move => {
            xf.cx += now.0 - start.0;
            xf.cy += now.1 - start.1;
        }
        Grip::Rotate => {
            let a0 = (start.1 - xf0.cy).atan2(start.0 - xf0.cx);
            let a1 = (now.1 - xf0.cy).atan2(now.0 - xf0.cx);
            let mut a = xf0.angle + (a1 - a0);
            if keep_aspect {
                let step = std::f32::consts::PI / 12.0;
                a = (a / step).round() * step;
            }
            xf.angle = a;
        }
        Grip::Corner(_) | Grip::Edge(_) => {
            let (hw, hh) = (sw as f32 / 2.0, sh as f32 / 2.0);
            // the opposite corner/edge stays where it is
            let (ux, uy, ax, ay): (f32, f32, bool, bool) = match grip {
                Grip::Corner(0) => (-hw, -hh, true, true),
                Grip::Corner(1) => (hw, -hh, true, true),
                Grip::Corner(2) => (hw, hh, true, true),
                Grip::Corner(_) => (-hw, hh, true, true),
                Grip::Edge(0) => (0.0, -hh, false, true),
                Grip::Edge(1) => (hw, 0.0, true, false),
                Grip::Edge(2) => (0.0, hh, false, true),
                _ => (-hw, 0.0, true, false),
            };
            let anchor = xf0.apply(-ux, -uy);
            // pointer position in the picture's own (unrotated, unscaled-axis) frame relative to the anchor
            let (s, c) = xf0.angle.sin_cos();
            let (dx, dy) = (now.0 - anchor.0, now.1 - anchor.1);
            let (lx, ly) = (dx * c + dy * s, -dx * s + dy * c);
            let full_w = 2.0 * hw;
            let full_h = 2.0 * hh;
            let mut sx = if ax { lx / (ux.signum() * full_w) } else { xf0.sx };
            let mut sy = if ay { ly / (uy.signum() * full_h) } else { xf0.sy };
            // never collapse to nothing: at least one pixel wide/tall (dragging past the anchor mirrors it)
            let min_sx = 1.0 / full_w;
            let min_sy = 1.0 / full_h;
            if ax && sx.abs() < min_sx { sx = if sx < 0.0 { -min_sx } else { min_sx }; }
            if ay && sy.abs() < min_sy { sy = if sy < 0.0 { -min_sy } else { min_sy }; }
            if keep_aspect && ax && ay {
                // same proportions as before
                let r0 = xf0.sx.abs() / xf0.sy.abs().max(1e-6);
                let m = (sx.abs() / r0).max(sy.abs());
                sx = sx.signum() * m * r0;
                sy = sy.signum() * m;
            } else if keep_aspect {
                // an edge handle with Shift: the other side grows by the same factor, centred on the edge
                if ax {
                    let k = (sx / xf0.sx.abs().max(1e-6)).abs();
                    sy = xf0.sy * k;
                } else {
                    let k = (sy / xf0.sy.abs().max(1e-6)).abs();
                    sx = xf0.sx * k;
                }
            }
            xf.sx = sx;
            xf.sy = sy;
            // keep the anchor fixed
            let moved = xf.apply(-ux, -uy);
            xf.cx += anchor.0 - moved.0;
            xf.cy += anchor.1 - moved.1;
        }
    }
    xf
}

/// Mirror a placement horizontally / vertically across a canvas of width w / height h.
pub fn flip(xf: &mut Xform, w: usize, h: usize, horizontal: bool) {
    if horizontal {
        xf.cx = w as f32 - xf.cx;
        xf.sx = -xf.sx;
        xf.angle = -xf.angle;
    } else {
        xf.cy = h as f32 - xf.cy;
        xf.sy = -xf.sy;
        xf.angle = -xf.angle;
    }
}

/// Turn a placement 90 degrees clockwise with the canvas (old size w x h; new size h x w).
pub fn rot90(xf: &mut Xform, _w: usize, h: usize) {
    let (x, y) = (xf.cx, xf.cy);
    xf.cx = h as f32 - y;
    xf.cy = x;
    xf.angle += std::f32::consts::FRAC_PI_2;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_placement_copies_exactly_and_keeps_offcanvas_parts() {
        let src: Vec<Px> = (0..16).map(|i| [i as u8 * 10, 0, 0, 255]).collect();
        let s = Smart::new(src, 4, 4, Xform::at(-2.0, -2.0, 4, 4));
        let out = s.render(4, 4);
        assert_eq!(out[0], [100, 0, 0, 255]); // source (2,2)
        assert!(s.sticks_out(4, 4));
        // move it fully inside again: the hidden part is still there
        let mut s2 = s.clone();
        s2.xf = Xform::at(0.0, 0.0, 4, 4);
        assert_eq!(s2.render(4, 4)[0], [0, 0, 0, 255]);
        assert!(!s2.sticks_out(4, 4));
    }

    #[test]
    fn corner_drag_scales_and_keeps_opposite_corner() {
        let xf = Xform::at(10.0, 10.0, 20, 10);
        let n = drag(Grip::Corner(2), &xf, 20, 10, (30.0, 20.0), (50.0, 30.0), false);
        let c = n.corners(20, 10);
        assert!((c[0].0 - 10.0).abs() < 1e-3 && (c[0].1 - 10.0).abs() < 1e-3, "top-left stays");
        assert!((c[2].0 - 50.0).abs() < 1e-3 && (c[2].1 - 30.0).abs() < 1e-3, "bottom-right follows");
        let k = drag(Grip::Corner(2), &xf, 20, 10, (30.0, 20.0), (50.0, 22.0), true);
        assert!((k.sx / k.sy - 1.0).abs() < 1e-3, "Shift keeps proportions");
    }

    #[test]
    fn rotate_and_hit_test() {
        let xf = Xform::at(0.0, 0.0, 10, 10);
        let r = drag(Grip::Rotate, &xf, 10, 10, (10.0, 5.0), (5.0, 10.0), false);
        assert!((r.angle - std::f32::consts::FRAC_PI_2).abs() < 1e-3);
        let id = |x: f32, y: f32| (x, y);
        assert_eq!(hit(&xf, 10, 10, (5.0, 5.0), id, 3.0), Some(Grip::Move));
        assert_eq!(hit(&xf, 10, 10, (0.5, 0.5), id, 3.0), Some(Grip::Corner(0)));
        assert_eq!(hit(&xf, 10, 10, (50.0, 50.0), id, 3.0), None);
    }
}
