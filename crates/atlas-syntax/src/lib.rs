//! Syntax highlighting with tree-sitter. It returns a [`Kind`] per character,
//! independent of colors and of the interface, and it is tested on the PC.

use std::collections::HashMap;

use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

/// Visual category of a character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Kind {
    #[default]
    Plain,
    Comment,
    Keyword,
    String,
    Number,
    Function,
    Type,
    Constant,
    Property,
    Operator,
    Punctuation,
    /// Line added in a diff.
    Added,
    /// Line removed in a diff.
    Removed,
}

/// Capture names we recognize, and the category each one belongs
/// to. tree-sitter looks for the longest prefix (`keyword.control` -> `keyword`).
const CAPTURES: &[(&str, Kind)] = &[
    ("attribute", Kind::Property),
    ("comment", Kind::Comment),
    ("constant", Kind::Constant),
    ("constant.builtin", Kind::Constant),
    ("constructor", Kind::Type),
    ("embedded", Kind::Plain),
    ("escape", Kind::String),
    ("function", Kind::Function),
    ("function.builtin", Kind::Function),
    ("function.method", Kind::Function),
    ("function.macro", Kind::Function),
    ("keyword", Kind::Keyword),
    ("label", Kind::Constant),
    ("module", Kind::Type),
    ("number", Kind::Number),
    ("operator", Kind::Operator),
    ("property", Kind::Property),
    ("punctuation", Kind::Punctuation),
    ("punctuation.bracket", Kind::Punctuation),
    ("punctuation.delimiter", Kind::Punctuation),
    ("string", Kind::String),
    ("string.special", Kind::String),
    ("tag", Kind::Keyword),
    ("type", Kind::Type),
    ("type.builtin", Kind::Type),
    ("variable.builtin", Kind::Constant),
];

/// Lenguajes soportados.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Rust,
    Python,
    JavaScript,
    Json,
    C,
    /// Unified patch (`git diff`). It is colored by lines, without tree-sitter.
    Diff,
}

impl Lang {
    /// Guesses the language from the file name's extension.
    pub fn from_filename(name: &str) -> Option<Lang> {
        if name == "[git diff]" {
            return Some(Lang::Diff);
        }
        let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
        Some(match ext.as_str() {
            "rs" => Lang::Rust,
            "py" => Lang::Python,
            "js" | "mjs" | "cjs" | "jsx" => Lang::JavaScript,
            "json" => Lang::Json,
            "c" | "h" => Lang::C,
            "diff" | "patch" => Lang::Diff,
            _ => return None,
        })
    }

    fn config(self) -> Result<HighlightConfiguration, String> {
        if self == Lang::Diff {
            return Err("diff does not use tree-sitter".into());
        }
        let (language, name, highlights, injections, locals) = match self {
            Lang::Rust => (
                tree_sitter_rust::LANGUAGE.into(),
                "rust",
                tree_sitter_rust::HIGHLIGHTS_QUERY,
                tree_sitter_rust::INJECTIONS_QUERY,
                "",
            ),
            Lang::Python => (
                tree_sitter_python::LANGUAGE.into(),
                "python",
                tree_sitter_python::HIGHLIGHTS_QUERY,
                "",
                "",
            ),
            Lang::JavaScript => (
                tree_sitter_javascript::LANGUAGE.into(),
                "javascript",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::INJECTIONS_QUERY,
                tree_sitter_javascript::LOCALS_QUERY,
            ),
            Lang::Json => (
                tree_sitter_json::LANGUAGE.into(),
                "json",
                tree_sitter_json::HIGHLIGHTS_QUERY,
                "",
                "",
            ),
            Lang::C => (
                tree_sitter_c::LANGUAGE.into(),
                "c",
                tree_sitter_c::HIGHLIGHT_QUERY,
                "",
                "",
            ),
            Lang::Diff => unreachable!("devuelto arriba"),
        };
        let mut cfg = HighlightConfiguration::new(language, name, highlights, injections, locals)
            .map_err(|e| format!("invalid query for {name}: {e}"))?;
        let names: Vec<&str> = CAPTURES.iter().map(|(n, _)| *n).collect();
        cfg.configure(&names);
        Ok(cfg)
    }
}

/// Highlighter with the configurations loaded on demand.
#[derive(Default)]
pub struct Syntax {
    highlighter: Highlighter,
    configs: HashMap<Lang, Option<HighlightConfiguration>>,
}

impl Syntax {
    pub fn new() -> Self {
        Self::default()
    }

    /// One category per **character** of `text`. If the language cannot be
    /// loaded or the parsing fails, everything is [`Kind::Plain`].
    pub fn highlight(&mut self, lang: Lang, text: &str) -> Vec<Kind> {
        if lang == Lang::Diff {
            return highlight_diff(text);
        }
        let plain = || vec![Kind::Plain; text.chars().count()];
        let cfg = self.configs.entry(lang).or_insert_with(|| lang.config().ok());
        let Some(cfg) = cfg.as_ref() else { return plain() };
        let Ok(events) = self.highlighter.highlight(cfg, text.as_bytes(), None, None, |_| None) else {
            return plain();
        };

        // First one category per byte, then reduced to one per character.
        let mut per_byte = vec![Kind::Plain; text.len()];
        let mut stack: Vec<Kind> = Vec::new();
        for event in events {
            match event {
                Ok(HighlightEvent::HighlightStart(h)) => stack.push(CAPTURES[h.0].1),
                Ok(HighlightEvent::HighlightEnd) => {
                    stack.pop();
                }
                Ok(HighlightEvent::Source { start, end }) => {
                    if let Some(&kind) = stack.last() {
                        per_byte[start..end].fill(kind);
                    }
                }
                Err(_) => return plain(),
            }
        }
        text.char_indices().map(|(i, _)| per_byte[i]).collect()
    }
}

/// Colors a patch by lines: added, removed, headers and hunks.
fn highlight_diff(text: &str) -> Vec<Kind> {
    let mut kinds = Vec::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let kind = if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff ") {
            Kind::Comment
        } else if line.starts_with('+') {
            Kind::Added
        } else if line.starts_with('-') {
            Kind::Removed
        } else if line.starts_with("@@") {
            Kind::Keyword
        } else if line.starts_with("index ") || line.starts_with("new file") || line.starts_with("deleted file") {
            Kind::Comment
        } else {
            Kind::Plain
        };
        kinds.extend(std::iter::repeat(kind).take(line.chars().count()));
    }
    kinds
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Category of the first character of `needle` inside `text`.
    fn kind_of(text: &str, kinds: &[Kind], needle: &str) -> Kind {
        let byte = text.find(needle).unwrap_or_else(|| panic!("{needle:?} is not in the text"));
        kinds[text[..byte].chars().count()]
    }

    #[test]
    fn language_from_extension() {
        assert_eq!(Lang::from_filename("main.rs"), Some(Lang::Rust));
        assert_eq!(Lang::from_filename("src/App.JS"), Some(Lang::JavaScript));
        assert_eq!(Lang::from_filename("a.py"), Some(Lang::Python));
        assert_eq!(Lang::from_filename("datos.json"), Some(Lang::Json));
        assert_eq!(Lang::from_filename("x.h"), Some(Lang::C));
        assert_eq!(Lang::from_filename("README"), None);
        assert_eq!(Lang::from_filename("notas.txt"), None);
    }

    #[test]
    fn rust_basics() {
        let src = "// nota\nfn main() { let n: u32 = 42; let s = \"hola\"; }\n";
        let k = Syntax::new().highlight(Lang::Rust, src);
        assert_eq!(k.len(), src.chars().count());
        assert_eq!(kind_of(src, &k, "// nota"), Kind::Comment);
        assert_eq!(kind_of(src, &k, "fn "), Kind::Keyword);
        assert_eq!(kind_of(src, &k, "main"), Kind::Function);
        assert_eq!(kind_of(src, &k, "u32"), Kind::Type);
        // The Rust grammar marks numeric literals as constants.
        assert!(matches!(kind_of(src, &k, "42"), Kind::Number | Kind::Constant));
        assert_eq!(kind_of(src, &k, "\"hola\""), Kind::String);
    }

    #[test]
    fn python_basics() {
        let src = "# x\ndef f(a):\n    return \"s\" + 1\n";
        let k = Syntax::new().highlight(Lang::Python, src);
        assert_eq!(kind_of(src, &k, "# x"), Kind::Comment);
        assert_eq!(kind_of(src, &k, "def"), Kind::Keyword);
        assert_eq!(kind_of(src, &k, "\"s\""), Kind::String);
        assert_eq!(kind_of(src, &k, "1"), Kind::Number);
    }

    #[test]
    fn javascript_basics() {
        let src = "// c\nconst x = 3; function f() { return 'a'; }\n";
        let k = Syntax::new().highlight(Lang::JavaScript, src);
        assert_eq!(kind_of(src, &k, "// c"), Kind::Comment);
        assert_eq!(kind_of(src, &k, "const"), Kind::Keyword);
        assert_eq!(kind_of(src, &k, "3"), Kind::Number);
        assert_eq!(kind_of(src, &k, "'a'"), Kind::String);
    }

    #[test]
    fn json_basics() {
        let src = "{\"clave\": 12, \"b\": true}";
        let k = Syntax::new().highlight(Lang::Json, src);
        assert_eq!(kind_of(src, &k, "12"), Kind::Number);
        assert_ne!(kind_of(src, &k, "\"clave\""), Kind::Plain);
    }

    #[test]
    fn c_basics() {
        let src = "/* c */\nint main(void) { return 0; }\n";
        let k = Syntax::new().highlight(Lang::C, src);
        assert_eq!(kind_of(src, &k, "/* c */"), Kind::Comment);
        assert_eq!(kind_of(src, &k, "return"), Kind::Keyword);
        assert_eq!(kind_of(src, &k, "0"), Kind::Number);
    }

    #[test]
    fn multibyte_text_keeps_one_kind_per_char() {
        let src = "// ñandú ✓\nlet x = \"áé\";\n";
        let k = Syntax::new().highlight(Lang::Rust, src);
        assert_eq!(k.len(), src.chars().count());
        assert_eq!(kind_of(src, &k, "ñ"), Kind::Comment);
        assert_eq!(kind_of(src, &k, "\"áé\""), Kind::String);
    }

    #[test]
    fn broken_code_does_not_panic() {
        let src = "fn ((( \"unclosed";
        let k = Syntax::new().highlight(Lang::Rust, src);
        assert_eq!(k.len(), src.chars().count());
        assert_eq!(Syntax::new().highlight(Lang::Json, "").len(), 0);
    }

    #[test]
    fn reuses_configuration_between_calls() {
        let mut s = Syntax::new();
        let a = s.highlight(Lang::Rust, "fn a() {}");
        let b = s.highlight(Lang::Rust, "fn a() {}");
        assert_eq!(a, b);
    }

    #[test]
    fn diff_lines_are_colored_by_prefix() {
        let src = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-viejo\n+nuevo\n contexto\n";
        let k = Syntax::new().highlight(Lang::Diff, src);
        assert_eq!(k.len(), src.chars().count());
        assert_eq!(kind_of(src, &k, "--- a/x"), Kind::Comment);
        assert_eq!(kind_of(src, &k, "+++ b/x"), Kind::Comment);
        assert_eq!(kind_of(src, &k, "@@"), Kind::Keyword);
        assert_eq!(kind_of(src, &k, "-viejo"), Kind::Removed);
        assert_eq!(kind_of(src, &k, "+nuevo"), Kind::Added);
        assert_eq!(kind_of(src, &k, " contexto"), Kind::Plain);
        assert_eq!(Lang::from_filename("[git diff]"), Some(Lang::Diff));
        assert_eq!(Lang::from_filename("x.patch"), Some(Lang::Diff));
    }
}
