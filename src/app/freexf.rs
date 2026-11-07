//! Free transform: move, resize and rotate smart objects and selected pixels with handles.
//!
//! - On a smart object layer the placement changes directly (nothing is lost; the canvas only masks it).
//! - On an ordinary layer the selected pixels (or the whole layer when nothing is selected) are lifted into a
//!   floating picture. It stays floating until it's deselected / finished (Enter, another tool, any other
//!   edit). If part of it then lies outside the canvas, it becomes a smart object layer instead of being cut.

use super::*;
use crate::smart::{self, Grip, Smart, Xform};

/// Selected pixels lifted off their layer while they're being transformed.
pub(super) struct Floating {
    pub doc: usize,
    pub layer: usize,
    pub src: Vec<Px>,
    pub sw: usize,
    pub sh: usize,
    /// the selection's shape, same size as `src` (0..255)
    pub mask_src: Vec<u8>,
    pub xf: Xform,
    pub xf0: Xform,
    /// the layer with the lifted pixels taken out
    pub base: Vec<Px>,
    pub before_px: Vec<Px>,
    pub sel_before: Option<Vec<u8>>,
    /// document version after our last change, to notice edits made by anything else
    pub ver: u64,
    /// a picture brought in from outside (paste, drop, import): it's put down even if never moved
    pub pasted: bool,
    /// a layer was added for it (removed again if it's cancelled)
    pub added_layer: bool,
    /// name of the history step when it's put down
    pub label: String,
    /// every placement it has had (one per finished drag), for undo/redo while it floats
    pub steps: Vec<Xform>,
    pub step: usize,
}

pub(super) struct XDrag {
    pub grip: Grip,
    pub start: (f32, f32),
    pub xf0: Xform,
    pub sw: usize,
    pub sh: usize,
}

impl App {
    /// What the transform box applies to right now: (placement, source width, source height).
    pub(super) fn xform_target(&self) -> Option<(Xform, usize, usize)> {
        if let Some(f) = &self.floating {
            if f.doc == self.cur {
                return Some((f.xf, f.sw, f.sh));
            }
        }
        let d = &self.docs[self.cur];
        d.layer().smart.as_ref().map(|s| (s.xf, s.sw, s.sh))
    }

    /// Lift the selection (or the whole layer's content) of the active ordinary layer into a floating picture.
    pub(super) fn lift_selection(&mut self) -> bool {
        if self.floating.is_some() || self.docs[self.cur].layer().smart.is_some() {
            return false;
        }
        let d = &mut self.docs[self.cur];
        let (w, h) = (d.w(), d.h());
        let a = d.s.active;
        let px = &d.s.layers[a].px;
        let sel = d.s.selection.clone();
        let bounds = match &sel {
            Some(_) => d.sel_bounds(),
            None => {
                // the layer's visible content
                let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
                for y in 0..h {
                    let row = &px[y * w..(y + 1) * w];
                    if let Some(f) = row.iter().position(|p| p[3] > 0) {
                        let l = row.iter().rposition(|p| p[3] > 0).unwrap();
                        x0 = x0.min(f); x1 = x1.max(l + 1); y0 = y0.min(y); y1 = y1.max(y + 1);
                    }
                }
                (x0 < x1).then_some((x0, y0, x1, y1))
            }
        };
        let Some((x0, y0, x1, y1)) = bounds else { return false };
        let (sw, sh) = (x1 - x0, y1 - y0);
        let mut src = Vec::with_capacity(sw * sh);
        let mut mask_src = Vec::with_capacity(sw * sh);
        for y in y0..y1 {
            for x in x0..x1 {
                let i = y * w + x;
                let m = sel.as_ref().map(|s| s[i]).unwrap_or(255);
                let mut p = px[i];
                p[3] = (p[3] as u32 * m as u32 / 255) as u8;
                src.push(p);
                mask_src.push(m);
            }
        }
        let before_px = px.clone();
        let base: Vec<Px> = px.iter().enumerate().map(|(i, p)| {
            let m = sel.as_ref().map(|s| s[i]).unwrap_or(255);
            let (x, y) = (i % w, i / w);
            if x < x0 || x >= x1 || y < y0 || y >= y1 { return *p; }
            let mut o = *p;
            o[3] = (p[3] as u32 * (255 - m as u32) / 255) as u8;
            o
        }).collect();
        let xf = Xform::at(x0 as f32, y0 as f32, sw, sh);
        d.s.selection = None; // the transform box shows the selection while it floats
        d.sel_version += 1;
        self.floating = Some(Floating { doc: self.cur, layer: a, src, sw, sh, mask_src, xf, xf0: xf, base, before_px, sel_before: sel, ver: 0, pasted: false, added_layer: false, label: "Move Selected Pixels".into(), steps: vec![xf], step: 0 });
        self.render_floating();
        true
    }

    fn render_floating(&mut self) {
        let Some(f) = self.floating.as_mut() else { return };
        let d = &mut self.docs[f.doc];
        let (w, h) = (d.w(), d.h());
        let Some(layer) = d.s.layers.get_mut(f.layer) else { return };
        layer.px.copy_from_slice(&f.base);
        smart::draw(&mut layer.px, w, h, &f.src, f.sw, f.sh, &f.xf, None);
        d.mark_full();
        f.ver = d.version;
    }

    /// Put the floating pixels down for good: into the layer, or as a smart object if part of them is off the
    /// canvas (so nothing is cut off).
    pub(super) fn finish_floating(&mut self) {
        let Some(f) = self.floating.take() else { return };
        self.xdrag = None;
        let Some(d) = self.docs.get_mut(f.doc) else { return };
        if f.layer >= d.s.layers.len() {
            return;
        }
        if f.ver != d.version {
            // something else changed the picture meanwhile; keep what's there
            return;
        }
        if f.xf == f.xf0 && !f.pasted {
            d.s.layers[f.layer].px = f.before_px;
            d.s.selection = f.sel_before;
            d.touch();
            return;
        }
        let (w, h) = (d.w(), d.h());
        // the selection moves along with the pixels
        let mask_img: Vec<Px> = f.mask_src.iter().map(|m| [255, 255, 255, *m]).collect();
        let mut sel_px = vec![[0u8; 4]; w * h];
        smart::draw(&mut sel_px, w, h, &mask_img, f.sw, f.sh, &f.xf, None);
        let sel: Vec<u8> = sel_px.iter().map(|p| p[3]).collect();
        let new_sel = sel.iter().any(|v| *v > 0).then_some(sel);
        if smart::sticks_out(&f.src, f.sw, f.sh, &f.xf, w, h) {
            // keep the parts outside the canvas: it becomes a smart object above the layer
            d.s.layers[f.layer].px = f.base;
            let layer = Layer::smart("Floating pixels", w, h, Smart::new(f.src, f.sw, f.sh, f.xf));
            d.s.layers.insert(f.layer + 1, layer);
            d.s.active = f.layer + 1;
        }
        d.s.selection = new_sel;
        d.commit(&f.label);
    }

    /// Put the floating pixels back where they were (Escape, Undo).
    pub(super) fn cancel_floating(&mut self) -> bool {
        let Some(f) = self.floating.take() else { return false };
        self.xdrag = None;
        if let Some(d) = self.docs.get_mut(f.doc) {
            if f.layer < d.s.layers.len() {
                if f.added_layer && d.s.layers.len() > 1 {
                    // a paste into a new layer that's cancelled takes its layer away again
                    d.s.layers.remove(f.layer);
                    d.s.active = f.layer.saturating_sub(1).min(d.s.layers.len() - 1);
                } else {
                    d.s.layers[f.layer].px = f.before_px;
                }
                d.s.selection = f.sel_before;
                d.touch();
            }
        }
        true
    }

    /// Undo: while a picture floats, step back through its transforms first; the first step (the paste or
    /// lift itself) takes it away. Otherwise the picture's history.
    pub(super) fn undo(&mut self) {
        if let Some(f) = self.floating.as_mut() {
            if f.doc == self.cur {
                if f.step > 0 {
                    f.step -= 1;
                    let xf = f.steps[f.step];
                    self.apply_xform(xf);
                } else {
                    self.cancel_floating();
                }
                return;
            }
        }
        self.finish_floating();
        self.doc().undo();
    }

    /// Redo: while a picture floats, step forward through its transforms.
    pub(super) fn redo(&mut self) {
        if let Some(f) = self.floating.as_mut() {
            if f.doc == self.cur {
                if f.step + 1 < f.steps.len() {
                    f.step += 1;
                    let xf = f.steps[f.step];
                    self.apply_xform(xf);
                }
                return;
            }
        }
        self.doc().redo();
    }

    /// The floating picture's steps for the History panel: (names, current index).
    pub(super) fn floating_steps(&self) -> Option<(Vec<String>, usize)> {
        let f = self.floating.as_ref().filter(|f| f.doc == self.cur)?;
        let mut names = vec![f.label.clone()];
        names.extend((1..f.steps.len()).map(|_| "Free Transform".to_string()));
        Some((names, f.step))
    }

    /// Jump to one of the floating picture's steps (History panel).
    pub(super) fn floating_goto(&mut self, i: usize) {
        if let Some(f) = self.floating.as_mut() {
            if i < f.steps.len() {
                f.step = i;
                let xf = f.steps[i];
                self.apply_xform(xf);
            }
        }
    }

    /// Change the placement of what's being transformed.
    pub(super) fn apply_xform(&mut self, xf: Xform) {
        if let Some(f) = self.floating.as_mut() {
            f.xf = xf;
            self.render_floating();
            return;
        }
        let d = &mut self.docs[self.cur];
        let (w, h) = (d.w(), d.h());
        let l = d.layer_mut();
        if let Some(s) = l.smart.as_mut() {
            s.xf = xf;
            l.rerender(w, h);
            d.mark_full();
        }
    }

    /// Pointer pressed on the canvas with the Move Selected Pixels / free transform tool.
    pub(super) fn xform_press(&mut self, screen: (f32, f32), img: (f32, f32), secondary: bool, to_scr: &dyn Fn(f32, f32) -> (f32, f32)) {
        if self.xform_target().is_none() && !self.lift_selection() {
            return;
        }
        let Some((xf, sw, sh)) = self.xform_target() else { return };
        let grip = if secondary { Grip::Rotate } else { smart::hit(&xf, sw, sh, screen, to_scr, 7.0).unwrap_or(Grip::Move) };
        self.xdrag = Some(XDrag { grip, start: img, xf0: xf, sw, sh });
    }

    pub(super) fn xform_update(&mut self, img: (f32, f32), shift: bool) {
        let Some(x) = &self.xdrag else { return };
        let xf = smart::drag(x.grip, &x.xf0, x.sw, x.sh, x.start, img, shift);
        self.apply_xform(xf);
    }

    pub(super) fn xform_release(&mut self) {
        let Some(x) = self.xdrag.take() else { return };
        if let Some(f) = self.floating.as_mut() {
            // each finished drag is one undo step while the picture floats
            if f.steps.get(f.step) != Some(&f.xf) {
                f.steps.truncate(f.step + 1);
                f.steps.push(f.xf);
                f.step = f.steps.len() - 1;
            }
            return;
        }
        if self.floating.is_none() {
            let d = &mut self.docs[self.cur];
            let changed = d.layer().smart.as_ref().map(|s| s.xf != x.xf0).unwrap_or(false);
            if changed {
                d.commit("Free Transform");
            }
        }
    }

    /// Cursor for the grip under the pointer.
    pub(super) fn xform_cursor(&self, screen: (f32, f32), to_scr: &dyn Fn(f32, f32) -> (f32, f32)) -> egui::CursorIcon {
        if let Some(x) = &self.xdrag {
            return grip_cursor(x.grip);
        }
        match self.xform_target().and_then(|(xf, sw, sh)| smart::hit(&xf, sw, sh, screen, to_scr, 7.0)) {
            Some(g) => grip_cursor(g),
            None => egui::CursorIcon::Move,
        }
    }

    /// Draw the transform box with its handles.
    pub(super) fn draw_xform_box(&self, painter: &egui::Painter, to_scr: &dyn Fn(f32, f32) -> (f32, f32)) {
        let Some((xf, sw, sh)) = self.xform_target() else { return };
        let cs = xf.corners(sw, sh).map(|(x, y)| to_scr(x, y));
        let pts: Vec<Pos2> = cs.iter().chain(std::iter::once(&cs[0])).map(|p| pos2(p.0, p.1)).collect();
        let accent = self.theme().accent;
        painter.add(egui::Shape::line(pts.clone(), Stroke::new(1.0, Color32::WHITE)));
        painter.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.0, accent), 5.0, 3.0));
        let rh = smart::rotate_handle(&cs);
        let top = ((cs[0].0 + cs[1].0) / 2.0, (cs[0].1 + cs[1].1) / 2.0);
        painter.line_segment([pos2(top.0, top.1), pos2(rh.0, rh.1)], Stroke::new(1.0, accent));
        painter.circle(pos2(rh.0, rh.1), 5.0, Color32::WHITE, Stroke::new(1.5, accent));
        let mut handles: Vec<(f32, f32)> = cs.to_vec();
        for i in 0..4 {
            let (a, b) = (cs[i], cs[(i + 1) % 4]);
            handles.push(((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0));
        }
        for (x, y) in handles {
            let r = Rect::from_center_size(pos2(x, y), vec2(8.0, 8.0));
            painter.rect_filled(r, 1.0, Color32::WHITE);
            painter.rect_stroke(r, 1.0, Stroke::new(1.0, accent), egui::StrokeKind::Inside);
        }
    }

    /// Painting on a smart object needs it turned into ordinary pixels first. Returns true when it's
    /// a smart object (and asks the person).
    pub(super) fn smart_guard(&mut self) -> bool {
        if self.docs[self.cur].layer().smart.is_some() {
            self.dialog = Some(Dialog::Rasterize);
            true
        } else {
            false
        }
    }

    /// Turn the active smart object into ordinary pixels (what's outside the canvas is dropped).
    pub(super) fn rasterize_layer(&mut self) {
        self.finish_floating();
        let d = self.doc();
        if d.layer().smart.is_some() {
            d.layer_mut().smart = None;
            d.commit("Rasterize Layer");
        }
    }

    /// Bring in a picture from outside (paste, drop, import) as floating pixels with the transform handles on.
    /// It keeps its whole picture while it floats; when it's deselected it becomes ordinary pixels of the
    /// layer, and only what sticks out past the canvas is kept as a smart object (so nothing is cut off).
    pub(super) fn float_picture(&mut self, pic: clip::Picture, label: &str, at: (usize, usize), new_layer: bool) {
        self.finish_floating();
        // a smart object layer can't take loose pixels: paste above it
        let new_layer = new_layer || self.docs[self.cur].layer().smart.is_some();
        if new_layer {
            let d = self.doc();
            let n = d.s.layers.len() + 1;
            let l = Layer::new(&format!("Layer {n}"), d.w(), d.h(), [0, 0, 0, 0]);
            let at_index = d.s.active + 1;
            d.s.layers.insert(at_index, l);
            d.s.active = at_index;
        }
        let d = self.doc();
        let a = d.s.active;
        let before_px = d.s.layers[a].px.clone();
        let sel_before = d.s.selection.take();
        d.sel_version += 1;
        let xf = Xform::at(at.0 as f32, at.1 as f32, pic.w, pic.h);
        self.floating = Some(Floating {
            doc: self.cur,
            layer: a,
            mask_src: vec![255; pic.w * pic.h],
            src: pic.px,
            sw: pic.w,
            sh: pic.h,
            xf,
            xf0: xf,
            base: before_px.clone(),
            before_px,
            sel_before,
            ver: 0,
            pasted: true,
            added_layer: new_layer,
            label: label.to_string(),
            steps: vec![xf],
            step: 0,
        });
        self.render_floating();
        self.tool = Tool::MovePixels;
    }
}

fn grip_cursor(g: Grip) -> egui::CursorIcon {
    match g {
        Grip::Move => egui::CursorIcon::Move,
        Grip::Rotate => egui::CursorIcon::Grab,
        Grip::Corner(0) | Grip::Corner(2) => egui::CursorIcon::ResizeNwSe,
        Grip::Corner(_) => egui::CursorIcon::ResizeNeSw,
        Grip::Edge(0) | Grip::Edge(2) => egui::CursorIcon::ResizeVertical,
        Grip::Edge(_) => egui::CursorIcon::ResizeHorizontal,
    }
}
