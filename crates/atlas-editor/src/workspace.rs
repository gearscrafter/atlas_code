//! Working directory: reading, writing and listing files, and
//! applying the editor's file actions (`:w`, `:e`, `:ls`).

use std::path::{Component, Path, PathBuf};

use crate::browser::join;
use crate::file_info::MAX_READ_BYTES;
use crate::{parent_of, Action, Browser, Editor, Entry, FileInfo, Mode};

/// Root of the workspace. Every name is resolved inside it.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolves a relative name. It rejects absolute paths and `..` so that a
    /// command like `:e ../../x` cannot leave the workspace.
    pub fn resolve(&self, name: &str) -> Result<PathBuf, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("E32: missing file name".into());
        }
        let rel = Path::new(name);
        if !rel.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)) {
            return Err(format!("E212: path not allowed: {name}"));
        }
        Ok(self.root.join(rel))
    }

    /// `Ok(None)` if the file does not exist yet.
    pub fn read(&self, name: &str) -> Result<Option<String>, String> {
        let path = self.resolve(name)?;
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("E484: cannot read {name}: {e}")),
        }
    }

    pub fn write(&self, name: &str, text: &str) -> Result<usize, String> {
        let path = self.resolve(name)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("E482: {e}"))?;
        }
        std::fs::write(&path, text).map_err(|e| format!("E514: cannot write {name}: {e}"))?;
        Ok(text.len())
    }

    /// The details of each entry of `dir` to show them on cards. What cannot be
    /// read (permissions, binary or very large files) is left without lines or a preview.
    pub fn file_infos(&self, dir: &str, entries: &[Entry]) -> Vec<FileInfo> {
        entries.iter().map(|e| self.file_info(dir, e)).collect()
    }

    fn file_info(&self, dir: &str, entry: &Entry) -> FileInfo {
        let mut info = FileInfo::named(&entry.name, entry.is_dir);
        let Ok(path) = self.resolve(&join(dir, &entry.name)) else { return info };
        let Ok(meta) = std::fs::metadata(&path) else { return info };
        info.size = meta.len();
        info.modified_ago = meta.modified().ok().and_then(|t| t.elapsed().ok()).map(|d| d.as_secs());
        if entry.is_dir {
            info.children = std::fs::read_dir(&path).ok().map(|r| r.filter_map(Result::ok).filter(|e| e.file_name() != ".git").count());
        } else if meta.len() <= MAX_READ_BYTES {
            if let Ok(text) = std::fs::read_to_string(&path) {
                info.read_text(&text);
            }
        }
        info
    }

    /// Entries of a folder (`dir` relative; empty is the root). Folders
    /// come first and then files, each group in alphabetical order, case
    /// insensitive. `.git` is hidden.
    pub fn list_dir(&self, dir: &str) -> Result<Vec<Entry>, String> {
        let path = if dir.is_empty() { self.root.clone() } else { self.resolve(dir)? };
        let read = std::fs::read_dir(&path).map_err(|e| format!("E484: cannot open {dir:?}: {e}"))?;
        let mut entries: Vec<Entry> = read
            .filter_map(Result::ok)
            .map(|e| Entry {
                name: e.file_name().to_string_lossy().into_owned(),
                is_dir: e.file_type().map(|t| t.is_dir()).unwrap_or(false),
            })
            .filter(|e| e.name != ".git")
            .collect();
        entries.sort_by(|a, b| {
            b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(entries)
    }
}

/// Runs the file action the editor requested. Returns `true` if the
/// application must close.
pub fn perform(editor: &mut Editor, action: &Action, ws: &Workspace) -> bool {
    match action {
        Action::Save => {
            save(editor, ws);
            false
        }
        Action::SaveAs(name) => {
            save_as(editor, ws, name);
            false
        }
        Action::SaveAndQuit => save(editor, ws),
        Action::Close => {
            close_to_browser(editor, ws);
            false
        }
        Action::SaveAndClose => {
            // If it could not be saved, the file stays open with the error in view.
            if save(editor, ws) {
                close_to_browser(editor, ws);
            }
            false
        }
        Action::Open(name) => {
            open(editor, ws, name);
            false
        }
        Action::Browse(dir) => {
            browse(editor, ws, dir);
            false
        }
        Action::Quit => true,
        // Git can take a while: the caller runs it, outside the input loop.
        // The caller runs it: the headset runs the scripts and git.
        Action::Git(_) | Action::Run | Action::Reload | Action::Set { .. } | Action::Bridge | Action::Send(_) | Action::SelfTest(_) | Action::Screen(_) | Action::Remote(_) | Action::Caption(_) | Action::None | Action::Redraw => false,
    }
}

/// Saves and returns whether it worked.
fn save(editor: &mut Editor, ws: &Workspace) -> bool {
    let name = editor.file.clone();
    match ws.write(&name, &editor.buffer.to_string()) {
        Ok(n) => {
            editor.set_unmodified();
            editor.message = format!("\"{name}\" written, {n} bytes");
            true
        }
        Err(e) => {
            editor.message = e;
            false
        }
    }
}

fn save_as(editor: &mut Editor, ws: &Workspace, name: &str) {
    match ws.write(name, &editor.buffer.to_string()) {
        Ok(n) => {
            editor.file = name.trim().to_string();
            editor.set_unmodified();
            editor.message = format!("\"{}\" written, {n} bytes", editor.file);
        }
        Err(e) => editor.message = e,
    }
}

/// Closes the file and shows its folder's list.
fn close_to_browser(editor: &mut Editor, ws: &Workspace) {
    let dir = parent_of(&editor.file);
    let closed = std::mem::take(&mut editor.message);
    editor.close_file();
    browse(editor, ws, &dir);
    // The notice of what just happened (e.g. "saved") must not get lost.
    if editor.message.is_empty() {
        editor.message = closed;
    }
}

fn browse(editor: &mut Editor, ws: &Workspace, dir: &str) {
    match ws.list_dir(dir) {
        Ok(entries) => {
            // When going up, the folder we came from is left selected.
            let came_from = editor.browser.dir.clone();
            let mut browser = Browser::new(dir, entries);
            if let Some(i) = browser
                .entries
                .iter()
                .position(|e| e.is_dir && browser.path_of(&e.name) == came_from)
            {
                browser.selected = i;
            }
            editor.browser = browser;
            editor.mode = Mode::Browser;
        }
        Err(e) => editor.message = e,
    }
}

fn open(editor: &mut Editor, ws: &Workspace, name: &str) {
    match ws.read(name) {
        Ok(Some(text)) => {
            editor.load(name.trim(), &text);
            editor.message = format!("\"{}\" abierto", editor.file);
        }
        Ok(None) => {
            editor.load(name.trim(), "");
            editor.message = format!("\"{}\" (new file)", editor.file);
        }
        Err(e) => editor.message = e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Key;

    /// Unique temporary folder per test, deleted at the end.
    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("atlas-ws-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Tmp(dir)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn typed(e: &mut Editor, keys: &str, ws: &Workspace) -> bool {
        let mut quit = false;
        for c in keys.chars() {
            let key = if c == '\n' { Key::Enter } else { Key::Char(c) };
            let action = e.handle_key(key);
            quit |= perform(e, &action, ws);
        }
        quit
    }

    #[test]
    fn file_infos_describe_files_and_folders() {
        let tmp = Tmp::new("infos");
        let ws = Workspace::new(&tmp.0);
        ws.write("a.rs", "fn main() {}
let x = 1;
").unwrap();
        ws.write("src/b.lua", "print(1)").unwrap();
        ws.write("big.txt", &"x".repeat(100_000)).unwrap();
        let entries = ws.list_dir("").unwrap();
        let infos = ws.file_infos("", &entries);
        let by = |n: &str| infos.iter().find(|i| i.name == n).unwrap();
        let a = by("a.rs");
        assert_eq!((a.kind.as_str(), a.size, a.lines), ("RS", 24, Some(2)));
        assert_eq!(a.preview, vec!["fn main() {}".to_string(), "let x = 1;".to_string()]);
        assert!(a.modified_ago.is_some());
        let src = by("src");
        assert!(src.is_dir && src.children == Some(1) && src.preview.is_empty());
        let big = by("big.txt");
        assert_eq!(big.size, 100_000);
        assert!(big.lines.is_none() && big.preview.is_empty(), "very large files are not read");
    }

    #[test]
    fn resolve_rejects_escapes() {
        let ws = Workspace::new("/raiz");
        assert!(ws.resolve("a/b.rs").is_ok());
        assert!(ws.resolve("./a.rs").is_ok());
        assert!(ws.resolve("../fuera").is_err());
        assert!(ws.resolve("a/../../fuera").is_err());
        assert!(ws.resolve("/etc/passwd").is_err());
        assert!(ws.resolve("   ").is_err());
    }

    #[test]
    fn write_creates_parents_and_read_roundtrips() {
        let tmp = Tmp::new("rw");
        let ws = Workspace::new(&tmp.0);
        assert_eq!(ws.read("nada.txt"), Ok(None));
        assert_eq!(ws.write("src/a.rs", "hola ñ").unwrap(), "hola ñ".len());
        assert_eq!(ws.read("src/a.rs"), Ok(Some("hola ñ".to_string())));
    }

    #[test]
    fn list_dir_puts_folders_first_hides_git_and_ignores_case() {
        let tmp = Tmp::new("ls");
        let ws = Workspace::new(&tmp.0);
        ws.write("b.rs", "").unwrap();
        ws.write("A.rs", "").unwrap();
        ws.write("src/x.rs", "").unwrap();
        ws.write("Zeta/y.rs", "").unwrap();
        ws.write(".git/config", "").unwrap();
        let names: Vec<(String, bool)> =
            ws.list_dir("").unwrap().into_iter().map(|e| (e.name, e.is_dir)).collect();
        assert_eq!(
            names,
            vec![
                ("src".to_string(), true),
                ("Zeta".to_string(), true),
                ("A.rs".to_string(), false),
                ("b.rs".to_string(), false),
            ]
        );
        assert_eq!(ws.list_dir("src").unwrap()[0].name, "x.rs");
    }

    #[test]
    fn list_dir_rejects_escapes_and_missing_dirs() {
        let tmp = Tmp::new("lsx");
        let ws = Workspace::new(&tmp.0);
        assert!(ws.list_dir("..").unwrap_err().starts_with("E212"));
        assert!(ws.list_dir("does/not/exist").unwrap_err().starts_with("E484"));
    }

    #[test]
    fn write_command_saves_and_clears_modified() {
        let tmp = Tmp::new("w");
        let ws = Workspace::new(&tmp.0);
        let mut e = Editor::new("");
        e.file = "n.txt".into();
        typed(&mut e, "ihola", &ws);
        e.handle_key(Key::Escape);
        assert!(e.modified);
        typed(&mut e, ":w\n", &ws);
        assert!(!e.modified);
        assert!(e.message.contains("written"));
        assert_eq!(ws.read("n.txt"), Ok(Some("hola".to_string())));
    }

    #[test]
    fn write_and_close_saves_then_shows_the_folder_and_keeps_the_file_on_failure() {
        let tmp = Tmp::new("wq");
        let ws = Workspace::new(&tmp.0);
        ws.write("src/ok.txt", "viejo").unwrap();
        let mut e = Editor::new("x");
        e.file = "src/ok.txt".into();
        assert!(!typed(&mut e, ":wq\n", &ws), "closing the file is not quitting the app");
        assert_eq!(ws.read("src/ok.txt"), Ok(Some("x".to_string())));
        assert_eq!(e.mode, Mode::Browser);
        assert_eq!(e.browser.dir, "src");
        assert_eq!(e.file, "");
        assert!(e.message.contains("written"), "{}", e.message);

        let mut bad = Editor::new("x");
        bad.file = "../outside.txt".into();
        assert!(!typed(&mut bad, ":wq\n", &ws));
        assert!(bad.message.starts_with("E212"));
        assert_eq!(bad.file, "../outside.txt", "if it could not save, it must not close the file");
        assert_ne!(bad.mode, Mode::Browser);
    }

    #[test]
    fn q_bang_discards_changes_and_shows_the_folder_without_quitting() {
        let tmp = Tmp::new("qbang");
        let ws = Workspace::new(&tmp.0);
        ws.write("src/a.rs", "original").unwrap();
        let mut e = Editor::new("original");
        e.file = "src/a.rs".into();
        typed(&mut e, "iCAMBIO", &ws);
        e.handle_key(Key::Escape);
        assert!(!typed(&mut e, ":q!\n", &ws), ":q! must not close the app");
        assert_eq!(e.mode, Mode::Browser);
        assert_eq!(e.browser.dir, "src");
        assert_eq!(ws.read("src/a.rs"), Ok(Some("original".to_string())), "nothing was saved");
        assert!(!e.modified);
    }

    #[test]
    fn app_exits_only_from_the_file_list_or_with_qa() {
        let tmp = Tmp::new("exit");
        let ws = Workspace::new(&tmp.0);
        ws.write("a.rs", "x").unwrap();
        let mut e = Editor::new("x");
        e.file = "a.rs".into();
        assert!(!typed(&mut e, ":q\n", &ws), "first :q: closes the file");
        assert_eq!(e.mode, Mode::Browser);
        assert!(typed(&mut e, ":q\n", &ws), "second :q, from the list: quits the app");

        let mut e = Editor::new("x");
        e.file = "a.rs".into();
        assert!(typed(&mut e, ":qa\n", &ws), ":qa sale directamente");
    }

    #[test]
    fn edit_opens_existing_and_creates_new() {
        let tmp = Tmp::new("e");
        let ws = Workspace::new(&tmp.0);
        ws.write("viejo.rs", "contenido").unwrap();
        let mut e = Editor::new("");
        typed(&mut e, ":e viejo.rs\n", &ws);
        assert_eq!(e.file, "viejo.rs");
        assert_eq!(e.buffer.to_string(), "contenido");
        typed(&mut e, ":e nuevo.rs\n", &ws);
        assert_eq!(e.file, "nuevo.rs");
        assert_eq!(e.buffer.to_string(), "");
        assert!(e.message.contains("new file"));
    }

    #[test]
    fn edit_outside_workspace_is_rejected_and_keeps_buffer() {
        let tmp = Tmp::new("esc");
        let ws = Workspace::new(&tmp.0);
        let mut e = Editor::new("intacto");
        typed(&mut e, ":e ../secreto\n", &ws);
        assert_eq!(e.buffer.to_string(), "intacto");
        assert!(e.message.starts_with("E212"));
    }

    #[test]
    fn write_as_renames_current_file() {
        let tmp = Tmp::new("wa");
        let ws = Workspace::new(&tmp.0);
        let mut e = Editor::new("datos");
        e.file = "a.txt".into();
        typed(&mut e, ":w b.txt\n", &ws);
        assert_eq!(e.file, "b.txt");
        assert_eq!(ws.read("b.txt"), Ok(Some("datos".to_string())));
        assert_eq!(ws.read("a.txt"), Ok(None));
    }

    /// Presses single keys, applying the file actions.
    fn keys(e: &mut Editor, keys: &[Key], ws: &Workspace) {
        for &k in keys {
            let action = e.handle_key(k);
            perform(e, &action, ws);
        }
    }

    fn project(tag: &str) -> (Tmp, Workspace) {
        let tmp = Tmp::new(tag);
        let ws = Workspace::new(&tmp.0);
        ws.write("README.md", "# hola").unwrap();
        ws.write("src/main.rs", "fn main() {}").unwrap();
        ws.write("src/util/mod.rs", "// util").unwrap();
        (tmp, ws)
    }

    #[test]
    fn ls_opens_browser_on_root() {
        let (_tmp, ws) = project("b1");
        let mut e = Editor::new("");
        e.file = "x.rs".into();
        typed(&mut e, ":ls\n", &ws);
        assert_eq!(e.mode, Mode::Browser);
        assert_eq!(e.browser.dir, "");
        let names: Vec<&str> = e.browser.entries.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, vec!["src", "README.md"]);
    }

    #[test]
    fn browser_opens_from_the_folder_of_the_current_file() {
        let (_tmp, ws) = project("b2");
        let mut e = Editor::new("");
        e.file = "src/main.rs".into();
        typed(&mut e, ":e\n", &ws);
        assert_eq!(e.browser.dir, "src");
    }

    #[test]
    fn enter_descends_and_dash_goes_back_keeping_the_folder_selected() {
        let (_tmp, ws) = project("b3");
        let mut e = Editor::new("");
        typed(&mut e, ":ls\n", &ws);
        keys(&mut e, &[Key::Enter], &ws); // src/
        assert_eq!(e.browser.dir, "src");
        let names: Vec<&str> = e.browser.entries.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, vec!["util", "main.rs"]);
        keys(&mut e, &[Key::Enter], &ws); // util/
        assert_eq!(e.browser.dir, "src/util");
        keys(&mut e, &[Key::Char('-')], &ws);
        assert_eq!(e.browser.dir, "src");
        assert_eq!(e.browser.entries[e.browser.selected].name, "util");
        keys(&mut e, &[Key::Char('-')], &ws);
        assert_eq!(e.browser.dir, "");
        keys(&mut e, &[Key::Char('-')], &ws);
        assert!(e.message.contains("root"));
        assert_eq!(e.browser.dir, "");
    }

    #[test]
    fn enter_on_file_opens_it_and_leaves_the_browser() {
        let (_tmp, ws) = project("b4");
        let mut e = Editor::new("");
        typed(&mut e, ":ls\n", &ws);
        keys(&mut e, &[Key::Enter, Key::Char('j'), Key::Enter], &ws); // src/ -> main.rs
        assert_eq!(e.mode, Mode::Normal);
        assert_eq!(e.file, "src/main.rs");
        assert_eq!(e.buffer.to_string(), "fn main() {}");
    }

    #[test]
    fn opening_from_browser_is_refused_with_unsaved_changes() {
        let (_tmp, ws) = project("b5");
        let mut e = Editor::new("");
        typed(&mut e, "ix", &ws);
        e.handle_key(Key::Escape);
        typed(&mut e, ":ls\n", &ws);
        keys(&mut e, &[Key::Char('j'), Key::Enter], &ws); // README.md
        assert_eq!(e.mode, Mode::Browser);
        assert!(e.message.starts_with("E37"));
        assert_eq!(e.buffer.to_string(), "x");
    }

    #[test]
    fn escape_closes_browser_and_selection_stays_in_range() {
        let (_tmp, ws) = project("b6");
        let mut e = Editor::new("");
        e.file = "README.md".into(); // with a file open, Esc goes back to it
        typed(&mut e, ":ls\n", &ws);
        keys(&mut e, &[Key::Char('j'), Key::Char('j'), Key::Char('j'), Key::Char('j')], &ws);
        assert_eq!(e.browser.selected, 1);
        keys(&mut e, &[Key::Char('k'), Key::Char('k'), Key::Char('k')], &ws);
        assert_eq!(e.browser.selected, 0);
        keys(&mut e, &[Key::Escape], &ws);
        assert_eq!(e.mode, Mode::Normal);

        // With no file open there is nowhere to go back to: the list stays and says so.
        let mut vacio = Editor::new("");
        typed(&mut vacio, ":ls\n", &ws);
        keys(&mut vacio, &[Key::Escape], &ws);
        assert_eq!(vacio.mode, Mode::Browser);
        assert!(vacio.message.contains(":q"), "{}", vacio.message);
    }

    #[test]
    fn browsing_a_missing_folder_reports_error_and_keeps_state() {
        let (_tmp, ws) = project("b7");
        let mut e = Editor::new("");
        perform(&mut e, &Action::Browse("does/not/exist".into()), &ws);
        assert!(e.message.starts_with("E484"));
        assert_eq!(e.mode, Mode::Normal);
    }
}
