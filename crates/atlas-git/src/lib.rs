//! Git operations for Atlas Code.
//!
//! * Cloning uses `gix` (pure Rust, HTTPS through rustls with bundled root
//!   certificates, so it does not depend on the system store).
//! * Status, diff, commit and log use `libgit2`.
//!
//! Everything works on paths and returns `Result<_, String>` with messages ready
//! to show in the status bar.

mod net;
mod sync;

use std::path::{Path, PathBuf};

use atlas_editor::{parent_of, GitRequest, Workspace};
pub use atlas_editor::Progress;
pub use sync::{align_unborn_head, clone_authenticated, fetch, pull, push};
pub use net::{
    host_of, install_transport, migrate_plaintext, redact, require_encryption, set_credentials_file,
    set_secret_box, strip_userinfo, unreadable_hosts, Credential, SecretBox, DEFAULT_USER,
};
use std::sync::atomic::AtomicBool;

use git2::{DiffFormat, DiffOptions, IndexAddOption, Repository, Signature, StatusOptions};

/// Default identity when the repository has no `user.name`/`user.email`.
const FALLBACK_NAME: &str = "Atlas Code";
const FALLBACK_EMAIL: &str = "atlas@localhost";

pub(crate) fn err(context: &str, e: impl std::fmt::Display) -> String {
    format!("git: {context}: {e}")
}

/// A file with changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    /// Short code like the one in `git status -s`: `M`, `A`, `D`, `R`, `??`...
    pub code: String,
}

/// The current phase, from `gix`'s task tree.
///
/// `gix` only knows totals in three tasks (indexed objects, deltas and checked-out
/// files); the download itself is measured in bytes without a total. The most
/// advanced phase that has already started is shown, as git's terminal does.
fn current_progress(root: &gix::progress::tree::Root) -> Option<Progress> {
    use std::sync::atomic::Ordering::Relaxed;
    let mut snapshot = Vec::new();
    root.sorted_snapshot(&mut snapshot);
    let tasks: Vec<(String, u64, Option<u64>)> = snapshot
        .into_iter()
        .filter_map(|(_, task)| {
            let value = task.progress?;
            Some((task.name.to_lowercase(), value.step.load(Relaxed) as u64, value.done_at.map(|t| t as u64)))
        })
        .collect();

    let counted = |needle: &str| {
        tasks
            .iter()
            .rev()
            .find(|(name, done, total)| name.contains(needle) && total.is_some_and(|t| t > 0) && *done > 0)
    };
    let phase = |label: &str, (_, done, total): &(String, u64, Option<u64>)| Progress {
        label: label.to_string(),
        done: *done,
        total: *total,
    };

    if let Some(t) = counted("checkout") {
        return Some(phase("extrayendo archivos", t));
    }
    if let Some(t) = counted("resolving") {
        return Some(phase("resolviendo deltas", t));
    }
    if let Some(t) = counted("indexing") {
        return Some(phase("recibiendo objetos", t));
    }
    // Download in bytes: the largest counter is taken.
    let bytes = tasks
        .iter()
        .filter(|(name, done, _)| {
            *done > 0
                && ["read pack", "receiving", "decompress", "decoding", "writing"].iter().any(|n| name.contains(n))
        })
        .map(|(_, done, _)| *done)
        .max();
    if let Some(done) = bytes {
        return Some(Progress { label: "descargando".into(), done, total: None });
    }
    tasks
        .iter()
        .any(|(name, ..)| ["handshake", "list refs", "negotiate"].iter().any(|n| name.contains(n)))
        .then(|| Progress { label: "negotiating with the server".into(), done: 0, total: None })
}

/// Clones `url` into `dest` (which must not exist or must be empty).
///
/// It is a long, blocking operation: call it outside the render thread.
pub fn clone(url: &str, dest: &Path) -> Result<(), String> {
    clone_with_progress(url, dest, &|_| {})
}

/// Like [`clone`], but reports progress to `on_progress` about 10 times per
/// second while the download lasts.
pub fn clone_with_progress(url: &str, dest: &Path, on_progress: &(dyn Fn(Progress) + Sync)) -> Result<(), String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("git: missing URL".into());
    }
    if dest.exists() && dest.read_dir().map(|mut d| d.next().is_some()).unwrap_or(true) {
        return Err(format!("git: {} already exists and is not empty", dest.display()));
    }
    let interrupt = AtomicBool::new(false);
    let root = gix::progress::tree::Root::new();
    let finished = AtomicBool::new(false);

    std::thread::scope(|scope| {
        scope.spawn(|| {
            let mut last: Option<Progress> = None;
            while !finished.load(std::sync::atomic::Ordering::Relaxed) {
                let now = current_progress(&root);
                if now != last {
                    if let Some(p) = &now {
                        on_progress(p.clone());
                    }
                    last = now;
                }
                std::thread::sleep(std::time::Duration::from_millis(40));
            }
        });

        let result: Result<(), String> = (|| {
            let mut prepare = gix::prepare_clone(url, dest).map_err(|e| err("clonar", e))?;
            let (mut checkout, _) = prepare
                .fetch_then_checkout(root.add_child("descarga"), &interrupt)
                .map_err(|e| err("descarga", e))?;
            checkout
                .main_worktree(root.add_child("checkout"), &interrupt)
                .map_err(|e| err("checkout", e))?;
            Ok(())
        })();
        finished.store(true, std::sync::atomic::Ordering::Relaxed);
        result
    })?;
    // `gix` tasks disappear when they finish, so the end is reported separately.
    on_progress(Progress { label: "listo".into(), done: 1, total: Some(1) });
    Ok(())
}

/// Default folder name when cloning `url` (`https://x/y/repo.git` -> `repo`).
pub fn repo_name_from_url(url: &str) -> Option<String> {
    let last = url.trim().trim_end_matches('/').rsplit(['/', ':']).next()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    (!name.is_empty()).then(|| name.to_string())
}

pub(crate) fn open(dir: &Path) -> Result<Repository, String> {
    Repository::discover(dir).map_err(|_| format!("git: {} is not inside a repository", dir.display()))
}

/// Root of the repository that contains `dir`.
pub fn repo_root(dir: &Path) -> Result<PathBuf, String> {
    let repo = open(dir)?;
    repo.workdir().map(Path::to_path_buf).ok_or_else(|| "git: repository has no working tree".into())
}

/// Current branch (or a detached `HEAD` / `(no commits)`).
pub fn branch(dir: &Path) -> Result<String, String> {
    let repo = open(dir)?;
    let name = match repo.head() {
        Ok(head) => head.shorthand().unwrap_or("HEAD").to_string(),
        Err(_) => "(no commits)".to_string(),
    };
    Ok(name)
}

/// Modified, added, deleted or untracked files.
pub fn status(dir: &Path) -> Result<Vec<Change>, String> {
    let repo = open(dir)?;
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true).renames_head_to_index(true);
    let statuses = repo.statuses(Some(&mut opts)).map_err(|e| err("estado", e))?;
    let mut out = Vec::new();
    for entry in statuses.iter() {
        let s = entry.status();
        if s.is_ignored() {
            continue;
        }
        let code = if s.is_wt_new() && !s.is_index_new() {
            "??"
        } else if s.is_conflicted() {
            "U"
        } else if s.is_index_new() {
            "A"
        } else if s.is_index_deleted() || s.is_wt_deleted() {
            "D"
        } else if s.is_index_renamed() || s.is_wt_renamed() {
            "R"
        } else {
            "M"
        };
        out.push(Change { path: entry.path().unwrap_or("?").to_string(), code: code.into() });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// `git status -s` as text, with the branch on the first line.
pub fn status_text(dir: &Path) -> Result<String, String> {
    let changes = status(dir)?;
    let mut text = format!("## {}\n", branch(dir)?);
    if changes.is_empty() {
        text.push_str("(no changes)\n");
    }
    for c in changes {
        text.push_str(&format!("{:>2} {}\n", c.code, c.path));
    }
    Ok(text)
}

/// Changes of the working tree against `HEAD` as a unified patch.
pub fn diff(dir: &Path) -> Result<String, String> {
    let repo = open(dir)?;
    let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let mut opts = DiffOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true).show_untracked_content(true);
    let diff = repo
        .diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut opts))
        .map_err(|e| err("diff", e))?;
    let mut text = String::new();
    diff.print(DiffFormat::Patch, |_, _, line| {
        if matches!(line.origin(), '+' | '-' | ' ') {
            text.push(line.origin());
        }
        text.push_str(&String::from_utf8_lossy(line.content()));
        true
    })
    .map_err(|e| err("diff", e))?;
    if text.is_empty() {
        text.push_str("(no changes)\n");
    }
    Ok(text)
}

/// Adds all the changes and creates a commit. Returns the short id.
pub fn commit_all(dir: &Path, message: &str) -> Result<String, String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("git: the commit message cannot be empty".into());
    }
    let repo = open(dir)?;
    let mut index = repo.index().map_err(|e| err("index", e))?;
    index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None).map_err(|e| err("add", e))?;
    index.update_all(["*"].iter(), None).map_err(|e| err("actualizar", e))?; // registra borrados
    index.write().map_err(|e| err("index", e))?;
    let tree_id = index.write_tree().map_err(|e| err("tree", e))?;
    let tree = repo.find_tree(tree_id).map_err(|e| err("tree", e))?;

    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    if let Some(p) = &parent {
        if p.tree_id() == tree_id {
            return Err("git: nothing to commit".into());
        }
    }
    let sig = repo
        .signature()
        .or_else(|_| Signature::now(FALLBACK_NAME, FALLBACK_EMAIL))
        .map_err(|e| err("firma", e))?;
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    let id = repo
        .commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
        .map_err(|e| err("commit", e))?;
    Ok(id.to_string()[..7].to_string())
}

/// Last `n` commits, one per line: `id  summary  (author)`.
pub fn log(dir: &Path, n: usize) -> Result<String, String> {
    let repo = open(dir)?;
    let mut walk = repo.revwalk().map_err(|e| err("log", e))?;
    if walk.push_head().is_err() {
        return Ok("(no commits)\n".into());
    }
    let mut text = String::new();
    for oid in walk.take(n) {
        let commit = repo.find_commit(oid.map_err(|e| err("log", e))?).map_err(|e| err("log", e))?;
        text.push_str(&format!(
            "{}  {}  ({})\n",
            &commit.id().to_string()[..7],
            commit.summary().ok().flatten().unwrap_or("(message is not UTF-8)"),
            commit.author().name().unwrap_or("?")
        ));
    }
    if text.is_empty() {
        text.push_str("(no commits)\n");
    }
    Ok(text)
}

/// Result of a request, ready to show in the editor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitOutcome {
    /// The operation finished well.
    pub ok: bool,
    /// Message for the status bar.
    pub message: String,
    /// Long text (status, diff, log) to open in a buffer: (name, content).
    pub view: Option<(String, String)>,
    /// Workspace folder worth showing in the browser.
    pub browse: Option<String>,
    /// Repository that was operated on, to remember it for the next order.
    pub repo: Option<PathBuf>,
}

/// Finds the repository to operate on: the open file's folder, then the
/// last one used and finally the workspace root.
fn locate(ws: &Workspace, file: &str, last_repo: Option<&Path>) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    // The git output buffers (`[git status]`) do not belong to any folder.
    if !file.starts_with('[') {
        candidates.push(ws.root().join(parent_of(file)));
    }
    candidates.extend(last_repo.map(Path::to_path_buf));
    candidates.push(ws.root().to_path_buf());
    candidates
        .into_iter()
        .find_map(|dir| repo_root(&dir).ok())
        .ok_or_else(|| "git: open a file in a repository, or clone one with :clone <url>".to_string())
}

/// Host of `origin` of the repository we are in, or `github.com` if there is none.
fn default_host(ws: &Workspace, file: &str, last_repo: Option<&Path>) -> String {
    locate(ws, file, last_repo)
        .ok()
        .and_then(|dir| open(&dir).ok())
        .and_then(|repo| {
            let remote = repo.find_remote("origin").ok()?;
            host_of(remote.url().ok()?)
        })
        .unwrap_or_else(|| "github.com".to_string())
}

/// Hints for clone failures: `gix`'s messages are technical and a 401 arrives
/// as `Transport handshake failed`, without mentioning authentication.
fn clone_failure(message: String, has_token: bool) -> String {
    let lower = message.to_lowercase();
    let auth = ["401", "403", "authenticat", "unauthorized", "credential", "permission denied"]
        .iter()
        .any(|k| lower.contains(k));
    let handshake = lower.contains("handshake");
    match (auth, handshake, has_token) {
        (true, _, false) => format!("{message} (private repository? save a token with :git token <token>)"),
        (true, _, true) => format!("{message} (the saved token was rejected or lacks permission)"),
        (false, true, false) => format!(
            "{message} (if the repository is private, save a token with :git token <token>; otherwise check the URL and your connection)"
        ),
        (false, true, true) => format!(
            "{message} (the server refused the connection: check the URL, and that the token is valid and has permission)"
        ),
        _ => message,
    }
}

/// Runs a git request. It can take a while (network, disk): call it outside the render thread.
pub fn execute(
    req: &GitRequest,
    ws: &Workspace,
    file: &str,
    last_repo: Option<&Path>,
    on_progress: &(dyn Fn(Progress) + Sync),
) -> GitOutcome {
    let fail = |message: String| GitOutcome { message, ..GitOutcome::default() };
    if let GitRequest::Clone { url, name } = req {
        let Some(name) = name.clone().or_else(|| repo_name_from_url(url)) else {
            return fail("git: could not work out the folder name; give one: :clone <url> <folder>".into());
        };
        let dest = match ws.resolve(&name) {
            Ok(d) => d,
            Err(e) => return fail(e),
        };
        // Only what this attempt could have created is cleaned up: never a pre-existing folder.
        let existed_before = dest.exists();
        // If the URL carried a user and a password, they are discarded: the valid credentials are the
        // ones saved with `:git token`, and they must never end up in the repo's configuration.
        let clean_url = strip_userinfo(url.trim());
        let credential = host_of(&clean_url).and_then(|h| net::lookup(&h));
        let result = if credential.is_some() && clean_url.starts_with("http") {
            clone_authenticated(&clean_url, &dest, on_progress)
        } else {
            clone_with_progress(&clean_url, &dest, on_progress)
        };
        // A repository just created on the server has no commits or branches.
        if result.is_ok() {
            let _ = align_unborn_head(&dest);
        }
        return match result {
            Ok(()) => GitOutcome {
                ok: true,
                message: format!("cloned into {name}/"),
                browse: Some(name),
                repo: Some(dest),
                ..GitOutcome::default()
            },
            Err(e) => {
                // A half-done clone must not block the next attempt.
                if !existed_before {
                    let _ = std::fs::remove_dir_all(&dest);
                }
                fail(net::redact(&clone_failure(e, credential.is_some())))
            }
        };
    }

    // Credentials: they do not need a repository.
    match req {
        GitRequest::SetToken { host, user, token } => {
            let host = host.clone().unwrap_or_else(|| default_host(ws, file, last_repo));
            let user = user.clone().unwrap_or_else(|| DEFAULT_USER.to_string());
            return match net::save(&host, &user, token.expose()) {
                Ok(()) => GitOutcome { ok: true, message: format!("token saved for {host}"), ..GitOutcome::default() },
                Err(e) => fail(net::redact(&e)),
            };
        }
        GitRequest::Logout(host) => {
            return match net::remove(host.as_deref()) {
                Ok(0) => fail("git: there were no saved credentials for that".into()),
                Ok(n) => GitOutcome { ok: true, message: format!("credentials forgotten ({n})"), ..GitOutcome::default() },
                Err(e) => fail(e),
            };
        }
        GitRequest::Credentials => {
            let hosts: Vec<String> = net::load().into_iter().map(|c| format!("{} ({})", c.host, c.user)).collect();
            let unreadable = net::unreadable_hosts();
            let mut message = if hosts.is_empty() && unreadable.is_empty() {
                "no saved credentials (:git token <token>)".to_string()
            } else if hosts.is_empty() {
                "ninguna credencial legible".to_string()
            } else {
                format!("saved credentials: {}", hosts.join(", "))
            };
            if !unreadable.is_empty() {
                message.push_str(&format!(
                    " | cannot be decrypted (save the token again): {}",
                    unreadable.join(", ")
                ));
            }
            return GitOutcome { ok: true, message, ..GitOutcome::default() };
        }
        _ => {}
    }

    let dir = match locate(ws, file, last_repo) {
        Ok(d) => d,
        Err(e) => return fail(e),
    };
    let view = |name: &str, text: Result<String, String>| match text {
        Ok(text) => GitOutcome {
            ok: true,
            message: format!("{name}"),
            view: Some((name.to_string(), text)),
            repo: Some(dir.clone()),
            ..GitOutcome::default()
        },
        Err(e) => fail(e),
    };
    match req {
        GitRequest::Status => view("[git status]", status_text(&dir)),
        GitRequest::Diff => view("[git diff]", diff(&dir)),
        GitRequest::Log => view("[git log]", log(&dir, 50)),
        GitRequest::Commit(message) => match commit_all(&dir, message) {
            Ok(id) => GitOutcome {
                ok: true,
                message: format!("commit {id}: {message}"),
                repo: Some(dir.clone()),
                ..GitOutcome::default()
            },
            Err(e) => fail(e),
        },
        GitRequest::Fetch | GitRequest::Pull | GitRequest::Push => {
            let result = match req {
                GitRequest::Fetch => fetch(&dir, on_progress),
                GitRequest::Pull => pull(&dir, on_progress),
                _ => push(&dir, on_progress),
            };
            match result {
                Ok(message) => GitOutcome { ok: true, message, repo: Some(dir.clone()), ..GitOutcome::default() },
                Err(e) => fail(net::redact(&e)),
            }
        }
        GitRequest::Clone { .. } | GitRequest::SetToken { .. } | GitRequest::Logout(_) | GitRequest::Credentials => {
            unreachable!("tratado arriba")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Unique temporary folder, deleted at the end.
    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("atlas-git-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Tmp(dir)
        }
        fn write(&self, rel: &str, text: &str) {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn init(tmp: &Tmp) {
        Repository::init(&tmp.0).unwrap();
    }

    #[test]
    fn repo_name_is_taken_from_the_url() {
        assert_eq!(repo_name_from_url("https://github.com/o/repo.git"), Some("repo".into()));
        assert_eq!(repo_name_from_url("https://github.com/o/repo"), Some("repo".into()));
        assert_eq!(repo_name_from_url("https://github.com/o/repo/"), Some("repo".into()));
        assert_eq!(repo_name_from_url("git@github.com:o/repo.git"), Some("repo".into()));
        assert_eq!(repo_name_from_url("   "), None);
    }

    #[test]
    fn outside_a_repo_gives_a_clear_error() {
        let tmp = Tmp::new("norepo");
        // The system's temporary folder should not be inside a repo.
        let e = status(&tmp.0);
        assert!(e.is_err() || e.unwrap().is_empty());
    }

    #[test]
    fn status_reports_untracked_then_added_then_modified() {
        let tmp = Tmp::new("status");
        init(&tmp);
        tmp.write("a.txt", "uno\n");
        assert_eq!(status(&tmp.0).unwrap(), vec![Change { path: "a.txt".into(), code: "??".into() }]);

        commit_all(&tmp.0, "primero").unwrap();
        assert!(status(&tmp.0).unwrap().is_empty());

        tmp.write("a.txt", "uno\ndos\n");
        tmp.write("src/b.rs", "fn b() {}\n");
        let s = status(&tmp.0).unwrap();
        assert_eq!(
            s,
            vec![
                Change { path: "a.txt".into(), code: "M".into() },
                Change { path: "src/b.rs".into(), code: "??".into() },
            ]
        );
    }

    #[test]
    fn commit_creates_history_and_refuses_empty_commits() {
        let tmp = Tmp::new("commit");
        init(&tmp);
        assert_eq!(branch(&tmp.0).unwrap(), "(no commits)");
        assert!(log(&tmp.0, 5).unwrap().contains("no commits"));

        tmp.write("a.txt", "uno\n");
        let id = commit_all(&tmp.0, "añade a").unwrap();
        assert_eq!(id.len(), 7);
        assert!(commit_all(&tmp.0, "again").unwrap_err().contains("nothing to commit"));

        tmp.write("a.txt", "uno\ndos\n");
        commit_all(&tmp.0, "cambia a").unwrap();
        let log = log(&tmp.0, 10).unwrap();
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("cambia a"), "{log}");
        assert!(lines[1].contains("añade a"), "{log}");
        assert!(lines[0].contains("Atlas Code") || lines[0].contains('('), "{log}");
    }

    #[test]
    fn commit_requires_a_message() {
        let tmp = Tmp::new("msg");
        init(&tmp);
        tmp.write("a.txt", "x");
        assert!(commit_all(&tmp.0, "   ").is_err());
    }

    #[test]
    fn commit_registers_deleted_files() {
        let tmp = Tmp::new("del");
        init(&tmp);
        tmp.write("a.txt", "x\n");
        tmp.write("b.txt", "y\n");
        commit_all(&tmp.0, "dos archivos").unwrap();
        fs::remove_file(tmp.0.join("b.txt")).unwrap();
        assert_eq!(status(&tmp.0).unwrap(), vec![Change { path: "b.txt".into(), code: "D".into() }]);
        commit_all(&tmp.0, "borra b").unwrap();
        assert!(status(&tmp.0).unwrap().is_empty());
    }

    #[test]
    fn diff_shows_added_and_removed_lines() {
        let tmp = Tmp::new("diff");
        init(&tmp);
        tmp.write("a.txt", "uno\ndos\n");
        commit_all(&tmp.0, "base").unwrap();
        assert!(diff(&tmp.0).unwrap().contains("no changes"));
        tmp.write("a.txt", "uno\ntres\n");
        let d = diff(&tmp.0).unwrap();
        assert!(d.contains("-dos"), "{d}");
        assert!(d.contains("+tres"), "{d}");
        assert!(d.contains("a.txt"), "{d}");
    }

    #[test]
    fn status_text_has_branch_header() {
        let tmp = Tmp::new("text");
        init(&tmp);
        tmp.write("a.txt", "x");
        let t = status_text(&tmp.0).unwrap();
        assert!(t.starts_with("## "), "{t}");
        assert!(t.contains("?? a.txt"), "{t}");
    }

    #[test]
    fn repo_root_is_found_from_a_subfolder() {
        let tmp = Tmp::new("root");
        init(&tmp);
        tmp.write("src/deep/x.rs", "");
        let root = repo_root(&tmp.0.join("src/deep")).unwrap();
        assert_eq!(root.canonicalize().unwrap(), tmp.0.canonicalize().unwrap());
    }

    #[test]
    fn clone_rejects_bad_arguments_without_touching_the_network() {
        let tmp = Tmp::new("clonebad");
        assert!(clone("  ", &tmp.0.join("x")).unwrap_err().contains("missing URL"));
        tmp.write("ocupada/archivo", "x");
        assert!(clone("https://example.invalid/r.git", &tmp.0.join("ocupada")).unwrap_err().contains("is not empty"));
    }

    /// Necesita red: `cargo test -p atlas-git -- --ignored`.
    #[test]
    #[ignore = "uses the network"]
    fn clone_a_small_public_repo_over_https() {
        let tmp = Tmp::new("clonenet");
        let dest = tmp.0.join("Hello-World");
        clone("https://github.com/octocat/Hello-World.git", &dest).unwrap();
        assert!(dest.join("README").exists() || dest.join("README.md").exists());
        assert!(!log(&dest, 3).unwrap().contains("no commits"));
        assert!(status(&dest).unwrap().is_empty());
    }

    // ----- execute -----

    fn workspace(tag: &str) -> (Tmp, Workspace) {
        let tmp = Tmp::new(tag);
        let ws = Workspace::new(&tmp.0);
        (tmp, ws)
    }

    #[test]
    fn execute_status_diff_log_commit_roundtrip() {
        let (tmp, ws) = workspace("exec");
        Repository::init(tmp.0.join("proy")).unwrap();
        tmp.write("proy/src/a.rs", "fn a() {}
");

        let st = execute(&GitRequest::Status, &ws, "proy/src/a.rs", None, &|_| {});
        let (name, text) = st.view.clone().unwrap();
        assert_eq!(name, "[git status]");
        assert!(text.contains("?? src/a.rs"), "{text}");
        assert!(st.repo.is_some());

        let c = execute(&GitRequest::Commit("primero".into()), &ws, "proy/src/a.rs", None, &|_| {});
        assert!(c.message.starts_with("commit "), "{}", c.message);
        assert!(c.message.contains("primero"));

        tmp.write("proy/src/a.rs", "fn a() { 1 }
");
        let d = execute(&GitRequest::Diff, &ws, "proy/src/a.rs", None, &|_| {});
        assert!(d.view.unwrap().1.contains("+fn a() { 1 }"));

        let l = execute(&GitRequest::Log, &ws, "proy/src/a.rs", None, &|_| {});
        assert!(l.view.unwrap().1.contains("primero"));
    }

    #[test]
    fn execute_remembers_the_repo_for_output_buffers() {
        let (tmp, ws) = workspace("exec-last");
        Repository::init(tmp.0.join("proy")).unwrap();
        tmp.write("proy/a.txt", "x");
        let first = execute(&GitRequest::Status, &ws, "proy/a.txt", None, &|_| {});
        let repo = first.repo.unwrap();
        // From a `[git status]` buffer there is no folder: it must use the remembered repo.
        let again = execute(&GitRequest::Commit("from the view".into()), &ws, "[git status]", Some(&repo), &|_| {});
        assert!(again.message.starts_with("commit "), "{}", again.message);
    }

    #[test]
    fn execute_outside_a_repo_explains_what_to_do() {
        let (tmp, ws) = workspace("exec-norepo");
        tmp.write("suelto/a.txt", "x");
        let out = execute(&GitRequest::Status, &ws, "suelto/a.txt", None, &|_| {});
        assert!(out.view.is_none());
        assert!(out.message.contains(":clone"), "{}", out.message);
    }

    #[test]
    fn execute_clone_failure_reports_error_and_leaves_no_folder() {
        let (tmp, ws) = workspace("exec-clonefail");
        let req = GitRequest::Clone { url: "https://127.0.0.1:9/nada.git".into(), name: None };
        let out = execute(&req, &ws, "", None, &|_| {});
        assert!(out.message.starts_with("git:"), "{}", out.message);
        assert!(out.browse.is_none());
        assert!(!tmp.0.join("nada").exists(), "no half-done folder must remain");
    }

    #[test]
    fn execute_clone_rejects_paths_that_escape_the_workspace() {
        let (_tmp, ws) = workspace("exec-cloneesc");
        let req = GitRequest::Clone { url: "https://x/y/r.git".into(), name: Some("../fuera".into()) };
        assert!(execute(&req, &ws, "", None, &|_| {}).message.starts_with("E212"));
    }

    #[test]
    fn execute_clone_needs_a_deducible_name() {
        let (_tmp, ws) = workspace("exec-clonename");
        let req = GitRequest::Clone { url: "   ".into(), name: None };
        assert!(execute(&req, &ws, "", None, &|_| {}).message.contains("folder name"));
    }

    #[test]
    fn failed_clone_never_deletes_a_preexisting_folder() {
        let (tmp, ws) = workspace("exec-keep");
        tmp.write("existente/importante.txt", "do not delete me");
        let req = GitRequest::Clone { url: "https://x/y/r.git".into(), name: Some("existente".into()) };
        let out = execute(&req, &ws, "", None, &|_| {});
        assert!(out.message.contains("is not empty"), "{}", out.message);
        assert_eq!(fs::read_to_string(tmp.0.join("existente/importante.txt")).unwrap(), "do not delete me");
    }

    #[test]
    #[ignore = "uses the network"]
    fn clone_reports_phases_and_percentages() {
        use std::sync::Mutex;
        let tmp = Tmp::new("cloneprog");
        let seen: Mutex<Vec<Progress>> = Mutex::new(Vec::new());
        clone_with_progress("https://github.com/dtolnay/anyhow.git", &tmp.0.join("anyhow"), &|p| {
            seen.lock().unwrap().push(p)
        })
        .unwrap();
        let seen = seen.into_inner().unwrap();
        let mut phases: Vec<String> = Vec::new();
        for p in &seen {
            if phases.last() != Some(&p.label) {
                phases.push(p.label.clone());
            }
        }
        println!("fases vistas: {phases:?}");
        let percents: Vec<u8> = seen.iter().filter_map(Progress::percent).collect();
        println!("porcentajes: {:?}", percents.iter().step_by((percents.len() / 12).max(1)).collect::<Vec<_>>());
        assert!(!seen.is_empty(), "no progress was reported");
        assert!(percents.iter().all(|p| *p <= 100));
        assert!(phases.iter().any(|p| p == "recibiendo objetos"), "{phases:?}");
        assert_eq!(*percents.last().unwrap(), 100, "debe terminar al 100 %");
        assert_eq!(phases.last().unwrap(), "listo");
    }

    /// Needs the network. It is the path private repos use: libgit2 + our own transport
    /// + TLS with the bundled root certificates, against a real HTTPS server.
    #[test]
    #[ignore = "uses the network"]
    fn authenticated_clone_path_works_over_real_https() {
        let tmp = Tmp::new("https-real");
        let dest = tmp.0.join("Hello-World");
        clone_authenticated("https://github.com/octocat/Hello-World.git", &dest, &|_| {}).unwrap();
        assert!(dest.join("README").exists());
        assert!(!log(&dest, 3).unwrap().contains("no commits"));
        // fetch and pull also go through our own transport.
        assert!(fetch(&dest, &|_| {}).unwrap().starts_with("fetch:"));
        assert!(pull(&dest, &|_| {}).unwrap().contains("up to date"));
    }
}

