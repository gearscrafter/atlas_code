use crate::browser::{parent_of, Browser};
use crate::{Buffer, Cursor, Log, Mode, Motion, Secret, Snapshot};

/// Maximum number of steps that can be undone.
const MAX_HISTORY: usize = 200;

/// Platform-independent keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    /// Ctrl + letter (lowercase).
    Ctrl(char),
    Enter,
    Backspace,
    Delete,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}

/// Git operation requested by a command. The caller runs it (it can take a while,
/// so it does not run inside the input loop).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitRequest {
    /// `:clone <url> [folder]`
    Clone { url: String, name: Option<String> },
    Status,
    Diff,
    Log,
    /// `:git commit <message>`: adds everything and commits.
    Commit(String),
    /// `:git fetch`: downloads the remote's changes without touching your branch.
    Fetch,
    /// `:git pull`: downloads and advances your branch if possible without merging (fast-forward).
    Pull,
    /// `:git push`: pushes the current branch to `origin`.
    Push,
    /// `:git token [host] [usuario] <token>`: guarda unas credenciales.
    SetToken { host: Option<String>, user: Option<String>, token: Secret },
    /// `:git logout [host]`: forgets a host's credentials (or all of them).
    Logout(Option<String>),
    /// `:git creds`: lists the hosts with saved credentials (never the tokens).
    Credentials,
}

/// What the caller must do after processing a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Nothing changed.
    None,
    /// It needs a redraw.
    Redraw,
    Save,
    /// Save under another name (`:w name`).
    SaveAs(String),
    /// Open or create a file (`:e name`).
    Open(String),
    /// Show the contents of a folder in the browser (`:ls`, `:e` without a name).
    Browse(String),
    /// Git operation (see [`GitRequest`]).
    Git(GitRequest),
    /// Closes the file (discarding the changes) and shows its folder's list.
    Close,
    /// Saves, closes the file and shows its folder's list.
    SaveAndClose,
    /// Runs the open file as a Lua script (`:run`).
    Run,
    /// Reloads `init.lua` and the script (`:reload`).
    Reload,
    /// Listens for Atlas Bridge on the laptop (`:bridge`).
    Bridge,
    /// Sends a file to the laptop through Atlas Bridge (`:send` sends the open one; `:send name`, that one).
    Send(String),
    /// Automated check with a test hand (`:selftest drop`), to validate gestures without hands.
    SelfTest(String),
    /// The laptop screen: `:screen here` brings it in front of you; `:screen 1.4` gives it that width (m).
    Screen(String),
    /// Control the laptop from the headset (`:remote`, `:remote on`, `:remote off`).
    Remote(String),
    /// A large caption in front of you for a few seconds (`:caption Big text | small text`), for demos and
    /// videos.
    Caption(String),
    /// Changes a reading setting (`:font`, `:panel` or `:distance`). `value` is a number, or `+n` / `-n` to
    /// add to or subtract from the current one.
    Set { name: String, value: String },
    /// Quits the application.
    Quit,
    /// Saves and quits the application.
    SaveAndQuit,
}

/// Maximum number of previous buffers remembered for going back.
const MAX_BACK: usize = 10;

/// An abandoned buffer, to be able to go back to it.
#[derive(Debug, Clone)]
struct Saved {
    file: String,
    text: String,
    cursor: Cursor,
}

/// The last thing copied or deleted (Vim's unnamed register).
#[derive(Debug, Clone, Default)]
struct Register {
    text: String,
    /// If it is whole lines (`yy`, `dd`) it is pasted on a new line.
    linewise: bool,
}

/// Modal editor: buffer + mode + command line + history.
#[derive(Debug, Clone, Default)]
pub struct Editor {
    pub buffer: Buffer,
    pub mode: Mode,
    /// Name of the open file.
    pub file: String,
    /// Text after `:` in Command mode.
    pub command: String,
    /// Mode to return to when the command line finishes (Normal or Browser).
    command_origin: Mode,
    /// Last message for the status bar.
    pub message: String,
    pub modified: bool,
    /// First key of a sequence (`dd`, `gg`, `yy`).
    pending: Option<char>,
    history: Vec<Snapshot>,
    future: Vec<Snapshot>,
    register: Register,
    /// Folder shown by the file browser (Browser mode).
    pub browser: Browser,
    /// Output of long operations (git...), in its own panel.
    pub log: Log,
    /// Previous buffers, from the oldest to the most recent.
    back: Vec<Saved>,
    /// Origin of the selection in Visual mode.
    anchor: Option<Cursor>,
    /// Whether the current Insert session has changed anything.
    insert_changed: bool,
}

impl Editor {
    pub fn new(text: &str) -> Self {
        Self { buffer: Buffer::from_text(text), ..Self::default() }
    }

    /// Opens another file or view. Resets undo, but keeps what is not
    /// part of the buffer: the output log, the copied text and the way back.
    pub fn load(&mut self, file: &str, text: &str) {
        self.remember_current();
        self.replace_buffer(file, text, Cursor::default());
    }

    /// A view (`[git status]`...) is not a file: it is neither remembered nor saved.
    pub fn is_view(&self) -> bool {
        self.file.starts_with('[')
    }

    /// Goes back to the previous buffer. Returns `false` if there is none.
    pub fn go_back(&mut self) -> bool {
        match self.back.pop() {
            Some(saved) => {
                self.replace_buffer(&saved.file, &saved.text, saved.cursor);
                true
            }
            None => false,
        }
    }

    /// Remembers the current buffer before replacing it. Unsaved changes are not
    /// remembered: forcing `:e!` discards them, as in Vim.
    fn remember_current(&mut self) {
        if self.file.is_empty() || self.modified || self.is_view() {
            return;
        }
        self.back.push(Saved {
            file: self.file.clone(),
            text: self.buffer.to_string(),
            cursor: self.buffer.cursor(),
        });
        if self.back.len() > MAX_BACK {
            self.back.remove(0);
        }
    }

    fn replace_buffer(&mut self, file: &str, text: &str, cursor: Cursor) {
        self.buffer = Buffer::from_text(text);
        self.buffer.set_cursor(cursor);
        self.buffer.clamp_cursor(false);
        self.file = file.to_string();
        self.mode = Mode::Normal;
        self.modified = false;
        self.history.clear();
        self.future.clear();
        self.pending = None;
        self.anchor = None;
        self.insert_changed = false;
        self.message.clear();
    }

    /// Leaves the editor with no file open. The caller must show the file list.
    pub fn close_file(&mut self) {
        self.replace_buffer("", "", Cursor::default());
    }

    /// Closing a view goes back to the previous file; if there is none, it opens the list.
    fn close_view(&mut self) -> Action {
        if self.go_back() {
            Action::Redraw
        } else {
            Action::Close
        }
    }

    /// `ZZ`, `:wq`, `:x`: saves and closes the file (a view is only closed).
    fn save_and_close(&mut self) -> Action {
        if self.is_view() {
            self.close_view()
        } else {
            Action::SaveAndClose
        }
    }

    /// `ZQ`: closes without saving. With no file open, it quits the app.
    fn discard_and_close(&mut self) -> Action {
        if self.is_view() {
            self.close_view()
        } else if self.file.is_empty() {
            Action::Quit
        } else {
            Action::Close
        }
    }

    /// `Backspace` or `-` in Normal mode: from a view, goes back to the file; from
    /// a file, opens its folder's list.
    fn step_back(&mut self, key: Key) -> Action {
        if self.is_view() && matches!(key, Key::Backspace | Key::Char('q')) {
            if !self.go_back() {
                self.message = "no previous buffer".into();
            }
            return Action::Redraw;
        }
        Action::Browse(parent_of(&self.file))
    }

    pub fn set_unmodified(&mut self) {
        self.modified = false;
    }

    /// Selected character range `[start, end)` in Visual mode.
    /// It includes the character under the cursor, as in Vim.
    pub fn selection(&self) -> Option<(usize, usize)> {
        if self.mode != Mode::Visual {
            return None;
        }
        let a = self.buffer.char_index(self.anchor?);
        let b = self.buffer.char_index(self.buffer.cursor());
        Some((a.min(b), (a.max(b) + 1).min(self.buffer.len_chars())))
    }

    /// Pending keys of a sequence (`d`, `g`, `y`, `Z`), to show them.
    pub fn pending_keys(&self) -> Option<char> {
        self.pending
    }

    /// The panel must show the file list: in Browser mode or while a command is
    /// typed from it.
    pub fn shows_browser(&self) -> bool {
        self.mode == Mode::Browser || (self.mode == Mode::Command && self.command_origin == Mode::Browser)
    }

    pub fn handle_key(&mut self, key: Key) -> Action {
        match key {
            // Saving works in any mode and does not change the mode.
            Key::Ctrl('s') if self.is_view() => {
                self.message = "E382: git views are read-only (:w name saves them as a file)".into();
                return Action::Redraw;
            }
            Key::Ctrl('s') => return Action::Save,
            // `Ctrl+R` redoes in Normal mode; the rest of Ctrl does nothing.
            Key::Ctrl('r') if self.mode == Mode::Normal => {}
            Key::Ctrl(c) => {
                self.message = format!(
                    "Ctrl+{}: no action here (Ctrl+S saves, Ctrl+R redoes, :q quits)",
                    c.to_ascii_uppercase()
                );
                return Action::Redraw;
            }
            _ => {}
        }
        match self.mode {
            Mode::Insert => self.insert(key),
            Mode::Command => self.command_mode(key),
            Mode::Visual => self.visual(key),
            Mode::Browser => self.browse(key),
            Mode::Normal => self.normal(key),
        }
    }

    // ---------- historial ----------

    fn checkpoint(&mut self) {
        self.history.push(self.buffer.snapshot());
        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
        }
        self.future.clear();
    }

    fn undo(&mut self) {
        match self.history.pop() {
            Some(snap) => {
                self.future.push(self.buffer.snapshot());
                self.buffer.restore(snap);
                self.buffer.clamp_cursor(false);
                self.modified = true;
                self.message = "undone".into();
            }
            None => self.message = "nothing to undo".into(),
        }
    }

    fn redo(&mut self) {
        match self.future.pop() {
            Some(snap) => {
                self.history.push(self.buffer.snapshot());
                self.buffer.restore(snap);
                self.buffer.clamp_cursor(false);
                self.modified = true;
                self.message = "redone".into();
            }
            None => self.message = "nothing to redo".into(),
        }
    }

    // ---------- modo Insert ----------

    /// Enters Insert. The whole typing session is a single undo step.
    fn begin_insert(&mut self, checkpoint: bool) {
        if checkpoint {
            self.checkpoint();
        }
        self.mode = Mode::Insert;
        // With `o`/`O` the text was already changed before entering.
        self.insert_changed = !checkpoint;
        self.message.clear();
    }

    fn motion_for(key: Key) -> Option<Motion> {
        Some(match key {
            Key::Char('h') | Key::Left => Motion::Left,
            Key::Char('l') | Key::Right => Motion::Right,
            Key::Char('j') | Key::Down => Motion::Down,
            Key::Char('k') | Key::Up => Motion::Up,
            Key::Char('0') | Key::Home => Motion::LineStart,
            Key::Char('$') | Key::End => Motion::LineEnd,
            Key::Char('w') => Motion::WordForward,
            Key::Char('b') => Motion::WordBackward,
            _ => return None,
        })
    }

    // ---------- registro ----------

    fn yank_line(&mut self) {
        let line = self.buffer.line(self.buffer.cursor().line).unwrap_or_default();
        self.register = Register { text: format!("{line}\n"), linewise: true };
    }

    fn paste(&mut self, after: bool) {
        if self.register.text.is_empty() {
            self.message = "register is empty".into();
            return;
        }
        self.checkpoint();
        let cur = self.buffer.cursor();
        let text = self.register.text.clone();
        if self.register.linewise {
            if after {
                let end = self.buffer.char_index(Cursor { line: cur.line, col: usize::MAX });
                let body = text.trim_end_matches('\n');
                self.buffer.insert_str_at(end, &format!("\n{body}"));
                self.buffer.set_cursor(Cursor { line: cur.line + 1, col: 0 });
            } else {
                let start = self.buffer.char_index(Cursor { line: cur.line, col: 0 });
                self.buffer.insert_str_at(start, &text);
                self.buffer.set_cursor(Cursor { line: cur.line, col: 0 });
            }
        } else {
            let at = self.buffer.char_index(cur);
            let at = if after && self.buffer.line_len(cur.line) > 0 { at + 1 } else { at };
            self.buffer.insert_str_at(at, &text);
            // The cursor stays on the last pasted character.
            let end = self.buffer.char_index(self.buffer.cursor());
            self.buffer.set_cursor(self.buffer.cursor_at(end.saturating_sub(1)));
        }
        self.modified = true;
    }

    // ---------- modo Normal ----------

    fn normal(&mut self, key: Key) -> Action {
        let pending = self.pending.take();
        self.message.clear();
        if let Some(m) = Self::motion_for(key) {
            self.buffer.move_cursor(m);
        } else {
            match key {
                Key::Char('G') => {
                    let last = self.buffer.line_count().saturating_sub(1);
                    self.buffer.goto_line(last);
                }
                Key::Char('g') => {
                    if pending == Some('g') {
                        self.buffer.goto_line(0);
                    } else {
                        self.pending = Some('g');
                        return Action::Redraw;
                    }
                }
                Key::Char('i') => self.begin_insert(true),
                Key::Char('a') => {
                    self.buffer.move_cursor(Motion::Right);
                    self.begin_insert(true);
                }
                Key::Char('I') => {
                    self.buffer.move_cursor(Motion::LineStart);
                    self.begin_insert(true);
                }
                Key::Char('A') => {
                    self.buffer.move_cursor(Motion::LineEnd);
                    self.begin_insert(true);
                }
                Key::Char('o') | Key::Char('O') => {
                    self.checkpoint();
                    self.buffer.open_line(key == Key::Char('o'));
                    self.modified = true;
                    self.begin_insert(false);
                }
                Key::Char('x') | Key::Delete => {
                    let idx = self.buffer.char_index(self.buffer.cursor());
                    let ch = self.buffer.slice_chars(idx, idx + 1);
                    if !ch.is_empty() && ch != "\n" {
                        self.checkpoint();
                        self.register = Register { text: ch, linewise: false };
                        self.buffer.delete_after();
                        self.modified = true;
                    }
                }
                Key::Char('d') => {
                    if pending == Some('d') {
                        self.checkpoint();
                        self.yank_line();
                        self.buffer.delete_line();
                        self.modified = true;
                    } else {
                        self.pending = Some('d');
                        return Action::Redraw;
                    }
                }
                Key::Char('y') => {
                    if pending == Some('y') {
                        self.yank_line();
                        self.message = "1 line yanked".into();
                    } else {
                        self.pending = Some('y');
                        return Action::Redraw;
                    }
                }
                Key::Char('p') => self.paste(true),
                Key::Char('P') => self.paste(false),
                Key::Char('u') => self.undo(),
                Key::Ctrl('r') => self.redo(),
                Key::Char('v') => {
                    self.anchor = Some(self.buffer.cursor());
                    self.mode = Mode::Visual;
                }
                // `ZZ` saves and quits; `ZQ` quits without saving (as in Vim).
                Key::Char('Z') => {
                    if pending == Some('Z') {
                        return self.save_and_close();
                    }
                    self.pending = Some('Z');
                    return Action::Redraw;
                }
                Key::Char('Q') if pending == Some('Z') => return self.discard_and_close(),
                Key::Backspace | Key::Char('-') => return self.step_back(key),
                // In a view, `q` closes it and goes back to the file.
                Key::Char('q') if self.is_view() => return self.step_back(key),
                Key::Char('q') => {
                    self.message = "q does nothing here: Backspace opens the file list, :q quits".into();
                }
                Key::Char(':') => {
                    self.command_origin = Mode::Normal;
                    self.mode = Mode::Command;
                    self.command.clear();
                }
                Key::Escape => {}
                _ => return Action::None,
            }
        }
        if self.mode == Mode::Normal {
            self.buffer.clamp_cursor(false);
        }
        Action::Redraw
    }

    // ---------- modo Visual ----------

    fn visual(&mut self, key: Key) -> Action {
        self.message.clear();
        if let Some(m) = Self::motion_for(key) {
            self.buffer.move_cursor(m);
            self.buffer.clamp_cursor(false);
            return Action::Redraw;
        }
        match key {
            Key::Escape | Key::Char('v') => self.leave_visual(),
            Key::Char('y') => {
                if let Some((s, e)) = self.selection() {
                    self.register = Register { text: self.buffer.slice_chars(s, e), linewise: false };
                    self.message = format!("{} characters yanked", e - s);
                    self.buffer.set_cursor(self.buffer.cursor_at(s));
                }
                self.leave_visual();
            }
            Key::Char('d') | Key::Char('x') | Key::Char('c') => {
                if let Some((s, e)) = self.selection() {
                    self.checkpoint();
                    self.register = Register { text: self.buffer.slice_chars(s, e), linewise: false };
                    self.buffer.remove_chars(s, e);
                    self.modified = true;
                    if key == Key::Char('c') {
                        self.anchor = None;
                        self.begin_insert(false);
                        return Action::Redraw;
                    }
                }
                self.leave_visual();
            }
            _ => return Action::None,
        }
        Action::Redraw
    }

    fn leave_visual(&mut self) {
        self.mode = Mode::Normal;
        self.anchor = None;
        self.buffer.clamp_cursor(false);
    }

    // ---------- modo Insert ----------

    fn insert(&mut self, key: Key) -> Action {
        let mut edit = true;
        match key {
            Key::Char('\t') => {
                for _ in 0..4 {
                    self.buffer.insert_char(' ');
                }
            }
            Key::Char(c) => self.buffer.insert_char(c),
            Key::Enter => self.buffer.insert_char('\n'),
            Key::Backspace => self.buffer.delete_before(),
            Key::Delete => self.buffer.delete_after(),
            Key::Left => {
                edit = false;
                self.buffer.move_cursor(Motion::Left);
            }
            Key::Right => {
                edit = false;
                self.buffer.move_cursor(Motion::Right);
            }
            Key::Up => {
                edit = false;
                self.buffer.move_cursor(Motion::Up);
            }
            Key::Down => {
                edit = false;
                self.buffer.move_cursor(Motion::Down);
            }
            Key::Home => {
                edit = false;
                self.buffer.move_cursor(Motion::LineStart);
            }
            Key::End => {
                edit = false;
                self.buffer.move_cursor(Motion::LineEnd);
            }
            Key::Ctrl(_) => return Action::None,
            Key::Escape => {
                edit = false;
                // A session without changes must not leave an empty undo step.
                if !self.insert_changed {
                    self.history.pop();
                }
                self.mode = Mode::Normal;
                // As in Vim, the cursor moves back when leaving Insert.
                self.buffer.move_cursor(Motion::Left);
                self.buffer.clamp_cursor(false);
            }
        }
        if edit {
            self.modified = true;
            self.insert_changed = true;
        }
        Action::Redraw
    }

    // ---------- git ----------

    fn git_clone(&mut self, args: &str) -> Action {
        let mut parts = args.split_whitespace();
        let Some(url) = parts.next() else {
            self.message = "E32: usage: :clone <url> [folder]".into();
            return Action::Redraw;
        };
        let name = parts.next().map(str::to_string);
        Action::Git(GitRequest::Clone { url: url.to_string(), name })
    }

    /// `:git token <token>` | `<host> <token>` | `<host> <usuario> <token>`.
    fn git_token(&mut self, rest: &str) -> Action {
        let parts: Vec<&str> = rest.split_whitespace().collect();
        let (host, user, token) = match parts.as_slice() {
            [token] => (None, None, *token),
            [host, token] => (Some(*host), None, *token),
            [host, user, token] => (Some(*host), Some(*user), *token),
            _ => {
                self.message = "E32: usage: :git token [host] [user] <token>".into();
                return Action::Redraw;
            }
        };
        Action::Git(GitRequest::SetToken {
            host: host.map(str::to_string),
            user: user.map(str::to_string),
            token: Secret::new(token),
        })
    }

    /// What is shown of the command line: the token is hidden with `*` so that
    /// it is not in view (not even in a recording or a capture of the headset).
    pub fn command_display(&self) -> String {
        match self.command.strip_prefix("git token ") {
            Some(rest) => {
                // Only the last word is the token; the host and the user stay visible.
                let (visible, token) = match rest.rfind(char::is_whitespace) {
                    Some(i) => rest.split_at(i + 1),
                    None => ("", rest),
                };
                format!("git token {visible}{}", "*".repeat(token.chars().count()))
            }
            None => self.command.clone(),
        }
    }

    fn git_command(&mut self, sub: &str, rest: &str) -> Action {
        match sub {
            "status" | "st" => Action::Git(GitRequest::Status),
            "diff" => Action::Git(GitRequest::Diff),
            "log" => Action::Git(GitRequest::Log),
            "fetch" => Action::Git(GitRequest::Fetch),
            "pull" => Action::Git(GitRequest::Pull),
            "push" => Action::Git(GitRequest::Push),
            "token" => self.git_token(rest),
            "logout" => Action::Git(GitRequest::Logout(
                Some(rest.trim().to_string()).filter(|h| !h.is_empty()),
            )),
            "creds" => Action::Git(GitRequest::Credentials),
            "commit" | "ci" => {
                if rest.is_empty() {
                    self.message = "E32: usage: :git commit <message>".into();
                    Action::Redraw
                } else if self.modified {
                    // What is not saved on disk would not go into the commit.
                    self.message = "E37: save before committing (:w)".into();
                    Action::Redraw
                } else {
                    Action::Git(GitRequest::Commit(rest.to_string()))
                }
            }
            _ => {
                self.message = format!("E492: git: unknown subcommand: {sub:?} (status, diff, log, commit, fetch, pull, push, token, logout, creds)");
                Action::Redraw
            }
        }
    }

    // ---------- file browser ----------

    fn browse(&mut self, key: Key) -> Action {
        self.message.clear();
        match key {
            Key::Char('j') | Key::Down => self.browser.move_selection(1),
            Key::Char('k') | Key::Up => self.browser.move_selection(-1),
            Key::Char('g') | Key::Home => self.browser.select_first(),
            Key::Char('G') | Key::End => self.browser.select_last(),
            Key::Escape | Key::Char('q') => {
                if self.file.is_empty() {
                    self.message = "no file open: Enter opens one, :q quits the app".into();
                } else {
                    self.mode = Mode::Normal;
                }
            }
            Key::Char('-') | Key::Backspace | Key::Char('h') | Key::Left => {
                match self.browser.parent() {
                    Some(parent) => return Action::Browse(parent),
                    None => self.message = "already at the root".into(),
                }
            }
            Key::Enter | Key::Char('l') | Key::Right => {
                let Some(entry) = self.browser.selected_entry() else { return Action::None };
                let path = self.browser.path_of(&entry.name);
                if entry.is_dir {
                    return Action::Browse(path);
                }
                if self.modified {
                    self.message = "E37: unsaved changes (go back with q and use :w)".into();
                } else {
                    return Action::Open(path);
                }
            }
            Key::Char(':') => {
                self.command_origin = Mode::Browser;
                self.mode = Mode::Command;
                self.command.clear();
            }
            _ => return Action::None,
        }
        Action::Redraw
    }

    // ---------- modo Command ----------

    fn command_mode(&mut self, key: Key) -> Action {
        match key {
            Key::Char(c) => self.command.push(c),
            Key::Backspace => {
                if self.command.pop().is_none() {
                    self.mode = self.command_origin;
                }
            }
            Key::Escape => {
                self.mode = self.command_origin;
                self.command.clear();
            }
            Key::Enter => {
                let cmd = std::mem::take(&mut self.command);
                self.mode = self.command_origin;
                self.buffer.clamp_cursor(false);
                return self.run_command(cmd.trim());
            }
            _ => return Action::None,
        }
        Action::Redraw
    }

    fn run_command(&mut self, cmd: &str) -> Action {
        let (name, arg) = match cmd.split_once(char::is_whitespace) {
            Some((n, a)) => (n, a.trim()),
            None => (cmd, ""),
        };
        match (name, arg) {
            ("w", "") if self.is_view() => {
                self.message = "E382: git views are read-only (:w name saves them as a file)".into();
                Action::Redraw
            }
            ("w", "") => Action::Save,
            ("w", file) => Action::SaveAs(file.to_string()),
            // `:q` in a view closes it and goes back to the file.
            ("q" | "q!", _) if self.is_view() && !self.back.is_empty() => {
                self.go_back();
                Action::Redraw
            }
            ("back" | "b" | "bp", _) => {
                if !self.go_back() {
                    self.message = "E23: no previous buffer".into();
                }
                Action::Redraw
            }
            ("q" | "qa", _) if self.modified => {
                let all = if name == "qa" { "qa!" } else { "q!" };
                self.message = format!("E37: unsaved changes (use :{all} to discard)");
                Action::Redraw
            }
            // From the file list (or with no file open), `:q` quits the app.
            ("q" | "q!", _) if self.command_origin == Mode::Browser || self.file.is_empty() => Action::Quit,
            // In a file, `:q` closes it and leaves its folder's list.
            ("q" | "q!", _) => Action::Close,
            ("qa" | "qa!" | "quitall", _) => Action::Quit,
            ("wq" | "x", _) => self.save_and_close(),
            ("wqa" | "xa", _) => Action::SaveAndQuit,
            ("e", "") | ("ls" | "files", _) => Action::Browse(parent_of(&self.file)),
            ("e", _) if self.modified => {
                self.message = "E37: unsaved changes (use :e! file to discard)".into();
                Action::Redraw
            }
            ("e" | "e!", file) if !file.is_empty() => Action::Open(file.to_string()),
            ("font" | "panel" | "distance", arg) => {
                if arg.parse::<f32>().is_ok_and(f32::is_finite) {
                    Action::Set { name: name.to_string(), value: arg.to_string() }
                } else {
                    self.message = format!("E474: :{name} <number> (for example :{name} 1.2, or :{name} +0.1)");
                    Action::Redraw
                }
            }
            ("bridge", _) => Action::Bridge,
            ("send", arg) => Action::Send(arg.to_string()),
            ("selftest", arg) => Action::SelfTest(arg.to_string()),
            ("screen", arg) => Action::Screen(arg.to_string()),
            ("remote", arg) => Action::Remote(arg.to_string()),
            ("caption", arg) => Action::Caption(arg.to_string()),
            ("run", _) => Action::Run,
            ("reload" | "source", _) => Action::Reload,
            ("log", arg) => {
                match arg {
                    "" => self.log.toggle(),
                    "show" => self.log.set_visible(true),
                    "hide" => self.log.set_visible(false),
                    "clear" => self.log.clear(),
                    other => self.message = format!("E474: :log {other}: use show, hide or clear"),
                }
                Action::Redraw
            }
            ("clone", args) => self.git_clone(args),
            ("git", args) => {
                let (sub, rest) = match args.split_once(char::is_whitespace) {
                    Some((sub, rest)) => (sub, rest.trim()),
                    None => (args, ""),
                };
                self.git_command(sub, rest)
            }
            ("gs", _) => self.git_command("status", ""),
            ("gd", _) => self.git_command("diff", ""),
            ("gl", _) => self.git_command("log", ""),
            ("gc", msg) => self.git_command("commit", msg),
            ("", _) => Action::Redraw,
            _ => {
                self.message = format!("E492: not a command: {cmd}");
                Action::Redraw
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(e: &mut Editor, keys: &str) {
        for c in keys.chars() {
            e.handle_key(Key::Char(c));
        }
    }

    #[test]
    fn insert_text_then_escape() {
        let mut e = Editor::new("");
        press(&mut e, "ihola");
        assert_eq!(e.mode, Mode::Insert);
        e.handle_key(Key::Escape);
        assert_eq!(e.mode, Mode::Normal);
        assert_eq!(e.buffer.to_string(), "hola");
        assert_eq!(e.buffer.cursor().col, 3);
        assert!(e.modified);
    }

    #[test]
    fn normal_motions_stay_inside_text() {
        let mut e = Editor::new("ab\ncdef");
        press(&mut e, "$");
        assert_eq!(e.buffer.cursor().col, 1);
        press(&mut e, "j");
        assert_eq!(e.buffer.cursor().col, 1);
        press(&mut e, "$");
        assert_eq!(e.buffer.cursor().col, 3);
        press(&mut e, "l");
        assert_eq!(e.buffer.cursor().col, 3);
    }

    #[test]
    fn dd_deletes_line_and_gg_upper_g_jump() {
        let mut e = Editor::new("uno\ndos\ntres");
        press(&mut e, "G");
        assert_eq!(e.buffer.cursor().line, 2);
        press(&mut e, "dd");
        assert_eq!(e.buffer.to_string(), "uno\ndos");
        assert_eq!(e.buffer.cursor().line, 1);
        press(&mut e, "gg");
        assert_eq!(e.buffer.cursor().line, 0);
        press(&mut e, "dd");
        assert_eq!(e.buffer.to_string(), "dos");
    }

    #[test]
    fn single_d_does_nothing_until_second_d() {
        let mut e = Editor::new("a\nb");
        press(&mut e, "dj");
        assert_eq!(e.buffer.to_string(), "a\nb");
    }

    #[test]
    fn open_line_below_and_above() {
        let mut e = Editor::new("a\nc");
        press(&mut e, "ob");
        e.handle_key(Key::Escape);
        assert_eq!(e.buffer.to_string(), "a\nb\nc");
        press(&mut e, "gg");
        press(&mut e, "O");
        assert_eq!(e.buffer.cursor().line, 0);
        press(&mut e, "z");
        assert_eq!(e.buffer.to_string(), "z\na\nb\nc");
    }

    #[test]
    fn x_deletes_char_under_cursor() {
        let mut e = Editor::new("abc");
        press(&mut e, "x");
        assert_eq!(e.buffer.to_string(), "bc");
    }

    #[test]
    fn append_goes_after_cursor() {
        let mut e = Editor::new("ab");
        press(&mut e, "aX");
        assert_eq!(e.buffer.to_string(), "aXb");
        e.handle_key(Key::Escape);
        press(&mut e, "AY");
        assert_eq!(e.buffer.to_string(), "aXbY");
    }

    #[test]
    fn command_write_and_quit() {
        let mut e = Editor::new("x");
        press(&mut e, ":");
        assert_eq!(e.mode, Mode::Command);
        press(&mut e, "w");
        assert_eq!(e.handle_key(Key::Enter), Action::Save);
        assert_eq!(e.mode, Mode::Normal);
        press(&mut e, ":wq");
        assert_eq!(e.handle_key(Key::Enter), Action::SaveAndClose);
        press(&mut e, ":wqa");
        assert_eq!(e.handle_key(Key::Enter), Action::SaveAndQuit);
    }

    #[test]
    fn quit_refuses_with_unsaved_changes() {
        let mut e = Editor::new("");
        press(&mut e, "ia");
        e.handle_key(Key::Escape);
        press(&mut e, ":q");
        assert_eq!(e.handle_key(Key::Enter), Action::Redraw);
        assert!(e.message.starts_with("E37"));
        press(&mut e, ":q!");
        assert_eq!(e.handle_key(Key::Enter), Action::Quit);
    }

    #[test]
    fn unknown_command_reports_error() {
        let mut e = Editor::new("");
        press(&mut e, ":zzz");
        e.handle_key(Key::Enter);
        assert!(e.message.contains("zzz"));
    }

    #[test]
    fn tab_inserts_four_spaces() {
        let mut e = Editor::new("");
        press(&mut e, "i\t");
        assert_eq!(e.buffer.to_string(), "    ");
    }

    // ----- deshacer / rehacer -----

    #[test]
    fn undo_reverts_a_whole_insert_session() {
        let mut e = Editor::new("base");
        press(&mut e, "Ahola mundo");
        e.handle_key(Key::Escape);
        assert_eq!(e.buffer.to_string(), "basehola mundo");
        press(&mut e, "u");
        assert_eq!(e.buffer.to_string(), "base");
    }

    #[test]
    fn undo_redo_roundtrip() {
        let mut e = Editor::new("a\nb\nc");
        press(&mut e, "dd");
        press(&mut e, "dd");
        assert_eq!(e.buffer.to_string(), "c");
        press(&mut e, "u");
        assert_eq!(e.buffer.to_string(), "b\nc");
        press(&mut e, "u");
        assert_eq!(e.buffer.to_string(), "a\nb\nc");
        e.handle_key(Key::Ctrl('r'));
        assert_eq!(e.buffer.to_string(), "b\nc");
        e.handle_key(Key::Ctrl('r'));
        assert_eq!(e.buffer.to_string(), "c");
        e.handle_key(Key::Ctrl('r'));
        assert!(e.message.contains("nothing to redo"));
    }

    #[test]
    fn empty_insert_session_leaves_no_undo_step() {
        let mut e = Editor::new("a\nb");
        press(&mut e, "x"); // paso 1
        press(&mut e, "i");
        e.handle_key(Key::Escape); // empty session: it must not create a step
        press(&mut e, "u");
        assert_eq!(e.buffer.to_string(), "a\nb");
    }

    #[test]
    fn new_edit_discards_redo_history() {
        let mut e = Editor::new("abc");
        press(&mut e, "x");
        press(&mut e, "u");
        press(&mut e, "x");
        e.handle_key(Key::Ctrl('r'));
        assert!(e.message.contains("nothing to redo"));
    }

    #[test]
    fn open_line_and_typing_undo_as_one_step() {
        let mut e = Editor::new("a");
        press(&mut e, "ohola");
        e.handle_key(Key::Escape);
        assert_eq!(e.buffer.to_string(), "a\nhola");
        press(&mut e, "u");
        assert_eq!(e.buffer.to_string(), "a");
    }

    // ----- copy and paste -----

    #[test]
    fn yy_then_p_duplicates_line_below() {
        let mut e = Editor::new("uno\ndos");
        press(&mut e, "yyp");
        assert_eq!(e.buffer.to_string(), "uno\nuno\ndos");
        assert_eq!(e.buffer.cursor().line, 1);
    }

    #[test]
    fn dd_then_p_moves_line() {
        let mut e = Editor::new("uno\ndos\ntres");
        press(&mut e, "ddp");
        assert_eq!(e.buffer.to_string(), "dos\nuno\ntres");
    }

    #[test]
    fn dd_then_upper_p_pastes_above() {
        let mut e = Editor::new("uno\ndos");
        press(&mut e, "jddP");
        assert_eq!(e.buffer.to_string(), "dos\nuno");
    }

    #[test]
    fn x_then_p_swaps_chars() {
        let mut e = Editor::new("ab");
        press(&mut e, "xp");
        assert_eq!(e.buffer.to_string(), "ba");
    }

    #[test]
    fn paste_with_empty_register_is_a_noop() {
        let mut e = Editor::new("ab");
        press(&mut e, "p");
        assert_eq!(e.buffer.to_string(), "ab");
    }

    // ----- modo Visual -----

    #[test]
    fn visual_selection_includes_char_under_cursor() {
        let mut e = Editor::new("hola mundo");
        press(&mut e, "vll");
        assert_eq!(e.mode, Mode::Visual);
        assert_eq!(e.selection(), Some((0, 3)));
    }

    #[test]
    fn visual_yank_and_paste() {
        let mut e = Editor::new("hola mundo");
        press(&mut e, "vlly");
        assert_eq!(e.mode, Mode::Normal);
        press(&mut e, "$p");
        assert_eq!(e.buffer.to_string(), "hola mundohol");
    }

    #[test]
    fn visual_delete_and_undo() {
        let mut e = Editor::new("hola mundo");
        press(&mut e, "wvd");
        assert_eq!(e.buffer.to_string(), "hola undo");
        press(&mut e, "u");
        assert_eq!(e.buffer.to_string(), "hola mundo");
    }

    #[test]
    fn visual_change_enters_insert() {
        let mut e = Editor::new("abc");
        press(&mut e, "vcX");
        assert_eq!(e.mode, Mode::Insert);
        assert_eq!(e.buffer.to_string(), "Xbc");
    }

    #[test]
    fn visual_escape_cancels() {
        let mut e = Editor::new("abc");
        press(&mut e, "vl");
        e.handle_key(Key::Escape);
        assert_eq!(e.mode, Mode::Normal);
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn word_motions_in_normal_mode() {
        let mut e = Editor::new("foo bar baz");
        press(&mut e, "w");
        assert_eq!(e.buffer.cursor().col, 4);
        press(&mut e, "w");
        assert_eq!(e.buffer.cursor().col, 8);
        press(&mut e, "b");
        assert_eq!(e.buffer.cursor().col, 4);
    }

    // ----- archivos -----

    #[test]
    fn edit_command_requests_open() {
        let mut e = Editor::new("");
        press(&mut e, ":e src/main.rs");
        assert_eq!(e.handle_key(Key::Enter), Action::Open("src/main.rs".into()));
    }

    #[test]
    fn edit_is_refused_with_unsaved_changes_but_bang_forces() {
        let mut e = Editor::new("");
        press(&mut e, "ix");
        e.handle_key(Key::Escape);
        press(&mut e, ":e otro.rs");
        assert_eq!(e.handle_key(Key::Enter), Action::Redraw);
        assert!(e.message.starts_with("E37"));
        press(&mut e, ":e! otro.rs");
        assert_eq!(e.handle_key(Key::Enter), Action::Open("otro.rs".into()));
    }

    #[test]
    fn write_as_and_list() {
        let mut e = Editor::new("");
        press(&mut e, ":w nuevo.txt");
        assert_eq!(e.handle_key(Key::Enter), Action::SaveAs("nuevo.txt".into()));
        press(&mut e, ":ls");
        assert_eq!(e.handle_key(Key::Enter), Action::Browse(String::new()));
    }

    #[test]
    fn load_resets_state() {
        let mut e = Editor::new("viejo");
        press(&mut e, "x");
        e.load("b.rs", "nuevo");
        assert_eq!(e.file, "b.rs");
        assert_eq!(e.buffer.to_string(), "nuevo");
        assert!(!e.modified);
        press(&mut e, "u");
        assert!(e.message.contains("nothing to undo"));
    }

    // ----- git commands -----

    fn run(e: &mut Editor, cmd: &str) -> Action {
        press(e, cmd);
        e.handle_key(Key::Enter)
    }

    #[test]
    fn clone_command_parses_url_and_optional_folder() {
        let mut e = Editor::new("");
        assert_eq!(
            run(&mut e, ":clone https://x/y/repo.git"),
            Action::Git(GitRequest::Clone { url: "https://x/y/repo.git".into(), name: None })
        );
        assert_eq!(
            run(&mut e, ":clone https://x/y/repo.git mio"),
            Action::Git(GitRequest::Clone { url: "https://x/y/repo.git".into(), name: Some("mio".into()) })
        );
        assert_eq!(run(&mut e, ":clone"), Action::Redraw);
        assert!(e.message.starts_with("E32"));
    }

    #[test]
    fn git_subcommands_and_aliases() {
        let mut e = Editor::new("");
        assert_eq!(run(&mut e, ":git status"), Action::Git(GitRequest::Status));
        assert_eq!(run(&mut e, ":git st"), Action::Git(GitRequest::Status));
        assert_eq!(run(&mut e, ":gs"), Action::Git(GitRequest::Status));
        assert_eq!(run(&mut e, ":git diff"), Action::Git(GitRequest::Diff));
        assert_eq!(run(&mut e, ":gd"), Action::Git(GitRequest::Diff));
        assert_eq!(run(&mut e, ":git log"), Action::Git(GitRequest::Log));
        assert_eq!(run(&mut e, ":gl"), Action::Git(GitRequest::Log));
        assert_eq!(run(&mut e, ":git"), Action::Redraw);
        assert!(e.message.contains("subcommand"));
    }

    #[test]
    fn commit_needs_a_message_and_saved_buffer() {
        let mut e = Editor::new("");
        assert_eq!(
            run(&mut e, ":git commit fix the bug"),
            Action::Git(GitRequest::Commit("fix the bug".into()))
        );
        assert_eq!(run(&mut e, ":gc otro mensaje"), Action::Git(GitRequest::Commit("otro mensaje".into())));
        assert_eq!(run(&mut e, ":git commit"), Action::Redraw);
        assert!(e.message.starts_with("E32"));

        press(&mut e, "ix");
        e.handle_key(Key::Escape);
        assert_eq!(run(&mut e, ":git commit algo"), Action::Redraw);
        assert!(e.message.starts_with("E37"), "{}", e.message);
    }

    // ----- scripts -----

    #[test]
    fn run_and_reload_commands_ask_the_host_to_run_scripts() {
        let mut e = Editor::new("print('hola')");
        e.file = "a.lua".into();
        assert_eq!(run(&mut e, ":run"), Action::Run);
        assert_eq!(run(&mut e, ":reload"), Action::Reload);
        assert_eq!(run(&mut e, ":source"), Action::Reload);
        // Running a script does not depend on it being saved: what is in the buffer is used.
        e.modified = true;
        assert_eq!(run(&mut e, ":run"), Action::Run);
    }

    // ----- output panel -----

    #[test]
    fn log_command_toggles_shows_hides_and_clears() {
        let mut e = Editor::new("");
        assert!(!e.log.is_visible());
        assert_eq!(run(&mut e, ":log"), Action::Redraw);
        assert!(e.log.is_visible());
        run(&mut e, ":log");
        assert!(!e.log.is_visible());
        run(&mut e, ":log show");
        assert!(e.log.is_visible());
        e.log.push("algo");
        run(&mut e, ":log clear");
        assert_eq!(e.log.lines().len(), 0);
        run(&mut e, ":log hide");
        assert!(!e.log.is_visible());
        run(&mut e, ":log raro");
        assert!(e.message.starts_with("E474"), "{}", e.message);
    }

    // ----- going back -----

    fn open(e: &mut Editor, file: &str, text: &str) {
        e.load(file, text);
    }

    #[test]
    fn load_keeps_the_output_log_and_the_yank_register() {
        let mut e = Editor::new("uno\ndos");
        e.file = "a.rs".into();
        e.log.set_visible(true);
        e.log.push("> git clone x");
        press(&mut e, "yy");
        open(&mut e, "[git status]", "## master\n");
        assert!(e.log.is_visible(), "opening a view must not hide the output");
        assert_eq!(e.log.lines().len(), 1);
        open(&mut e, "b.rs", "x");
        press(&mut e, "p");
        assert_eq!(e.buffer.to_string(), "x\nuno", "the register must survive the file change");
    }

    #[test]
    fn go_back_restores_the_previous_file_text_and_cursor() {
        let mut e = Editor::new("uno\ndos\ntres");
        e.file = "a.rs".into();
        press(&mut e, "jl");
        open(&mut e, "b.rs", "otro");
        assert_eq!(e.file, "b.rs");
        assert!(e.go_back());
        assert_eq!(e.file, "a.rs");
        assert_eq!(e.buffer.to_string(), "uno\ndos\ntres");
        assert_eq!(e.buffer.cursor(), Cursor { line: 1, col: 1 });
        assert!(!e.go_back(), "nothing left to go back to");
    }

    #[test]
    fn views_and_modified_buffers_are_not_remembered() {
        let mut e = Editor::new("a");
        e.file = "a.rs".into();
        open(&mut e, "[git status]", "x");
        open(&mut e, "[git diff]", "y");
        assert!(e.go_back(), "it goes back to the file, not to the previous view");
        assert_eq!(e.file, "a.rs");

        press(&mut e, "ix");
        e.handle_key(Key::Escape);
        assert!(e.modified);
        open(&mut e, "b.rs", "b"); // like `:e! b.rs`: the modified text is discarded
        assert!(!e.go_back(), "unsaved changes are not remembered");
    }

    #[test]
    fn back_history_is_limited() {
        let mut e = Editor::new("");
        e.file = "0.rs".into();
        for i in 1..=(MAX_BACK + 5) {
            open(&mut e, &format!("{i}.rs"), "");
        }
        let mut steps = 0;
        while e.go_back() {
            steps += 1;
        }
        assert_eq!(steps, MAX_BACK);
    }

    #[test]
    fn dash_and_backspace_open_the_folder_list_from_a_file() {
        let mut e = Editor::new("x");
        e.file = "src/main.rs".into();
        assert_eq!(e.handle_key(Key::Char('-')), Action::Browse("src".into()));
        assert_eq!(e.handle_key(Key::Backspace), Action::Browse("src".into()));
    }

    #[test]
    fn q_and_backspace_close_a_view_and_return_to_the_file() {
        for key in [Key::Char('q'), Key::Backspace] {
            let mut e = Editor::new("codigo");
            e.file = "a.rs".into();
            open(&mut e, "[git status]", "## master");
            assert_eq!(e.handle_key(key), Action::Redraw);
            assert_eq!(e.file, "a.rs", "{key:?}");
            assert_eq!(e.buffer.to_string(), "codigo");
        }
    }

    #[test]
    fn q_in_a_view_without_history_explains_instead_of_quitting() {
        let mut e = Editor::new("");
        open(&mut e, "[git log]", "x");
        assert_eq!(e.handle_key(Key::Char('q')), Action::Redraw);
        assert!(e.message.contains("no previous buffer"));
    }

    #[test]
    fn colon_q_in_a_view_goes_back_but_in_a_file_closes_it() {
        let mut e = Editor::new("codigo");
        e.file = "a.rs".into();
        open(&mut e, "[git diff]", "x");
        assert_eq!(run(&mut e, ":q"), Action::Redraw);
        assert_eq!(e.file, "a.rs");
        assert_eq!(run(&mut e, ":q"), Action::Close, "in a file it closes the file, not the app");
    }

    #[test]
    fn back_command_and_its_error() {
        let mut e = Editor::new("a");
        e.file = "a.rs".into();
        assert_eq!(run(&mut e, ":back"), Action::Redraw);
        assert!(e.message.starts_with("E23"), "{}", e.message);
        open(&mut e, "b.rs", "b");
        run(&mut e, ":b");
        assert_eq!(e.file, "a.rs");
    }

    // ----- Ctrl, ZZ, command line -----

    #[test]
    fn ctrl_s_saves_in_every_mode_without_changing_it() {
        let mut e = Editor::new("x");
        assert_eq!(e.handle_key(Key::Ctrl('s')), Action::Save);
        press(&mut e, "i");
        assert_eq!(e.handle_key(Key::Ctrl('s')), Action::Save);
        assert_eq!(e.mode, Mode::Insert, "saving must not leave Insert");
        e.handle_key(Key::Escape);
        press(&mut e, ":w");
        assert_eq!(e.handle_key(Key::Ctrl('s')), Action::Save);
        assert_eq!(e.mode, Mode::Command);
        assert_eq!(e.command, "w", "nor lose what was being typed");
    }

    #[test]
    fn other_ctrl_keys_explain_themselves_instead_of_doing_nothing() {
        let mut e = Editor::new("x");
        assert_eq!(e.handle_key(Key::Ctrl('x')), Action::Redraw);
        assert!(e.message.contains("Ctrl+X") && e.message.contains("Ctrl+S"), "{}", e.message);
        press(&mut e, "i");
        e.handle_key(Key::Ctrl('x'));
        assert_eq!(e.buffer.to_string(), "x", "it must not insert anything");
        assert_eq!(e.mode, Mode::Insert);
    }

    #[test]
    fn ctrl_r_still_redoes_in_normal_mode() {
        let mut e = Editor::new("abc");
        press(&mut e, "xu");
        e.handle_key(Key::Ctrl('r'));
        assert_eq!(e.buffer.to_string(), "bc");
    }

    #[test]
    fn zz_saves_and_quits_and_zq_quits_without_saving() {
        let mut e = Editor::new("x");
        e.handle_key(Key::Char('Z'));
        assert_eq!(e.pending_keys(), Some('Z'));
        assert_eq!(e.handle_key(Key::Char('Z')), Action::SaveAndClose);

        let mut e = Editor::new("x"); // no file open: ZQ does quit the app
        press(&mut e, "ix");
        e.handle_key(Key::Escape);
        assert!(e.modified);
        press(&mut e, "Z");
        assert_eq!(e.handle_key(Key::Char('Q')), Action::Quit);

        let mut e = Editor::new("x");
        e.file = "a.rs".into();
        press(&mut e, "ix");
        e.handle_key(Key::Escape);
        press(&mut e, "Z");
        assert_eq!(e.handle_key(Key::Char('Q')), Action::Close, "ZQ discards and closes the file");

        let mut e = Editor::new("x");
        press(&mut e, "Zj"); // Z followed by something else is cancelled
        assert_eq!(e.pending_keys(), None);
    }

    #[test]
    fn pending_keys_are_visible_and_trigger_a_redraw() {
        let mut e = Editor::new("a\nb");
        assert_eq!(e.handle_key(Key::Char('d')), Action::Redraw);
        assert_eq!(e.pending_keys(), Some('d'));
        e.handle_key(Key::Char('d'));
        assert_eq!(e.pending_keys(), None);
    }

    #[test]
    fn q_in_a_plain_file_gives_a_hint() {
        let mut e = Editor::new("x");
        e.file = "a.rs".into();
        assert_eq!(e.handle_key(Key::Char('q')), Action::Redraw);
        assert!(e.message.contains("Backspace"), "{}", e.message);
    }

    #[test]
    fn command_line_from_the_browser_returns_to_the_browser() {
        let mut e = Editor::new("x");
        e.mode = Mode::Browser;
        e.handle_key(Key::Char(':'));
        assert_eq!(e.mode, Mode::Command);
        assert!(e.shows_browser(), "the list must stay visible while the command is typed");
        press(&mut e, "zzz");
        e.handle_key(Key::Enter); // comando desconocido
        assert_eq!(e.mode, Mode::Browser);
        e.handle_key(Key::Char(':'));
        e.handle_key(Key::Escape);
        assert_eq!(e.mode, Mode::Browser);
        e.handle_key(Key::Char(':'));
        e.handle_key(Key::Backspace);
        assert_eq!(e.mode, Mode::Browser);
        assert!(!e.shows_browser() || e.mode == Mode::Browser);
    }

    #[test]
    fn command_line_from_normal_returns_to_normal_and_hides_the_browser() {
        let mut e = Editor::new("x");
        press(&mut e, ":");
        assert!(!e.shows_browser());
        e.handle_key(Key::Escape);
        assert_eq!(e.mode, Mode::Normal);
    }

    // ----- closing a file vs. quitting the app -----

    #[test]
    fn q_closes_the_file_and_q_bang_discards_while_q_refuses_with_changes() {
        let mut e = Editor::new("x");
        e.file = "src/a.rs".into();
        assert_eq!(run(&mut e, ":q"), Action::Close);

        press(&mut e, "ix");
        e.handle_key(Key::Escape);
        assert_eq!(run(&mut e, ":q"), Action::Redraw);
        assert!(e.message.starts_with("E37") && e.message.contains(":q!"), "{}", e.message);
        assert_eq!(run(&mut e, ":q!"), Action::Close);
    }

    #[test]
    fn q_from_the_file_list_quits_the_app_but_refuses_with_unsaved_changes() {
        let mut e = Editor::new("x");
        e.file = "a.rs".into();
        e.mode = Mode::Browser;
        assert_eq!(run(&mut e, ":q"), Action::Quit);

        let mut e = Editor::new("x");
        e.file = "a.rs".into();
        press(&mut e, "ix");
        e.handle_key(Key::Escape);
        e.mode = Mode::Browser; // `:ls` with unsaved changes
        assert_eq!(run(&mut e, ":q"), Action::Redraw, "it must not lose the changes when quitting");
        assert!(e.message.starts_with("E37"));
    }

    #[test]
    fn qa_always_quits_the_app_and_respects_unsaved_changes() {
        let mut e = Editor::new("x");
        e.file = "a.rs".into();
        assert_eq!(run(&mut e, ":qa"), Action::Quit);
        press(&mut e, "ix");
        e.handle_key(Key::Escape);
        assert_eq!(run(&mut e, ":qa"), Action::Redraw);
        assert!(e.message.contains(":qa!"), "{}", e.message);
        assert_eq!(run(&mut e, ":qa!"), Action::Quit);
        assert_eq!(run(&mut e, ":wqa"), Action::SaveAndQuit);
    }

    #[test]
    fn wq_and_zz_in_a_view_only_close_the_view() {
        let mut e = Editor::new("codigo");
        e.file = "a.rs".into();
        open(&mut e, "[git status]", "x");
        assert_eq!(run(&mut e, ":wq"), Action::Redraw);
        assert_eq!(e.file, "a.rs");

        open(&mut e, "[git log]", "y");
        press(&mut e, "Z");
        assert_eq!(e.handle_key(Key::Char('Z')), Action::Redraw);
        assert_eq!(e.file, "a.rs");

        let mut e = Editor::new("");
        open(&mut e, "[git log]", "y"); // no previous file: it closes to the list
        assert_eq!(run(&mut e, ":wq"), Action::Close);
    }

    #[test]
    fn views_are_read_only() {
        let mut e = Editor::new("codigo");
        e.file = "a.rs".into();
        open(&mut e, "[git diff]", "x");
        assert_eq!(run(&mut e, ":w"), Action::Redraw);
        assert!(e.message.starts_with("E382"), "{}", e.message);
        assert_eq!(e.handle_key(Key::Ctrl('s')), Action::Redraw);
        assert!(e.message.starts_with("E382"));
        // But it can be exported under another name.
        assert_eq!(run(&mut e, ":w salida.diff"), Action::SaveAs("salida.diff".into()));
    }

    #[test]
    fn close_file_leaves_no_file_and_escape_in_the_empty_browser_explains() {
        let mut e = Editor::new("x");
        e.file = "a.rs".into();
        e.close_file();
        assert_eq!(e.file, "");
        assert_eq!(e.buffer.to_string(), "");
        assert!(!e.modified);
        e.mode = Mode::Browser;
        e.handle_key(Key::Escape);
        assert_eq!(e.mode, Mode::Browser, "with no file there is nowhere to go back to");
        assert!(e.message.contains(":q"), "{}", e.message);
    }

    // ----- fetch / pull / push / token -----

    #[test]
    fn sync_commands_parse() {
        let mut e = Editor::new("");
        assert_eq!(run(&mut e, ":git fetch"), Action::Git(GitRequest::Fetch));
        assert_eq!(run(&mut e, ":git pull"), Action::Git(GitRequest::Pull));
        assert_eq!(run(&mut e, ":git push"), Action::Git(GitRequest::Push));
        assert_eq!(run(&mut e, ":git creds"), Action::Git(GitRequest::Credentials));
        assert_eq!(run(&mut e, ":git logout"), Action::Git(GitRequest::Logout(None)));
        assert_eq!(
            run(&mut e, ":git logout github.com"),
            Action::Git(GitRequest::Logout(Some("github.com".into())))
        );
    }

    #[test]
    fn token_command_accepts_one_two_or_three_arguments() {
        let mut e = Editor::new("");
        let token = |host: Option<&str>, user: Option<&str>, t: &str| {
            Action::Git(GitRequest::SetToken {
                host: host.map(str::to_string),
                user: user.map(str::to_string),
                token: Secret::new(t),
            })
        };
        assert_eq!(run(&mut e, ":git token abc123"), token(None, None, "abc123"));
        assert_eq!(run(&mut e, ":git token gitlab.com abc123"), token(Some("gitlab.com"), None, "abc123"));
        assert_eq!(
            run(&mut e, ":git token bitbucket.org yo abc123"),
            token(Some("bitbucket.org"), Some("yo"), "abc123")
        );
        assert_eq!(run(&mut e, ":git token"), Action::Redraw);
        assert!(e.message.starts_with("E32"), "{}", e.message);
        assert_eq!(run(&mut e, ":git token a b c d"), Action::Redraw);
    }

    #[test]
    fn a_token_request_never_prints_the_token() {
        let mut e = Editor::new("");
        let action = run(&mut e, ":git token ghp_muysecreto");
        let printed = format!("{action:?} {:?}", e);
        assert!(!printed.contains("muysecreto"), "the token leaked: {printed}");
        assert!(e.command.is_empty(), "the command line must be cleared on execute");
    }

    #[test]
    fn the_command_line_hides_the_token_while_typing() {
        let mut e = Editor::new("");
        press(&mut e, ":git token github.com ghp_abc");
        assert_eq!(e.command_display(), "git token github.com *******", "the host is visible, the token is not");
        e.handle_key(Key::Escape);

        // With a single argument, everything is the token.
        press(&mut e, ":git token ghp_solo");
        assert_eq!(e.command_display(), format!("git token {}", "*".repeat("ghp_solo".len())));
        e.handle_key(Key::Escape);

        press(&mut e, ":git commit plain message");
        assert_eq!(e.command_display(), "git commit plain message", "only the token is hidden");
    }

    #[test]
    fn the_reading_commands_ask_for_a_setting_change_and_reject_garbage() {
        let mut e = Editor::new("x");
        assert_eq!(run(&mut e, ":font 1.2"), Action::Set { name: "font".into(), value: "1.2".into() });
        assert_eq!(run(&mut e, ":panel +0.1"), Action::Set { name: "panel".into(), value: "+0.1".into() });
        assert_eq!(run(&mut e, ":distance -0.05"), Action::Set { name: "distance".into(), value: "-0.05".into() });
        assert_eq!(run(&mut e, ":font"), Action::Redraw);
        assert!(e.message.starts_with("E474"), "{}", e.message);
        assert_eq!(run(&mut e, ":font mucho"), Action::Redraw);
        assert_eq!(run(&mut e, ":panel NaN"), Action::Redraw, "not a finite number");
    }

    #[test]
    fn bridge_asks_to_listen_for_the_laptop() {
        let mut e = Editor::new("x");
        assert_eq!(run(&mut e, ":bridge"), Action::Bridge);
        assert_eq!(run(&mut e, ":send"), Action::Send(String::new()));
        assert_eq!(run(&mut e, ":send src/main.rs"), Action::Send("src/main.rs".into()));
        assert_eq!(run(&mut e, ":selftest drop"), Action::SelfTest("drop".into()));
        assert_eq!(run(&mut e, ":screen here"), Action::Screen("here".into()));
        assert_eq!(run(&mut e, ":remote"), Action::Remote(String::new()));
        assert_eq!(run(&mut e, ":caption Hola | mundo"), Action::Caption("Hola | mundo".into()));
    }
}
