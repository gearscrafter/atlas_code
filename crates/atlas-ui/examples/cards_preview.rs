//! Dumps the file cards to a PNG over a test sky, to review them without the headset:
//! `cargo run -p atlas-ui --example cards_preview -- cards.png [folder]`

use atlas_editor::Workspace;
use atlas_ui::{CardsPanel, CardsView};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "fichas.png".into());
    let dir = args.next().unwrap_or_else(|| "crates/atlas-editor/src".into());

    let ws = Workspace::new(&dir);
    let entries = ws.list_dir("").expect("readable folder");
    let cards = ws.file_infos("", &entries);
    let view = CardsView { cards, highlighted: Some(1), folder: "/ · 1/5".into() };
    let cards = CardsPanel::new().render(&view, 2048, 1152);

    // They are composed over a sky gradient, the worst case for light text.
    let (w, h) = (cards.width as usize, cards.height as usize);
    let mut rgb = vec![0u8; w * h * 4];
    for y in 0..h {
        let t = y as f32 / h as f32;
        let sky = [(150.0 + 60.0 * t) as u8, (195.0 + 35.0 * t) as u8, 245u8];
        for x in 0..w {
            let i = (y * w + x) * 4;
            let a = cards.rgba[i + 3] as f32 / 255.0;
            for c in 0..3 {
                rgb[i + c] = (cards.rgba[i + c] as f32 * a + sky[c] as f32 * (1.0 - a)) as u8;
            }
            rgb[i + 3] = 255;
        }
    }
    let file = std::fs::File::create(&out).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), cards.width, cards.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&rgb).unwrap();
    println!("escrito {out}");
}
