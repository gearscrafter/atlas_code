//! File cards: almost transparent glass
//! sheets, small dense orange text, a large title in spaced capitals
//! and a bright line at the top and on the left.
//!
//! All the cards are drawn on a single canvas (a grid); in the headset each card is
//! a layer with its own crop of that texture.

use std::collections::HashMap;

use atlas_editor::{ago, human_size, FileInfo};
use fontdue::{Font, FontSettings, Metrics};

use crate::canvas::{Canvas, Rgb};

const FONT: &[u8] = include_bytes!("../../../assets/fonts/RobotoMono-Regular.ttf");

/// Columns and rows of the card grid.
pub const CARD_COLS: u32 = 4;
pub const CARD_ROWS: u32 = 3;
pub const MAX_CARDS: usize = (CARD_COLS * CARD_ROWS) as usize;

const ORANGE: Rgb = [255, 150, 70];
const ORANGE_DIM: Rgb = [245, 150, 80];
const ORANGE_LIGHT: Rgb = [255, 205, 160];
const GLASS: Rgb = [205, 228, 255];

const TITLE_PX: u32 = 38;
const BODY_PX: u32 = 18;
const SMALL_PX: u32 = 15;

/// What to draw.
#[derive(Debug, Clone, Default)]
pub struct CardsView {
    pub cards: Vec<FileInfo>,
    /// The card the hand points at, highlighted.
    pub highlighted: Option<usize>,
    /// What the title card says under its name: the current folder and where you are (`/src · 3/20`).
    pub folder: String,
}

pub struct CardsPanel {
    font: Font,
    cache: HashMap<(char, u32), (Metrics, Vec<u8>)>,
}

impl Default for CardsPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl CardsPanel {
    pub fn new() -> Self {
        Self { font: Font::from_bytes(FONT, FontSettings::default()).expect("valid embedded font"), cache: HashMap::new() }
    }

    fn glyph(&mut self, c: char, px: u32) -> &(Metrics, Vec<u8>) {
        let font = &self.font;
        self.cache.entry((c, px)).or_insert_with(|| font.rasterize(c, px as f32))
    }

    fn advance(&mut self, px: u32) -> f32 {
        self.glyph('0', px).0.advance_width
    }

    /// Writes `text` with the baseline at `baseline`; `spacing` separates the letters and `bold` repeats
    /// the stroke one pixel to the right.
    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, canvas: &mut Canvas, x: i32, baseline: i32, text: &str, px: u32, color: Rgb, alpha: u8, spacing: f32, bold: bool) {
        let advance = self.advance(px) + spacing;
        let mut pen = x as f32;
        for c in text.chars() {
            if !c.is_whitespace() {
                let (m, coverage) = self.glyph(c, px).clone();
                let gx = pen.round() as i32 + m.xmin;
                let gy = baseline - m.ymin - m.height as i32;
                for row in 0..m.height {
                    for col in 0..m.width {
                        let a = (coverage[row * m.width + col] as u32 * alpha as u32 / 255) as u8;
                        canvas.blend_pixel(gx + col as i32, gy + row as i32, color, a);
                        if bold {
                            canvas.blend_pixel(gx + col as i32 + 1, gy + row as i32, color, a);
                        }
                    }
                }
            }
            pen += advance;
        }
    }

    /// How many characters of `px` (with `spacing`) fit in `width` pixels.
    fn fits(&mut self, width: i32, px: u32, spacing: f32) -> usize {
        (width as f32 / (self.advance(px) + spacing)).floor().max(1.0) as usize
    }

    /// Draws the card grid on a `w` x `h` canvas (transparent where there is no card).
    pub fn render(&mut self, view: &CardsView, w: u32, h: u32) -> Canvas {
        let mut canvas = Canvas::new(w, h);
        let (cw, ch) = ((w / CARD_COLS) as i32, (h / CARD_ROWS) as i32);
        for (i, info) in view.cards.iter().take(MAX_CARDS).enumerate() {
            let x0 = (i as u32 % CARD_COLS) as i32 * cw;
            let y0 = (i as u32 / CARD_COLS) as i32 * ch;
            self.card(&mut canvas, info, &view.folder, view.highlighted == Some(i), x0 + 10, y0 + 10, cw - 20, ch - 20);
        }
        canvas
    }

    #[allow(clippy::too_many_arguments)]
    fn card(&mut self, c: &mut Canvas, info: &FileInfo, folder: &str, lit: bool, x: i32, y: i32, w: i32, h: i32) {
        // Glass: almost transparent, with an orange veil.
        // A dark, warm veil under the glass: the orange text also reads over a light sky.
        c.fill_rect(x, y, w, h, [38, 18, 8], if lit { 88 } else { 72 });
        c.fill_rect(x, y, w, h, GLASS, if lit { 40 } else { 22 });
        c.fill_rect(x, y, w, h, [255, 120, 40], if lit { 38 } else { 18 });
        // Border: bright at the top and left, faint at the bottom and right.
        let edge = if lit { [255, 240, 225] } else { ORANGE_LIGHT };
        c.fill_rect(x, y, w, 3, edge, 240);
        c.fill_rect(x, y, 2, h, edge, 205);
        c.fill_rect(x + w - 1, y, 1, h, ORANGE, 90);
        c.fill_rect(x, y + h - 1, w, 1, ORANGE, 90);
        // Esquinas marcadas.
        let k = 26;
        for (cx, cy, dx, dy) in [(x, y, 1, 1), (x + w - 1, y, -1, 1), (x, y + h - 1, 1, -1), (x + w - 1, y + h - 1, -1, -1)] {
            c.fill_rect(cx.min(cx + dx * k), cy, k, 3, ORANGE_LIGHT, 255);
            c.fill_rect(cx, cy.min(cy + dy * k), 3, k, ORANGE_LIGHT, 255);
        }

        let pad = 22;
        let inner = w - 2 * pad;
        // The title card is not a file: it says where you are. `..` also goes up a folder when tapped; `/` is the root.
        if info.name == ".." || info.name == "/" {
            let title = if info.name == ".." { "UP" } else { "FILES" };
            self.text(c, x + pad, y + 70, title, TITLE_PX, if lit { [255, 215, 170] } else { ORANGE }, 255, 3.0, true);
            let max = self.fits(inner, BODY_PX, 1.0);
            let shown: String = if folder.chars().count() > max {
                // What matters is the end of the path: the folder you are in.
                let tail: String = folder.chars().rev().take(max.saturating_sub(1)).collect::<Vec<_>>().into_iter().rev().collect();
                format!("…{tail}")
            } else {
                folder.to_string()
            };
            self.text(c, x + pad, y + 100, &shown, BODY_PX, ORANGE_LIGHT, 235, 1.0, false);
            c.fill_rect(x + pad, y + 116, inner, 1, ORANGE, 150);
            if info.name == ".." {
                self.text(c, x + pad, y + 150, "tap to go up", SMALL_PX, ORANGE_DIM, 220, 1.0, false);
            }
            return;
        }
        // Title: the name without extension, in capitals with spaced letters.
        let stem = if info.is_dir { info.name.as_str() } else { info.name.rsplit_once('.').map_or(info.name.as_str(), |(s, _)| s) };
        let spacing = 3.0;
        let max = self.fits(inner, TITLE_PX, spacing);
        let title: String = {
            let upper = stem.to_uppercase();
            if upper.chars().count() > max {
                let mut t: String = upper.chars().take(max.saturating_sub(1)).collect();
                t.push('…');
                t
            } else {
                upper
            }
        };
        self.text(c, x + pad, y + 56, &title, TITLE_PX, if lit { [255, 215, 170] } else { ORANGE }, 255, spacing, true);

        // Subtitle: type.
        let kind = if info.is_dir { "FOLDER".to_string() } else { format!("FILE · {}", info.kind) };
        self.text(c, x + pad, y + 84, &kind, BODY_PX, ORANGE_LIGHT, 230, 2.0, false);
        c.fill_rect(x + pad, y + 98, inner, 1, ORANGE, 150);

        // Details: faint label and light value.
        let mut rows: Vec<(&str, String)> = vec![("SIZE", human_size(info.size))];
        if let Some(s) = info.modified_ago {
            rows.push(("MODIFIED", ago(s)));
        }
        if let Some(n) = info.lines {
            rows.push(("LINES", n.to_string()));
        }
        if let Some(n) = info.children {
            rows.push(("ITEMS", n.to_string()));
        }
        let mut baseline = y + 126;
        for (label, value) in rows {
            self.text(c, x + pad, baseline, label, SMALL_PX, ORANGE_DIM, 235, 1.5, false);
            self.text(c, x + pad + 130, baseline, &value, BODY_PX, ORANGE_LIGHT, 255, 0.0, false);
            baseline += 25;
        }

        // Preview: small, dense text, like a dense data sheet.
        c.fill_rect(x + pad, baseline - 12, inner, 1, ORANGE, 110);
        baseline += 12;
        let max = self.fits(inner, SMALL_PX, 0.0);
        for line in info.preview.iter().take(7) {
            if baseline > y + h - 12 {
                break;
            }
            let clipped: String = line.chars().take(max).collect();
            self.text(c, x + pad, baseline, &clipped, SMALL_PX, ORANGE, 205, 0.0, false);
            baseline += 20;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str, is_dir: bool) -> FileInfo {
        let mut info = FileInfo::named(name, is_dir);
        info.size = 1229;
        info.modified_ago = Some(7200);
        if is_dir {
            info.children = Some(3);
        } else {
            info.read_text("fn main() {\n    println!(\"hola\");\n}\n");
        }
        info
    }

    fn opaque(c: &Canvas, x: u32, y: u32, w: u32, h: u32) -> usize {
        (y..y + h).flat_map(|yy| (x..x + w).map(move |xx| (xx, yy))).filter(|&(xx, yy)| c.rgba[((yy * c.width + xx) * 4 + 3) as usize] > 0).count()
    }

    #[test]
    fn only_cells_with_a_card_are_drawn() {
        let mut panel = CardsPanel::new();
        let view = CardsView { cards: vec![sample("main.rs", false), sample("src", true)], highlighted: None, folder: String::new() };
        let c = panel.render(&view, 2048, 1152);
        assert_eq!((c.width, c.height), (2048, 1152));
        let (cw, ch) = (2048 / CARD_COLS, 1152 / CARD_ROWS);
        assert!(opaque(&c, 0, 0, cw, ch) > 10_000, "the first card has glass and text");
        assert!(opaque(&c, cw, 0, cw, ch) > 10_000, "so does the second");
        assert_eq!(opaque(&c, 2 * cw, 0, cw, ch), 0, "a cell without a card stays transparent");
    }

    #[test]
    fn a_highlighted_card_is_brighter() {
        let mut panel = CardsPanel::new();
        let plain = panel.render(&CardsView { cards: vec![sample("a.rs", false)], highlighted: None, folder: String::new() }, 2048, 1152);
        let lit = panel.render(&CardsView { cards: vec![sample("a.rs", false)], highlighted: Some(0), folder: String::new() }, 2048, 1152);
        assert_ne!(plain.rgba, lit.rgba);
        let alpha_sum = |c: &Canvas| c.rgba.chunks_exact(4).map(|p| p[3] as u64).sum::<u64>();
        assert!(alpha_sum(&lit) > alpha_sum(&plain));
    }

    #[test]
    fn long_names_and_empty_infos_do_not_panic() {
        let mut panel = CardsPanel::new();
        let long = "un_nombre_de_archivo_larguisimo_que_no_cabe_en_la_ficha.rs";
        let view = CardsView {
            cards: vec![sample(long, false), FileInfo::named("vacio", false), FileInfo::default()],
            highlighted: Some(9),
            folder: String::new(),
        };
        let c = panel.render(&view, 2048, 1152);
        assert_eq!(c.rgba.len(), 2048 * 1152 * 4);
    }

    #[test]
    fn at_most_a_grid_of_cards_is_drawn() {
        let mut panel = CardsPanel::new();
        let cards = (0..40).map(|i| sample(&format!("f{i}.rs"), false)).collect();
        let c = panel.render(&CardsView { cards, highlighted: None, folder: String::new() }, 2048, 1152);
        assert_eq!(c.rgba.len(), 2048 * 1152 * 4);
    }

    #[test]
    fn the_title_card_shows_where_you_are_and_keeps_the_end_of_a_long_path() {
        let mut panel = CardsPanel::new();
        let up = FileInfo::named("..", true);
        let short = panel.render(&CardsView { cards: vec![up.clone()], highlighted: None, folder: "/src · 3/20".into() }, 2048, 1152);
        let other = panel.render(&CardsView { cards: vec![up.clone()], highlighted: None, folder: "/docs · 1/2".into() }, 2048, 1152);
        assert_ne!(short.rgba, other.rgba, "the title card changes with the folder");
        let long = "/".to_string() + &"carpeta/".repeat(20) + "fin";
        let c = panel.render(&CardsView { cards: vec![up], highlighted: None, folder: long }, 2048, 1152);
        assert_eq!(c.rgba.len(), 2048 * 1152 * 4, "a long path does not break it");
        let root = panel.render(&CardsView { cards: vec![FileInfo::named("/", true)], highlighted: None, folder: "/ · 1/4".into() }, 2048, 1152);
        assert_ne!(root.rgba, short.rgba, "at the root it says something else (there is nowhere to go up)");
    }
}
