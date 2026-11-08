//! Adjustments and effects. Each works on one layer's pixels and returns new pixels.

use crate::doc::{intensity, Px};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fx {
    // adjustments
    AutoLevel,
    BlackAndWhite,
    BrightnessContrast,
    HueSaturation,
    InvertColors,
    InvertAlpha,
    Levels,
    Posterize,
    Sepia,
    // effects
    GaussianBlur,
    MotionBlur,
    Pixelate,
    Bulge,
    Twist,
    AddNoise,
    Sharpen,
    Vignette,
    Emboss,
    EdgeDetect,
    Clouds,
    DropShadow,
}

pub struct Param {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub integer: bool,
}

const fn p(name: &'static str, min: f32, max: f32, default: f32, integer: bool) -> Param {
    Param { name, min, max, default, integer }
}

impl Fx {
    pub const ADJUSTMENTS: [Fx; 9] = [
        Fx::AutoLevel, Fx::BlackAndWhite, Fx::BrightnessContrast, Fx::HueSaturation, Fx::InvertColors,
        Fx::InvertAlpha, Fx::Levels, Fx::Posterize, Fx::Sepia,
    ];
    pub const EFFECTS: [(&'static str, &'static [Fx]); 7] = [
        ("Blurs", &[Fx::GaussianBlur, Fx::MotionBlur]),
        ("Distort", &[Fx::Bulge, Fx::Pixelate, Fx::Twist]),
        ("Noise", &[Fx::AddNoise]),
        ("Object", &[Fx::DropShadow]),
        ("Photo", &[Fx::Sharpen, Fx::Vignette]),
        ("Render", &[Fx::Clouds]),
        ("Stylize", &[Fx::EdgeDetect, Fx::Emboss]),
    ];

    pub fn name(self) -> &'static str {
        match self {
            Fx::AutoLevel => "Auto-Level",
            Fx::BlackAndWhite => "Black and White",
            Fx::BrightnessContrast => "Brightness / Contrast",
            Fx::HueSaturation => "Hue / Saturation",
            Fx::InvertColors => "Invert Colors",
            Fx::InvertAlpha => "Invert Alpha",
            Fx::Levels => "Levels",
            Fx::Posterize => "Posterize",
            Fx::Sepia => "Sepia",
            Fx::GaussianBlur => "Gaussian Blur",
            Fx::MotionBlur => "Motion Blur",
            Fx::Pixelate => "Pixelate",
            Fx::Bulge => "Bulge",
            Fx::Twist => "Twist",
            Fx::AddNoise => "Add Noise",
            Fx::Sharpen => "Sharpen",
            Fx::Vignette => "Vignette",
            Fx::Emboss => "Emboss",
            Fx::EdgeDetect => "Edge Detect",
            Fx::Clouds => "Clouds",
            Fx::DropShadow => "Drop Shadow",
        }
    }

    pub fn params(self) -> Vec<Param> {
        match self {
            Fx::BrightnessContrast => vec![p("Brightness", -100.0, 100.0, 0.0, true), p("Contrast", -100.0, 100.0, 0.0, true)],
            Fx::HueSaturation => vec![p("Hue", -180.0, 180.0, 0.0, true), p("Saturation", 0.0, 200.0, 100.0, true), p("Lightness", -100.0, 100.0, 0.0, true)],
            Fx::Levels => vec![p("Input black", 0.0, 254.0, 0.0, true), p("Input white", 1.0, 255.0, 255.0, true), p("Gamma (above 1 darkens)", 0.1, 10.0, 1.0, false)],
            Fx::Posterize => vec![p("Red", 2.0, 64.0, 16.0, true), p("Green", 2.0, 64.0, 16.0, true), p("Blue", 2.0, 64.0, 16.0, true), p("Linked (1 = on)", 0.0, 1.0, 1.0, true)],
            Fx::GaussianBlur => vec![p("Radius", 0.0, 100.0, 2.0, true)],
            Fx::MotionBlur => vec![p("Angle", -180.0, 180.0, 25.0, true), p("Distance", 1.0, 200.0, 10.0, true)],
            Fx::Pixelate => vec![p("Cell size", 1.0, 100.0, 2.0, true)],
            Fx::Bulge => vec![p("Bulge", -200.0, 100.0, 45.0, true)],
            Fx::Twist => vec![p("Amount", -100.0, 100.0, 30.0, true)],
            Fx::AddNoise => vec![p("Intensity", 0.0, 100.0, 64.0, true), p("Color saturation", 0.0, 400.0, 100.0, true), p("Coverage", 0.0, 100.0, 100.0, true)],
            Fx::Sharpen => vec![p("Amount", 1.0, 20.0, 2.0, true)],
            Fx::Vignette => vec![p("Radius", 10.0, 200.0, 50.0, true), p("Density", 0.0, 100.0, 100.0, true)],
            Fx::Emboss | Fx::EdgeDetect => vec![p("Angle", -180.0, 180.0, 45.0, true)],
            Fx::Clouds => vec![p("Scale", 2.0, 1000.0, 250.0, true), p("Roughness", 0.0, 100.0, 50.0, true), p("Seed", 0.0, 9999.0, 0.0, true)],
            // DropShadowGpuEffect.OnCreatePropertyCollection (PaintDotNet.Effects.Gpu.dll 0x060002b6)
            Fx::DropShadow => vec![p("Blur radius", 0.0, 100.0, 10.0, true), p("Distance", 0.0, 100.0, 10.0, true), p("Angle", -180.0, 180.0, -45.0, true), p("Opacity %", 0.0, 100.0, 75.0, true), p("Shadow only (1 = on)", 0.0, 1.0, 0.0, true)],
            _ => vec![],
        }
    }

}

fn clamp8(v: f32) -> u8 { v.round().clamp(0.0, 255.0) as u8 }

fn get(px: &[Px], w: usize, h: usize, x: i32, y: i32) -> Px {
    let x = x.clamp(0, w as i32 - 1) as usize;
    let y = y.clamp(0, h as i32 - 1) as usize;
    px[y * w + x]
}

fn sample_bilinear(px: &[Px], w: usize, h: usize, fx: f32, fy: f32) -> Px {
    let x0 = fx.floor();
    let y0 = fy.floor();
    let tx = fx - x0;
    let ty = fy - y0;
    let (x0, y0) = (x0 as i32, y0 as i32);
    let a = get(px, w, h, x0, y0);
    let b = get(px, w, h, x0 + 1, y0);
    let c = get(px, w, h, x0, y0 + 1);
    let d = get(px, w, h, x0 + 1, y0 + 1);
    let mut o = [0u8; 4];
    for i in 0..4 {
        let top = a[i] as f32 * (1.0 - tx) + b[i] as f32 * tx;
        let bot = c[i] as f32 * (1.0 - tx) + d[i] as f32 * tx;
        o[i] = clamp8(top * (1.0 - ty) + bot * ty);
    }
    o
}

/// Brightness / Contrast exactly as the original builds its lookup table (see MECHANICS.md).
fn brightness_contrast(px: &[Px], brightness: i32, contrast: i32) -> Vec<Px> {
    let (mul, div) = if contrast < 0 { (contrast + 100, 100) } else if contrast > 0 { (100, 100 - contrast) } else { (1, 1) };
    let mut table = vec![0u8; 65536];
    if div == 0 {
        for i in 0..256 {
            table[i] = if i as i32 + brightness < 128 { 0 } else { 255 };
        }
        return px.iter().map(|p| { let v = table[intensity(*p) as usize]; [v, v, v, p[3]] }).collect();
    }
    for i in 0..256i32 {
        let shift = if div == 100 {
            (i - 127) * mul / div + 127 - i + brightness
        } else {
            (i - 127 + brightness) * mul / div + 127 - i
        };
        for c in 0..256i32 {
            table[(i * 256 + c) as usize] = (c + shift).clamp(0, 255) as u8;
        }
    }
    px.iter()
        .map(|p| {
            let row = intensity(*p) as usize * 256;
            [table[row + p[0] as usize], table[row + p[1] as usize], table[row + p[2] as usize], p[3]]
        })
        .collect()
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let c = v * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [clamp8((r + m) * 255.0), clamp8((g + m) * 255.0), clamp8((b + m) * 255.0)]
}

fn box_blur_h(src: &[[f32; 4]], dst: &mut [[f32; 4]], w: usize, h: usize, r: usize) {
    let norm = 1.0 / (2 * r + 1) as f32;
    for y in 0..h {
        let row = &src[y * w..(y + 1) * w];
        let mut acc = [0f32; 4];
        for i in 0..=(2 * r) {
            let x = (i as i32 - r as i32).clamp(0, w as i32 - 1) as usize;
            for c in 0..4 { acc[c] += row[x][c]; }
        }
        for x in 0..w {
            for c in 0..4 { dst[y * w + x][c] = acc[c] * norm; }
            let out = (x as i32 - r as i32).clamp(0, w as i32 - 1) as usize;
            let inn = (x as i32 + r as i32 + 1).clamp(0, w as i32 - 1) as usize;
            for c in 0..4 { acc[c] += row[inn][c] - row[out][c]; }
        }
    }
}

fn transpose(src: &[[f32; 4]], w: usize, h: usize) -> Vec<[f32; 4]> {
    let mut o = vec![[0f32; 4]; w * h];
    for y in 0..h { for x in 0..w { o[x * h + y] = src[y * w + x]; } }
    o
}

/// Approximate Gaussian with three box blurs, done on premultiplied colour.
pub fn gaussian_blur(px: &[Px], w: usize, h: usize, radius: usize) -> Vec<Px> {
    if radius == 0 { return px.to_vec(); }
    let mut a: Vec<[f32; 4]> = px.iter().map(|p| { let al = p[3] as f32 / 255.0; [p[0] as f32 * al, p[1] as f32 * al, p[2] as f32 * al, p[3] as f32] }).collect();
    let sigma = radius as f32 / 2.0 + 0.5;
    let ideal = ((12.0 * sigma * sigma / 3.0) + 1.0).sqrt();
    let r = (((ideal - 1.0) / 2.0).round() as usize).max(1);
    let mut b = vec![[0f32; 4]; w * h];
    for _ in 0..3 { box_blur_h(&a, &mut b, w, h, r); std::mem::swap(&mut a, &mut b); }
    let t = transpose(&a, w, h);
    let mut a = t;
    let mut b = vec![[0f32; 4]; w * h];
    for _ in 0..3 { box_blur_h(&a, &mut b, h, w, r); std::mem::swap(&mut a, &mut b); }
    let a = transpose(&a, h, w);
    a.iter().map(|q| {
        let al = q[3];
        if al <= 0.01 { [0, 0, 0, 0] } else { let k = 255.0 / al; [clamp8(q[0] * k), clamp8(q[1] * k), clamp8(q[2] * k), clamp8(al)] }
    }).collect()
}

fn hash(x: u32) -> u32 {
    let mut x = x.wrapping_mul(0x9E3779B1) ^ 0x85EB_CA6B;
    x ^= x >> 15; x = x.wrapping_mul(0x2C1B_3C6D); x ^= x >> 12; x = x.wrapping_mul(0x297A_2D39); x ^= x >> 15;
    x
}
fn rnd(i: u32, seed: u32) -> f32 { (hash(i ^ seed.wrapping_mul(0x632B_E5AB)) & 0xFFFFFF) as f32 / 16_777_215.0 }

fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (tx, ty) = (x - xi as f32, y - yi as f32);
    let sm = |t: f32| t * t * (3.0 - 2.0 * t);
    let at = |a: i32, b: i32| rnd((a as u32).wrapping_mul(73_856_093) ^ (b as u32).wrapping_mul(19_349_663), seed);
    let top = at(xi, yi) + (at(xi + 1, yi) - at(xi, yi)) * sm(tx);
    let bot = at(xi, yi + 1) + (at(xi + 1, yi + 1) - at(xi, yi + 1)) * sm(tx);
    top + (bot - top) * sm(ty)
}

/// Auto-Level, as the original's `HistogramRgbLegacy.MakeLevelsAuto` (PaintDotNet.Effects.Legacy.dll
/// 0x060000f9) and `LevelOpLegacy.AutoFromLoMidHi` (0x0600012d): per channel, a histogram of the selected
/// pixels gives the dark end (0.5 % point), the bright end (99.5 % point) and the mean; the channel is then
/// stretched so the ends become black and white, with a gamma that puts the mean in the middle:
/// gamma = clamp(log(0.5) / log((mean - lo) / (hi - lo)), 0.1, 10), or 1 when lo < mean < hi doesn't hold.
/// Fully transparent pixels are left out of the histogram (our addition: they have no visible colour).
pub fn auto_level(px: &[Px], mask: Option<&[u8]>) -> Vec<Px> {
    let mut hist = [[0u64; 256]; 3];
    for (i, p) in px.iter().enumerate() {
        if p[3] == 0 { continue; }
        if let Some(m) = mask { if m[i] == 0 { continue; } }
        for c in 0..3 { hist[c][p[c] as usize] += 1; }
    }
    let total: u64 = hist[0].iter().sum();
    if total == 0 { return px.to_vec(); }
    // HistogramLegacy.GetPercentile: the first bin where the running count exceeds total * fraction
    let percentile = |h: &[u64; 256], f: f32| -> f32 {
        let mut run = 0u64;
        for (i, n) in h.iter().enumerate() { run += n; if run as f32 > total as f32 * f { return i as f32; } }
        255.0
    };
    let mut tables = [[0u8; 256]; 3];
    for c in 0..3 {
        let lo = percentile(&hist[c], 0.005);
        let hi = percentile(&hist[c], 0.995);
        // GetMeanColor: mean + 0.5, truncated to a byte
        let mean = hist[c].iter().enumerate().map(|(i, n)| i as u64 * n).sum::<u64>() as f32 / total as f32;
        let mid = (mean + 0.5).floor().min(255.0);
        let gamma = if lo < mid && mid < hi { (0.5f64.ln() / (((mid - lo) / (hi - lo)) as f64).ln()).clamp(0.1, 10.0) } else { 1.0 };
        for v in 0..256 {
            let x = v as f32;
            tables[c][v] = if hi <= lo { v as u8 } else if x <= lo { 0 } else if x >= hi { 255 } else {
                (255.0 * (((x - lo) / (hi - lo)) as f64).powf(gamma)).clamp(0.0, 255.0) as u8
            };
        }
    }
    px.iter().map(|p| [tables[0][p[0] as usize], tables[1][p[1] as usize], tables[2][p[2] as usize], p[3]]).collect()
}

/// Drop shadow under the layer's objects: the layer's shape (its transparency), moved `distance` pixels
/// towards `angle` (degrees, counter-clockwise from the right, so -45 is down-right), blurred by `blur`
/// and drawn in `color` at `opacity` behind the layer (or alone with `shadow_only`). Needs a layer with
/// transparency, like the original says.
pub fn drop_shadow(px: &[Px], w: usize, h: usize, blur: f32, distance: f32, angle: f32, opacity: f32, shadow_only: bool, color: Px) -> Vec<Px> {
    let a = angle.to_radians();
    let (dx, dy) = ((a.cos() * distance).round() as i64, (-a.sin() * distance).round() as i64);
    let mut shape = vec![[0u8, 0, 0, 0]; w * h];
    for y in 0..h as i64 {
        let sy = y - dy;
        if sy < 0 || sy >= h as i64 { continue; }
        for x in 0..w as i64 {
            let sx = x - dx;
            if sx < 0 || sx >= w as i64 { continue; }
            let al = px[sy as usize * w + sx as usize][3];
            shape[y as usize * w + x as usize] = [color[0], color[1], color[2], al];
        }
    }
    let blurred = gaussian_blur(&shape, w, h, blur.max(0.0) as usize);
    let op = opacity.clamp(0.0, 1.0) * color[3] as f32 / 255.0;
    blurred.iter().zip(px.iter()).map(|(s, p)| {
        let sh = [color[0], color[1], color[2], (s[3] as f32 * op).round() as u8];
        if shadow_only { sh } else { crate::doc::over(sh, *p, 255) }
    }).collect()
}

/// Run an effect. `prim`/`sec` are the palette colours (used by Clouds).
pub fn apply(fx: Fx, px: &[Px], w: usize, h: usize, v: &[f32], prim: Px, sec: Px) -> Vec<Px> {
    let vi = |i: usize| v.get(i).copied().unwrap_or(0.0);
    match fx {
        Fx::BrightnessContrast => brightness_contrast(px, vi(0) as i32, vi(1) as i32),
        Fx::InvertColors => px.iter().map(|p| [255 - p[0], 255 - p[1], 255 - p[2], p[3]]).collect(),
        Fx::InvertAlpha => px.iter().map(|p| [p[0], p[1], p[2], 255 - p[3]]).collect(),
        Fx::BlackAndWhite => px.iter().map(|p| { let i = intensity(*p); [i, i, i, p[3]] }).collect(),
        Fx::Sepia => {
            // original: Desaturate, then Level(black, white, gamma {B 1.2, G 1.0, R 0.8}, black, white)
            let lv = |i: u8, g: f64| ((255.0 * (i as f64 / 255.0).powf(g)).clamp(0.0, 255.0)) as u8;
            px.iter().map(|p| { let i = intensity(*p); [lv(i, 0.8), i, lv(i, 1.2), p[3]] }).collect()
        }
        Fx::Posterize => {
            let r = vi(0).clamp(2.0, 64.0) as usize;
            let linked = vi(3) >= 0.5;
            let (g, b) = if linked { (r, r) } else { (vi(1).clamp(2.0, 64.0) as usize, vi(2).clamp(2.0, 64.0) as usize) };
            let (tr, tg, tb) = (posterize_table(r), posterize_table(g), posterize_table(b));
            px.iter().map(|p| [tr[p[0] as usize], tg[p[1] as usize], tb[p[2] as usize], p[3]]).collect()
        }
        Fx::Levels => {
            // the original's Level op: out = trunc(255 * ((v - lo) / (hi - lo)) ^ gamma)
            let (lo, hi, g) = (vi(0), vi(1).max(vi(0) + 1.0), vi(2).max(0.01) as f64);
            let t: Vec<u8> = (0..256).map(|c| {
                let v = c as f32 - lo;
                if v < 0.0 { 0 } else if c as f32 >= hi { 255 } else { (255.0 * ((v / (hi - lo)) as f64).powf(g)).clamp(0.0, 255.0) as u8 }
            }).collect();
            px.iter().map(|p| [t[p[0] as usize], t[p[1] as usize], t[p[2] as usize], p[3]]).collect()
        }
        Fx::AutoLevel => auto_level(px, None),
        Fx::DropShadow => drop_shadow(px, w, h, vi(0), vi(1), vi(2), vi(3) / 100.0, vi(4) >= 0.5, [0, 0, 0, 255]),
        Fx::HueSaturation => {
            let mut sat = vi(1) as i32;
            if sat > 100 { sat = (sat - 100) * 3 + 100; } // HueAndSaturationAdjustment.OnSetRenderInfo
            hue_sat_lightness(px, vi(0) as i32, sat, vi(2) as i32)
        }
        Fx::GaussianBlur => gaussian_blur(px, w, h, vi(0) as usize),
        Fx::MotionBlur => {
            let ang = vi(0).to_radians();
            let dist = vi(1).max(1.0);
            let n = (dist as usize).max(2);
            let (dx, dy) = (ang.cos(), ang.sin());
            let mut o = vec![[0u8; 4]; w * h];
            for y in 0..h {
                for x in 0..w {
                    let mut acc = [0f32; 4];
                    for i in 0..n {
                        let t = i as f32 / (n - 1) as f32 * dist - dist / 2.0;
                        let q = get(px, w, h, (x as f32 + dx * t).round() as i32, (y as f32 + dy * t).round() as i32);
                        let a = q[3] as f32;
                        for c in 0..3 { acc[c] += q[c] as f32 * a; }
                        acc[3] += a;
                    }
                    o[y * w + x] = if acc[3] <= 0.0 { [0, 0, 0, 0] } else { [clamp8(acc[0] / acc[3]), clamp8(acc[1] / acc[3]), clamp8(acc[2] / acc[3]), clamp8(acc[3] / n as f32)] };
                }
            }
            o
        }
        Fx::Pixelate => {
            let cs = vi(0).max(1.0) as usize;
            let mut o = px.to_vec();
            for by in (0..h).step_by(cs) {
                for bx in (0..w).step_by(cs) {
                    let mut acc = [0u32; 4];
                    let mut n = 0;
                    for y in by..(by + cs).min(h) { for x in bx..(bx + cs).min(w) { for c in 0..4 { acc[c] += px[y * w + x][c] as u32; } n += 1; } }
                    let avg = [ (acc[0] / n) as u8, (acc[1] / n) as u8, (acc[2] / n) as u8, (acc[3] / n) as u8 ];
                    for y in by..(by + cs).min(h) { for x in bx..(bx + cs).min(w) { o[y * w + x] = avg; } }
                }
            }
            o
        }
        Fx::Bulge | Fx::Twist => {
            let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
            let rad = cx.min(cy).max(1.0);
            let amt = vi(0) / 100.0;
            let mut o = px.to_vec();
            for y in 0..h {
                for x in 0..w {
                    let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                    let d = (dx * dx + dy * dy).sqrt();
                    if d >= rad { continue; }
                    let t = 1.0 - d / rad;
                    let (sx, sy) = if fx == Fx::Bulge {
                        let k = 1.0 - amt * t * t;
                        (cx + dx * k, cy + dy * k)
                    } else {
                        let a = amt * std::f32::consts::PI * 2.0 * t * t;
                        let (s, c) = a.sin_cos();
                        (cx + dx * c - dy * s, cy + dx * s + dy * c)
                    };
                    o[y * w + x] = sample_bilinear(px, w, h, sx - 0.5, sy - 0.5);
                }
            }
            o
        }
        Fx::AddNoise => {
            let (int, sat, cov) = (vi(0) / 100.0, vi(1) / 100.0, vi(2) / 100.0);
            px.iter().enumerate().map(|(i, p)| {
                if rnd(i as u32, 7) > cov { return *p; }
                let base = (rnd(i as u32, 11) - 0.5) * 2.0 * int * 128.0;
                let mut o = *p;
                for c in 0..3 {
                    let col = (rnd(i as u32 * 3 + c as u32, 13) - 0.5) * 2.0 * int * 128.0 * sat;
                    o[c] = clamp8(p[c] as f32 + base * (1.0 - sat.min(1.0)) + col);
                }
                o
            }).collect()
        }
        Fx::Sharpen => {
            let blur = gaussian_blur(px, w, h, 2);
            let k = vi(0) / 4.0;
            px.iter().zip(blur.iter()).map(|(p, b)| {
                let mut o = *p;
                for c in 0..3 { o[c] = clamp8(p[c] as f32 + (p[c] as f32 - b[c] as f32) * k); }
                o
            }).collect()
        }
        Fx::Vignette => {
            let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
            let r = (cx * cx + cy * cy).sqrt() * vi(0) / 100.0;
            let dens = vi(1) / 100.0;
            px.iter().enumerate().map(|(i, p)| {
                let (x, y) = ((i % w) as f32 + 0.5 - cx, (i / w) as f32 + 0.5 - cy);
                let d = ((x * x + y * y).sqrt() / r.max(1.0)).min(1.0);
                let f = 1.0 - dens * d * d;
                [clamp8(p[0] as f32 * f), clamp8(p[1] as f32 * f), clamp8(p[2] as f32 * f), p[3]]
            }).collect()
        }
        Fx::Emboss | Fx::EdgeDetect => {
            let a = vi(0).to_radians();
            let (dx, dy) = (a.cos(), a.sin());
            let mut o = px.to_vec();
            for y in 0..h as i32 {
                for x in 0..w as i32 {
                    let s1 = get(px, w, h, (x as f32 - dx).round() as i32, (y as f32 - dy).round() as i32);
                    let s2 = get(px, w, h, (x as f32 + dx).round() as i32, (y as f32 + dy).round() as i32);
                    let i = (y as usize) * w + x as usize;
                    if fx == Fx::Emboss {
                        let v = clamp8(128.0 + intensity(s1) as f32 - intensity(s2) as f32);
                        o[i] = [v, v, v, px[i][3]];
                    } else {
                        for c in 0..3 { o[i][c] = clamp8(128.0 + s1[c] as f32 - s2[c] as f32); }
                    }
                }
            }
            o
        }
        Fx::Clouds => {
            let scale = vi(0).max(2.0);
            let rough = vi(1) / 100.0;
            let seed = vi(2) as u32;
            px.iter().enumerate().map(|(i, _)| {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                let mut amp = 1.0;
                let mut freq = 1.0 / scale;
                let mut sum = 0.0;
                let mut norm = 0.0;
                for o in 0..6 {
                    sum += value_noise(x * freq, y * freq, seed + o) * amp;
                    norm += amp;
                    amp *= rough.max(0.05);
                    freq *= 2.0;
                }
                let t = (sum / norm).clamp(0.0, 1.0);
                crate::raster::lerp_px(prim, sec, t)
            }).collect()
        }
    }
}

/// The original's `PosterizePixelOp.CalcLevels` (PaintDotNet.Effects.Legacy.dll 0x060002a6).
fn posterize_table(n: usize) -> [u8; 256] {
    let mut levels = vec![0u8; n];
    for (i, l) in levels.iter_mut().enumerate().skip(1) {
        *l = (255 * i / (n - 1)) as u8;
    }
    let mut t = [0u8; 256];
    let (mut j, mut acc) = (0usize, 0usize);
    for v in t.iter_mut() {
        *v = levels[j.min(n - 1)];
        acc += n;
        if acc > 255 { acc -= 255; j += 1; }
    }
    t
}

/// The original's `UnaryPixelOps.HueSaturationLightness` (PaintDotNet.Core.dll 0x0600113d/0x06001140):
/// saturation pulls channels toward the grey value, hue turns around max/min, lightness uses a table.
fn hue_sat_lightness(px: &[Px], hue: i32, sat: i32, light: i32) -> Vec<Px> {
    let hue_delta = (hue * 6) as f64 / 360.0;
    let sat_factor = sat * 65536 / 100;
    let table: Option<Vec<u8>> = (light != 0).then(|| {
        let amount = light.abs() as f64 / 100.0;
        let target = if light > 0 { 1.0 } else { 0.0 };
        (0..256).map(|i| { let x = i as f64 / 255.0; (0.5 + 255.0 * (x + (target - x) * amount)) as u8 }).collect()
    });
    px.iter().map(|p| {
        let (mut r, mut g, mut b) = (p[0] as i32, p[1] as i32, p[2] as i32);
        if sat_factor != 65536 {
            let i = intensity(*p) as i32;
            let f = |c: i32| (((i << 16) + (c - i) * sat_factor) >> 16).clamp(0, 255);
            r = f(r); g = f(g); b = f(b);
        }
        if hue_delta != 0.0 {
            let mx = r.max(g).max(b);
            let mn = r.min(g).min(b);
            let d = (mx - mn) as f64;
            if d > 0.0 {
                let mut h = if r == mx { (g - b) as f64 / d } else if g == mx { (b - r) as f64 / d + 2.0 } else { (r - g) as f64 / d + 4.0 };
                h += hue_delta;
                while h < 0.0 { h += 6.0; }
                while h >= 6.0 { h -= 6.0; }
                let sext = h as i32;
                let fr = h - sext as f64;
                let up = (mn as f64 + d * fr) as i32;
                let down = (mx as f64 - d * fr) as i32;
                (r, g, b) = match sext {
                    0 => (mx, up, mn),
                    1 => (down, mx, mn),
                    2 => (mn, mx, up),
                    3 => (mn, down, mx),
                    4 => (up, mn, mx),
                    _ => (mx, mn, down),
                };
            }
        }
        if let Some(t) = &table { r = t[r as usize] as i32; g = t[g as usize] as i32; b = t[b as usize] as i32; }
        [r as u8, g as u8, b as u8, p[3]]
    }).collect()
}
