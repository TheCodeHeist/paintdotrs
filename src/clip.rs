//! The system clipboard: copy and paste pictures with other programs, and fetch a picture when only a
//! link to it was copied (for example "Copy image" in some browsers puts the address or HTML there).

use crate::doc::Px;
use std::path::PathBuf;
use std::sync::mpsc;

pub struct Picture {
    pub w: usize,
    pub h: usize,
    pub px: Vec<Px>,
}

/// What the clipboard currently offers.
pub enum Offer {
    Picture(Picture),
    /// a web address of a picture (to download)
    Link(String),
    Nothing,
}

pub struct SysClipboard {
    inner: Option<arboard::Clipboard>,
}

impl SysClipboard {
    pub fn new() -> Self {
        SysClipboard { inner: arboard::Clipboard::new().ok() }
    }

    fn cb(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.inner.is_none() {
            self.inner = arboard::Clipboard::new().ok();
        }
        self.inner.as_mut()
    }

    /// Put a picture on the system clipboard (kept available while the app runs).
    pub fn put(&mut self, w: usize, h: usize, px: &[Px]) {
        // native Wayland: our own clipboard support in the window library (works on every compositor)
        if wl::available() {
            let bytes: Vec<u8> = px.iter().flatten().copied().collect();
            if let Some(png) = encode_png(w, h, bytes) {
                if wl::write(vec![("image/png".into(), png)]) {
                    return;
                }
            }
        }
        if let Some(cb) = self.cb() {
            let bytes: Vec<u8> = px.iter().flatten().copied().collect();
            let _ = cb.set_image(arboard::ImageData { width: w, height: h, bytes: bytes.into() });
        }
    }

    /// Text on the clipboard (for text boxes).
    pub fn text(&mut self) -> Option<String> {
        if wl::available() {
            let types = wl::types();
            for t in ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "TEXT", "STRING"] {
                if types.iter().any(|x| x == t) {
                    if let Some(b) = wl::read(t, WAIT) {
                        return Some(String::from_utf8_lossy(&b).into_owned());
                    }
                }
            }
        }
        self.cb()?.get_text().ok()
    }

    /// The picture on the clipboard, or a link to one.
    pub fn offer(&mut self) -> Offer {
        if wl::available() {
            match wayland_offer() {
                Offer::Nothing => {}
                found => return found,
            }
        }
        let Some(cb) = self.cb() else { return Offer::Nothing };
        if let Ok(img) = cb.get_image() {
            if img.width > 0 && img.height > 0 && img.bytes.len() >= img.width * img.height * 4 {
                let px = img.bytes.chunks_exact(4).take(img.width * img.height).map(|c| [c[0], c[1], c[2], c[3]]).collect();
                return Offer::Picture(Picture { w: img.width, h: img.height, px });
            }
        }
        if let Ok(html) = cb.get().html() {
            if let Some(url) = img_src(&html) {
                return Offer::Link(url);
            }
        }
        if let Ok(text) = cb.get_text() {
            let t = text.trim();
            if (t.starts_with("http://") || t.starts_with("https://")) && !t.contains(char::is_whitespace) {
                return Offer::Link(t.to_string());
            }
            if let Some(url) = img_src(t) {
                return Offer::Link(url);
            }
            // a copied file in a file manager
            if let Some(path) = t.lines().next().and_then(|l| l.strip_prefix("file://")) {
                if let Ok(img) = image::open(percent_decode(path)) {
                    let img = img.to_rgba8();
                    let (w, h) = (img.width() as usize, img.height() as usize);
                    return Offer::Picture(Picture { w, h, px: img.pixels().map(|p| p.0).collect() });
                }
            }
        }
        Offer::Nothing
    }
}

use winit::platform::wayland::clipboard as wl;

/// How long to wait for another program to hand over clipboard data.
const WAIT: std::time::Duration = std::time::Duration::from_secs(3);

fn encode_png(w: usize, h: usize, rgba: Vec<u8>) -> Option<Vec<u8>> {
    let img = image::RgbaImage::from_raw(w as u32, h as u32, rgba)?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

fn decode_picture(bytes: &[u8]) -> Option<Picture> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    (w > 0 && h > 0).then(|| Picture { w, h, px: img.pixels().map(|p| p.0).collect() })
}

/// Read the Wayland clipboard: a picture, a copied picture file, or a link to a picture.
fn wayland_offer() -> Offer {
    let types = wl::types();
    let has = |t: &str| types.iter().any(|x| x == t);
    // 1. picture data, in the formats we read best first
    for t in ["image/png", "image/webp", "image/jpeg", "image/bmp", "image/gif", "image/tiff", "image/x-tga", "image/qoi"] {
        if has(t) {
            if let Some(p) = wl::read(t, WAIT).as_deref().and_then(decode_picture) {
                return Offer::Picture(p);
            }
        }
    }
    // any other image type the image library might know
    if let Some(t) = types.iter().find(|t| t.starts_with("image/")) {
        if let Some(p) = wl::read(t, WAIT).as_deref().and_then(decode_picture) {
            return Offer::Picture(p);
        }
    }
    // 2. copied files (file managers) or links
    if has("text/uri-list") {
        if let Some(b) = wl::read("text/uri-list", WAIT) {
            let text = String::from_utf8_lossy(&b).into_owned();
            for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
                if let Some(path) = line.strip_prefix("file://") {
                    let path = if path.starts_with('/') { path.to_string() } else { path.find('/').map(|i| path[i..].to_string()).unwrap_or_default() };
                    if let Ok(bytes) = std::fs::read(percent_decode(&path)) {
                        if let Some(p) = decode_picture(&bytes) {
                            return Offer::Picture(p);
                        }
                    }
                } else if line.starts_with("http://") || line.starts_with("https://") {
                    return Offer::Link(line.to_string());
                }
            }
        }
    }
    // 3. a web page fragment with a picture in it
    if has("text/html") {
        if let Some(url) = wl::read("text/html", WAIT).map(|b| String::from_utf8_lossy(&b).into_owned()).and_then(|h| img_src(&h)) {
            return Offer::Link(url);
        }
    }
    // 4. a picture's address as plain text
    for t in ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"] {
        if has(t) {
            if let Some(b) = wl::read(t, WAIT) {
                let text = String::from_utf8_lossy(&b).trim().to_string();
                if (text.starts_with("http://") || text.starts_with("https://")) && !text.contains(char::is_whitespace) {
                    return Offer::Link(text);
                }
                if let Some(url) = img_src(&text) {
                    return Offer::Link(url);
                }
            }
            break;
        }
    }
    Offer::Nothing
}

/// First `<img src="...">` address in some HTML.
fn img_src(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let at = lower.find("<img")?;
    let rest = &html[at..];
    let src = rest.to_ascii_lowercase().find("src=")?;
    let after = &rest[src + 4..];
    let quote = after.chars().next()?;
    let url: String = if quote == '"' || quote == '\'' {
        after[1..].chars().take_while(|c| *c != quote).collect()
    } else {
        after.chars().take_while(|c| !c.is_whitespace() && *c != '>').collect()
    };
    let url = url.replace("&amp;", "&");
    (url.starts_with("http://") || url.starts_with("https://")).then_some(url)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Download a picture in the background with the system's `curl` (or `wget`), keeping its full size.
pub fn download(url: String) -> mpsc::Receiver<Result<Picture, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(fetch(&url));
    });
    rx
}

fn fetch(url: &str) -> Result<Picture, String> {
    let tmp: PathBuf = std::env::temp_dir().join(format!("paint-rs-paste-{}", std::process::id()));
    let ok = std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "30", "--max-filesize", "200000000", "-o"])
        .arg(&tmp)
        .arg(url)
        .status()
        .map(|s| s.success())
        .or_else(|_| {
            std::process::Command::new("wget").args(["-q", "-T", "30", "-O"]).arg(&tmp).arg(url).status().map(|s| s.success())
        })
        .map_err(|_| "Couldn't download the picture: this computer has neither curl nor wget.".to_string())?;
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Couldn't download the picture from {url}."));
    }
    let res = image::open(&tmp).map_err(|e| format!("The copied link isn't a picture this app can read ({e})."));
    let _ = std::fs::remove_file(&tmp);
    let img = res?.to_rgba8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    Ok(Picture { w, h, px: img.pixels().map(|p| p.0).collect() })
}

/// A picture written into a web address (`data:image/png;base64,...`).
pub fn decode_data_url(url: &str) -> Result<Picture, String> {
    let comma = url.find(',').ok_or("That dropped address holds no picture.")?;
    let (head, body) = (&url[..comma], &url[comma + 1..]);
    let bytes = if head.ends_with(";base64") { base64_decode(body).ok_or("The dropped picture is damaged.")? } else { percent_decode(body).into_bytes() };
    let img = image::load_from_memory(&bytes).map_err(|e| format!("The dropped picture couldn't be read ({e})."))?.to_rgba8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    Ok(Picture { w, h, px: img.pixels().map(|p| p.0).collect() })
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        })
    };
    let clean: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=').collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut acc = 0u32;
        for (i, c) in chunk.iter().enumerate() { acc |= val(*c)? << (18 - 6 * i); }
        let n = chunk.len();
        if n < 2 { return None; }
        out.push((acc >> 16) as u8);
        if n > 2 { out.push((acc >> 8) as u8); }
        if n > 3 { out.push(acc as u8); }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_dropped_data_pictures() {
        // a 1x1 red PNG
        let url = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";
        let p = super::decode_data_url(url).unwrap();
        assert_eq!((p.w, p.h), (1, 1));
        assert_eq!(super::base64_decode("aGVsbG8=").unwrap(), b"hello");
    }

    #[test]
    fn finds_image_links() {
        assert_eq!(super::img_src(r#"<meta charset="utf-8"><img src="https://example.com/a.png?x=1&amp;y=2" alt="">"#).as_deref(), Some("https://example.com/a.png?x=1&y=2"));
        assert_eq!(super::img_src("<img src='http://e.org/b.jpg'>").as_deref(), Some("http://e.org/b.jpg"));
        assert_eq!(super::img_src("<img src=data:image/png;base64,AAA>"), None);
        assert_eq!(super::percent_decode("/home/a%20b.png"), "/home/a b.png");
    }
}
