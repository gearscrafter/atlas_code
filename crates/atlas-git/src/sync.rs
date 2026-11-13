//! Syncing with the remote: `fetch`, `pull` (fast-forward only) and `push`.
//!
//! It uses our own HTTP transport from [`crate::net`], which adds the saved token.

use std::path::Path;

use atlas_editor::Progress;
use git2::build::CheckoutBuilder;
use git2::{AutotagOption, BranchType, FetchOptions, PushOptions, RemoteCallbacks, Repository};

use crate::{err, net, open};

fn origin(repo: &Repository) -> Result<git2::Remote<'_>, String> {
    repo.find_remote("origin")
        .map_err(|_| "git: this repository has no 'origin' remote (clone it with :clone)".to_string())
}

fn current_branch(repo: &Repository) -> Result<String, String> {
    let head = repo.head().map_err(|_| "git: the repository has no commits yet".to_string())?;
    if !head.is_branch() {
        return Err("git: HEAD is not on a branch; switch to a branch first".into());
    }
    head.shorthand().map(str::to_string).map_err(|_| "git: invalid branch name".to_string())
}

fn progress_callbacks<'a>(on_progress: &'a (dyn Fn(Progress) + Sync)) -> RemoteCallbacks<'a> {
    let mut callbacks = RemoteCallbacks::new();
    callbacks.transfer_progress(move |stats| {
        let (label, done, total) = if stats.total_objects() > 0 && stats.received_objects() < stats.total_objects() {
            ("recibiendo objetos", stats.received_objects(), stats.total_objects())
        } else if stats.total_deltas() > 0 {
            ("resolviendo deltas", stats.indexed_deltas(), stats.total_deltas())
        } else {
            ("negotiating with the server", 0, 0)
        };
        on_progress(Progress { label: label.into(), done: done as u64, total: Some(total as u64).filter(|t| *t > 0) });
        true
    });
    callbacks
}

/// Clones with `libgit2` and our own transport, which sends the saved token in a
/// header. It is used for hosts with credentials: that way the token never travels in the URL
/// nor ends up written in the repository's configuration.
pub fn clone_authenticated(
    url: &str,
    dest: &Path,
    on_progress: &(dyn Fn(Progress) + Sync),
) -> Result<(), String> {
    net::install_transport();
    let mut options = FetchOptions::new();
    options.remote_callbacks(progress_callbacks(on_progress));
    git2::build::RepoBuilder::new()
        .fetch_options(options)
        .clone(url, dest)
        .map_err(|e| err("clonar", e))?;
    on_progress(Progress { label: "listo".into(), done: 1, total: Some(1) });
    Ok(())
}

/// Branch the server declares as its main one (`refs/heads/main`), if it announces it.
fn remote_default_branch(repo: &Repository) -> Option<String> {
    let mut remote = repo.find_remote("origin").ok()?;
    remote.connect(git2::Direction::Fetch).ok()?;
    let name = remote.default_branch().ok().and_then(|b| b.as_str().ok().map(str::to_string));
    let _ = remote.disconnect();
    name.filter(|n| n.starts_with("refs/heads/"))
}

/// After cloning an **empty** repository (just created, without commits) the client leaves
/// `HEAD` on its default branch (`master`), which may not be the server's (`main`):
/// the first `push` would then create a second branch. It is aligned with the server's and,
/// if it does not announce one, `main` is used. With commits it does nothing.
pub fn align_unborn_head(dir: &Path) -> Result<(), String> {
    net::install_transport();
    let repo = open(dir)?;
    if repo.head().is_ok() {
        return Ok(());
    }
    let branch = remote_default_branch(&repo).unwrap_or_else(|| "refs/heads/main".to_string());
    repo.set_head(&branch).map_err(|e| err("rama inicial", e))
}

/// Downloads the current branch from `origin` to `refs/remotes/origin/<branch>`.
pub fn fetch(dir: &Path, on_progress: &(dyn Fn(Progress) + Sync)) -> Result<String, String> {
    net::install_transport();
    let repo = open(dir)?;
    let branch = current_branch(&repo)?;
    let mut remote = origin(&repo)?;
    let mut options = FetchOptions::new();
    options.remote_callbacks(progress_callbacks(on_progress));
    options.download_tags(AutotagOption::None);
    let refspec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
    remote.fetch(&[&refspec], Some(&mut options), None).map_err(|e| err("fetch", e))?;
    on_progress(Progress { label: "listo".into(), done: 1, total: Some(1) });
    Ok(format!("fetch: origin/{branch} actualizado"))
}

/// `fetch` and fast-forward of the current branch. It does not merge diverged branches or overwrite
/// local changes: in those cases it stops with a clear message.
pub fn pull(dir: &Path, on_progress: &(dyn Fn(Progress) + Sync)) -> Result<String, String> {
    fetch(dir, on_progress)?;
    let repo = open(dir)?;
    let branch = current_branch(&repo)?;
    let upstream = repo
        .refname_to_id(&format!("refs/remotes/origin/{branch}"))
        .map_err(|_| format!("git: origin has no branch {branch}"))?;
    let annotated = repo.find_annotated_commit(upstream).map_err(|e| err("pull", e))?;
    let (analysis, _) = repo.merge_analysis(&[&annotated]).map_err(|e| err("pull", e))?;

    if analysis.is_up_to_date() {
        return Ok(format!("pull: {branch} is already up to date"));
    }
    if !analysis.is_fast_forward() {
        return Err(format!(
            "git: pull: {branch} and origin/{branch} have diverged; only fast-forward is supported \
             (do the merge outside the headset)"
        ));
    }
    // First the working tree is updated in safe mode: if it would overwrite local changes
    // it fails without touching anything, and only then is the branch moved.
    let target = repo.find_object(upstream, None).map_err(|e| err("pull", e))?;
    repo.checkout_tree(&target, Some(CheckoutBuilder::new().safe()))
        .map_err(|e| err("pull: you have local changes that would be lost; commit or discard them", e))?;
    let refname = format!("refs/heads/{branch}");
    repo.find_reference(&refname)
        .and_then(|mut r| r.set_target(upstream, "pull: fast-forward"))
        .map_err(|e| err("pull", e))?;
    repo.set_head(&refname).map_err(|e| err("pull", e))?;
    Ok(format!("pull: {branch} advanced to {}", &upstream.to_string()[..7]))
}

/// Pushes the current branch to `origin` with the same name.
pub fn push(dir: &Path, on_progress: &(dyn Fn(Progress) + Sync)) -> Result<String, String> {
    net::install_transport();
    let repo = open(dir)?;
    let branch = current_branch(&repo)?;
    let mut remote = origin(&repo)?;

    let mut rejected: Option<String> = None;
    {
        let mut callbacks = RemoteCallbacks::new();
        callbacks.push_transfer_progress(|current, total, _bytes| {
            on_progress(Progress {
                label: "enviando objetos".into(),
                done: current as u64,
                total: Some(total as u64).filter(|t| *t > 0),
            });
        });
        // The server can accept the connection and still reject the update
        // (a protected branch, changes that do not fast-forward...): `push` does not fail in that case.
        callbacks.push_update_reference(|name, status| {
            if let Some(message) = status {
                rejected = Some(format!("{name}: {message}"));
            }
            Ok(())
        });
        let mut options = PushOptions::new();
        options.remote_callbacks(callbacks);
        let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
        remote.push(&[&refspec], Some(&mut options)).map_err(|e| err("push", e))?;
    }
    if let Some(reason) = rejected {
        return Err(format!("git: push rejected by the server: {reason}"));
    }
    // Leaves the branch tracking origin so that the status shows ahead/behind.
    if let Ok(mut local) = repo.find_branch(&branch, BranchType::Local) {
        let _ = local.set_upstream(Some(&format!("origin/{branch}")));
    }
    on_progress(Progress { label: "listo".into(), done: 1, total: Some(1) });
    Ok(format!("push: {branch} -> origin/{branch}"))
}
