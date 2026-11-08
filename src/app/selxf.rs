//! Free transform of the selection outline itself (Move Selection tool): move, resize and rotate the
//! selection with the same handles as Move Selected Pixels, without touching any pixels.

use super::*;
use crate::smart::{self, Xform};

/// The selection, kept as the shape it had when first grabbed, plus its current placement. Transforming
/// always starts from that original shape, so repeated resizes and rotations don't blur it.
pub(super) struct SelXf {
    pub doc: usize,
    /// the selection's shape cropped to its box, as alpha (0..255)
    pub src: Vec<Px>,
    pub sw: usize,
    pub sh: usize,
    pub xf: Xform,
    /// selection version this belongs to; anything else changing the selection starts afresh
    pub ver: u64,
    /// outline of the shape around its own centre, to preview a drag without rebuilding the mask
    pub segs: Vec<((f32, f32), (f32, f32))>,
}

impl App {
    /// The selection's transform box, set up from the current selection when needed.
    pub(super) fn selxf_target(&mut self) -> Option<(Xform, usize, usize)> {
        let cur = self.cur;
        let d = &self.docs[cur];
        if let Some(s) = &self.selxf {
            if s.doc == cur && s.ver == d.sel_version {
                return Some((s.xf, s.sw, s.sh));
            }
        }
        self.selxf = None;
        let (x0, y0, x1, y1) = d.sel_bounds()?;
        let m = d.s.selection.as_ref()?;
        let w = d.w();
        let (sw, sh) = (x1 - x0, y1 - y0);
        let mut src = vec![[255u8, 255, 255, 0]; sw * sh];
        let mut crop = vec![0u8; sw * sh];
        for y in 0..sh {
            for x in 0..sw {
                let v = m[(y0 + y) * w + x0 + x];
                src[y * sw + x][3] = v;
                crop[y * sw + x] = v;
            }
        }
        let (hw, hh) = (sw as f32 / 2.0, sh as f32 / 2.0);
        let segs = raster::mask_outline(&crop, sw, sh).into_iter().map(|(a, b)| ((a.0 - hw, a.1 - hh), (b.0 - hw, b.1 - hh))).collect();
        let xf = Xform::at(x0 as f32, y0 as f32, sw, sh);
        self.selxf = Some(SelXf { doc: cur, src, sw, sh, xf, ver: d.sel_version, segs });
        Some((xf, sw, sh))
    }

    pub(super) fn selxf_press(&mut self, screen: (f32, f32), img: (f32, f32), secondary: bool, to_scr: &dyn Fn(f32, f32) -> (f32, f32)) {
        let Some((xf, sw, sh)) = self.selxf_target() else { return };
        let grip = if secondary { smart::Grip::Rotate } else { smart::hit(&xf, sw, sh, screen, to_scr, 7.0).unwrap_or(smart::Grip::Move) };
        self.seldrag = Some(freexf::XDrag { grip, start: img, xf0: xf, sw, sh });
    }

    pub(super) fn selxf_update(&mut self, img: (f32, f32), shift: bool) {
        let Some(x) = &self.seldrag else { return };
        let xf = smart::drag(x.grip, &x.xf0, x.sw, x.sh, x.start, img, shift);
        if let Some(s) = self.selxf.as_mut() { s.xf = xf; }
    }

    pub(super) fn selxf_release(&mut self) {
        let Some(x) = self.seldrag.take() else { return };
        let changed = self.selxf.as_ref().map(|s| s.xf != x.xf0).unwrap_or(false);
        if changed { self.selxf_apply("Move Selection"); }
    }

    /// Build the selection mask from the transformed shape and record it as one history step.
    pub(super) fn selxf_apply(&mut self, label: &str) {
        let Some(s) = &self.selxf else { return };
        let d = &mut self.docs[self.cur];
        let (w, h) = (d.w(), d.h());
        let mut out = vec![[0u8; 4]; w * h];
        smart::draw(&mut out, w, h, &s.src, s.sw, s.sh, &s.xf, None);
        // resampling leaves faint fringes on a hard edge: keep the edge crisp, partial values only where
        // the shape is really soft (ellipses, antialiased lassos)
        let hard = s.src.iter().all(|p| p[3] == 0 || p[3] == 255);
        let m: Vec<u8> = if hard {
            // a hard-edged selection (rectangle, magic wand) stays hard: a pixel is in when its centre is
            out.iter().map(|p| if p[3] >= 128 { 255 } else { 0 }).collect()
        } else {
            out.iter().map(|p| match p[3] { 0..=24 => 0, 231..=255 => 255, v => v }).collect()
        };
        d.s.selection = if m.iter().any(|v| *v > 0) { Some(m) } else { None };
        d.commit_quiet(label);
        let ver = d.sel_version;
        if let Some(s) = self.selxf.as_mut() { s.ver = ver; }
    }

    /// Whether the selection is being dragged right now (the normal outline is hidden meanwhile).
    pub(super) fn selxf_dragging(&self) -> bool {
        self.seldrag.is_some() || self.nudge_pending()
    }

    /// The selection's transformed outline (while dragging) and its handles.
    pub(super) fn draw_selxf(&mut self, painter: &egui::Painter, to_scr: &dyn Fn(f32, f32) -> (f32, f32)) {
        let Some((xf, sw, sh)) = self.selxf_target() else { return };
        if self.seldrag.is_some() || self.nudge_pending() {
            if let Some(s) = &self.selxf {
                if s.segs.len() <= 20000 {
                    let segs: Vec<egui::Shape> = s.segs.iter().map(|(a, b)| {
                        let (pa, pb) = (xf.apply(a.0, a.1), xf.apply(b.0, b.1));
                        let (pa, pb) = (to_scr(pa.0, pa.1), to_scr(pb.0, pb.1));
                        egui::Shape::line_segment([pos2(pa.0, pa.1), pos2(pb.0, pb.1)], Stroke::new(1.0, Color32::BLACK))
                    }).collect();
                    painter.extend(segs);
                }
            }
        }
        freexf::draw_box(painter, to_scr, &xf, sw, sh, self.theme().accent);
    }

    pub(super) fn selxf_cursor(&mut self, screen: (f32, f32), to_scr: &dyn Fn(f32, f32) -> (f32, f32)) -> egui::CursorIcon {
        if let Some(x) = &self.seldrag { return freexf::grip_cursor(x.grip); }
        match self.selxf_target().and_then(|(xf, sw, sh)| smart::hit(&xf, sw, sh, screen, to_scr, 7.0)) {
            Some(g) => freexf::grip_cursor(g),
            None => egui::CursorIcon::Move,
        }
    }

    /// Arrow keys, as the original's `TransformControl.ProcessArrowKey` (paintdotnet.dll 0x060020e6): each
    /// press moves 1 pixel, 10 with Ctrl. Presses in a row are one edit (`TransformControl.OnKeyDown`
    /// 0x060020e5), recorded once the keys have been left alone for `NUDGE_IDLE_MS`. With other tools the
    /// arrows scroll the view.
    pub(super) fn nudge(&mut self, dx: f32, dy: f32) {
        let tool = self.tool;
        if let Some(n) = &self.nudging { if n.tool != tool || n.doc != self.cur { self.flush_nudge(true); } }
        match tool {
            Tool::MovePixels => {
                if self.xdrag.is_some() { return; }
                if self.xform_target().is_none() && !self.lift_selection() { return; }
                let Some((xf, sw, sh)) = self.xform_target() else { return };
                if self.nudging.is_none() { self.nudging = Some(Nudge { tool, doc: self.cur, xf0: xf, sw, sh, last: std::time::Instant::now() }); }
                let mut nx = xf;
                nx.cx += dx;
                nx.cy += dy;
                self.apply_xform(nx);
            }
            Tool::MoveSelection => {
                if self.seldrag.is_some() { return; }
                let Some((xf, sw, sh)) = self.selxf_target() else { return };
                if self.nudging.is_none() { self.nudging = Some(Nudge { tool, doc: self.cur, xf0: xf, sw, sh, last: std::time::Instant::now() }); }
                if let Some(s) = self.selxf.as_mut() { s.xf.cx += dx; s.xf.cy += dy; }
            }
            _ => {
                // scroll the view
                let d = self.doc();
                d.pan.0 -= dx * 20.0;
                d.pan.1 -= dy * 20.0;
                return;
            }
        }
        if let Some(n) = self.nudging.as_mut() { n.last = std::time::Instant::now(); }
        // every press is a finished history step right away (the person's rule; the original gathers a
        // run of presses into one step)
        self.flush_nudge(true);
    }

    /// Record a finished run of arrow presses as one step (now, or once they've paused long enough).
    pub(super) fn flush_nudge(&mut self, now: bool) {
        let Some(n) = &self.nudging else { return };
        if !now && n.last.elapsed() < std::time::Duration::from_millis(NUDGE_IDLE_MS) { return; }
        let n = self.nudging.take().unwrap();
        if n.doc != self.cur { return; }
        match n.tool {
            Tool::MovePixels => {
                self.xdrag = Some(freexf::XDrag { grip: smart::Grip::Move, start: (0.0, 0.0), xf0: n.xf0, sw: n.sw, sh: n.sh });
                self.xform_release();
            }
            Tool::MoveSelection => {
                let moved = self.selxf.as_ref().map(|s| s.xf != n.xf0).unwrap_or(false);
                if moved { self.selxf_apply("Move Selection"); }
            }
            _ => {}
        }
    }

    pub(super) fn nudge_pending(&self) -> bool {
        self.nudging.as_ref().map(|n| n.tool == Tool::MoveSelection).unwrap_or(false)
    }
}

/// Keys left alone this long end a run of arrow presses: the original waits max(500 ms, 1.5 × the system
/// keyboard repeat delay) (`TransformControl.OnKeyDown`, paintdotnet.dll 0x060020e5).
pub const NUDGE_IDLE_MS: u64 = 500;

/// A run of arrow-key presses being gathered into one step.
pub(super) struct Nudge {
    pub tool: Tool,
    pub doc: usize,
    pub xf0: Xform,
    pub sw: usize,
    pub sh: usize,
    pub last: std::time::Instant,
}
