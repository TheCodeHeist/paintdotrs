//! Opening and saving pictures. Flat formats via the `image` crate; layers in our own `.pxe` file.

use crate::doc::{BlendMode, Document, Layer, Px};
use std::io::{Read, Write};
use std::path::Path;

pub const OPEN_EXTS: &[&str] = &["pxe", "png", "jpg", "jpeg", "bmp", "gif", "tif", "tiff", "tga", "webp", "ico", "qoi"];
/// Flat formats for File > Export (always written as a separate copy).
pub const EXPORT_EXTS: &[&str] = &["png", "jpg", "bmp", "tga", "tiff", "webp", "gif", "qoi"];
/// Version 1: layers only. Version 2: layers may carry a smart object (source picture + placement).
const MAGIC_V1: &[u8; 8] = b"PXEASEL1";
const MAGIC: &[u8; 8] = b"PXEASEL2";

pub fn file_name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Untitled".into())
}

pub fn load_pixels(path: &Path) -> Result<(usize, usize, Vec<Px>), String> {
    let img = image::open(path).map_err(|e| e.to_string())?.to_rgba8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    Ok((w, h, img.pixels().map(|p| p.0).collect()))
}

pub fn open(path: &Path) -> Result<Document, String> {
    let is_pxe = path.extension().map(|e| e.eq_ignore_ascii_case("pxe")).unwrap_or(false);
    let mut d = if is_pxe {
        let (w, h, layers) = read_pxe(path)?;
        Document::from_layers(&file_name(path), w, h, layers, "Open Image")
    } else {
        let (w, h, px) = load_pixels(path)?;
        let mut l = Layer::new("Background", w, h, [0, 0, 0, 0]);
        l.px = px;
        Document::from_layers(&file_name(path), w, h, vec![l], "Open Image")
    };
    // a layered project is saved back to itself; a flat picture is only the source and is never overwritten
    if is_pxe { d.path = Some(path.to_path_buf()); } else { d.source = Some(path.to_path_buf()); }
    Ok(d)
}

pub fn save(doc: &Document, path: &Path, jpeg_quality: u8) -> Result<(), String> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ext == "pxe" {
        return write_pxe(doc, path);
    }
    let flat = doc.composite();
    let (w, h) = (doc.w() as u32, doc.h() as u32);
    let bytes: Vec<u8> = flat.iter().flatten().copied().collect();
    let img = image::RgbaImage::from_raw(w, h, bytes).ok_or("bad size")?;
    match ext.as_str() {
        "jpg" | "jpeg" => {
            // JPEG has no transparency: put it on white first
            let rgb = image::RgbImage::from_fn(w, h, |x, y| {
                let p = img.get_pixel(x, y).0;
                let a = p[3] as u32;
                image::Rgb([0, 1, 2].map(|c| ((p[c] as u32 * a + 255 * (255 - a)) / 255) as u8))
            });
            let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(f), jpeg_quality);
            enc.encode_image(&rgb).map_err(|e| e.to_string())
        }
        "bmp" => image::DynamicImage::ImageRgba8(img).to_rgb8().save(path).map_err(|e| e.to_string()),
        "" => Err("Please give the file name an extension such as .png".into()),
        _ => img.save(path).map_err(|e| e.to_string()),
    }
}

fn write_pxe(doc: &Document, path: &Path) -> Result<(), String> {
    let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut z = flate2::write::ZlibEncoder::new(std::io::BufWriter::new(f), flate2::Compression::default());
    let mut put = |b: &[u8]| z.write_all(b).map_err(|e| e.to_string());
    put(MAGIC)?;
    put(&(doc.w() as u32).to_le_bytes())?;
    put(&(doc.h() as u32).to_le_bytes())?;
    put(&(doc.s.layers.len() as u32).to_le_bytes())?;
    for l in &doc.s.layers {
        put(&(l.name.len() as u32).to_le_bytes())?;
        put(l.name.as_bytes())?;
        put(&[l.visible as u8, l.opacity, l.blend.to_u8()])?;
        let raw: Vec<u8> = l.px.iter().flatten().copied().collect();
        put(&raw)?;
        match &l.smart {
            None => put(&[0])?,
            Some(s) => {
                put(&[1])?;
                put(&(s.sw as u32).to_le_bytes())?;
                put(&(s.sh as u32).to_le_bytes())?;
                for v in [s.xf.cx, s.xf.cy, s.xf.sx, s.xf.sy, s.xf.angle] {
                    put(&v.to_le_bytes())?;
                }
                let raw: Vec<u8> = s.src.iter().flatten().copied().collect();
                put(&raw)?;
            }
        }
    }
    z.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn read_pxe(path: &Path) -> Result<(usize, usize, Vec<Layer>), String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut z = flate2::read::ZlibDecoder::new(std::io::BufReader::new(f));
    let mut buf = Vec::new();
    z.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let mut at = 0usize;
    let mut take = |n: usize| -> Result<&[u8], String> {
        if at + n > buf.len() { return Err("The layered file is damaged.".into()); }
        let s = &buf[at..at + n];
        at += n;
        Ok(s)
    };
    let magic = take(8)?.to_vec();
    let v2 = magic == MAGIC;
    if !v2 && magic != MAGIC_V1 { return Err("Not a layered picture file.".into()); }
    let u32le = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
    let w = u32le(take(4)?);
    let h = u32le(take(4)?);
    let n = u32le(take(4)?);
    if w == 0 || h == 0 || w * h > 400_000_000 || n == 0 || n > 1000 { return Err("The layered file is damaged.".into()); }
    let mut layers = Vec::new();
    for _ in 0..n {
        let nl = u32le(take(4)?);
        let name = String::from_utf8_lossy(take(nl)?).into_owned();
        let meta = take(3)?.to_vec();
        let raw = take(w * h * 4)?;
        let px = raw.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
        let mut smart = None;
        if v2 && take(1)?[0] == 1 {
            let sw = u32le(take(4)?);
            let sh = u32le(take(4)?);
            if sw == 0 || sh == 0 || sw * sh > 400_000_000 { return Err("The layered file is damaged.".into()); }
            let mut f = [0f32; 5];
            for v in f.iter_mut() {
                let b = take(4)?;
                *v = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            }
            let src: Vec<crate::doc::Px> = take(sw * sh * 4)?.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
            let xf = crate::smart::Xform { cx: f[0], cy: f[1], sx: f[2], sy: f[3], angle: f[4] };
            smart = Some(crate::smart::Smart::new(src, sw, sh, xf));
        }
        layers.push(Layer { name, visible: meta[0] != 0, opacity: meta[1], blend: BlendMode::from_u8(meta[2]), px, smart });
    }
    Ok((w, h, layers))
}
