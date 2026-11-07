//! The tool list, its help text and simple vector icons.

use eframe::egui::{self, pos2, vec2, Color32, Painter, Rect, Stroke};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    MovePixels,
    MoveSelection,
    RectSelect,
    LassoSelect,
    EllipseSelect,
    MagicWand,
    Zoom,
    Pan,
    PaintBucket,
    Gradient,
    Paintbrush,
    Eraser,
    Pencil,
    ColorPicker,
    CloneStamp,
    Recolor,
    Text,
    Line,
    Rectangle,
    RoundedRect,
    Ellipse,
}

impl Tool {
    pub const ALL: [Tool; 21] = [
        Tool::MovePixels, Tool::MoveSelection, Tool::RectSelect, Tool::LassoSelect, Tool::EllipseSelect,
        Tool::MagicWand, Tool::Zoom, Tool::Pan, Tool::PaintBucket, Tool::Gradient, Tool::Paintbrush,
        Tool::Eraser, Tool::Pencil, Tool::ColorPicker, Tool::CloneStamp, Tool::Recolor, Tool::Text,
        Tool::Line, Tool::Rectangle, Tool::RoundedRect, Tool::Ellipse,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tool::MovePixels => "Move Selected Pixels",
            Tool::MoveSelection => "Move Selection",
            Tool::RectSelect => "Rectangle Select",
            Tool::LassoSelect => "Lasso Select",
            Tool::EllipseSelect => "Ellipse Select",
            Tool::MagicWand => "Magic Wand",
            Tool::Zoom => "Zoom",
            Tool::Pan => "Pan",
            Tool::PaintBucket => "Paint Bucket",
            Tool::Gradient => "Gradient",
            Tool::Paintbrush => "Paintbrush",
            Tool::Eraser => "Eraser",
            Tool::Pencil => "Pencil",
            Tool::ColorPicker => "Color Picker",
            Tool::CloneStamp => "Clone Stamp",
            Tool::Recolor => "Recolor",
            Tool::Text => "Text",
            Tool::Line => "Line",
            Tool::Rectangle => "Rectangle",
            Tool::RoundedRect => "Rounded Rectangle",
            Tool::Ellipse => "Ellipse",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Tool::MovePixels | Tool::MoveSelection => "M",
            Tool::RectSelect | Tool::LassoSelect | Tool::EllipseSelect | Tool::MagicWand => "S",
            Tool::Zoom => "Z",
            Tool::Pan => "H",
            Tool::PaintBucket | Tool::Gradient => "F",
            Tool::Paintbrush => "B",
            Tool::Eraser => "E",
            Tool::Pencil => "P",
            Tool::ColorPicker => "K",
            Tool::CloneStamp => "L",
            Tool::Recolor => "R",
            Tool::Text => "T",
            Tool::Line | Tool::Rectangle | Tool::RoundedRect | Tool::Ellipse => "O",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Tool::MovePixels => "Free transform: drag to move, drag the square handles to resize (Shift keeps proportions), drag the round handle or right-drag to rotate. Enter or Deselect puts floating pixels down.",
            Tool::MoveSelection => "Drag to move the selection outline without touching any pixels.",
            Tool::RectSelect => "Drag to select a rectangle. Hold Shift for a square. A plain click deselects.",
            Tool::LassoSelect => "Drag to draw around the area you want to select.",
            Tool::EllipseSelect => "Drag to select an ellipse. Hold Shift for a circle.",
            Tool::MagicWand => "Click to select an area of similar colour. Hold Shift to select that colour everywhere.",
            Tool::Zoom => "Left click to zoom in, right click to zoom out.",
            Tool::Pan => "Drag to move around the picture when zoomed in.",
            Tool::PaintBucket => "Left click fills with the primary colour, right click with the secondary. Hold Shift to fill everywhere.",
            Tool::Gradient => "Drag to draw a gradient from the primary to the secondary colour (right drag reverses). Shift snaps the angle.",
            Tool::Paintbrush => "Left drag paints with the primary colour, right drag with the secondary.",
            Tool::Eraser => "Drag to erase to transparency.",
            Tool::Pencil => "Draw one-pixel lines: left for primary, right for secondary colour.",
            Tool::ColorPicker => "Left click picks the primary colour, right click the secondary. Hold Ctrl to pick from the whole picture.",
            Tool::CloneStamp => "Ctrl+click to choose where to copy from, then drag to paint a copy of it.",
            Tool::Recolor => "Drag over pixels close to the secondary colour to turn them into the primary colour.",
            Tool::Text => "Click to place the text, type it in the box, then press Finish (or click elsewhere).",
            Tool::Line => "Drag to draw a straight line. Right drag uses the secondary colour. Shift snaps the angle.",
            Tool::Rectangle => "Drag to draw a rectangle (right drag swaps colours). Hold Shift for a square.",
            Tool::RoundedRect => "Drag to draw a rounded rectangle (right drag swaps colours). Hold Shift for a square.",
            Tool::Ellipse => "Drag to draw an ellipse (right drag swaps colours). Hold Shift for a circle.",
        }
    }

    pub fn is_shape(self) -> bool { matches!(self, Tool::Rectangle | Tool::RoundedRect | Tool::Ellipse) }
    pub fn is_select(self) -> bool { matches!(self, Tool::RectSelect | Tool::LassoSelect | Tool::EllipseSelect | Tool::MagicWand) }
    pub fn uses_brush_width(self) -> bool {
        matches!(self, Tool::Paintbrush | Tool::Eraser | Tool::CloneStamp | Tool::Recolor | Tool::Line | Tool::Rectangle | Tool::RoundedRect | Tool::Ellipse)
    }
}

/// Draw a small icon for a History step, so steps can be told apart at a glance: the tool that made it,
/// or a symbol for its kind (selection, layers, the whole picture, adjustments and effects, files).
pub fn history_icon(p: &Painter, r: Rect, name: &str, col: Color32) {
    // steps named after a tool get that tool's icon
    if let Some(t) = Tool::ALL.iter().find(|t| t.name() == name) {
        return icon(p, r, *t, col);
    }
    let by_tool = match name {
        "Select All" | "Deselect" | "Invert Selection" | "Paste Selection" => Some(Tool::RectSelect),
        "Paste" | "Paste into New Layer" | "Import From File" | "Free Transform" => Some(Tool::MovePixels),
        "Erase Selection" | "Cut" => Some(Tool::Eraser),
        "Fill Selection" => Some(Tool::PaintBucket),
        _ => None,
    };
    if let Some(t) = by_tool {
        return icon(p, r, t, col);
    }
    let c = r.center();
    let s = r.width().min(r.height()) / 24.0;
    let pt = |x: f32, y: f32| pos2(c.x + (x - 12.0) * s, c.y + (y - 12.0) * s);
    let st = Stroke::new(1.5 * s, col);
    let layer_words = ["Layer", "Flatten", "Merge"];
    let image_words = ["Resize", "Canvas", "Crop", "Flip", "Rotate", "Expand"];
    if layer_words.iter().any(|w| name.contains(w)) {
        // stacked sheets
        for (i, y) in [6.0, 11.0, 16.0].iter().enumerate() {
            let pts = vec![pt(12.0, y - 3.0), pt(20.0, *y), pt(12.0, y + 3.0), pt(4.0, *y)];
            let fill = if i == 0 { col.gamma_multiply(0.35) } else { Color32::TRANSPARENT };
            p.add(egui::Shape::convex_polygon(pts, fill, st));
        }
    } else if image_words.iter().any(|w| name.contains(w)) {
        // a picture frame with corner marks
        p.rect_stroke(Rect::from_min_max(pt(5.0, 5.0), pt(19.0, 19.0)), 1.0, st, egui::StrokeKind::Middle);
        p.add(egui::Shape::convex_polygon(vec![pt(6.0, 18.0), pt(11.0, 11.0), pt(15.0, 15.0), pt(18.0, 12.0), pt(18.0, 18.0)], col.gamma_multiply(0.6), Stroke::NONE));
    } else if name.contains("Open") || name.contains("New Image") {
        // a document
        p.add(egui::Shape::convex_polygon(vec![pt(6.0, 3.0), pt(15.0, 3.0), pt(19.0, 7.0), pt(19.0, 21.0), pt(6.0, 21.0)], Color32::TRANSPARENT, st));
        p.line_segment([pt(15.0, 3.0), pt(15.0, 7.0)], st);
        p.line_segment([pt(15.0, 7.0), pt(19.0, 7.0)], st);
    } else {
        // adjustments and effects: a sparkle
        p.line_segment([pt(12.0, 3.0), pt(12.0, 21.0)], st);
        p.line_segment([pt(3.0, 12.0), pt(21.0, 12.0)], st);
        p.line_segment([pt(6.0, 6.0), pt(18.0, 18.0)], Stroke::new(1.0 * s, col));
        p.line_segment([pt(18.0, 6.0), pt(6.0, 18.0)], Stroke::new(1.0 * s, col));
        p.circle_filled(c, 2.5 * s, col);
    }
}

/// Draw a small icon for a tool inside `r`.
pub fn icon(p: &Painter, r: Rect, tool: Tool, col: Color32) {
    let c = r.center();
    let s = r.width().min(r.height()) / 24.0;
    let pt = |x: f32, y: f32| pos2(c.x + (x - 12.0) * s, c.y + (y - 12.0) * s);
    let st = Stroke::new(1.6 * s, col);
    let thin = Stroke::new(1.0 * s, col);
    let dashed = |a: egui::Pos2, b: egui::Pos2| {
        p.extend(egui::Shape::dashed_line(&[a, b], thin, 2.0 * s, 2.0 * s));
    };
    match tool {
        Tool::MovePixels | Tool::MoveSelection => {
            p.line_segment([pt(12.0, 3.0), pt(12.0, 21.0)], st);
            p.line_segment([pt(3.0, 12.0), pt(21.0, 12.0)], st);
            for (a, b, d) in [((12.0, 3.0), (9.0, 6.0), (15.0, 6.0)), ((12.0, 21.0), (9.0, 18.0), (15.0, 18.0)), ((3.0, 12.0), (6.0, 9.0), (6.0, 15.0)), ((21.0, 12.0), (18.0, 9.0), (18.0, 15.0))] {
                p.add(egui::Shape::convex_polygon(vec![pt(a.0, a.1), pt(b.0, b.1), pt(d.0, d.1)], col, Stroke::NONE));
            }
            if tool == Tool::MoveSelection {
                let rr = Rect::from_min_max(pt(5.0, 5.0), pt(19.0, 19.0));
                for (a, b) in [(rr.left_top(), rr.right_top()), (rr.right_top(), rr.right_bottom()), (rr.right_bottom(), rr.left_bottom()), (rr.left_bottom(), rr.left_top())] { dashed(a, b); }
            }
        }
        Tool::RectSelect => {
            let rr = Rect::from_min_max(pt(4.0, 6.0), pt(20.0, 18.0));
            for (a, b) in [(rr.left_top(), rr.right_top()), (rr.right_top(), rr.right_bottom()), (rr.right_bottom(), rr.left_bottom()), (rr.left_bottom(), rr.left_top())] { dashed(a, b); }
        }
        Tool::EllipseSelect => {
            let pts: Vec<_> = (0..=24).map(|i| { let a = i as f32 / 24.0 * std::f32::consts::TAU; pt(12.0 + 8.0 * a.cos(), 12.0 + 6.0 * a.sin()) }).collect();
            p.extend(egui::Shape::dashed_line(&pts, thin, 2.0 * s, 2.0 * s));
        }
        Tool::LassoSelect => {
            let pts: Vec<_> = [(5.0, 10.0), (9.0, 5.0), (16.0, 5.0), (20.0, 9.0), (17.0, 14.0), (10.0, 15.0), (7.0, 18.0), (9.0, 21.0)].iter().map(|q| pt(q.0, q.1)).collect();
            p.extend(egui::Shape::dashed_line(&pts, thin, 2.0 * s, 2.0 * s));
        }
        Tool::MagicWand => {
            p.line_segment([pt(5.0, 20.0), pt(15.0, 10.0)], Stroke::new(2.4 * s, col));
            for (x, y) in [(18.0, 4.0), (20.0, 9.0), (13.0, 5.0)] {
                p.circle_filled(pt(x, y), 1.2 * s, Color32::from_rgb(230, 170, 30));
            }
        }
        Tool::Zoom => {
            p.circle_stroke(pt(10.0, 10.0), 6.0 * s, st);
            p.line_segment([pt(14.5, 14.5), pt(20.0, 20.0)], Stroke::new(2.6 * s, col));
        }
        Tool::Pan => {
            for (i, x) in [7.0, 10.0, 13.0, 16.0].iter().enumerate() {
                let top = if i == 0 || i == 3 { 8.0 } else { 5.0 };
                p.line_segment([pt(*x, top), pt(*x, 14.0)], Stroke::new(2.4 * s, col));
            }
            p.add(egui::Shape::convex_polygon(vec![pt(6.0, 13.0), pt(17.0, 13.0), pt(16.0, 19.0), pt(8.0, 20.0)], col, Stroke::NONE));
        }
        Tool::PaintBucket => {
            p.add(egui::Shape::convex_polygon(vec![pt(5.0, 11.0), pt(12.0, 4.0), pt(19.0, 11.0), pt(12.0, 18.0)], Color32::TRANSPARENT, st));
            p.circle_filled(pt(19.5, 17.0), 2.2 * s, Color32::from_rgb(40, 110, 220));
        }
        Tool::Gradient => {
            let rr = Rect::from_min_max(pt(4.0, 5.0), pt(20.0, 19.0));
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(rr.left_top(), col);
            mesh.colored_vertex(rr.right_top(), Color32::from_gray(128));
            mesh.colored_vertex(rr.right_bottom(), Color32::from_gray(128));
            mesh.colored_vertex(rr.left_bottom(), col);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            p.add(mesh);
            p.rect_stroke(rr, 0.0, thin, egui::StrokeKind::Inside);
        }
        Tool::Paintbrush => {
            p.line_segment([pt(19.0, 4.0), pt(11.0, 13.0)], Stroke::new(2.0 * s, Color32::from_rgb(150, 100, 50)));
            p.circle_filled(pt(9.0, 16.0), 3.2 * s, col);
            p.line_segment([pt(7.0, 18.0), pt(4.0, 21.0)], Stroke::new(2.0 * s, col));
        }
        Tool::Eraser => {
            p.add(egui::Shape::convex_polygon(vec![pt(4.0, 15.0), pt(13.0, 6.0), pt(20.0, 13.0), pt(11.0, 22.0)], Color32::from_rgb(240, 150, 170), st));
            p.line_segment([pt(8.0, 11.0), pt(15.0, 18.0)], thin);
        }
        Tool::Pencil => {
            p.add(egui::Shape::convex_polygon(vec![pt(17.0, 3.0), pt(21.0, 7.0), pt(8.0, 20.0), pt(4.0, 16.0)], Color32::from_rgb(240, 200, 60), thin));
            p.add(egui::Shape::convex_polygon(vec![pt(4.0, 16.0), pt(8.0, 20.0), pt(3.0, 21.0)], col, Stroke::NONE));
        }
        Tool::ColorPicker => {
            p.line_segment([pt(5.0, 19.0), pt(14.0, 10.0)], Stroke::new(2.2 * s, col));
            p.circle_filled(pt(17.0, 7.0), 3.5 * s, col);
        }
        Tool::CloneStamp => {
            p.rect_filled(Rect::from_min_max(pt(5.0, 16.0), pt(19.0, 20.0)), 1.0, col);
            p.rect_filled(Rect::from_min_max(pt(10.0, 6.0), pt(14.0, 16.0)), 0.0, col);
            p.circle_filled(pt(12.0, 6.0), 3.5 * s, col);
        }
        Tool::Recolor => {
            p.circle_filled(pt(9.0, 12.0), 5.0 * s, Color32::from_rgb(220, 60, 60));
            p.circle_filled(pt(15.0, 12.0), 5.0 * s, Color32::from_rgb(60, 120, 220));
        }
        Tool::Text => {
            p.text(c, egui::Align2::CENTER_CENTER, "T", egui::FontId::proportional(18.0 * s), col);
        }
        Tool::Line => {
            p.line_segment([pt(4.0, 19.0), pt(20.0, 5.0)], st);
        }
        Tool::Rectangle => {
            p.rect_stroke(Rect::from_min_max(pt(4.0, 6.0), pt(20.0, 18.0)), 0.0, st, egui::StrokeKind::Middle);
        }
        Tool::RoundedRect => {
            p.rect_stroke(Rect::from_min_max(pt(4.0, 6.0), pt(20.0, 18.0)), 4.0 * s, st, egui::StrokeKind::Middle);
        }
        Tool::Ellipse => {
            p.add(egui::Shape::ellipse_stroke(c, vec2(8.0 * s, 6.0 * s), st));
        }
    }
}
