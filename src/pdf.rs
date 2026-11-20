//! A document version as a PDF, for signing and for records: A4 pages,
//! headings, paragraphs, lists and tables set in DejaVu Sans (Latin,
//! Cyrillic, Arabic), the label on every page, and a last block naming
//! who signs. Pending suggestions are left as they were (not applied).
//! The PDF is plain (classic cross-reference table), so signatures can
//! be added as incremental updates.

use std::sync::LazyLock;

use pdf_writer::types::{CidFontType, FontFlags, SystemInfo};
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref, Str, TextStr};
use serde_json::Value;

use crate::ttf::{self, Font};

static REGULAR: LazyLock<Font> =
    LazyLock::new(|| Font::load(include_bytes!("../fonts/DejaVuSans.ttf")));
static BOLD: LazyLock<Font> =
    LazyLock::new(|| Font::load(include_bytes!("../fonts/DejaVuSans-Bold.ttf")));

const PAGE: (f32, f32) = (595.0, 842.0);
const MARGIN: f32 = 64.0;

/// Who signs, as the last block shows them.
pub struct Signatory {
    pub name: String,
    pub role: String,
}

#[derive(Clone, Copy)]
struct Style {
    bold: bool,
    size: f32,
    gap_before: f32,
    gap_after: f32,
}

const TEXT: Style = Style {
    bold: false,
    size: 11.0,
    gap_before: 0.0,
    gap_after: 8.0,
};
const H1: Style = Style {
    bold: true,
    size: 20.0,
    gap_before: 0.0,
    gap_after: 14.0,
};
const H2: Style = Style {
    bold: true,
    size: 14.0,
    gap_before: 10.0,
    gap_after: 8.0,
};
const H3: Style = Style {
    bold: true,
    size: 12.0,
    gap_before: 8.0,
    gap_after: 6.0,
};

/// Glyphs used, per font, for widths and the text layer.
#[derive(Default)]
struct Used {
    regular: std::collections::BTreeMap<u16, char>,
    bold: std::collections::BTreeMap<u16, char>,
}

struct Writer {
    pages: Vec<Content>,
    y: f32,
    used: Used,
    label: Option<String>,
}

fn font(bold: bool) -> &'static Font {
    if bold {
        &BOLD
    } else {
        &REGULAR
    }
}

/// Runs of text with their weight; pending insertions left out.
fn runs(node: &Value) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    fn walk(n: &Value, out: &mut Vec<(String, bool)>) {
        if n["type"] == "text" {
            let marks = n["marks"].as_array().cloned().unwrap_or_default();
            if marks.iter().any(|m| m["type"] == "insertion") {
                return;
            }
            let bold = marks.iter().any(|m| m["type"] == "strong");
            out.push((n["text"].as_str().unwrap_or_default().to_string(), bold));
        } else if n["type"] == "hard_break" {
            out.push(("\n".into(), false));
        }
        for c in n["content"].as_array().into_iter().flatten() {
            walk(c, out);
        }
    }
    walk(node, &mut out);
    out
}

impl Writer {
    fn new(label: Option<String>) -> Writer {
        let mut w = Writer {
            pages: Vec::new(),
            y: 0.0,
            used: Used::default(),
            label,
        };
        w.page();
        w
    }

    fn page(&mut self) {
        self.pages.push(Content::new());
        self.y = PAGE.1 - MARGIN;
        if let Some(label) = self.label.clone() {
            self.draw(
                &label.to_uppercase(),
                MARGIN,
                PAGE.1 - 36.0,
                8.0,
                true,
                0.45,
            );
        }
    }

    fn room(&mut self, needed: f32) {
        if self.y - needed < MARGIN {
            self.page();
        }
    }

    /// Draws text (already in visual order) at a point.
    fn draw(&mut self, text: &str, x: f32, y: f32, size: f32, bold: bool, gray: f32) {
        let f = font(bold);
        let mut bytes = Vec::new();
        for c in text.chars() {
            let g = f.glyph(c);
            if bold {
                self.used.bold.insert(g, c);
            } else {
                self.used.regular.insert(g, c);
            }
            bytes.extend_from_slice(&g.to_be_bytes());
        }
        let c = self.pages.last_mut().expect("a page");
        c.set_fill_gray(gray);
        c.begin_text();
        c.set_font(Name(if bold { b"F2" } else { b"F1" }), size);
        c.set_text_matrix([1.0, 0.0, 0.0, 1.0, x, y]);
        c.show(Str(&bytes));
        c.end_text();
    }

    /// Words laid in lines within a width; each line drawn from the start
    /// side (right for right-to-left text).
    fn paragraph(
        &mut self,
        parts: &[(String, bool)],
        style: Style,
        x: f32,
        width: f32,
        prefix: Option<&str>,
    ) {
        let text: String = parts.iter().map(|(t, _)| t.as_str()).collect();
        let rtl = text
            .chars()
            .find(|c| c.is_alphabetic())
            .is_some_and(ttf::is_arabic);
        // Words with their weight (a word takes the weight of its first run).
        let mut words: Vec<(String, bool)> = Vec::new();
        for (t, bold) in parts {
            for (i, w) in t.split(' ').enumerate() {
                if i > 0 || words.is_empty() {
                    words.push((String::new(), *bold || style.bold));
                } else if let Some(last) = words.last_mut().filter(|l| l.0.is_empty()) {
                    // A run that starts a word gives it its weight.
                    last.1 = *bold || style.bold;
                }
                words.last_mut().expect("a word").0.push_str(&ttf::shape(w));
            }
        }
        words.retain(|(w, _)| !w.is_empty());
        let line_height = style.size * 1.45;
        self.y -= style.gap_before;
        let indent = if prefix.is_some() { 16.0 } else { 0.0 };
        let space = |bold: bool| font(bold).width(" ", style.size);
        let mut lines: Vec<Vec<(String, bool)>> = vec![Vec::new()];
        let mut used = 0.0;
        for (w, bold) in words {
            let ww = font(bold).width(&w, style.size);
            let line = lines.last_mut().expect("a line");
            if !line.is_empty() && used + space(bold) + ww > width - indent {
                lines.push(vec![(w, bold)]);
                used = ww;
            } else {
                if !line.is_empty() {
                    used += space(bold);
                }
                used += ww;
                line.push((w, bold));
            }
        }
        for (n, line) in lines.iter().enumerate() {
            self.room(line_height);
            self.y -= style.size;
            if n == 0 {
                if let Some(p) = prefix {
                    let px = if rtl {
                        x + width - font(false).width(p, style.size)
                    } else {
                        x
                    };
                    self.draw(p, px, self.y, style.size, false, 0.0);
                }
            }
            let texts: Vec<String> = line.iter().map(|(w, _)| w.clone()).collect();
            let order = ttf::visual(&texts, rtl);
            let weights: Vec<bool> = if rtl {
                line.iter().rev().map(|(_, b)| *b).collect()
            } else {
                line.iter().map(|(_, b)| *b).collect()
            };
            let total: f32 = order
                .iter()
                .zip(&weights)
                .map(|(w, b)| font(*b).width(w, style.size))
                .sum::<f32>()
                + space(false) * (order.len().saturating_sub(1)) as f32;
            let mut cx = if rtl {
                x + width - indent - total
            } else {
                x + indent
            };
            for (w, b) in order.iter().zip(&weights) {
                self.draw(w, cx, self.y, style.size, *b, 0.0);
                cx += font(*b).width(w, style.size) + space(false);
            }
            self.y -= line_height - style.size;
        }
        self.y -= style.gap_after;
    }

    fn block(&mut self, node: &Value, x: f32, width: f32, list: Option<(bool, usize)>) {
        match node["type"].as_str().unwrap_or_default() {
            "heading" => {
                let style = match node["attrs"]["level"].as_i64() {
                    Some(1) => H1,
                    Some(2) => H2,
                    _ => H3,
                };
                self.paragraph(&runs(node), style, x, width, None);
            }
            "paragraph" => {
                let prefix = list.map(|(ordered, n)| {
                    if ordered {
                        format!("{n}.")
                    } else {
                        "•".into()
                    }
                });
                self.paragraph(&runs(node), TEXT, x, width, prefix.as_deref());
            }
            "bullet_list" | "ordered_list" => {
                let ordered = node["type"] == "ordered_list";
                for (i, item) in node["content"].as_array().into_iter().flatten().enumerate() {
                    for (k, b) in item["content"].as_array().into_iter().flatten().enumerate() {
                        let mark = if k == 0 { Some((ordered, i + 1)) } else { None };
                        self.block(b, x + if k == 0 { 0.0 } else { 16.0 }, width, mark);
                    }
                }
            }
            "table" => self.table(node, x, width),
            "blockquote" => {
                for b in node["content"].as_array().into_iter().flatten() {
                    self.block(b, x + 16.0, width - 16.0, None);
                }
            }
            _ => {}
        }
    }

    fn table(&mut self, node: &Value, x: f32, width: f32) {
        let rows: Vec<&Value> = node["content"].as_array().into_iter().flatten().collect();
        let cols = rows
            .iter()
            .map(|r| r["content"].as_array().map_or(0, Vec::len))
            .max()
            .unwrap_or(0)
            .max(1);
        let cw = width / cols as f32;
        self.y -= 4.0;
        for row in rows {
            let cells: Vec<&Value> = row["content"].as_array().into_iter().flatten().collect();
            // Each cell is laid on its own; the row is as tall as its tallest.
            let top = self.y;
            self.room(24.0);
            let top = top.min(self.y);
            let page = self.pages.len();
            let mut bottom = top;
            for (i, cell) in cells.iter().enumerate() {
                self.y = top - 4.0;
                let header = cell["type"] == "table_header";
                for b in cell["content"].as_array().into_iter().flatten() {
                    let mut parts = runs(b);
                    if header {
                        for p in &mut parts {
                            p.1 = true;
                        }
                    }
                    self.paragraph(
                        &parts,
                        Style {
                            gap_after: 2.0,
                            size: 10.0,
                            ..TEXT
                        },
                        x + i as f32 * cw + 6.0,
                        cw - 12.0,
                        None,
                    );
                }
                bottom = bottom.min(self.y);
            }
            if self.pages.len() == page {
                let c = self.pages.last_mut().expect("a page");
                c.set_stroke_gray(0.6);
                c.set_line_width(0.5);
                c.rect(x, bottom, width, top - bottom);
                for i in 1..cols {
                    c.move_to(x + i as f32 * cw, top);
                    c.line_to(x + i as f32 * cw, bottom);
                }
                c.stroke();
            }
            self.y = bottom;
        }
        self.y -= 10.0;
    }

    fn signatories(&mut self, who: &[Signatory], title: &str) {
        if who.is_empty() {
            return;
        }
        self.room(40.0 + 54.0 * who.len().div_ceil(3) as f32);
        self.y -= 16.0;
        self.paragraph(
            &[(title.to_string(), true)],
            H3,
            MARGIN,
            PAGE.0 - 2.0 * MARGIN,
            None,
        );
        let box_w = (PAGE.0 - 2.0 * MARGIN - 16.0) / 3.0;
        for (i, s) in who.iter().enumerate() {
            if i % 3 == 0 && i > 0 {
                self.y -= 58.0;
            }
            let x = MARGIN + (i % 3) as f32 * (box_w + 8.0);
            let top = self.y;
            let c = self.pages.last_mut().expect("a page");
            c.set_stroke_gray(0.6);
            c.set_line_width(0.5);
            c.rect(x, top - 48.0, box_w, 48.0);
            c.stroke();
            let name =
                ttf::visual(&[ttf::shape(&s.name)], s.name.chars().any(ttf::is_arabic)).join(" ");
            let role =
                ttf::visual(&[ttf::shape(&s.role)], s.role.chars().any(ttf::is_arabic)).join(" ");
            self.draw(&name, x + 8.0, top - 18.0, 10.0, true, 0.0);
            self.draw(&role, x + 8.0, top - 34.0, 9.0, false, 0.4);
        }
        self.y -= 58.0;
    }
}

/// The PDF of a document: its name, its structure (ProseMirror JSON), its
/// label, and who signs it.
pub fn render(
    name: &str,
    doc: &Value,
    label: Option<&str>,
    who: &[Signatory],
    signatures_title: &str,
) -> Vec<u8> {
    let mut w = Writer::new(label.map(str::to_string));
    let width = PAGE.0 - 2.0 * MARGIN;
    for b in doc["content"].as_array().into_iter().flatten() {
        w.block(b, MARGIN, width, None);
    }
    w.signatories(who, signatures_title);
    // The page numbers' glyphs.
    for c in "0123456789 /".chars() {
        w.used.regular.insert(REGULAR.glyph(c), c);
    }

    let mut pdf = Pdf::new();
    let mut next = 1;
    let mut id = || {
        next += 1;
        Ref::new(next - 1)
    };
    let (catalog, pages, info) = (id(), id(), id());
    let mut fonts = Vec::new();
    for (bold, used) in [(false, &w.used.regular), (true, &w.used.bold)] {
        let f = font(bold);
        let (t0, cid, desc, file, cmap) = (id(), id(), id(), id(), id());
        let base = Name(if bold {
            b"DejaVuSans-Bold"
        } else {
            b"DejaVuSans"
        });
        let system = SystemInfo {
            registry: Str(b"Adobe"),
            ordering: Str(b"Identity"),
            supplement: 0,
        };
        pdf.type0_font(t0)
            .base_font(base)
            .encoding_predefined(Name(b"Identity-H"))
            .descendant_font(cid)
            .to_unicode(cmap);
        let scale = 1000.0 / f.units;
        {
            let mut c = pdf.cid_font(cid);
            c.subtype(CidFontType::Type2)
                .base_font(base)
                .system_info(system)
                .font_descriptor(desc)
                .default_width(f.advance(0) as f32 * scale)
                .cid_to_gid_map_predefined(Name(b"Identity"));
            let mut widths = c.widths();
            for g in used.keys() {
                widths.consecutive(*g, [f.advance(*g) as f32 * scale]);
            }
        }
        pdf.font_descriptor(desc)
            .name(base)
            .flags(FontFlags::NON_SYMBOLIC)
            .bbox(Rect::new(
                f.bbox[0] as f32 * scale,
                f.bbox[1] as f32 * scale,
                f.bbox[2] as f32 * scale,
                f.bbox[3] as f32 * scale,
            ))
            .italic_angle(0.0)
            .ascent(f.ascent as f32 * scale)
            .descent(f.descent as f32 * scale)
            .cap_height(f.ascent as f32 * scale * 0.7)
            .stem_v(if bold { 120.0 } else { 80.0 })
            .font_file2(file);
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(f.data, 6);
        pdf.stream(file, &packed)
            .filter(Filter::FlateDecode)
            .pair(Name(b"Length1"), f.data.len() as i32);
        let mut map = pdf_writer::types::UnicodeCmap::new(Name(b"Ortus-UCS"), system);
        for (g, c) in used {
            map.pair(*g, *c);
        }
        pdf.cmap(cmap, &map.finish());
        fonts.push(t0);
    }
    let mut kids = Vec::new();
    let count = w.pages.len();
    for (n, content) in w.pages.into_iter().enumerate() {
        let (page, stream) = (id(), id());
        let mut c = content;
        // Page numbers at the foot.
        let foot = format!("{} / {count}", n + 1);
        let mut bytes = Vec::new();
        for ch in foot.chars() {
            bytes.extend_from_slice(&REGULAR.glyph(ch).to_be_bytes());
        }
        c.set_fill_gray(0.45)
            .begin_text()
            .set_font(Name(b"F1"), 8.0)
            .set_text_matrix([1.0, 0.0, 0.0, 1.0, PAGE.0 - MARGIN - 20.0, 30.0])
            .show(Str(&bytes))
            .end_text();
        let data = miniz_oxide::deflate::compress_to_vec_zlib(&c.finish(), 6);
        pdf.stream(stream, &data).filter(Filter::FlateDecode);
        let mut p = pdf.page(page);
        p.media_box(Rect::new(0.0, 0.0, PAGE.0, PAGE.1))
            .parent(pages)
            .contents(stream);
        p.resources()
            .fonts()
            .pair(Name(b"F1"), fonts[0])
            .pair(Name(b"F2"), fonts[1]);
        p.finish();
        kids.push(page);
    }
    pdf.pages(pages)
        .kids(kids.iter().copied())
        .count(count as i32);
    pdf.catalog(catalog).pages(pages);
    pdf.document_info(info)
        .title(TextStr(name))
        .producer(TextStr("Ortus"));
    pdf.finish()
}
