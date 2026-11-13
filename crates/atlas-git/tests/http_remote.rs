//! End-to-end test against an HTTP git server with authentication:
//! cloning a private repository, push, pull, divergence and credentials.
//!
//! Needs `git` and `python` on the PATH (if they are missing, the test is skipped with a notice).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use atlas_editor::{GitRequest, Secret, Workspace};
use atlas_git::{execute, set_credentials_file, GitOutcome};

/// These tests share the process's global state (the credentials file and git's
/// environment variables), so they cannot run at the same time.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

const TOKEN: &str = "tok-secreto-9f3a1c";

/// Temporary folder that is deleted at the end.
struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("atlas-http-{tag}-{}", std::process::id()));
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

/// The test server; it stops when dropped.
struct Server {
    child: Child,
    port: u16,
}
impl Server {
    fn start(root: &Path, auth: &str) -> Option<Server> {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/git_http_server.py");
        let mut child = Command::new("python")
            .arg(script)
            .arg(root)
            .arg(auth)
            .stdout(Stdio::piped())
            .spawn()
            .ok()?;
        let mut line = String::new();
        BufReader::new(child.stdout.take()?).read_line(&mut line).ok()?;
        Some(Server { port: line.trim().parse().ok()?, child })
    }
    fn url(&self, repo: &str) -> String {
        format!("http://127.0.0.1:{}/{repo}", self.port)
    }
    fn host(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Otro")
        .env("GIT_AUTHOR_EMAIL", "otro@localhost")
        .env("GIT_COMMITTER_NAME", "Otro")
        .env("GIT_COMMITTER_EMAIL", "otro@localhost")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git not available");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Isolates the test from the git configuration of whoever runs it: without this, `gix`
/// calls the system credential manager, which may ask for data interactively.
fn isolate_from_user_git_config(base: &Path) {
    let empty = base.join("gitconfig-vacio");
    std::fs::write(&empty, "").unwrap();
    std::env::set_var("GIT_CONFIG_GLOBAL", &empty);
    std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    std::env::set_var("GIT_TERMINAL_PROMPT", "0");
    std::env::set_var("GCM_INTERACTIVE", "never");
}

/// Reads a file ignoring line-ending conversion (`core.autocrlf` on Windows).
fn read(path: impl AsRef<Path>) -> String {
    std::fs::read_to_string(path).unwrap().replace("\r\n", "\n")
}

fn have(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

fn run(ws: &Workspace, req: GitRequest, file: &str) -> GitOutcome {
    execute(&req, ws, file, None, &|_| {})
}

fn ok(outcome: &GitOutcome) -> &str {
    assert!(outcome.ok, "it should have worked but failed: {}", outcome.message);
    &outcome.message
}

fn token(host: &str, secret: &str) -> GitRequest {
    GitRequest::SetToken { host: Some(host.into()), user: None, token: Secret::new(secret) }
}

/// Pushes a first commit to `remote` (an empty repository with receive-pack enabled).
fn seed_remote(base: &Path) {
    let bare = base.join("server/proyecto.git");
    std::fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "--bare", "-b", "main"]);
    git(&bare, &["config", "http.receivepack", "true"]);

    let seed = base.join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    git(&seed, &["init", "-b", "main"]);
    std::fs::write(seed.join("a.txt"), "uno\n").unwrap();
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-m", "inicial"]);
    git(&seed, &["push", bare.to_str().unwrap(), "main"]);
}

#[test]
fn private_repository_end_to_end() {
    if !have("git") || !have("python") {
        eprintln!("SKIPPED: git or python missing from the PATH");
        return;
    }
    let _serial = serial();
    let tmp = Tmp::new("e2e");
    let base = tmp.0.as_path();
    isolate_from_user_git_config(base);
    seed_remote(base);
    let server = Server::start(&base.join("server"), &format!("git:{TOKEN}")).expect("test server");
    set_credentials_file(base.join("credenciales"));
    atlas_git::install_transport();
    let ws = Workspace::new(base.join("ws"));
    std::fs::create_dir_all(ws.root()).unwrap();
    let url = server.url("proyecto.git");
    let clone = |name: &str| GitRequest::Clone { url: url.clone(), name: Some(name.into()) };

    // 1. Without a token, the private repository is refused with a useful hint and without leaving a trace.
    let denied = run(&ws, clone("proy"), "");
    assert!(!denied.ok);
    assert!(denied.message.contains(":git token"), "no hint of what to do: {}", denied.message);
    assert!(!ws.root().join("proy").exists(), "no half-done folder must remain");

    // 2. With the token saved, it clones.
    ok(&run(&ws, token(&server.host(), TOKEN), ""));
    let cloned = run(&ws, clone("proy"), "");
    assert_eq!(ok(&cloned), "cloned into proy/");
    let repo = ws.root().join("proy");
    assert_eq!(read(repo.join("a.txt")), "uno\n");

    // 3. The token is not written in the repository's configuration.
    let config = std::fs::read_to_string(repo.join(".git/config")).unwrap();
    assert!(!config.contains(TOKEN), "the token ended up in .git/config:\n{config}");
    assert!(config.contains(&format!("url = {url}")), "the remote must stay clean:\n{config}");

    // 4. Commit and push: the server receives it.
    std::fs::write(repo.join("b.txt"), "dos\n").unwrap();
    ok(&run(&ws, GitRequest::Commit("add b".into()), "proy/a.txt"));
    let pushed = run(&ws, GitRequest::Push, "proy/a.txt");
    assert!(ok(&pushed).starts_with("push: main -> origin/main"), "{}", pushed.message);
    let server_log = git(&base.join("server/proyecto.git"), &["log", "--oneline", "main"]);
    assert!(server_log.contains("add b"), "the server did not receive the commit:\n{server_log}");

    // 5. With a wrong token the push is refused with a clear message, without leaking anything.
    ok(&run(&ws, token(&server.host(), "token-equivocado"), ""));
    std::fs::write(repo.join("c.txt"), "tres\n").unwrap();
    ok(&run(&ws, GitRequest::Commit("add c".into()), "proy/a.txt"));
    let bad = run(&ws, GitRequest::Push, "proy/a.txt");
    assert!(!bad.ok);
    assert!(bad.message.contains("rejected"), "{}", bad.message);
    assert!(!bad.message.contains("equivocado") && !bad.message.contains(TOKEN), "token filtrado: {}", bad.message);
    ok(&run(&ws, token(&server.host(), TOKEN), ""));
    ok(&run(&ws, GitRequest::Push, "proy/a.txt"));

    // 6. Someone else pushes changes; a pull brings them by fast-forward.
    let other = base.join("otro");
    let auth_url = format!("http://git:{TOKEN}@127.0.0.1:{}/proyecto.git", server.port);
    git(base, &["clone", &auth_url, other.to_str().unwrap()]);
    std::fs::write(other.join("d.txt"), "cuatro\n").unwrap();
    git(&other, &["add", "."]);
    git(&other, &["commit", "-m", "desde otro"]);
    git(&other, &["push", "origin", "main"]);

    let pulled = run(&ws, GitRequest::Pull, "proy/a.txt");
    assert!(ok(&pulled).contains("advanced"), "{}", pulled.message);
    assert_eq!(read(repo.join("d.txt")), "cuatro\n");
    let again = run(&ws, GitRequest::Pull, "proy/a.txt");
    assert!(ok(&again).contains("up to date"), "{}", again.message);

    // 7. Divergence: nothing is merged and nothing is lost.
    std::fs::write(repo.join("local.txt"), "mio\n").unwrap();
    ok(&run(&ws, GitRequest::Commit("cambio local".into()), "proy/a.txt"));
    std::fs::write(other.join("remoto.txt"), "suyo\n").unwrap();
    git(&other, &["add", "."]);
    git(&other, &["commit", "-m", "cambio remoto"]);
    git(&other, &["push", "origin", "main"]);
    let diverged = run(&ws, GitRequest::Pull, "proy/a.txt");
    assert!(!diverged.ok);
    assert!(diverged.message.contains("diverged"), "{}", diverged.message);
    assert!(repo.join("local.txt").exists(), "the local work must still be there");
    assert!(!repo.join("remoto.txt").exists(), "it must not have merged anything");

    // 8. Push of a branch the server rejects (not a fast-forward): it is explained.
    let rejected = run(&ws, GitRequest::Push, "proy/a.txt");
    assert!(!rejected.ok, "a push that does not advance cannot count as good: {}", rejected.message);

    // 9. Credentials can be listed (without the token) and forgotten.
    let listed = run(&ws, GitRequest::Credentials, "");
    assert!(listed.message.contains(&server.host()) && !listed.message.contains(TOKEN), "{}", listed.message);
    ok(&run(&ws, GitRequest::Logout(None), ""));
    let anonymous = run(&ws, GitRequest::Fetch, "proy/a.txt");
    assert!(!anonymous.ok);
    assert!(anonymous.message.contains("authentication") || anonymous.message.contains(":git token"), "{}", anonymous.message);
}

#[test]
fn public_repository_works_without_any_token() {
    if !have("git") || !have("python") {
        eprintln!("SKIPPED: git or python missing from the PATH");
        return;
    }
    let _serial = serial();
    let tmp = Tmp::new("public");
    let base = tmp.0.as_path();
    isolate_from_user_git_config(base);
    seed_remote(base);
    let server = Server::start(&base.join("server"), "").expect("test server");
    set_credentials_file(base.join("credenciales-publico"));
    atlas_git::install_transport();
    let ws = Workspace::new(base.join("ws"));
    std::fs::create_dir_all(ws.root()).unwrap();

    let req = GitRequest::Clone { url: server.url("proyecto.git"), name: Some("pub".into()) };
    ok(&run(&ws, req, ""));
    assert!(ws.root().join("pub/a.txt").exists());
    // fetch and pull work without credentials when the server does not ask for them.
    ok(&run(&ws, GitRequest::Fetch, "pub/a.txt"));
    assert!(ok(&run(&ws, GitRequest::Pull, "pub/a.txt")).contains("up to date"));
}

/// The case of a repository just created on GitHub: empty, without branches or commits.
/// It is cloned, the first commit is created and it is pushed.
#[test]
fn empty_repository_first_commit_and_push() {
    if !have("git") || !have("python") {
        eprintln!("SKIPPED: git or python missing from the PATH");
        return;
    }
    let _serial = serial();
    let tmp = Tmp::new("empty");
    let base = tmp.0.as_path();
    isolate_from_user_git_config(base);
    let bare = base.join("server/vacio.git");
    std::fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "--bare", "-b", "main"]);
    git(&bare, &["config", "http.receivepack", "true"]);
    let server = Server::start(&base.join("server"), &format!("git:{TOKEN}")).expect("test server");
    set_credentials_file(base.join("credenciales-vacio"));
    atlas_git::install_transport();
    let ws = Workspace::new(base.join("ws"));
    std::fs::create_dir_all(ws.root()).unwrap();

    ok(&run(&ws, token(&server.host(), TOKEN), ""));
    let clone = GitRequest::Clone { url: server.url("vacio.git"), name: Some("mio".into()) };
    let cloned = run(&ws, clone, "");
    assert!(cloned.ok, "cloning an empty repository must work: {}", cloned.message);
    let repo = ws.root().join("mio");
    assert!(repo.join(".git").exists());

    // Without commits yet: the status and the log must not fail.
    let status = run(&ws, GitRequest::Status, "mio/x");
    assert!(status.ok, "status on an empty repo: {}", status.message);

    // First file, first commit and push of the branch.
    std::fs::write(repo.join("hola.rs"), "fn main() {}
").unwrap();
    ok(&run(&ws, GitRequest::Commit("first commit".into()), "mio/hola.rs"));
    let pushed = run(&ws, GitRequest::Push, "mio/hola.rs");
    assert!(pushed.ok, "the first push to an empty repo failed: {}", pushed.message);

    let log = git(&bare, &["log", "--oneline", "--all"]);
    assert!(log.contains("first commit"), "the server did not receive the commit:
{log}");
    let remote_branches = git(&bare, &["branch"]);
    let local_head = std::fs::read_to_string(repo.join(".git/HEAD")).unwrap();
    assert!(
        remote_branches.contains("main"),
        "the server's branch must be named like its HEAD (main). Server: [{remote_branches}] | local HEAD: [{}]",
        local_head.trim()
    );

    // And from there on it works normally.
    std::fs::write(repo.join("otro.rs"), "// 2
").unwrap();
    ok(&run(&ws, GitRequest::Commit("segundo".into()), "mio/hola.rs"));
    assert!(run(&ws, GitRequest::Push, "mio/hola.rs").ok);
    assert!(run(&ws, GitRequest::Pull, "mio/hola.rs").message.contains("up to date"));
}

