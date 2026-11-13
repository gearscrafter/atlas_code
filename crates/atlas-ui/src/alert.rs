//! A floating alert: a glass sheet with a title, a large text (a code, a file name) and a
//! detail line. It is for what must not get lost in the editor's message line, like the Atlas Bridge
//! pairing code.

use std::collections::HashMap;

use fontdue::{Font, FontSettings, Metrics};

use crate::canvas::{Canvas, Rgb};
use crate::keyboard::rounded_box;

const FONT: &[u8] = include_bytes!("../../../assets/fonts/RobotoMono-Regular.ttf");

/// Canvas of the alert and its width in the headset (m).
pub const ALERT_W: u32 = 1024;
pub const ALERT_H: u32 = 360;
pub const ALERT_W_M: f32 = 0.42;

const ORANGE: Rgb = [255, 150, 70];
const ORANGE_LIGHT: Rgb = [255, 205, 160];
const CYAN: Rgb = [120, 225, 255];
const TEXT: Rgb = [215, 240, 250];

/// What the alert says.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AlertView {
    pub title: String,
    /// The main text, large (e.g. the code `482 913`).
    pub big: String,
    pub body: String,
}

pub struct AlertPanel {
    font: Font,
    cache: HashMap<(char, u32), (Metrics, Vec<u8>)>,
}

impl Default for AlertPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl AlertPanel {
    pub fn new() -> Self {
        Self { font: Font::from_bytes(FONT, FontSettings::default()).expect("valid embedded font"), cache: HashMap::new() }
    }

    fn glyph(&mut self, c: char, px: u32) -> &(Metrics, Vec<u8>) {
        let font = &self.font;
        self.cache.entry((c, px)).or_insert_with(|| font.rasterize(c, px as f32))
    }

    /// Width (px) of `text` with letters spaced `spacing` px apart.
    fn width(&mut self, text: &str, px: u32, spacing: f32) -> f32 {
        (self.glyph('0', px).0.advance_width + spacing) * text.chars().count() as f32 - spacing
    }

    /// Writes `text` starting at `x` (or centered on `x` with `center`), with the baseline at `baseline`.
    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, canvas: &mut Canvas, x: f32, baseline: i32, text: &str, px: u32, color: Rgb, spacing: f32, center: bool) {
        let advance = self.glyph('0', px).0.advance_width + spacing;
        let mut pen = if center { x - self.width(text, px, spacing) / 2.0 } else { x };
        for c in text.chars() {
            if !c.is_whitespace() {
                let (m, coverage) = self.glyph(c, px).clone();
                let gx = pen.round() as i32 + m.xmin;
                let gy = baseline - m.ymin - m.height as i32;
                for row in 0..m.height {
                    for col in 0..m.width {
                        canvas.blend_pixel(gx + col as i32, gy + row as i32, color, coverage[row * m.width + col]);
                    }
                }
            }
            pen += advance;
        }
    }

    /// Trims `text` to fit in `max` characters, with an ellipsis.
    fn fit(text: &str, max: usize) -> String {
        if text.chars().count() <= max {
            text.to_string()
        } else {
            format!("{}…", text.chars().take(max.saturating_sub(1)).collect::<String>())
        }
    }

    pub fn render(&mut self, view: &AlertView, w: u32, h: u32) -> Canvas {
        let mut canvas = Canvas::new(w, h);
        let (wf, hf) = (w as f32, h as f32);
        rounded_box(&mut canvas, wf / 2.0, hf / 2.0, wf - 8.0, hf - 8.0, 28.0, 4.0, Some(([14, 8, 4], 225)), (CYAN, 235));
        // An orange strip at the top, like the lit edge of a card.
        canvas.fill_rect(40, 26, (wf - 80.0) as i32, 3, ORANGE_LIGHT, 220);

        let pad = 44.0;
        self.text(&mut canvas, pad, 82, &Self::fit(&view.title.to_uppercase(), 34), 34, ORANGE, 3.0, false);
        // The large text: smaller if it is long, so it always fits.
        let max_chars = view.big.chars().count().max(1);
        let big_px = if max_chars <= 9 { 118 } else if max_chars <= 18 { 64 } else { 44 };
        let spacing = if max_chars <= 9 { 10.0 } else { 2.0 };
        let big = Self::fit(&view.big, 30);
        self.text(&mut canvas, wf / 2.0, 222, &big, big_px, [255, 236, 215], spacing, true);
        self.text(&mut canvas, wf / 2.0, 300, &Self::fit(&view.body, 52), 26, TEXT, 0.0, true);
        canvas
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ink(c: &Canvas, x0: u32, y0: u32, x1: u32, y1: u32) -> usize {
        let mut n = 0;
        for y in y0..y1 {
            for x in x0..x1 {
                let i = ((y * c.width + x) * 4) as usize;
                if c.rgba[i + 3] > 200 && c.rgba[i] > 200 {
                    n += 1;
                }
            }
        }
        n
    }

    #[test]
    fn the_code_is_drawn_big_in_the_middle() {
        let mut p = AlertPanel::new();
        let view = AlertView { title: "Atlas Bridge".into(), big: "482 913".into(), body: "192.168.1.16 · waiting for your laptop".into() };
        let c = p.render(&view, ALERT_W, ALERT_H);
        assert_eq!((c.width, c.height), (ALERT_W, ALERT_H));
        assert!(ink(&c, 200, 120, 824, 230) > 3000, "the code fills the center");
        let empty = p.render(&AlertView { big: String::new(), ..view.clone() }, ALERT_W, ALERT_H);
        assert!(ink(&empty, 200, 120, 824, 230) < 100, "without a code, the center stays empty");
    }

    #[test]
    fn long_texts_do_not_spill_out_of_the_card() {
        let mut p = AlertPanel::new();
        let view = AlertView { title: "x".repeat(200), big: "a_really_long_file_name_that_does_not_fit.rs".into(), body: "y".repeat(300) };
        let c = p.render(&view, ALERT_W, ALERT_H);
        // The left and right edges (outside the sheet) stay transparent.
        for y in 0..ALERT_H {
            assert_eq!(c.rgba[((y * ALERT_W) * 4 + 3) as usize], 0, "row {y}");
            assert_eq!(c.rgba[((y * ALERT_W + ALERT_W - 1) * 4 + 3) as usize], 0, "row {y}");
        }
    }

    #[test]
    fn different_alerts_draw_differently() {
        let mut p = AlertPanel::new();
        let a = p.render(&AlertView { title: "Atlas Bridge".into(), big: "111 111".into(), body: String::new() }, ALERT_W, ALERT_H);
        let b = p.render(&AlertView { title: "Atlas Bridge".into(), big: "222 222".into(), body: String::new() }, ALERT_W, ALERT_H);
        assert_ne!(a.rgba, b.rgba);
    }
}
