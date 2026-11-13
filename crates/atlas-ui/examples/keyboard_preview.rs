//! Dumps the virtual keyboard (with its trackpad) to a PNG over a desk background, to review it without
//! the headset: `cargo run -p atlas-ui --example keyboard_preview -- keyboard.png`

use atlas_spatial::{Keyboard, Vec3, PAD_H, PAD_W, PAD_X, PAD_Y, SCROLL_W};
use atlas_ui::{KeyView, KeyboardPanel, KeyboardView, PadView, KB_H, KB_W};

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "keyboard.png".into());
    let kb = Keyboard::new(Vec3::ZERO, Vec3::Z);
    let keys: Vec<KeyView> =
        kb.keys().into_iter().map(|k| KeyView { label: k.label, x: k.x, y: k.y, w: k.w, h: k.h, lit: k.lit }).collect();
    let find = |l: &str| keys.iter().position(|k| k.label == l);
    // A pointed key, a just-pressed one, a finger resting on the trackpad and the corner grips in their three
    // states (idle, with the hand close and grabbed), to see them all.
    let view = KeyboardView {
        hover: find("j"),
        pressed: find("k"),
        pad: Some(PadView { x: PAD_X, y: PAD_Y, w: PAD_W, h: PAD_H, scroll_w: SCROLL_W, contacts: vec![(PAD_X - 0.02, 0.015)] }),
        keys,
        grips: [0, 1, 0, 2],
    };
    let kbd = KeyboardPanel::new().render(&view, KB_W, KB_H);

    // Composed over a wood tone: the worst case for a light outline.
    let (w, h) = (kbd.width as usize, kbd.height as usize);
    let mut rgb = vec![0u8; w * h * 4];
    for y in 0..h {
        let t = y as f32 / h as f32;
        let wood = [(150.0 + 40.0 * t) as u8, (105.0 + 30.0 * t) as u8, (60.0 + 20.0 * t) as u8];
        for x in 0..w {
            let i = (y * w + x) * 4;
            let a = kbd.rgba[i + 3] as f32 / 255.0;
            for c in 0..3 {
                rgb[i + c] = (kbd.rgba[i + c] as f32 * a + wood[c] as f32 * (1.0 - a)) as u8;
            }
            rgb[i + 3] = 255;
        }
    }
    let file = std::fs::File::create(&out).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), kbd.width, kbd.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&rgb).unwrap();
    println!("escrito {out}");
}
