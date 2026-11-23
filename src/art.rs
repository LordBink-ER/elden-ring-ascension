//! 0.7 "Wygląd": card illustrations. tools/build_card_art.py paints one 2:1
//! picture per power (plus the card back) into QOI atlases with an index; this
//! module reads and decodes them into RGBA for the overlay's textures. Pure
//! data code (no game or graphics calls), so the build validator checks it too.
use std::{collections::BTreeMap, path::Path};

/// Largest atlas side accepted (D3D12 allows 16384; ours are 2048).
const MAX_SIDE: u32 = 8192;

#[derive(Debug, serde::Deserialize)]
struct IndexFile {
    schema: u32,
    tile: [u32; 2],
    columns: u32,
    per_atlas: u32,
    atlases: Vec<AtlasEntry>,
    back: u32,
    cards: BTreeMap<String, u32>,
}
#[derive(Debug, serde::Deserialize)]
struct AtlasEntry {
    file: String,
    width: u32,
    height: u32,
}

/// One decoded atlas: RGBA8, `width * height * 4` bytes.
pub struct Atlas {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Every card picture, decoded and ready to upload.
pub struct ArtPack {
    pub tile: [u32; 2],
    pub columns: u32,
    pub per_atlas: u32,
    pub atlases: Vec<Atlas>,
    pub back: u32,
    pub cards: BTreeMap<String, u32>,
}

/// Where tile `index` sits: (atlas, uv min, uv max), inset half a texel so
/// neighbouring tiles never bleed in.
pub fn tile_uv(pack_tile: [u32; 2], columns: u32, per_atlas: u32, sizes: &[[u32; 2]], index: u32) -> Option<(usize, [f32; 2], [f32; 2])> {
    if columns == 0 || per_atlas == 0 {
        return None;
    }
    let atlas = (index / per_atlas) as usize;
    let size = sizes.get(atlas)?;
    let local = index % per_atlas;
    let (col, row) = (local % columns, local / columns);
    let (x0, y0) = (col * pack_tile[0], row * pack_tile[1]);
    let (x1, y1) = (x0 + pack_tile[0], y0 + pack_tile[1]);
    if x1 > size[0] || y1 > size[1] || size[0] == 0 || size[1] == 0 {
        return None;
    }
    let (w, h) = (size[0] as f32, size[1] as f32);
    Some((
        atlas,
        [(x0 as f32 + 0.5) / w, (y0 as f32 + 0.5) / h],
        [(x1 as f32 - 0.5) / w, (y1 as f32 - 0.5) / h],
    ))
}

impl ArtPack {
    /// Read assets/cards/index.json and decode every atlas it lists.
    pub fn load(root: &Path) -> Result<Self, String> {
        let dir = root.join("assets/cards");
        let text = std::fs::read_to_string(dir.join("index.json")).map_err(|e| format!("card art index: {e}"))?;
        let index: IndexFile = serde_json::from_str(&text).map_err(|e| format!("card art index: {e}"))?;
        if index.schema != 1 {
            return Err(format!("card art index schema {} is not supported", index.schema));
        }
        if index.tile[0] == 0 || index.tile[1] == 0 || index.columns == 0 || index.per_atlas == 0 {
            return Err("card art index has an empty tile layout".into());
        }
        let mut atlases = Vec::with_capacity(index.atlases.len());
        for entry in &index.atlases {
            if entry.file.contains(['/', '\\']) || entry.file.contains("..") {
                return Err(format!("card art atlas name {} is not a plain file name", entry.file));
            }
            let bytes = std::fs::read(dir.join(&entry.file)).map_err(|e| format!("{}: {e}", entry.file))?;
            let (width, height, rgba) = decode_qoi(&bytes).map_err(|e| format!("{}: {e}", entry.file))?;
            if width != entry.width || height != entry.height {
                return Err(format!(
                    "{}: {width}x{height} does not match the index ({}x{})",
                    entry.file, entry.width, entry.height
                ));
            }
            atlases.push(Atlas { width, height, rgba });
        }
        let pack = Self {
            tile: index.tile,
            columns: index.columns,
            per_atlas: index.per_atlas,
            atlases,
            back: index.back,
            cards: index.cards,
        };
        let sizes = pack.sizes();
        for (id, tile) in pack.cards.iter().map(|(k, v)| (k.as_str(), *v)).chain([("card back", pack.back)]) {
            if tile_uv(pack.tile, pack.columns, pack.per_atlas, &sizes, tile).is_none() {
                return Err(format!("card art for {id} points outside its atlas"));
            }
        }
        Ok(pack)
    }
    pub fn sizes(&self) -> Vec<[u32; 2]> {
        self.atlases.iter().map(|a| [a.width, a.height]).collect()
    }
    pub fn bytes(&self) -> usize {
        self.atlases.iter().map(|a| a.rgba.len()).sum()
    }
}

/// QOI ("Quite OK Image") decoder, RGB or RGBA input, RGBA8 output.
/// Malformed data never panics: missing bytes repeat the last pixel.
pub fn decode_qoi(data: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    if data.len() < 22 || &data[..4] != b"qoif" {
        return Err("not a QOI image".into());
    }
    let width = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    let height = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
    let channels = data[12];
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE || !(3..=4).contains(&channels) {
        return Err(format!("unsupported QOI header {width}x{height}, {channels} channels"));
    }
    let count = width as usize * height as usize;
    let mut out = Vec::with_capacity(count * 4);
    let mut index = [[0u8; 4]; 64];
    let mut px = [0u8, 0, 0, 255];
    let mut p = 14usize;
    let end = data.len() - 8;
    let mut run = 0u32;
    let byte = |i: usize| data.get(i).copied().unwrap_or(0);
    for _ in 0..count {
        if run > 0 {
            run -= 1;
        } else if p < end {
            let b1 = data[p];
            p += 1;
            if b1 == 0xfe {
                px[0] = byte(p);
                px[1] = byte(p + 1);
                px[2] = byte(p + 2);
                p += 3;
            } else if b1 == 0xff {
                px = [byte(p), byte(p + 1), byte(p + 2), byte(p + 3)];
                p += 4;
            } else {
                match b1 >> 6 {
                    0 => px = index[usize::from(b1)],
                    1 => {
                        px[0] = px[0].wrapping_add((b1 >> 4) & 3).wrapping_sub(2);
                        px[1] = px[1].wrapping_add((b1 >> 2) & 3).wrapping_sub(2);
                        px[2] = px[2].wrapping_add(b1 & 3).wrapping_sub(2);
                    }
                    2 => {
                        let b2 = byte(p);
                        p += 1;
                        let vg = (b1 & 0x3f).wrapping_sub(32);
                        px[0] = px[0].wrapping_add(vg.wrapping_sub(8).wrapping_add((b2 >> 4) & 0x0f));
                        px[1] = px[1].wrapping_add(vg);
                        px[2] = px[2].wrapping_add(vg.wrapping_sub(8).wrapping_add(b2 & 0x0f));
                    }
                    _ => run = u32::from(b1 & 0x3f),
                }
            }
            let hash = (usize::from(px[0]) * 3 + usize::from(px[1]) * 5 + usize::from(px[2]) * 7 + usize::from(px[3]) * 11) % 64;
            index[hash] = px;
        }
        out.extend_from_slice(&px);
    }
    Ok((width, height, out))
}
