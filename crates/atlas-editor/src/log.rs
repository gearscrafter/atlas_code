//! Output log: lines of text and an optional progress bar, to
//! show what long operations are doing (cloning, committing...).

use std::collections::VecDeque;

/// Lines kept; the oldest are discarded.
const MAX_LINES: usize = 200;

/// A secret (token, password). It is never printed: `Debug` and `Display` hide it,
/// so it cannot leak by accident into a log or an error message.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The real value. It must only be used when sending it to whoever needs it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl std::fmt::Display for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("***")
    }
}

/// Progress of a long operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// Fase actual (`recibiendo objetos`, `extrayendo archivos`...).
    pub label: String,
    pub done: u64,
    /// Total, if known. Without a total, `done` is read as bytes.
    pub total: Option<u64>,
}

impl Progress {
    /// Percentage 0..=100 if there is a total.
    pub fn percent(&self) -> Option<u8> {
        let total = self.total.filter(|t| *t > 0)?;
        Some((self.done.min(total) * 100 / total) as u8)
    }
}

/// Output log with a progress bar.
#[derive(Debug, Clone, Default)]
pub struct Log {
    lines: VecDeque<String>,
    progress: Option<Progress>,
    visible: bool,
    /// Increases with each visible change; it tells whether a redraw is needed.
    revision: u64,
}

impl Log {
    pub fn push(&mut self, line: impl Into<String>) {
        if self.lines.len() == MAX_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line.into());
        self.revision += 1;
    }

    pub fn lines(&self) -> impl DoubleEndedIterator<Item = &String> + ExactSizeIterator {
        self.lines.iter()
    }

    pub fn progress(&self) -> Option<&Progress> {
        self.progress.as_ref()
    }

    pub fn set_progress(&mut self, progress: Option<Progress>) {
        if self.progress != progress {
            self.progress = progress;
            self.revision += 1;
        }
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.progress = None;
        self.revision += 1;
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn set_visible(&mut self, visible: bool) {
        if self.visible != visible {
            self.visible = visible;
            self.revision += 1;
        }
    }

    pub fn toggle(&mut self) {
        self.set_visible(!self.visible);
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(done: u64, total: Option<u64>) -> Progress {
        Progress { label: "x".into(), done, total }
    }

    #[test]
    fn percent_needs_a_total_and_is_clamped() {
        assert_eq!(p(50, Some(200)).percent(), Some(25));
        assert_eq!(p(0, Some(10)).percent(), Some(0));
        assert_eq!(p(10, Some(10)).percent(), Some(100));
        assert_eq!(p(99, Some(10)).percent(), Some(100));
        assert_eq!(p(5, None).percent(), None);
        assert_eq!(p(5, Some(0)).percent(), None);
    }

    #[test]
    fn old_lines_are_dropped_beyond_the_limit() {
        let mut log = Log::default();
        for i in 0..(MAX_LINES + 25) {
            log.push(format!("line {i}"));
        }
        assert_eq!(log.lines().len(), MAX_LINES);
        assert_eq!(log.lines().next().unwrap(), "line 25");
        assert_eq!(log.lines().next_back().unwrap(), &format!("line {}", MAX_LINES + 24));
    }

    #[test]
    fn revision_changes_only_when_something_visible_changes() {
        let mut log = Log::default();
        let r0 = log.revision();
        log.set_progress(None);
        log.set_visible(false);
        assert_eq!(log.revision(), r0, "without changes there must be no new revision");

        log.set_progress(Some(p(1, Some(10))));
        let r1 = log.revision();
        assert!(r1 > r0);
        log.set_progress(Some(p(1, Some(10))));
        assert_eq!(log.revision(), r1, "same progress: nothing to redraw");
        log.set_progress(Some(p(2, Some(10))));
        assert!(log.revision() > r1);

        let r2 = log.revision();
        log.push("hola");
        assert!(log.revision() > r2);
    }

    #[test]
    fn toggle_and_clear() {
        let mut log = Log::default();
        assert!(!log.is_visible());
        log.toggle();
        assert!(log.is_visible());
        log.push("a");
        log.set_progress(Some(p(1, None)));
        log.clear();
        assert_eq!(log.lines().len(), 0);
        assert!(log.progress().is_none());
        assert!(log.is_visible(), "clearing does not hide the panel");
        log.toggle();
        assert!(!log.is_visible());
    }

    #[test]
    fn secrets_never_show_up_in_debug_or_display() {
        let secret = Secret::new("ghp_supersecreto");
        assert_eq!(format!("{secret:?}"), "Secret(***)");
        assert_eq!(format!("{secret}"), "***");
        assert_eq!(secret.expose(), "ghp_supersecreto");
        // Not inside the structures that contain it either.
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Holder {
            token: Secret,
        }
        assert!(!format!("{:?}", Holder { token: secret }).contains("supersecreto"));
    }
}
