//! Dumps the panel to a PNG to review it without the headset:
//! `cargo run -p atlas-ui --example preview -- panel.png [teclas]`
//!
//! `keys` simulates key presses: each character is a key, `^` is Escape and
//! `~` is Enter. Example: `"jjjiHello^:w~"`.

use atlas_editor::{perform, Editor, Key, Workspace};
use atlas_ui::CodePanel;

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "panel.png".into());
    let keys = args.next().unwrap_or_else(|| "12j".into());

    let mut editor = Editor::new(include_str!("../../atlas-editor/src/buffer.rs"));
    editor.file = "buffer.rs".into();
    // PREVIEW_WS=<folder> lets you try the browser and the file commands.
    let ws = std::env::var("PREVIEW_WS").ok().map(Workspace::new);
    let mut count = String::new();
    for c in keys.chars() {
        if c.is_ascii_digit() && c != '0' {
            count.push(c);
            continue;
        }
        let key = match c {
            '^' => Key::Escape,
            '~' => Key::Enter,
            c => Key::Char(c),
        };
        let n = count.parse::<usize>().unwrap_or(1);
        count.clear();
        for _ in 0..n {
            let action = editor.handle_key(key);
            if let Some(ws) = &ws {
                perform(&mut editor, &action, ws);
            }
        }
    }

    let canvas = if std::env::var("PREVIEW_LOG").is_ok() {
        use atlas_editor::Progress;
        let mut log = atlas_editor::Log::default();
        log.push("> git clone https://github.com/dtolnay/anyhow.git");
        log.push("negotiating with the server");
        log.push("OK recibidos 3855 objetos");
        log.push("ERROR example of a network failure");
        log.set_progress(Some(Progress { label: "resolviendo deltas".into(), done: 1200, total: Some(2800) }));
        CodePanel::new(24.0).render_log(&log, 1536, 400)
    } else {
        CodePanel::new(30.0).render(&editor, 1536, 1024)
    };

    let file = std::fs::File::create(&out).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), canvas.width, canvas.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&canvas.rgba).unwrap();
    println!("escrito {out}");
}
