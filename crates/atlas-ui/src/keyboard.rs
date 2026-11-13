//! Atlas's own virtual keyboard: keys with a glowing outline on the desk, with a trackpad on the
//! right and a scroll strip on its edge. It is drawn on a single canvas; in the headset it is
//! a flat Quad layer on the desk.

use std::collections::HashMap;

use fontdue::{Font, FontSettings, Metrics};

use crate::canvas::{Canvas, Rgb};

const FONT: &[u8] = include_bytes!("../../../assets/fonts/RobotoMono-Regular.ttf");

/// Size (m) of the panel that the canvas covers and pixels per meter: the canvas is
/// `KB_W_M * KB_PX_PER_M` wide.
pub const KB_W_M: f32 = 0.70;
pub const KB_PX_PER_M: f32 = 2400.0;
pub const KB_W: u32 = (KB_W_M * KB_PX_PER_M) as u32;
pub const KB_H: u32 = 650;

const ORANGE: Rgb = [255, 150, 70];
const ORANGE_LIGHT: Rgb = [255, 205, 160];
const CYAN: Rgb = [120, 225, 255];
const VEIL: Rgb = [14, 8, 4];

/// A key: position of its center relative to the panel's center (X right, Y up, in meters).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KeyView {
    pub label: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Shift or Ctrl armed.
    pub lit: bool,
}

/// The trackpad: its rectangle on the panel, the width of the scroll strip on its right edge
/// and where the fingers rest now (X right, Y up, in meters from the panel's center).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PadView {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub scroll_w: f32,
    pub contacts: Vec<(f32, f32)>,
}

/// What to draw.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KeyboardView {
    pub keys: Vec<KeyView>,
    pub pad: Option<PadView>,
    /// The key the finger points at (near the panel).
    pub hover: Option<usize>,
    /// The key just pressed.
    pub pressed: Option<usize>,
    /// The grips at the plate's corners (top left, top right, bottom left, bottom right):
    /// 0 idle, 1 with the hand close, 2 grabbed.
    pub grips: [u8; 4],
}

/// Length (m) of each arm of the L-shaped corner grips, and their thickness.
const GRIP_ARM: f32 = 0.045;
const GRIP_THICK: f32 = 0.007;

pub struct KeyboardPanel {
    font: Font,
    cache: HashMap<(char, u32), (Metrics, Vec<u8>)>,
}

impl Default for KeyboardPanel {
    fn default() -> Self {
        Self::new()
    }
}

/// Signed distance (px) from `(px, py)` to a rounded rectangle centered at `(cx, cy)`:
/// negativa dentro, positiva fuera.
fn rounded_box_sdf(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let (qx, qy) = ((px - cx).abs() - (hw - r), (py - cy).abs() - (hh - r));
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - r
}

/// A rounded rectangle with a fill and a `stroke` px outline, with smoothed edges.
#[allow(clippy::too_many_arguments)]
pub(crate) fn rounded_box(c: &mut Canvas, cx: f32, cy: f32, w: f32, h: f32, radius: f32, stroke: f32, fill: Option<(Rgb, u8)>, line: (Rgb, u8)) {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let (x0, y0) = ((cx - hw).floor() as i32 - 1, (cy - hh).floor() as i32 - 1);
    for y in y0..=(cy + hh).ceil() as i32 + 1 {
        for x in x0..=(cx + hw).ceil() as i32 + 1 {
            let d = rounded_box_sdf(x as f32 + 0.5, y as f32 + 0.5, cx, cy, hw, hh, radius);
            let inside = (0.5 - d).clamp(0.0, 1.0);
            if inside <= 0.0 {
                continue;
            }
            if let Some((color, alpha)) = fill {
                c.blend_pixel(x, y, color, (alpha as f32 * inside) as u8);
            }
            // The outline is the band from `-stroke` to 0 from the edge.
            let band = inside * (0.5 + d + stroke).clamp(0.0, 1.0);
            c.blend_pixel(x, y, line.0, (line.1 as f32 * band) as u8);
        }
    }
}

/// A soft circle: the glow where a finger rests.
fn glow(c: &mut Canvas, cx: f32, cy: f32, radius: f32, color: Rgb, alpha: u8) {
    for y in (cy - radius).floor() as i32..=(cy + radius).ceil() as i32 {
        for x in (cx - radius).floor() as i32..=(cx + radius).ceil() as i32 {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt() / radius;
            if d < 1.0 {
                c.blend_pixel(x, y, color, (alpha as f32 * (1.0 - d * d)) as u8);
            }
        }
    }
}

impl KeyboardPanel {
    pub fn new() -> Self {
        Self { font: Font::from_bytes(FONT, FontSettings::default()).expect("valid embedded font"), cache: HashMap::new() }
    }

    fn glyph(&mut self, c: char, px: u32) -> &(Metrics, Vec<u8>) {
        let font = &self.font;
        self.cache.entry((c, px)).or_insert_with(|| font.rasterize(c, px as f32))
    }

    /// Writes `text` centered on `cx`, with the baseline at `baseline`.
    fn centered(&mut self, canvas: &mut Canvas, cx: i32, baseline: i32, text: &str, px: u32, color: Rgb, alpha: u8) {
        let advance = self.glyph('0', px).0.advance_width;
        let mut pen = cx as f32 - advance * text.chars().count() as f32 / 2.0;
        for c in text.chars() {
            if !c.is_whitespace() {
                let (m, coverage) = self.glyph(c, px).clone();
                let gx = pen.round() as i32 + m.xmin;
                let gy = baseline - m.ymin - m.height as i32;
                for row in 0..m.height {
                    for col in 0..m.width {
                        let a = (coverage[row * m.width + col] as u32 * alpha as u32 / 255) as u8;
                        canvas.blend_pixel(gx + col as i32, gy + row as i32, color, a);
                    }
                }
            }
            pen += advance;
        }
    }

    /// Draws the keyboard on a `w` x `h` canvas.
    pub fn render(&mut self, view: &KeyboardView, w: u32, h: u32) -> Canvas {
        let mut canvas = Canvas::new(w, h);
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        let px = |m: f32| m * KB_PX_PER_M;
        // From panel meters (Y up) to canvas pixels (Y down).
        let at = |x: f32, y: f32| (cx + px(x), cy - px(y));

        // Outline of the whole plate: keys and trackpad.
        let mut bounds: Option<(f32, f32, f32, f32)> = None;
        let mut grow = |l: f32, r: f32, t: f32, b: f32| {
            bounds = Some(match bounds {
                None => (l, r, t, b),
                Some((l0, r0, t0, b0)) => (l0.min(l), r0.max(r), t0.max(t), b0.min(b)),
            });
        };
        for k in &view.keys {
            grow(k.x - k.w / 2.0, k.x + k.w / 2.0, k.y + k.h / 2.0, k.y - k.h / 2.0);
        }
        if let Some(p) = &view.pad {
            grow(p.x - p.w / 2.0, p.x + p.w / 2.0, p.y + p.h / 2.0, p.y - p.h / 2.0);
        }
        if let Some((l, r, t, b)) = bounds {
            let pad = 0.014;
            let (mx, my) = at((l + r) / 2.0, (t + b) / 2.0);
            rounded_box(&mut canvas, mx, my, px(r - l + 2.0 * pad), px(t - b + 2.0 * pad), px(0.02), 4.0, Some((VEIL, 190)), (CYAN, 215));
            // L-shaped grips at the four corners: that is where you pinch the keyboard to move it. They light up when
            // the hand comes close and turn white while you carry it.
            let (l, r, t, b) = (l - pad, r + pad, t + pad, b - pad);
            for (i, (x, y)) in [(l, t), (r, t), (l, b), (r, b)].into_iter().enumerate() {
                let (sx, sy) = (if x < 0.0 { 1.0 } else { -1.0 }, if y > 0.0 { -1.0 } else { 1.0 });
                let (color, alpha, halo) = match view.grips[i] {
                    2 => ([255, 245, 235], 255, Some((ORANGE, 150, 70.0))),
                    1 => (ORANGE_LIGHT, 255, Some((ORANGE, 110, 55.0))),
                    _ => (CYAN, 235, None),
                };
                if let Some((hc, ha, hr)) = halo {
                    let (gx, gy) = at(x + sx * 0.012, y + sy * 0.012);
                    glow(&mut canvas, gx, gy, hr, hc, ha);
                }
                // Horizontal arm and vertical arm, with the outer edge on the plate's outline.
                let (hx, hy) = at(x + sx * GRIP_ARM / 2.0, y + sy * GRIP_THICK / 2.0);
                rounded_box(&mut canvas, hx, hy, px(GRIP_ARM), px(GRIP_THICK), px(GRIP_THICK) / 2.0, 1.0, Some((color, alpha)), (color, alpha));
                let (vx, vy) = at(x + sx * GRIP_THICK / 2.0, y + sy * GRIP_ARM / 2.0);
                rounded_box(&mut canvas, vx, vy, px(GRIP_THICK), px(GRIP_ARM), px(GRIP_THICK) / 2.0, 1.0, Some((color, alpha)), (color, alpha));
            }
        }

        for (i, key) in view.keys.iter().enumerate() {
            let (kx, ky) = at(key.x, key.y);
            let (kw, kh) = (px(key.w), px(key.h));
            let hovered = view.hover == Some(i);
            let pressed = view.pressed == Some(i);

            // Glowing outline; the key lights up as the finger approaches and fills when pressed.
            let (line, fill) = if pressed {
                ((ORANGE_LIGHT, 255), Some(([255, 236, 215], 215)))
            } else if key.lit {
                ((CYAN, 255), Some((CYAN, 70)))
            } else if hovered {
                (([255, 240, 225], 255), Some((ORANGE, 70)))
            } else {
                ((ORANGE_LIGHT, 225), Some((VEIL, 205)))
            };
            rounded_box(&mut canvas, kx, ky, kw, kh, kh * 0.18, 3.0, fill, line);

            let size = if key.label.chars().count() > 2 { 24 } else { 34 };
            let color = if pressed { [40, 20, 10] } else if key.lit || hovered { [255, 235, 210] } else { ORANGE };
            self.centered(&mut canvas, kx as i32, (ky + (size as f32) / 3.0) as i32, &key.label, size, color, 255);
        }

        if let Some(p) = &view.pad {
            let (mx, my) = at(p.x, p.y);
            let touching = !p.contacts.is_empty();
            rounded_box(&mut canvas, mx, my, px(p.w), px(p.h), px(0.016), 3.0, Some((VEIL, if touching { 215 } else { 190 })), (CYAN, if touching { 255 } else { 190 }));
            // Scroll strip: a line and two arrows on the right edge.
            let sx = at(p.x + p.w / 2.0 - p.scroll_w, 0.0).0;
            canvas.fill_rect(sx as i32, (my - px(p.h) / 2.0 + 14.0) as i32, 2, (px(p.h) - 28.0) as i32, CYAN, 150);
            let strip = at(p.x + p.w / 2.0 - p.scroll_w / 2.0, 0.0).0 as i32;
            self.centered(&mut canvas, strip, (my - px(p.h) / 4.0) as i32, "^", 26, CYAN, 220);
            self.centered(&mut canvas, strip, (my + px(p.h) / 4.0 + 10.0) as i32, "v", 26, CYAN, 220);
            for (x, y) in &p.contacts {
                let (gx, gy) = at(*x, *y);
                glow(&mut canvas, gx, gy, 46.0, CYAN, 120);
                glow(&mut canvas, gx, gy, 16.0, [255, 245, 235], 240);
            }
        }
        canvas
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> KeyboardView {
        KeyboardView {
            keys: vec![
                KeyView { label: "a".into(), x: -0.05, y: 0.0, w: 0.036, h: 0.036, lit: false },
                KeyView { label: "Enter".into(), x: 0.05, y: 0.0, w: 0.07, h: 0.036, lit: false },
            ],
            pad: Some(PadView { x: 0.25, y: 0.0, w: 0.17, h: 0.14, scroll_w: 0.03, contacts: vec![] }),
            hover: None,
            pressed: None,
            grips: [0; 4],
        }
    }

    #[test]
    fn the_corner_grips_are_drawn_and_light_up_when_a_hand_is_near() {
        let mut p = KeyboardPanel::new();
        let idle = p.render(&view(), KB_W, KB_H);
        let near = p.render(&KeyboardView { grips: [1, 0, 0, 0], ..view() }, KB_W, KB_H);
        let held = p.render(&KeyboardView { grips: [2, 0, 0, 0], ..view() }, KB_W, KB_H);
        // The plate's top-left corner (the "a" key is the leftmost and topmost).
        let corner_x = (KB_W as f32 / 2.0 + (-0.05 - 0.018 - 0.014) * KB_PX_PER_M) as usize;
        let corner_y = (KB_H as f32 / 2.0 - (0.07 + 0.014) * KB_PX_PER_M) as usize;
        let pixel = |c: &Canvas, x: usize, y: usize| {
            let i = (y * c.width as usize + x) * 4;
            [c.rgba[i], c.rgba[i + 1], c.rgba[i + 2], c.rgba[i + 3]]
        };
        // A little inside the grip's horizontal arm.
        let (x, y) = (corner_x + 40, corner_y + 6);
        assert!(pixel(&idle, x, y)[3] > 200, "the grip shows when idle: {:?}", pixel(&idle, x, y));
        assert_ne!(pixel(&idle, x, y), pixel(&near, x, y), "it changes when the hand comes close");
        assert_ne!(pixel(&near, x, y), pixel(&held, x, y), "and again when grabbed");
        // The other corners do not change (the bottom-left one, for example).
        let other = (corner_x + 40, KB_H as usize - corner_y - 6);
        assert_eq!(pixel(&idle, other.0, other.1), pixel(&near, other.0, other.1));
    }

    fn opaque(c: &Canvas, x0: i32, y0: i32, w: i32, h: i32) -> usize {
        let mut n = 0;
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                if c.rgba[((y as u32 * c.width + x as u32) * 4 + 3) as usize] > 0 {
                    n += 1;
                }
            }
        }
        n
    }

    #[test]
    fn keys_and_pad_are_drawn_where_they_say_and_the_rest_is_transparent() {
        let mut p = KeyboardPanel::new();
        let c = p.render(&view(), KB_W, KB_H);
        assert_eq!((c.width, c.height), (KB_W, KB_H));
        let (cx, cy) = (KB_W as i32 / 2, KB_H as i32 / 2);
        let s = |m: f32| (m * KB_PX_PER_M) as i32;
        assert!(opaque(&c, cx + s(-0.05) - 10, cy - 10, 20, 20) > 300, "there is a key at -5 cm");
        assert!(opaque(&c, cx + s(0.25) - 20, cy - 20, 40, 40) > 600, "there is a trackpad at +25 cm");
        assert_eq!(opaque(&c, 0, 0, 40, 40), 0, "the canvas corner is untouched");
    }

    #[test]
    fn hovering_and_pressing_change_the_drawing() {
        let mut p = KeyboardPanel::new();
        let base = p.render(&view(), KB_W, KB_H).rgba;
        let hover = p.render(&KeyboardView { hover: Some(0), ..view() }, KB_W, KB_H).rgba;
        let pressed = p.render(&KeyboardView { pressed: Some(0), ..view() }, KB_W, KB_H).rgba;
        assert_ne!(base, hover);
        assert_ne!(hover, pressed);
    }

    #[test]
    fn a_finger_on_the_pad_leaves_a_glow_there() {
        let mut p = KeyboardPanel::new();
        let mut v = view();
        let idle = p.render(&v, KB_W, KB_H);
        v.pad.as_mut().unwrap().contacts = vec![(0.24, 0.02)];
        let touched = p.render(&v, KB_W, KB_H);
        let (cx, cy) = (KB_W as f32 / 2.0, KB_H as f32 / 2.0);
        let (gx, gy) = ((cx + 0.24 * KB_PX_PER_M) as usize, (cy - 0.02 * KB_PX_PER_M) as usize);
        let i = (gy * KB_W as usize + gx) * 4;
        assert!(touched.rgba[i] > idle.rgba[i] + 100 || touched.rgba[i + 1] > idle.rgba[i + 1] + 100, "the center of the finger is light");
    }

    #[test]
    fn an_empty_keyboard_is_just_transparent() {
        let mut p = KeyboardPanel::new();
        let c = p.render(&KeyboardView::default(), 64, 64);
        assert!(c.rgba.iter().all(|&b| b == 0));
    }

    #[test]
    fn the_rounded_box_distance_is_negative_inside_and_positive_outside() {
        assert!(rounded_box_sdf(0.0, 0.0, 0.0, 0.0, 10.0, 5.0, 2.0) < 0.0);
        assert!(rounded_box_sdf(20.0, 0.0, 0.0, 0.0, 10.0, 5.0, 2.0) > 0.0);
        assert!(rounded_box_sdf(10.0, 0.0, 0.0, 0.0, 10.0, 5.0, 2.0).abs() < 1e-5, "on the edge");
    }
}
