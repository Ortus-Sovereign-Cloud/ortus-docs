//! Just enough of a TrueType font to set text in a PDF: which glyph a
//! character has, how wide it is, the font's measures. And Arabic letters
//! shaped (their joined forms, lam-alef) and laid right to left.

use std::collections::HashMap;

pub struct Font {
    pub data: &'static [u8],
    pub units: f32,
    pub ascent: i16,
    pub descent: i16,
    pub bbox: [i16; 4],
    glyphs: HashMap<char, u16>,
    widths: Vec<u16>,
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    b.get(at..at + 2)
        .map_or(0, |x| u16::from_be_bytes([x[0], x[1]]))
}

fn i16_at(b: &[u8], at: usize) -> i16 {
    u16_at(b, at) as i16
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4)
        .map_or(0, |x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]))
}

fn table(b: &'static [u8], tag: &[u8; 4]) -> Option<&'static [u8]> {
    let n = u16_at(b, 4) as usize;
    for i in 0..n {
        let rec = 12 + i * 16;
        if b.get(rec..rec + 4) == Some(tag) {
            let off = u32_at(b, rec + 8) as usize;
            let len = u32_at(b, rec + 12) as usize;
            return b.get(off..off + len);
        }
    }
    None
}

impl Font {
    pub fn load(data: &'static [u8]) -> Font {
        let head = table(data, b"head").unwrap_or_default();
        let hhea = table(data, b"hhea").unwrap_or_default();
        let hmtx = table(data, b"hmtx").unwrap_or_default();
        let cmap = table(data, b"cmap").unwrap_or_default();
        let metrics = u16_at(hhea, 34) as usize;
        let widths = (0..metrics).map(|i| u16_at(hmtx, i * 4)).collect();
        let mut glyphs = HashMap::new();
        // The Unicode subtable: full repertoire (3, 10, format 12) or BMP (3, 1, format 4).
        let count = u16_at(cmap, 2) as usize;
        let mut best = None;
        for i in 0..count {
            let rec = 4 + i * 8;
            let (platform, encoding, off) = (
                u16_at(cmap, rec),
                u16_at(cmap, rec + 2),
                u32_at(cmap, rec + 4) as usize,
            );
            match (platform, encoding) {
                (3, 10) => best = Some(off),
                (3, 1) if best.is_none() => best = Some(off),
                _ => {}
            }
        }
        if let Some(off) = best {
            let sub = &cmap[off.min(cmap.len())..];
            match u16_at(sub, 0) {
                4 => {
                    let segs = u16_at(sub, 6) as usize / 2;
                    let (ends, starts, deltas, ranges) =
                        (14, 16 + segs * 2, 16 + segs * 4, 16 + segs * 6);
                    for s in 0..segs {
                        let (end, start) = (u16_at(sub, ends + s * 2), u16_at(sub, starts + s * 2));
                        let delta = u16_at(sub, deltas + s * 2);
                        let range = u16_at(sub, ranges + s * 2) as usize;
                        if start == 0xFFFF {
                            continue;
                        }
                        for c in start..=end {
                            let g = if range == 0 {
                                c.wrapping_add(delta)
                            } else {
                                let at = ranges + s * 2 + range + (c - start) as usize * 2;
                                let g = u16_at(sub, at);
                                if g == 0 {
                                    0
                                } else {
                                    g.wrapping_add(delta)
                                }
                            };
                            if let Some(ch) = char::from_u32(c as u32) {
                                if g != 0 {
                                    glyphs.insert(ch, g);
                                }
                            }
                        }
                    }
                }
                12 => {
                    let groups = u32_at(sub, 12) as usize;
                    for k in 0..groups {
                        let at = 16 + k * 12;
                        let (start, end, first) =
                            (u32_at(sub, at), u32_at(sub, at + 4), u32_at(sub, at + 8));
                        for c in start..=end.min(start + 0xFFFF) {
                            if let Some(ch) = char::from_u32(c) {
                                glyphs.insert(ch, (first + c - start) as u16);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Font {
            data,
            units: u16_at(head, 18).max(1) as f32,
            ascent: i16_at(hhea, 4),
            descent: i16_at(hhea, 6),
            bbox: [
                i16_at(head, 36),
                i16_at(head, 38),
                i16_at(head, 40),
                i16_at(head, 42),
            ],
            glyphs,
            widths,
        }
    }

    pub fn glyph(&self, c: char) -> u16 {
        self.glyphs.get(&c).copied().unwrap_or(0)
    }

    /// A glyph's advance, in font units.
    pub fn advance(&self, g: u16) -> u16 {
        let g = g as usize;
        self.widths
            .get(g)
            .or(self.widths.last())
            .copied()
            .unwrap_or(0)
    }

    /// Width of text at a size, in points.
    pub fn width(&self, text: &str, size: f32) -> f32 {
        text.chars()
            .map(|c| self.advance(self.glyph(c)) as f32)
            .sum::<f32>()
            * size
            / self.units
    }
}

// --- Arabic ---

/// The isolated presentation form and the number of forms of each
/// letter from U+0621 to U+064A (0: not a letter that joins).
const ARABIC: [(u16, u8); 42] = [
    (0xFE80, 1),
    (0xFE81, 2),
    (0xFE83, 2),
    (0xFE85, 2),
    (0xFE87, 2),
    (0xFE89, 4),
    (0xFE8D, 2),
    (0xFE8F, 4),
    (0xFE93, 2),
    (0xFE95, 4),
    (0xFE99, 4),
    (0xFE9D, 4),
    (0xFEA1, 4),
    (0xFEA5, 4),
    (0xFEA9, 2),
    (0xFEAB, 2),
    (0xFEAD, 2),
    (0xFEAF, 2),
    (0xFEB1, 4),
    (0xFEB5, 4),
    (0xFEB9, 4),
    (0xFEBD, 4),
    (0xFEC1, 4),
    (0xFEC5, 4),
    (0xFEC9, 4),
    (0xFECD, 4),
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
    (0xFED1, 4),
    (0xFED5, 4),
    (0xFED9, 4),
    (0xFEDD, 4),
    (0xFEE1, 4),
    (0xFEE5, 4),
    (0xFEE9, 4),
    (0xFEED, 2),
    (0xFEEF, 2),
    (0xFEF1, 4),
];

fn letter(c: char) -> Option<(u16, u8)> {
    let i = (c as u32).checked_sub(0x0621)? as usize;
    ARABIC.get(i).copied().filter(|(_, n)| *n > 0)
}

pub fn is_arabic(c: char) -> bool {
    matches!(c as u32, 0x0600..=0x06FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF)
}

/// Arabic letters in their joined forms, still in logical order; marks
/// (harakat) are left out.
pub fn shape(text: &str) -> String {
    let chars: Vec<char> = text
        .chars()
        .filter(|c| !matches!(*c as u32, 0x064B..=0x0652 | 0x0670))
        .collect();
    let joins_forward = |c: Option<&char>| {
        c.and_then(|c| letter(*c)).is_some_and(|(_, n)| n == 4) || c == Some(&'\u{0640}')
    };
    let joins_back = |c: Option<&char>| {
        c.and_then(|c| letter(*c)).is_some_and(|(_, n)| n >= 2) || c == Some(&'\u{0640}')
    };
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let prev = i.checked_sub(1).and_then(|p| chars.get(p));
        let from_prev = joins_forward(prev);
        // Lam followed by an alef: one ligature.
        if c == '\u{0644}' {
            let lig = match chars.get(i + 1) {
                Some('\u{0622}') => Some(0xFEF5),
                Some('\u{0623}') => Some(0xFEF7),
                Some('\u{0625}') => Some(0xFEF9),
                Some('\u{0627}') => Some(0xFEFB),
                _ => None,
            };
            if let Some(base) = lig {
                out.push(char::from_u32(base + u32::from(from_prev)).unwrap_or(c));
                i += 2;
                continue;
            }
        }
        match letter(c) {
            Some((iso, forms)) => {
                let to_next = forms == 4 && joins_back(chars.get(i + 1));
                let back = from_prev && forms >= 2;
                let form = match (back, to_next) {
                    (false, false) => 0,
                    (true, false) => 1,
                    (false, true) => 2,
                    (true, true) => 3,
                };
                out.push(char::from_u32(iso as u32 + form).unwrap_or(c));
            }
            None => out.push(c),
        }
        i += 1;
    }
    out
}

/// A line of words in the order they are drawn, left to right: a
/// right-to-left line has its words reversed and Arabic words' letters
/// reversed; other words (numbers, Latin) keep their own order.
pub fn visual(words: &[String], rtl: bool) -> Vec<String> {
    let mut out: Vec<String> = words
        .iter()
        .map(|w| {
            if w.chars().any(is_arabic) {
                w.chars().rev().collect()
            } else {
                w.clone()
            }
        })
        .collect();
    if rtl {
        out.reverse();
    }
    out
}
