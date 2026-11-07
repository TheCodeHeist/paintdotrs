//! The system's fonts for the Text tool: families, and each family's weights and italics.
//! Scanned once in the background when the app starts.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Face {
    pub weight: u16,
    pub italic: bool,
    pub id: fontdb::ID,
}

pub struct Catalog {
    db: fontdb::Database,
    /// family name → its faces, lightest first
    pub families: Vec<(String, Vec<Face>)>,
    cache: Mutex<Vec<(fontdb::ID, Arc<Vec<u8>>, u32)>>,
}

static CATALOG: OnceLock<Catalog> = OnceLock::new();

/// Start scanning the system fonts (call once at start-up).
pub fn load_in_background() {
    std::thread::spawn(|| {
        let _ = CATALOG.get_or_init(Catalog::load);
    });
}

/// The fonts, once scanned.
pub fn catalog() -> Option<&'static Catalog> {
    CATALOG.get()
}

/// The original's default font is Calibri (TextSection "FontFamilyName", paintdotnet.dll 0x06003575).
/// Linux rarely has it, so the closest free fonts come next (Carlito has Calibri's exact letter widths).
pub const PREFERRED_FAMILIES: &[&str] = &["Calibri", "Carlito", "Ubuntu", "Noto Sans", "Cantarell", "DejaVu Sans", "Liberation Sans"];

impl Catalog {
    fn load() -> Self {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let mut map: BTreeMap<String, Vec<Face>> = BTreeMap::new();
        for f in db.faces() {
            let Some((family, _)) = f.families.first() else { continue };
            let face = Face { weight: f.weight.0, italic: f.style != fontdb::Style::Normal, id: f.id };
            let list = map.entry(family.clone()).or_default();
            if !list.iter().any(|x| x.weight == face.weight && x.italic == face.italic) {
                list.push(face);
            }
        }
        let mut families: Vec<(String, Vec<Face>)> = map.into_iter().collect();
        for (_, faces) in families.iter_mut() {
            faces.sort_by_key(|f| (f.weight, f.italic));
        }
        families.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        Catalog { db, families, cache: Mutex::new(Vec::new()) }
    }

    pub fn faces(&self, family: &str) -> Option<&[Face]> {
        self.families.iter().find(|(n, _)| n == family).map(|(_, f)| f.as_slice())
    }

    /// The family to start with.
    pub fn default_family(&self) -> Option<String> {
        PREFERRED_FAMILIES.iter().find(|p| self.faces(p).is_some()).map(|s| s.to_string()).or_else(|| self.families.first().map(|f| f.0.clone()))
    }

    /// The face closest to a weight and italic choice.
    pub fn pick(&self, family: &str, weight: u16, italic: bool) -> Option<Face> {
        let faces = self.faces(family)?;
        faces.iter().copied().min_by_key(|f| (f.weight as i32 - weight as i32).abs() + if f.italic == italic { 0 } else { 1000 })
    }

    /// The font file bytes and the face index inside it.
    pub fn data(&self, id: fontdb::ID) -> Option<(Arc<Vec<u8>>, u32)> {
        let mut cache = self.cache.lock().ok()?;
        if let Some((_, d, i)) = cache.iter().find(|(c, _, _)| *c == id) {
            return Some((d.clone(), *i));
        }
        let (d, i) = self.db.with_face_data(id, |data, index| (Arc::new(data.to_vec()), index))?;
        cache.push((id, d.clone(), i));
        if cache.len() > 16 {
            cache.remove(0);
        }
        Some((d, i))
    }
}

/// Plain name for a font weight.
pub fn weight_name(w: u16) -> &'static str {
    match w {
        0..=149 => "Thin",
        150..=249 => "Extra Light",
        250..=349 => "Light",
        350..=449 => "Regular",
        450..=549 => "Medium",
        550..=649 => "Semibold",
        650..=749 => "Bold",
        750..=849 => "Extra Bold",
        _ => "Black",
    }
}
