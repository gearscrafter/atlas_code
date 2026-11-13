use ropey::Rope;

use crate::Motion;

/// Cursor position in line and column (in characters, not bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cursor {
    pub line: usize,
    pub col: usize,
}

/// Complete state of the text for undo/redo. Cloning a rope is O(1).
#[derive(Debug, Clone)]
pub struct Snapshot {
    rope: Rope,
    cursor: Cursor,
}

/// Editable text backed by a rope: O(log n) insertions and deletions.
#[derive(Debug, Clone, Default)]
pub struct Buffer {
    rope: Rope,
    cursor: Cursor,
    /// Desired column when moving vertically through shorter lines.
    goal_col: usize,
}

impl Buffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_text(text: &str) -> Self {
        Self { rope: Rope::from_str(text), ..Self::default() }
    }

    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// Text of a line without the final line break.
    pub fn line(&self, idx: usize) -> Option<String> {
        if idx >= self.rope.len_lines() {
            return None;
        }
        let s = self.rope.line(idx).to_string();
        Some(s.trim_end_matches(['\n', '\r']).to_string())
    }

    pub fn to_string(&self) -> String {
        self.rope.to_string()
    }

    /// Length of the line in characters, without the final line break.
    pub fn line_len(&self, line: usize) -> usize {
        self.line(line).map_or(0, |l| l.chars().count())
    }

    /// Clamps the cursor to the text. In Normal mode the cursor cannot go past the
    /// last character (`allow_past_end = false`); in Insert it can.
    pub fn clamp_cursor(&mut self, allow_past_end: bool) {
        self.cursor.line = self.cursor.line.min(self.line_count().saturating_sub(1));
        let len = self.line_len(self.cursor.line);
        let max = if allow_past_end { len } else { len.saturating_sub(1) };
        self.cursor.col = self.cursor.col.min(max);
        self.goal_col = self.cursor.col;
    }

    /// Deletes the character under the cursor (Delete / `x`). It does not join lines.
    pub fn delete_after(&mut self) {
        if self.cursor.col < self.line_len(self.cursor.line) {
            let idx = self.cursor_char_idx();
            self.rope.remove(idx..idx + 1);
        }
    }

    /// Deletes the whole current line (`dd`).
    pub fn delete_line(&mut self) {
        let line = self.cursor.line;
        let start = self.rope.line_to_char(line);
        let last = line + 1 >= self.rope.len_lines();
        let end = if last { self.rope.len_chars() } else { self.rope.line_to_char(line + 1) };
        // On the last line the previous line break is deleted too.
        let start = if last && line > 0 { start - 1 } else { start };
        self.rope.remove(start..end);
        if last && line > 0 {
            self.cursor.line = line - 1;
        }
        self.cursor.col = 0;
        self.goal_col = 0;
    }

    /// Opens an empty line below (`o`) or above (`O`) and leaves the cursor on it.
    pub fn open_line(&mut self, below: bool) {
        if below {
            self.move_cursor(Motion::LineEnd);
            self.insert_char('\n');
        } else {
            self.move_cursor(Motion::LineStart);
            self.insert_char('\n');
            self.move_cursor(Motion::Up);
        }
    }

    pub fn goto_line(&mut self, line: usize) {
        self.cursor.line = line.min(self.line_count().saturating_sub(1));
        self.cursor.col = 0;
        self.goal_col = 0;
    }

    fn cursor_char_idx(&self) -> usize {
        self.rope.line_to_char(self.cursor.line) + self.cursor.col
    }

    pub fn insert_char(&mut self, c: char) {
        let idx = self.cursor_char_idx();
        self.rope.insert_char(idx, c);
        if c == '\n' {
            self.cursor = Cursor { line: self.cursor.line + 1, col: 0 };
        } else {
            self.cursor.col += 1;
        }
        self.goal_col = self.cursor.col;
    }

    /// Deletes the character before the cursor (Backspace).
    pub fn delete_before(&mut self) {
        let idx = self.cursor_char_idx();
        if idx == 0 {
            return;
        }
        // The target column is measured before deleting, because when the lines
        // are joined the previous one comes to include the current one's text.
        let joined = (self.cursor.col == 0)
            .then(|| Cursor { line: self.cursor.line - 1, col: self.line_len(self.cursor.line - 1) });
        self.rope.remove(idx - 1..idx);
        match joined {
            Some(c) => self.cursor = c,
            None => self.cursor.col -= 1,
        }
        self.goal_col = self.cursor.col;
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot { rope: self.rope.clone(), cursor: self.cursor }
    }

    pub fn restore(&mut self, snap: Snapshot) {
        self.rope = snap.rope;
        self.cursor = snap.cursor;
        self.clamp_cursor(true);
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    /// Character (not byte) index of a line/column position.
    pub fn char_index(&self, c: Cursor) -> usize {
        let line = c.line.min(self.line_count().saturating_sub(1));
        (self.rope.line_to_char(line) + c.col.min(self.line_len(line))).min(self.rope.len_chars())
    }

    pub fn cursor_at(&self, idx: usize) -> Cursor {
        let idx = idx.min(self.rope.len_chars());
        let line = self.rope.char_to_line(idx);
        Cursor { line, col: idx - self.rope.line_to_char(line) }
    }

    pub fn set_cursor(&mut self, c: Cursor) {
        self.cursor = c;
        self.clamp_cursor(true);
    }

    /// Text between two character indices `[start, end)`.
    pub fn slice_chars(&self, start: usize, end: usize) -> String {
        let end = end.min(self.rope.len_chars());
        self.rope.slice(start.min(end)..end).to_string()
    }

    /// Deletes `[start, end)` and leaves the cursor at `start`.
    pub fn remove_chars(&mut self, start: usize, end: usize) {
        let end = end.min(self.rope.len_chars());
        if start < end {
            self.rope.remove(start..end);
        }
        self.cursor = self.cursor_at(start);
        self.goal_col = self.cursor.col;
    }

    /// Inserts text at the given index and leaves the cursor at the end of what was inserted.
    pub fn insert_str_at(&mut self, idx: usize, text: &str) {
        let idx = idx.min(self.rope.len_chars());
        self.rope.insert(idx, text);
        self.cursor = self.cursor_at(idx + text.chars().count());
        self.goal_col = self.cursor.col;
    }

    fn word_class(c: char) -> u8 {
        if c.is_whitespace() {
            0
        } else if c.is_alphanumeric() || c == '_' {
            1
        } else {
            2
        }
    }

    fn word_forward(&self) -> usize {
        let len = self.rope.len_chars();
        let mut i = self.char_index(self.cursor);
        if i >= len {
            return len;
        }
        let class = Self::word_class(self.rope.char(i));
        while i < len && class != 0 && Self::word_class(self.rope.char(i)) == class {
            i += 1;
        }
        while i < len && Self::word_class(self.rope.char(i)) == 0 {
            i += 1;
        }
        i
    }

    fn word_backward(&self) -> usize {
        let mut i = self.char_index(self.cursor);
        while i > 0 && Self::word_class(self.rope.char(i - 1)) == 0 {
            i -= 1;
        }
        if i == 0 {
            return 0;
        }
        let class = Self::word_class(self.rope.char(i - 1));
        while i > 0 && Self::word_class(self.rope.char(i - 1)) == class {
            i -= 1;
        }
        i
    }

    pub fn move_cursor(&mut self, motion: Motion) {
        let last_line = self.line_count().saturating_sub(1);
        match motion {
            Motion::WordForward => {
                self.cursor = self.cursor_at(self.word_forward());
                self.goal_col = self.cursor.col;
            }
            Motion::WordBackward => {
                self.cursor = self.cursor_at(self.word_backward());
                self.goal_col = self.cursor.col;
            }
            Motion::Left => {
                self.cursor.col = self.cursor.col.saturating_sub(1);
                self.goal_col = self.cursor.col;
            }
            Motion::Right => {
                self.cursor.col = (self.cursor.col + 1).min(self.line_len(self.cursor.line));
                self.goal_col = self.cursor.col;
            }
            Motion::LineStart => {
                self.cursor.col = 0;
                self.goal_col = 0;
            }
            Motion::LineEnd => {
                self.cursor.col = self.line_len(self.cursor.line);
                self.goal_col = self.cursor.col;
            }
            Motion::Up => {
                self.cursor.line = self.cursor.line.saturating_sub(1);
                self.cursor.col = self.goal_col.min(self.line_len(self.cursor.line));
            }
            Motion::Down => {
                self.cursor.line = (self.cursor.line + 1).min(last_line);
                self.cursor.col = self.goal_col.min(self.line_len(self.cursor.line));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_motions() {
        let mut b = Buffer::from_text("foo bar_baz, qux");
        b.move_cursor(Motion::WordForward);
        assert_eq!(b.cursor().col, 4);
        b.move_cursor(Motion::WordForward);
        assert_eq!(b.cursor().col, 11); // the comma is its own word
        b.move_cursor(Motion::WordForward);
        assert_eq!(b.cursor().col, 13);
        b.move_cursor(Motion::WordBackward);
        assert_eq!(b.cursor().col, 11);
        b.move_cursor(Motion::WordBackward);
        assert_eq!(b.cursor().col, 4);
        b.move_cursor(Motion::WordBackward);
        assert_eq!(b.cursor().col, 0);
    }

    #[test]
    fn char_index_roundtrip_with_multibyte() {
        let b = Buffer::from_text("añb
cd");
        let c = Cursor { line: 1, col: 1 };
        let idx = b.char_index(c);
        assert_eq!(idx, 5);
        assert_eq!(b.cursor_at(idx), c);
        assert_eq!(b.slice_chars(1, 3), "ñb");
    }

    #[test]
    fn snapshot_restores_text_and_cursor() {
        let mut b = Buffer::from_text("abc");
        let snap = b.snapshot();
        b.move_cursor(Motion::LineEnd);
        b.insert_char('!');
        b.restore(snap);
        assert_eq!(b.to_string(), "abc");
        assert_eq!(b.cursor(), Cursor { line: 0, col: 0 });
    }

    #[test]
    fn insert_and_newline() {
        let mut b = Buffer::new();
        for c in "ab\ncd".chars() {
            b.insert_char(c);
        }
        assert_eq!(b.to_string(), "ab\ncd");
        assert_eq!(b.cursor(), Cursor { line: 1, col: 2 });
    }

    #[test]
    fn backspace_joins_lines() {
        let mut b = Buffer::from_text("ab\ncd");
        b.move_cursor(Motion::Down);
        b.move_cursor(Motion::LineStart);
        b.delete_before();
        assert_eq!(b.to_string(), "abcd");
        assert_eq!(b.cursor(), Cursor { line: 0, col: 2 });
    }

    #[test]
    fn vertical_motion_keeps_goal_column() {
        let mut b = Buffer::from_text("abcdef\nab\nabcdef");
        b.move_cursor(Motion::LineEnd);
        b.move_cursor(Motion::Down);
        assert_eq!(b.cursor().col, 2);
        b.move_cursor(Motion::Down);
        assert_eq!(b.cursor().col, 6);
    }

    #[test]
    fn multibyte_columns_are_chars() {
        let mut b = Buffer::from_text("añb");
        b.move_cursor(Motion::LineEnd);
        assert_eq!(b.cursor().col, 3);
        b.delete_before();
        assert_eq!(b.to_string(), "añ");
    }
}
