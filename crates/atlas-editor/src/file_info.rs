//! The details of a file or folder to show them on a card: type, size, how long
//! ago it changed, number of lines and the first lines of the content.

/// Files larger than this are not read for lines or a preview.
pub const MAX_READ_BYTES: u64 = 64 * 1024;
/// Lines of the preview.
pub const PREVIEW_LINES: usize = 7;
/// Maximum width (characters) of each preview line.
pub const PREVIEW_WIDTH: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileInfo {
    pub name: String,
    pub is_dir: bool,
    /// Short label of the type: the extension in capitals, or `DIR`.
    pub kind: String,
    pub size: u64,
    /// Seconds since the last modification, if known.
    pub modified_ago: Option<u64>,
    /// Lines of the text file.
    pub lines: Option<usize>,
    /// Items in a folder.
    pub children: Option<usize>,
    pub preview: Vec<String>,
}

impl FileInfo {
    /// Data from the name and the type; the workspace fills in the rest.
    pub fn named(name: &str, is_dir: bool) -> Self {
        let kind = if is_dir {
            "DIR".to_string()
        } else {
            match name.rsplit_once('.') {
                Some((stem, ext)) if !stem.is_empty() && ext.len() <= 5 => ext.to_ascii_uppercase(),
                _ => "TXT".to_string(),
            }
        };
        Self { name: name.to_string(), is_dir, kind, ..Self::default() }
    }

    /// Fills in the lines and the preview from the text.
    pub fn read_text(&mut self, text: &str) {
        self.lines = Some(text.lines().count());
        self.preview = text
            .lines()
            .take(PREVIEW_LINES)
            .map(|l| clip(&l.replace('\t', "    "), PREVIEW_WIDTH))
            .collect();
    }
}

/// Trims to `max` characters, with `…` if it does not fit.
pub fn clip(text: &str, max: usize) -> String {
    let text = text.trim_end();
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut s: String = text.chars().take(max.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

/// `1,2 KB`, `340 B`...
pub fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// `hace 5 min`, `hace 3 h`...
pub fn ago(seconds: u64) -> String {
    match seconds {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{} min ago", seconds / 60),
        3600..=86_399 => format!("{} h ago", seconds / 3600),
        _ => format!("{} d ago", seconds / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_comes_from_the_extension() {
        assert_eq!(FileInfo::named("main.rs", false).kind, "RS");
        assert_eq!(FileInfo::named("script.lua", false).kind, "LUA");
        assert_eq!(FileInfo::named("Makefile", false).kind, "TXT");
        assert_eq!(FileInfo::named(".gitignore", false).kind, "TXT", "a hidden file is not an extension");
        assert_eq!(FileInfo::named("src", true).kind, "DIR");
    }

    #[test]
    fn preview_has_the_first_lines_clipped() {
        let mut info = FileInfo::named("a.rs", false);
        let long = "x".repeat(100);
        info.read_text(&format!("uno\n\tdos\n{long}\n3\n4\n5\n6\n7\n8\n9\n"));
        assert_eq!(info.lines, Some(10));
        assert_eq!(info.preview.len(), PREVIEW_LINES);
        assert_eq!(info.preview[0], "uno");
        assert_eq!(info.preview[1], "    dos", "tabs are expanded");
        assert!(info.preview[2].chars().count() == PREVIEW_WIDTH && info.preview[2].ends_with('…'));
    }

    #[test]
    fn empty_text_has_zero_lines_and_no_preview() {
        let mut info = FileInfo::named("a.txt", false);
        info.read_text("");
        assert_eq!(info.lines, Some(0));
        assert!(info.preview.is_empty());
    }

    #[test]
    fn sizes_and_ages_read_naturally() {
        assert_eq!(human_size(340), "340 B");
        assert_eq!(human_size(1229), "1.2 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(ago(10), "just now");
        assert_eq!(ago(300), "5 min ago");
        assert_eq!(ago(3 * 3600), "3 h ago");
        assert_eq!(ago(2 * 86_400), "2 d ago");
    }

    #[test]
    fn clip_keeps_short_text_and_cuts_long_text() {
        assert_eq!(clip("hola   ", 10), "hola");
        assert_eq!(clip("abcdefghij", 5), "abcd…");
    }
}
