//! State of the file browser: a folder, its entries and the selection.

/// A folder entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
}

/// Folder being shown. `dir` is relative to the workspace and
/// empty at the root; it never ends in `/`.
#[derive(Debug, Clone, Default)]
pub struct Browser {
    pub dir: String,
    pub entries: Vec<Entry>,
    pub selected: usize,
}

impl Browser {
    pub fn new(dir: impl Into<String>, entries: Vec<Entry>) -> Self {
        Self { dir: dir.into(), entries, selected: 0 }
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        self.entries.get(self.selected)
    }

    /// Moves the selection without leaving the list.
    pub fn move_selection(&mut self, delta: isize) {
        let last = self.entries.len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    pub fn select_last(&mut self) {
        self.selected = self.entries.len().saturating_sub(1);
    }

    /// Path, relative to the workspace, of an entry in this folder.
    pub fn path_of(&self, name: &str) -> String {
        join(&self.dir, name)
    }

    /// Folder containing `dir`, or `None` if it is already the root.
    pub fn parent(&self) -> Option<String> {
        if self.dir.is_empty() {
            None
        } else {
            Some(parent_of(&self.dir))
        }
    }
}

/// Joins a folder and a name with `/`.
pub fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// Folder that contains `path` (`""` if it is at the root).
pub fn parent_of(path: &str) -> String {
    path.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool) -> Entry {
        Entry { name: name.into(), is_dir }
    }

    #[test]
    fn selection_is_clamped() {
        let mut b = Browser::new("", vec![entry("a", true), entry("b", false), entry("c", false)]);
        b.move_selection(-1);
        assert_eq!(b.selected, 0);
        b.move_selection(1);
        b.move_selection(1);
        b.move_selection(5);
        assert_eq!(b.selected, 2);
        b.select_first();
        assert_eq!(b.selected, 0);
        b.select_last();
        assert_eq!(b.selected, 2);
    }

    #[test]
    fn empty_listing_does_not_panic() {
        let mut b = Browser::new("", vec![]);
        b.move_selection(3);
        b.select_last();
        assert_eq!(b.selected, 0);
        assert!(b.selected_entry().is_none());
    }

    #[test]
    fn paths_join_and_split() {
        let b = Browser::new("src/ui", vec![]);
        assert_eq!(b.path_of("a.rs"), "src/ui/a.rs");
        assert_eq!(Browser::new("", vec![]).path_of("a.rs"), "a.rs");
        assert_eq!(b.parent(), Some("src".to_string()));
        assert_eq!(Browser::new("src", vec![]).parent(), Some(String::new()));
        assert_eq!(Browser::new("", vec![]).parent(), None);
        assert_eq!(parent_of("a/b/c.rs"), "a/b");
        assert_eq!(parent_of("c.rs"), "");
    }
}
