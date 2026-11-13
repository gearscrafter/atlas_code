//! Dumps the Atlas Bridge alert to a PNG to review it without the headset:
//! `cargo run -p atlas-ui --example alert_preview -- alerta.png`

use atlas_ui::{AlertPanel, AlertView, ALERT_H, ALERT_W};

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "alerta.png".into());
    let view = AlertView { title: "Atlas Bridge".into(), big: "482 913".into(), body: "On your laptop: 192.168.1.16 · waiting...".into() };
    let a = AlertPanel::new().render(&view, ALERT_W, ALERT_H);
    let (w, h) = (a.width as usize, a.height as usize);
    let mut rgb = vec![0u8; w * h * 4];
    for i in 0..w * h {
        let k = i * 4;
        let al = a.rgba[k + 3] as f32 / 255.0;
        let bg = [70.0, 60.0, 55.0];
        for c in 0..3 {
            rgb[k + c] = (a.rgba[k + c] as f32 * al + bg[c] * (1.0 - al)) as u8;
        }
        rgb[k + 3] = 255;
    }
    let file = std::fs::File::create(&out).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), a.width, a.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&rgb).unwrap();
    println!("escrito {out}");
}
