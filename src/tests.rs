//! Checks of the picture model, drawing and effects.

use crate::doc::{BlendMode, Document, Layer};
use crate::effects::{apply, Fx};
use crate::raster::{self, Coverage};

#[test]
fn undo_redo_and_history_limit() {
    let mut d = Document::new("t", 4, 4, [255, 255, 255, 255]);
    d.layer_mut().px[0] = [1, 2, 3, 255];
    d.commit("Paint");
    assert_eq!(d.history.len(), 2);
    d.undo();
    assert_eq!(d.layer().px[0], [255, 255, 255, 255]);
    d.redo();
    assert_eq!(d.layer().px[0], [1, 2, 3, 255]);
    for i in 0..100 { d.layer_mut().px[1] = [i as u8, 0, 0, 255]; d.commit("x"); }
    assert_eq!(d.history.len(), 102, "no step limit, like the original");
    // a new step after undo drops the redo branch
    d.undo();
    d.commit("branch");
    assert!(!d.can_redo());
}

#[test]
fn normal_blend_and_opacity() {
    let white = [255, 255, 255, 255];
    let red = [255, 0, 0, 255];
    assert_eq!(BlendMode::Normal.blend(white, red, 255), red);
    let half = BlendMode::Normal.blend(white, red, 128);
    assert!(half[1] > 120 && half[1] < 135);
    assert_eq!(BlendMode::Multiply.blend([200, 100, 50, 255], [255, 255, 255, 255], 255), [200, 100, 50, 255]);
    assert_eq!(BlendMode::Difference.blend(white, white, 255), [0, 0, 0, 255]);
    // over a transparent pixel the top colour shows unchanged in every mode
    for m in BlendMode::ALL { assert_eq!(m.blend([0, 0, 0, 0], [10, 20, 30, 255], 255), [10, 20, 30, 255]); }
}

#[test]
fn layers_composite_in_order() {
    let mut d = Document::new("t", 2, 1, [255, 255, 255, 255]);
    let mut top = Layer::new("L", 2, 1, [0, 0, 0, 0]);
    top.px[1] = [0, 0, 255, 255];
    d.s.layers.push(top);
    let c = d.composite();
    assert_eq!(c[0], [255, 255, 255, 255]);
    assert_eq!(c[1], [0, 0, 255, 255]);
    d.s.layers[1].visible = false;
    assert_eq!(d.composite()[1], [255, 255, 255, 255]);
}

#[test]
fn flood_fill_contiguous_and_global() {
    // two white regions split by a black column
    let w = 5;
    let h = 3;
    let mut px = vec![[255u8, 255, 255, 255]; w * h];
    for y in 0..h { px[y * w + 2] = [0, 0, 0, 255]; }
    let m = raster::flood_mask(&px, w, h, 0, 0, 0.5, false);
    assert_eq!(m.iter().filter(|v| **v > 0).count(), 6);
    let g = raster::flood_mask(&px, w, h, 0, 0, 0.5, true);
    assert_eq!(g.iter().filter(|v| **v > 0).count(), 12);
    // full tolerance takes everything
    let all = raster::flood_mask(&px, w, h, 0, 0, 1.0, false);
    assert_eq!(all.iter().filter(|v| **v > 0).count(), 15);
}

#[test]
fn tolerance_curve_matches_original() {
    assert_eq!(raster::tolerance_byte(0.0), 0);
    assert_eq!(raster::tolerance_byte(1.0), 255);
    assert_eq!(raster::tolerance_byte(0.5), 128 * 128 / 255);
}

#[test]
fn brightness_contrast_table() {
    let px = vec![[100u8, 100, 100, 255], [10, 200, 50, 128]];
    // zero settings change nothing
    assert_eq!(apply(Fx::BrightnessContrast, &px, 2, 1, &[0.0, 0.0], [0; 4], [0; 4]), px);
    // brightness raises a grey pixel by exactly that much
    let out = apply(Fx::BrightnessContrast, &px, 2, 1, &[20.0, 0.0], [0; 4], [0; 4]);
    assert_eq!(out[0], [120, 120, 120, 255]);
    assert_eq!(out[1][3], 128);
    // full contrast becomes a threshold on intensity
    let out = apply(Fx::BrightnessContrast, &px, 2, 1, &[0.0, 100.0], [0; 4], [0; 4]);
    assert_eq!(out[0], [0, 0, 0, 255]);
}

#[test]
fn effects_keep_size_and_basic_meaning() {
    let w = 8;
    let h = 6;
    let px: Vec<[u8; 4]> = (0..w * h).map(|i| [(i * 5) as u8, 100, 200, 255]).collect();
    for fx in Fx::ADJUSTMENTS.iter().copied().chain(Fx::EFFECTS.iter().flat_map(|(_, l)| l.iter().copied())) {
        let v: Vec<f32> = fx.params().iter().map(|p| p.default).collect();
        let out = apply(fx, &px, w, h, &v, [0, 0, 0, 255], [255, 255, 255, 255]);
        assert_eq!(out.len(), px.len(), "{}", fx.name());
    }
    let inv = apply(Fx::InvertColors, &px, w, h, &[], [0; 4], [0; 4]);
    assert_eq!(inv[0], [255, 155, 55, 255]);
    let bw = apply(Fx::BlackAndWhite, &px, w, h, &[], [0; 4], [0; 4]);
    assert!(bw.iter().all(|p| p[0] == p[1] && p[1] == p[2]));
}

#[test]
fn brush_and_shapes_cover_expected_pixels() {
    let mut c = Coverage::new(20, 20);
    c.dab(10.0, 10.0, 6.0, true);
    assert_eq!(c.c[10 * 20 + 10], 255);
    assert_eq!(c.c[0], 0);
    let (line, fill) = raster::shape_coverage(20, 20, raster::ShapeKind::Rectangle, (2.0, 2.0), (17.0, 17.0), 2.0, 0.0, false);
    assert_eq!(fill.c[10 * 20 + 10], 255);
    assert_eq!(line.c[10 * 20 + 10], 0);
    assert_eq!(line.c[2 * 20 + 10], 255);
}

#[test]
fn layered_file_round_trip() {
    let mut d = Document::new("t", 3, 2, [1, 2, 3, 255]);
    let mut l = Layer::new("Top", 3, 2, [9, 8, 7, 100]);
    l.blend = BlendMode::Screen;
    l.opacity = 77;
    d.s.layers.push(l);
    let path = std::env::temp_dir().join(format!("pxe-test-{}.pxe", std::process::id()));
    crate::io::save(&d, &path, 90).unwrap();
    let back = crate::io::open(&path).unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!(back.s.layers.len(), 2);
    assert_eq!(back.s.layers[1].name, "Top");
    assert_eq!(back.s.layers[1].blend, BlendMode::Screen);
    assert_eq!(back.s.layers[1].opacity, 77);
    assert_eq!(back.s.layers[1].px, d.s.layers[1].px);
}

#[test]
fn png_round_trip_and_rotate() {
    let mut d = Document::new("t", 3, 2, [10, 20, 30, 255]);
    d.layer_mut().px[0] = [200, 0, 0, 255];
    let path = std::env::temp_dir().join(format!("pxe-test-{}.png", std::process::id()));
    crate::io::save(&d, &path, 90).unwrap();
    let back = crate::io::open(&path).unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!((back.w(), back.h()), (3, 2));
    assert_eq!(back.layer().px[0], [200, 0, 0, 255]);
    let r = raster::rot90(&d.layer().px, 3, 2);
    assert_eq!(r[1], [200, 0, 0, 255]); // top-left moves to top-right
}

#[test]
fn decoded_rules_from_original() {
    // tolerance distance: identical = 0, any difference at least 1, black vs white RGB = round(0.5*255*sqrt(3))
    assert_eq!(raster::color_distance([1, 2, 3, 255], [1, 2, 3, 255]), 0);
    assert_eq!(raster::color_distance([1, 2, 3, 255], [1, 2, 4, 255]), 1);
    assert_eq!(raster::color_distance([0, 0, 0, 255], [255, 255, 255, 255]), 221);
    // sepia: green = grey, red brighter, blue darker
    let s = apply(Fx::Sepia, &[[128, 128, 128, 255]], 1, 1, &[], [0; 4], [0; 4]);
    assert_eq!(s[0][1], 128);
    assert!(s[0][0] > 128 && s[0][2] < 128);
    // posterize with 2 levels gives only 0 and 255
    let px: Vec<[u8; 4]> = (0..=255).map(|v| [v as u8, v as u8, v as u8, 255]).collect();
    let out = apply(Fx::Posterize, &px, 256, 1, &[2.0, 2.0, 2.0, 1.0], [0; 4], [0; 4]);
    assert!(out.iter().all(|p| p[0] == 0 || p[0] == 255));
    // hue/saturation at defaults changes nothing
    let src = vec![[200u8, 30, 90, 255], [5, 250, 100, 128]];
    assert_eq!(apply(Fx::HueSaturation, &src, 2, 1, &[0.0, 100.0, 0.0], [0; 4], [0; 4]), src);
    // saturation 0 gives grey
    let g = apply(Fx::HueSaturation, &src, 2, 1, &[0.0, 0.0, 0.0], [0; 4], [0; 4]);
    assert!(g[0][0] == g[0][1] && g[0][1] == g[0][2]);
    // composition: half-transparent red over white
    let m = BlendMode::Normal.blend([255, 255, 255, 255], [255, 0, 0, 128], 255);
    assert_eq!(m[3], 255);
    assert!(m[1] > 120 && m[1] < 130);
}

#[test]
fn partial_composite_matches_full_and_is_fast() {
    let (w, h) = (4000, 3000);
    let mut d = Document::new("t", w, h, [255, 255, 255, 255]);
    let mut top = Layer::new("L", w, h, [0, 0, 0, 0]);
    for (i, p) in top.px.iter_mut().enumerate() { *p = [(i % 251) as u8, (i % 7) as u8 * 30, 90, (i % 256) as u8]; }
    top.blend = BlendMode::Overlay;
    d.s.layers.push(top);
    let full = d.composite();
    let r = (1234, 567, 1400, 700);
    let part = d.composite_rect(r);
    for y in r.1..r.3 {
        for x in r.0..r.2 {
            assert_eq!(part[(y - r.1) * (r.2 - r.0) + (x - r.0)], full[y * w + x]);
        }
    }
    // a brush-sized update must be far cheaper than a whole 12-megapixel picture
    let t = std::time::Instant::now();
    for _ in 0..100 { std::hint::black_box(d.composite_rect((100, 100, 164, 164))); }
    let per = t.elapsed() / 100;
    assert!(per < std::time::Duration::from_millis(2), "partial update took {per:?}");
}

#[test]
fn coverage_window_reports_recent_area() {
    let mut c = Coverage::new(100, 100);
    c.dab(10.0, 10.0, 4.0, true);
    let first = c.take_recent().unwrap();
    assert!(first.2 <= 14 && first.3 <= 14);
    assert!(c.take_recent().is_none());
    c.dab(80.0, 80.0, 4.0, true);
    let second = c.take_recent().unwrap();
    assert!(second.0 >= 76, "only the new dab is reported");
    let line = raster::line_coverage(1000, 1000, (10.0, 10.0), (20.0, 20.0), 2.0, true);
    assert!(line.c.len() < 1000, "line preview uses a small window, not the whole picture");
    assert_eq!(line.get(15, 15), 255);
}

#[test]
fn history_shares_unchanged_layers() {
    let mut d = Document::new("t", 64, 64, [255, 255, 255, 255]);
    d.s.layers.push(Layer::new("L", 64, 64, [0, 0, 0, 0]));
    d.commit("Add New Layer");
    d.s.layers[1].px[0] = [1, 2, 3, 255];
    d.commit("Paint");
    let (a, b) = (&d.history[1].state, &d.history[2].state);
    assert!(std::sync::Arc::ptr_eq(&a.layers[0], &b.layers[0]), "untouched background is shared");
    assert!(!std::sync::Arc::ptr_eq(&a.layers[1], &b.layers[1]));
    d.undo();
    assert_eq!(d.s.layers[1].px[0], [0, 0, 0, 0]);
}
