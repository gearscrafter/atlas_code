use std::collections::HashMap;

use atlas_editor::{Cursor, Editor, Log, Mode, Progress};
use atlas_syntax::{Kind, Lang, Syntax};
use fontdue::{Font, FontSettings, Metrics};

use crate::canvas::{Canvas, Rgb};

const FONT: &[u8] = include_bytes!("../../../assets/fonts/RobotoMono-Regular.ttf");

/// Holographic palette: cool blue, cyan, and a
/// warm accent for the cursor.
pub struct Theme {
    pub background: (Rgb, u8),
    pub border: Rgb,
    pub title_bar: (Rgb, u8),
    pub text: Rgb,
    pub comment: Rgb,
    pub line_number: Rgb,
    pub current_line: (Rgb, u8),
    pub selection: (Rgb, u8),
    pub cursor: Rgb,
}

impl Theme {
    /// Color of a syntax category.
    pub fn kind_color(&self, kind: Kind) -> Rgb {
        match kind {
            Kind::Plain | Kind::Property => self.text,
            Kind::Comment => self.comment,
            Kind::Keyword => [120, 190, 255],
            Kind::String => [150, 235, 180],
            Kind::Number | Kind::Constant => [255, 190, 110],
            Kind::Function => [110, 235, 235],
            Kind::Type => [195, 175, 255],
            Kind::Operator => [150, 200, 220],
            Kind::Punctuation => [140, 185, 200],
            Kind::Added => [130, 240, 160],
            Kind::Removed => [255, 150, 150],
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: ([4, 18, 28], 232),
            border: [90, 220, 255],
            title_bar: ([20, 90, 120], 200),
            text: [215, 240, 250],
            // Comments and line numbers with at least 7:1 contrast even over a white wall
            // (the background is somewhat translucent); before, they were at 4.8:1 and 3.6:1.
            comment: [154, 182, 192],
            line_number: [148, 182, 197],
            current_line: ([60, 170, 210], 40),
            selection: ([255, 150, 90], 90),
            cursor: [255, 150, 90],
        }
    }
}

struct Glyph {
    metrics: Metrics,
    coverage: Vec<u8>,
}

/// Draws a code buffer on a canvas. Keeps the glyph cache
/// between frames.
pub struct CodePanel {
    font: Font,
    cache: HashMap<char, Glyph>,
    px: f32,
    /// First visible line; adjusted so the cursor is always visible.
    scroll: usize,
    browser_scroll: usize,
    syntax: Syntax,
    /// Cached highlighting: (file, text) it was computed for, and its categories.
    highlight: Option<(String, String, Vec<Kind>)>,
    pub theme: Theme,
}

impl CodePanel {
    pub fn new(font_px: f32) -> Self {
        let font = Font::from_bytes(FONT, FontSettings::default()).expect("valid embedded font");
        Self { font, cache: HashMap::new(), px: font_px, scroll: 0, browser_scroll: 0, syntax: Syntax::new(), highlight: None, theme: Theme::default() }
    }

    /// Changes the font size (px). The cached glyphs are of the previous size, so they are cleared.
    pub fn set_px(&mut self, font_px: f32) {
        if (self.px - font_px).abs() > 0.01 {
            self.px = font_px;
            self.cache.clear();
        }
    }

    fn glyph(&mut self, c: char) -> &Glyph {
        let (font, px) = (&self.font, self.px);
        self.cache.entry(c).or_insert_with(|| {
            let (metrics, coverage) = font.rasterize(c, px);
            Glyph { metrics, coverage }
        })
    }

    fn draw_text(&mut self, canvas: &mut Canvas, x: i32, baseline: i32, text: &str, color: Rgb) {
        self.draw_runs(canvas, x, baseline, text, |_| color);
    }

    /// Like `draw_text`, but the color depends on the character's position in `text`.
    fn draw_runs(
        &mut self,
        canvas: &mut Canvas,
        x: i32,
        baseline: i32,
        text: &str,
        color_at: impl Fn(usize) -> Rgb,
    ) {
        let advance = self.glyph(' ').metrics.advance_width;
        let mut pen = x as f32;
        for (i, c) in text.chars().enumerate() {
            if !c.is_whitespace() {
                let color = color_at(i);
                let g = self.glyph(c);
                let gx = pen.round() as i32 + g.metrics.xmin;
                let gy = baseline - g.metrics.ymin - g.metrics.height as i32;
                for row in 0..g.metrics.height {
                    for col in 0..g.metrics.width {
                        let a = g.coverage[row * g.metrics.width + col];
                        canvas.blend_pixel(gx + col as i32, gy + row as i32, color, a);
                    }
                }
            }
            pen += advance;
        }
    }

    /// Output panel: the last lines of the log and, at the bottom, the progress bar.
    pub fn render_log(&mut self, log: &Log, w: u32, h: u32) -> Canvas {
        let mut canvas = Canvas::new(w, h);
        let (w, h) = (w as i32, h as i32);
        let lm = self.font.horizontal_line_metrics(self.px).expect("line metrics");
        let line_h = (lm.new_line_size * 1.25).ceil() as i32;
        let advance = self.glyph(' ').metrics.advance_width;
        let pad = 24;
        let bar_h = line_h + 14;
        canvas.clear(self.theme.background.0, self.theme.background.1);
        canvas.fill_rect(0, 0, w, bar_h, self.theme.title_bar.0, self.theme.title_bar.1);
        canvas.stroke_rect(0, 0, w, h, 3, self.theme.border, 230);
        let border = self.theme.border;
        self.draw_text(&mut canvas, pad, 7 + lm.ascent as i32, "output", border);

        let max_chars = (((w - 2 * pad) as f32 / advance) as usize).max(1);
        // With progress, the bottom block reserves a text row and the bar.
        let progress_h = if log.progress().is_some() { line_h + 30 } else { 0 };
        let capacity = ((h - bar_h - progress_h - 16) / line_h).max(0) as usize;
        let skip = log.lines().len().saturating_sub(capacity);
        for (row, line) in log.lines().skip(skip).enumerate() {
            let baseline = bar_h + 10 + row as i32 * line_h + lm.ascent as i32;
            let color = if line.starts_with("ERROR") {
                self.theme.kind_color(Kind::Removed)
            } else if line.starts_with("OK") {
                self.theme.kind_color(Kind::Added)
            } else {
                self.theme.text
            };
            self.draw_text(&mut canvas, pad, baseline, &ellipsize(line, max_chars), color);
        }

        if let Some(p) = log.progress() {
            let top = h - progress_h - 8;
            let detail = progress_detail(p);
            let right = w - pad - (advance * detail.chars().count() as f32) as i32;
            let (label, text) = (ellipsize(&p.label, max_chars / 2), self.theme.text);
            let baseline = top + lm.ascent as i32;
            self.draw_text(&mut canvas, pad, baseline, &label, text);
            let numbers = self.theme.line_number;
            self.draw_text(&mut canvas, right, baseline, &detail, numbers);

            // Bar: faint background and proportional fill; without a total, a fixed strip.
            let (bx, by, bw, bh) = (pad, top + line_h + 6, w - 2 * pad, 14);
            canvas.fill_rect(bx, by, bw, bh, [90, 150, 180], 70);
            let fill = match p.percent() {
                Some(pct) => bw * pct as i32 / 100,
                None => 0,
            };
            canvas.fill_rect(bx, by, fill, bh, self.theme.border, 230);
            canvas.stroke_rect(bx, by, bw, bh, 2, self.theme.border, 160);
        }
        canvas
    }

    /// File list: folders in cyan, the selection highlighted.
    #[allow(clippy::too_many_arguments)]
    fn draw_browser(
        &mut self,
        canvas: &mut Canvas,
        editor: &Editor,
        capacity: usize,
        bar_h: i32,
        line_h: i32,
        ascent: i32,
        advance: f32,
        pad: i32,
    ) {
        let b = &editor.browser;
        // The selection always stays within the visible rows.
        if b.selected < self.browser_scroll {
            self.browser_scroll = b.selected;
        } else if b.selected >= self.browser_scroll + capacity {
            self.browser_scroll = b.selected + 1 - capacity;
        }
        if b.entries.is_empty() {
            let c = self.theme.comment;
            self.draw_text(canvas, pad, bar_h + 12 + ascent, "(empty folder)", c);
            return;
        }
        let max_chars = (((canvas.width as i32 - 2 * pad) as f32 / advance) as usize).saturating_sub(3);
        for (i, entry) in b.entries.iter().enumerate().skip(self.browser_scroll).take(capacity) {
            let top = bar_h + 12 + (i - self.browser_scroll) as i32 * line_h;
            let baseline = top + ascent;
            if i == b.selected {
                let (c, a) = self.theme.current_line;
                canvas.fill_rect(3, top, canvas.width as i32 - 6, line_h, c, a + 25);
                let cur = self.theme.cursor;
                canvas.fill_rect(3, top, 6, line_h, cur, 220);
            }
            let (name, color) = if entry.is_dir {
                (format!("{}/", entry.name), self.theme.border)
            } else {
                (entry.name.clone(), self.theme.text)
            };
            let name = ellipsize(&name, max_chars);
            self.draw_text(canvas, pad + 18, baseline, &name, color);
        }
    }

    /// Syntax categories of the editor text, recomputed only if
    /// the text or the file changed.
    fn kinds_for(&mut self, editor: &Editor) -> Vec<Kind> {
        let Some(lang) = Lang::from_filename(&editor.file) else { return Vec::new() };
        let text = editor.buffer.to_string();
        if let Some((file, cached, kinds)) = &self.highlight {
            if *file == editor.file && *cached == text {
                return kinds.clone();
            }
        }
        let kinds = self.syntax.highlight(lang, &text);
        self.highlight = Some((editor.file.clone(), text, kinds.clone()));
        kinds
    }

    /// Renders the editor: title, code, status bar and command line.
    pub fn render(&mut self, editor: &Editor, w: u32, h: u32) -> Canvas {
        let buffer = &editor.buffer;
        let mut canvas = Canvas::new(w, h);
        let (w, h) = (w as i32, h as i32);
        let lm = self.font.horizontal_line_metrics(self.px).expect("line metrics");
        let line_h = (lm.new_line_size * 1.25).ceil() as i32;
        let advance = self.glyph(' ').metrics.advance_width;
        let pad = 28;
        let bar_h = line_h + 16;
        let status_h = line_h + 14;
        // The command line is its own row, taller and darker, like in Vim.
        let cmd_h = line_h + 24;
        let bottom_h = status_h + cmd_h;

        canvas.clear(self.theme.background.0, self.theme.background.1);
        canvas.fill_rect(0, 0, w, bar_h, self.theme.title_bar.0, self.theme.title_bar.1);
        canvas.fill_rect(0, h - bottom_h, w, status_h, self.theme.title_bar.0, self.theme.title_bar.1);
        canvas.fill_rect(0, h - cmd_h, w, cmd_h, [2, 9, 14], 240);
        canvas.fill_rect(0, h - cmd_h, w, 2, self.theme.border, 200);
        canvas.stroke_rect(0, 0, w, h, 3, self.theme.border, 230);

        let border = self.theme.border;
        let marker = if editor.modified { " [+]" } else { "" };
        let title = if editor.shows_browser() {
            format!("files /{}", editor.browser.dir)
        } else {
            format!("{}{marker}", editor.file)
        };
        self.draw_text(&mut canvas, pad, 8 + lm.ascent as i32, &title, border);

        // Status bar: mode on the left, command or message, and position on the right.
        let (label, color) = match editor.mode {
            Mode::Normal => ("NORMAL", self.theme.border),
            Mode::Insert => ("INSERT", self.theme.cursor),
            Mode::Visual => ("VISUAL", self.theme.text),
            Mode::Command => ("COMMAND", self.theme.text),
            Mode::Browser => ("FILES", self.theme.border),
        };
        let status_base = h - bottom_h + 7 + lm.ascent as i32;
        self.draw_text(&mut canvas, pad, status_base, label, color);
        let cur = buffer.cursor();
        let pos = if editor.shows_browser() {
            let b = &editor.browser;
            format!("{}/{}", if b.entries.is_empty() { 0 } else { b.selected + 1 }, b.entries.len())
        } else {
            format!("{}:{}", cur.line + 1, cur.col + 1)
        };
        let right = w - pad - (advance * pos.chars().count() as f32) as i32;
        let ln_color = self.theme.line_number;
        self.draw_text(&mut canvas, right, status_base, &pos, ln_color);

        // Command line: `:` + what was typed + cursor; outside Command, the last message.
        let cmd_top = h - cmd_h + 10;
        let cmd_base = cmd_top + lm.ascent as i32;
        let cmd_chars = (((w - 2 * pad) as f32 / advance) as usize).saturating_sub(6).max(2);
        let text = self.theme.text;
        if editor.mode == Mode::Command {
            // `command_display` hides the token of `:git token ...`.
            let typed: Vec<char> = format!(":{}", editor.command_display()).chars().collect();
            // If it does not fit, the end is shown, which is where the cursor is.
            let shown: String = typed[typed.len().saturating_sub(cmd_chars)..].iter().collect();
            self.draw_text(&mut canvas, pad, cmd_base, &shown, text);
            let cx = pad + (shown.chars().count() as f32 * advance).round() as i32;
            canvas.fill_rect(cx, cmd_top, advance.ceil() as i32, line_h, self.theme.cursor, 220);
        } else {
            let info = if editor.shows_browser() && editor.message.is_empty() {
                "j/k move   Enter open   - up   : command   q close".to_string()
            } else {
                editor.message.clone()
            };
            // Vim errors (`E37: ...`) in red.
            let is_error = info.starts_with('E') && info.chars().nth(1).is_some_and(|c| c.is_ascii_digit());
            let color = if is_error { self.theme.kind_color(Kind::Removed) } else { text };
            self.draw_text(&mut canvas, pad, cmd_base, &ellipsize(&info, cmd_chars), color);
        }
        // Pending keys of a sequence (`d`, `g`, `Z`), on the right.
        if let Some(pending) = editor.pending_keys() {
            let c = self.theme.cursor;
            self.draw_text(&mut canvas, w - pad - 2 * advance as i32, cmd_base, &pending.to_string(), c);
        }

        if editor.shows_browser() {
            let capacity = ((h - bar_h - bottom_h - 16) / line_h).max(1) as usize;
            self.draw_browser(&mut canvas, editor, capacity, bar_h, line_h, lm.ascent as i32, advance, pad);
            return canvas;
        }

        let gutter = (advance * 4.0).ceil() as i32 + pad;
        // Visual line wrapping: long lines are split into several panel rows
        // without touching the text. `cols` is the usable width in characters.
        let cols = (((w - gutter - pad) as f32 / advance).floor() as usize).max(1);
        let capacity = ((h - bar_h - bottom_h - 16) / line_h).max(1) as usize;
        let rows_of = |idx: usize| -> usize {
            let len = buffer.line_len(idx);
            let mut n = len.div_ceil(cols).max(1);
            // With the cursor after the last character of a full row, another one is needed.
            if idx == cur.line && cur.col == len && len > 0 && len % cols == 0 {
                n += 1;
            }
            n
        };
        // The cursor must always stay within the visible rows.
        self.scroll = self.scroll.min(cur.line);
        while self.scroll < cur.line {
            let used: usize = (self.scroll..cur.line).map(rows_of).sum::<usize>() + cur.col / cols + 1;
            if used <= capacity {
                break;
            }
            self.scroll += 1;
        }
        let kinds = self.kinds_for(editor);
        let selection = editor.selection();
        let mut row = 0usize;
        'lines: for idx in self.scroll.. {
            let Some(line) = buffer.line(idx) else { break };
            let chars: Vec<char> = line.chars().collect();
            let line_start = buffer.char_index(Cursor { line: idx, col: 0 });
            let nrows = rows_of(idx);
            for seg in 0..nrows {
                if row >= capacity {
                    break 'lines;
                }
                let top = bar_h + 12 + row as i32 * line_h;
                let baseline = top + lm.ascent as i32;
                let from = (seg * cols).min(chars.len());
                let to = (from + cols).min(chars.len());
                let seg_text: String = chars[from..to].iter().collect();

                if let Some((sel_start, sel_end)) = selection {
                    let seg_start = line_start + from;
                    // `+ 1` also marks the line break, on the last row.
                    let limit = line_start + to + usize::from(seg + 1 == nrows);
                    if sel_start < limit && sel_end > seg_start {
                        let a = sel_start.max(seg_start) - seg_start;
                        let b = sel_end.min(limit) - seg_start;
                        let (c, alpha) = self.theme.selection;
                        let x = gutter + (a as f32 * advance).round() as i32;
                        let width = ((b - a) as f32 * advance).ceil() as i32;
                        canvas.fill_rect(x, top, width, line_h, c, alpha);
                    }
                }
                if idx == cur.line {
                    let (c, a) = self.theme.current_line;
                    canvas.fill_rect(3, top, w - 6, line_h, c, a);
                }
                if seg == 0 {
                    let ln = self.theme.line_number;
                    self.draw_text(&mut canvas, pad, baseline, &format!("{:>3}", idx + 1), ln);
                }

                let theme = &self.theme;
                let colors: Vec<Rgb> = (from..to)
                    .map(|col| theme.kind_color(kinds.get(line_start + col).copied().unwrap_or_default()))
                    .collect();
                self.draw_runs(&mut canvas, gutter, baseline, &seg_text, |i| colors[i]);

                if idx == cur.line && cur.col / cols == seg && editor.mode != Mode::Command {
                    let cx = gutter + ((cur.col % cols) as f32 * advance).round() as i32;
                    // Block in Normal/Visual, thin bar in Insert.
                    let cw = if editor.mode == Mode::Insert { 3 } else { advance.ceil() as i32 };
                    canvas.fill_rect(cx, top + 2, cw, line_h - 4, self.theme.cursor, 200);
                }
                row += 1;
            }
        }
        canvas
    }
}

/// Text to the right of the bar: `44 %  1200/2800`, or the downloaded size.
fn progress_detail(p: &Progress) -> String {
    match (p.percent(), p.total) {
        (Some(pct), Some(total)) if p.label != "listo" => format!("{pct}%  {}/{total}", p.done),
        (Some(pct), _) => format!("{pct}%"),
        (None, _) => human_bytes(p.done),
    }
}

/// `1234567` -> `1.2 MB`.
fn human_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    let n = n as f64;
    if n < KB {
        format!("{n:.0} B")
    } else if n < KB * KB {
        format!("{:.0} KB", n / KB)
    } else {
        format!("{:.1} MB", n / (KB * KB))
    }
}

/// Trims `text` to `max` characters, adding `…` if it does not fit.
fn ellipsize(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let keep = max.saturating_sub(1);
    text.chars().take(keep).chain(std::iter::once('…')).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ellipsize_fits_or_cuts() {
        assert_eq!(ellipsize("hola", 10), "hola");
        assert_eq!(ellipsize("hola mundo", 5), "hola…");
        assert_eq!(ellipsize("áéíóú", 3), "áé…");
        assert_eq!(ellipsize("x", 0), "x".chars().take(0).chain(std::iter::once('…')).collect::<String>());
    }

    #[test]
    fn render_returns_canvas_of_requested_size() {
        let mut panel = CodePanel::new(24.0);
        let editor = Editor::new("fn main() {}
");
        let canvas = panel.render(&editor, 640, 480);
        assert_eq!((canvas.width, canvas.height), (640, 480));
        assert_eq!(canvas.rgba.len(), 640 * 480 * 4);
        // The background is translucent, neither transparent nor opaque.
        let alpha = canvas.rgba[(240 * 640 + 320) * 4 + 3];
        assert!(alpha > 0 && alpha < 255, "background alpha = {alpha}");
    }

    /// Text pixels in the panel's right strip. Each row is compared with its
    /// own margin, because the current line is highlighted edge to edge.
    fn ink_in_right_margin(c: &Canvas) -> usize {
        let (w, h) = (c.width as usize, c.height as usize);
        let mut ink = 0;
        for y in 90..h - 90 {
            let reference = &c.rgba[(y * w + (w - 7)) * 4..][..4];
            for x in w - 24..w - 7 {
                if &c.rgba[(y * w + x) * 4..][..4] != reference {
                    ink += 1;
                }
            }
        }
        ink
    }

    #[test]
    fn long_lines_wrap_instead_of_overflowing() {
        let mut panel = CodePanel::new(24.0);
        let editor = Editor::new(&"abcdefghij ".repeat(60));
        let canvas = panel.render(&editor, 640, 480);
        assert_eq!(ink_in_right_margin(&canvas), 0, "the text reaches the right edge");
    }

    #[test]
    fn wrapped_cursor_stays_visible_and_moves_down_a_row() {
        let mut panel = CodePanel::new(24.0);
        let mut editor = Editor::new(&"x".repeat(300));
        let start = panel.render(&editor, 640, 480);
        editor.handle_key(atlas_editor::Key::Char('$'));
        let end = panel.render(&editor, 640, 480);
        assert_ne!(start.rgba, end.rgba, "the cursor must change rows");
        assert_eq!(ink_in_right_margin(&end), 0);
    }

    #[test]
    fn typing_exactly_to_the_edge_wraps_the_cursor_to_a_new_row() {
        let mut panel = CodePanel::new(24.0);
        let mut editor = Editor::new("");
        editor.handle_key(atlas_editor::Key::Char('i'));
        let mut previous = panel.render(&editor, 640, 480).rgba;
        for _ in 0..120 {
            editor.handle_key(atlas_editor::Key::Char('m'));
            let now = panel.render(&editor, 640, 480);
            assert_eq!(ink_in_right_margin(&now), 0, "it overflowed with {} letters", editor.buffer.to_string().len());
            previous = now.rgba;
        }
        assert!(!previous.is_empty());
    }

    #[test]
    fn browser_mode_renders_differently_and_never_panics_on_edge_cases() {
        use atlas_editor::{Browser, Entry};
        let mut panel = CodePanel::new(24.0);
        let mut editor = Editor::new("texto");
        let code = panel.render(&editor, 640, 480);

        // Empty folder.
        editor.mode = Mode::Browser;
        editor.browser = Browser::new("", vec![]);
        let empty = panel.render(&editor, 640, 480);
        assert_ne!(code.rgba, empty.rgba);

        // Many entries and the selection at the end: it must scroll without a panic.
        let entries: Vec<Entry> =
            (0..200).map(|i| Entry { name: format!("archivo_{i:03}.rs"), is_dir: i % 7 == 0 }).collect();
        editor.browser = Browser::new("src/ui", entries);
        let first = panel.render(&editor, 640, 480);
        editor.browser.select_last();
        let last = panel.render(&editor, 640, 480);
        assert_ne!(first.rgba, last.rgba, "the list must scroll");
        assert_eq!((last.width, last.height), (640, 480));

        // A very long name: it gets trimmed.
        editor.browser = Browser::new("", vec![Entry { name: "x".repeat(500), is_dir: false }]);
        panel.render(&editor, 640, 480);
    }

    #[test]
    fn human_bytes_and_progress_text() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2 KB");
        assert_eq!(human_bytes(1_572_864), "1.5 MB");
        let p = |done, total| Progress { label: "recibiendo objetos".into(), done, total };
        assert_eq!(progress_detail(&p(1200, Some(2800))), "42%  1200/2800");
        assert_eq!(progress_detail(&p(5_000_000, None)), "4.8 MB");
        let done = Progress { label: "listo".into(), done: 1, total: Some(1) };
        assert_eq!(progress_detail(&done), "100%");
    }

    #[test]
    fn log_panel_draws_lines_progress_and_survives_edge_cases() {
        let mut panel = CodePanel::new(22.0);
        let mut log = Log::default();
        let empty = panel.render_log(&log, 640, 240);
        assert_eq!((empty.width, empty.height), (640, 240));

        log.push("> git clone https://github.com/o/r.git");
        log.push("ERROR something failed");
        log.push("OK listo");
        let with_lines = panel.render_log(&log, 640, 240);
        assert_ne!(empty.rgba, with_lines.rgba);

        // The bar advances with the percentage.
        let progress = |done| Progress { label: "recibiendo objetos".into(), done, total: Some(100) };
        log.set_progress(Some(progress(10)));
        let low = panel.render_log(&log, 640, 240);
        log.set_progress(Some(progress(90)));
        let high = panel.render_log(&log, 640, 240);
        assert_ne!(low.rgba, high.rgba);

        // Many lines and a very long one: it must not overflow or fail.
        for i in 0..500 {
            log.push(format!("line {i} {}", "x".repeat(300)));
        }
        panel.render_log(&log, 640, 120);
        panel.render_log(&log, 64, 40); // lienzo diminuto
    }

    /// Orange pixels (the cursor's color) in the bottom `rows` rows.
    fn orange_in_bottom_rows(c: &Canvas, rows: usize) -> usize {
        let (w, h) = (c.width as usize, c.height as usize);
        let mut n = 0;
        for y in h - rows..h - 4 {
            for x in 4..w - 4 {
                let px = &c.rgba[(y * w + x) * 4..][..4];
                if px[0] > 190 && px[1] < 190 && px[2] < 130 {
                    n += 1;
                }
            }
        }
        n
    }

    #[test]
    fn command_line_is_a_visible_row_with_a_cursor() {
        let mut panel = CodePanel::new(24.0);
        let mut editor = Editor::new("texto");
        let normal = panel.render(&editor, 800, 520);
        for k in ":git commit prueba".chars() {
            editor.handle_key(atlas_editor::Key::Char(k));
        }
        let typing = panel.render(&editor, 800, 520);
        assert_ne!(normal.rgba, typing.rgba);
        assert_eq!(orange_in_bottom_rows(&normal, 60), 0, "without a command there must be no cursor at the bottom");
        assert!(orange_in_bottom_rows(&typing, 60) > 100, "the command line cursor must be visible");
    }

    #[test]
    fn the_token_is_never_drawn_on_the_command_line() {
        let mut panel = CodePanel::new(24.0);
        let mut editor = Editor::new("x");
        for k in ":git token github.com ghp_visible".chars() {
            editor.handle_key(atlas_editor::Key::Char(k));
        }
        let hidden = panel.render(&editor, 800, 520);
        // The same, but with a different token of the same length: the panel must be
        // identical, because the token is not drawn.
        let mut other = Editor::new("x");
        for k in ":git token github.com zzz_ocultos".chars() {
            other.handle_key(atlas_editor::Key::Char(k));
        }
        assert_eq!("ghp_visible".len(), "zzz_ocultos".len());
        assert_eq!(hidden.rgba, panel.render(&other, 800, 520).rgba, "the token affects what is drawn");
    }

    #[test]
    fn very_long_commands_do_not_overflow_the_command_row() {
        let mut panel = CodePanel::new(24.0);
        let mut editor = Editor::new("x");
        for k in format!(":{}", "z".repeat(400)).chars() {
            editor.handle_key(atlas_editor::Key::Char(k));
        }
        let c = panel.render(&editor, 640, 400);
        assert_eq!((c.width, c.height), (640, 400));
        assert!(orange_in_bottom_rows(&c, 60) > 100, "the cursor must stay visible at the end");
    }

    #[test]
    fn errors_and_pending_keys_show_in_the_command_row() {
        let mut panel = CodePanel::new(24.0);
        let mut editor = Editor::new("a\nb");
        let plain = panel.render(&editor, 800, 520);
        editor.handle_key(atlas_editor::Key::Char('d'));
        let pending = panel.render(&editor, 800, 520);
        assert_ne!(plain.rgba, pending.rgba, "the pending key must be drawn");
        editor.message = "E37: unsaved changes".into();
        let with_error = panel.render(&editor, 800, 520);
        assert_ne!(pending.rgba, with_error.rgba);
    }

    #[test]
    fn browser_stays_visible_while_typing_a_command() {
        use atlas_editor::{Browser, Entry};
        let mut panel = CodePanel::new(24.0);
        let mut editor = Editor::new("codigo");
        editor.mode = Mode::Browser;
        editor.browser = Browser::new("src", vec![Entry { name: "main.rs".into(), is_dir: false }]);
        let browsing = panel.render(&editor, 800, 520);
        editor.handle_key(atlas_editor::Key::Char(':'));
        let typing = panel.render(&editor, 800, 520);
        // The body (above the command line) must be the same: the list stays.
        let body = 100 * 800 * 4..400 * 800 * 4;
        assert_eq!(browsing.rgba[body.clone()], typing.rgba[body]);
    }

    #[test]
    fn visual_selection_is_painted() {
        let mut panel = CodePanel::new(24.0);
        let mut editor = Editor::new("hola mundo");
        let plain = panel.render(&editor, 640, 480);
        for k in ['v', 'l', 'l', 'l'] {
            editor.handle_key(atlas_editor::Key::Char(k));
        }
        let selected = panel.render(&editor, 640, 480);
        assert_ne!(plain.rgba, selected.rgba);
    }

    /// Relative luminance and WCAG contrast (1 to 21).
    fn contrast(a: Rgb, b: Rgb) -> f32 {
        let lum = |c: Rgb| {
            let f = |v: u8| {
                let v = v as f32 / 255.0;
                if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2])
        };
        let (hi, lo) = (lum(a).max(lum(b)), lum(a).min(lum(b)));
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn every_text_color_keeps_at_least_7_to_1_even_over_a_white_wall() {
        let t = Theme::default();
        // The panel background is translucent: over a white wall it gets lighter.
        let (bg, alpha) = t.background;
        let a = alpha as f32 / 255.0;
        let over_white = [0, 1, 2].map(|k| (a * bg[k] as f32 + (1.0 - a) * 255.0).round() as u8);
        let kinds = [Kind::Keyword, Kind::String, Kind::Number, Kind::Function, Kind::Type, Kind::Operator, Kind::Punctuation, Kind::Added, Kind::Removed];
        let mut colors = vec![("text", t.text), ("comment", t.comment), ("line numbers", t.line_number)];
        colors.extend(kinds.iter().map(|k| ("syntax", t.kind_color(*k))));
        for (name, color) in colors {
            let ratio = contrast(color, over_white);
            assert!(ratio >= 7.0, "{name} {color:?}: {ratio:.1}:1 sobre {over_white:?}");
        }
    }

    #[test]
    fn changing_the_font_size_clears_the_glyph_cache_and_changes_the_layout() {
        let mut panel = CodePanel::new(30.0);
        let mut e = Editor::new("x");
        e.buffer = atlas_editor::Buffer::from_text("hola");
        let small = panel.render(&e, 600, 300);
        panel.set_px(60.0);
        let big = panel.render(&e, 600, 300);
        assert_ne!(small.rgba, big.rgba, "with another font size the panel is drawn differently");
        let ink = |c: &Canvas| c.rgba.chunks(4).filter(|p| p[3] > 0 && p[0] > 150).count();
        assert!(ink(&big) > ink(&small), "the large font has more ink");
        panel.set_px(30.0);
        assert_eq!(panel.render(&e, 600, 300).rgba, small.rgba, "and back at the previous size, the same as before");
    }
}
