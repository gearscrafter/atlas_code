/// RGBA8 canvas with **non-premultiplied** alpha (rows top to bottom).
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub type Rgb = [u8; 3];

impl Canvas {
    /// Lienzo totalmente transparente.
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, rgba: vec![0; (width * height * 4) as usize] }
    }

    /// Fills the whole canvas with a color (without blending).
    pub fn clear(&mut self, color: Rgb, alpha: u8) {
        for px in self.rgba.chunks_exact_mut(4) {
            px.copy_from_slice(&[color[0], color[1], color[2], alpha]);
        }
    }

    /// Composes `color` with opacity `alpha` (0..=255) over the pixel (the "over" operator).
    pub fn blend_pixel(&mut self, x: i32, y: i32, color: Rgb, alpha: u8) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 || alpha == 0 {
            return;
        }
        let i = ((y as u32 * self.width + x as u32) * 4) as usize;
        let px = &mut self.rgba[i..i + 4];
        // Empty pixel: nothing to blend, it is written directly.
        if px[3] == 0 {
            px.copy_from_slice(&[color[0], color[1], color[2], alpha]);
            return;
        }
        let sa = alpha as f32 / 255.0;
        let da = px[3] as f32 / 255.0;
        let oa = sa + da * (1.0 - sa);
        if oa <= 0.0 {
            return;
        }
        for c in 0..3 {
            let s = color[c] as f32;
            let d = px[c] as f32;
            px[c] = ((s * sa + d * da * (1.0 - sa)) / oa).round() as u8;
        }
        px[3] = (oa * 255.0).round() as u8;
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: Rgb, alpha: u8) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.blend_pixel(xx, yy, color, alpha);
            }
        }
    }

    /// Border of `t` pixels inside the rectangle.
    pub fn stroke_rect(&mut self, x: i32, y: i32, w: i32, h: i32, t: i32, color: Rgb, alpha: u8) {
        self.fill_rect(x, y, w, t, color, alpha);
        self.fill_rect(x, y + h - t, w, t, color, alpha);
        self.fill_rect(x, y + t, t, h - 2 * t, color, alpha);
        self.fill_rect(x + w - t, y + t, t, h - 2 * t, color, alpha);
    }
}
