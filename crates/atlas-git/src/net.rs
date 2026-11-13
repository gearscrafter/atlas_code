//! Git networking: saved credentials and our own HTTP(S) transport for
//! `libgit2`, which was compiled without HTTPS support.
//!
//! The transport uses `reqwest` with the same certificate verifier that already
//! serves for cloning (see `vendor/rustls-platform-verifier`) and adds the token to
//! each request.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;

use git2::transport::{self, Service, SmartSubtransport, SmartSubtransportStream, Transport};
use git2::{Error, Remote};

/// Credentials of a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub host: String,
    pub user: String,
    pub token: String,
}

/// Default user: GitHub and GitLab accept any with a token.
pub const DEFAULT_USER: &str = "git";

static STORE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Sets where the credentials are saved. It must be a private folder of the app.
pub fn set_credentials_file(path: impl Into<PathBuf>) {
    *STORE.lock().unwrap_or_else(|e| e.into_inner()) = Some(path.into());
}

fn store_path() -> Option<PathBuf> {
    STORE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// `https://user:password@github.com:443/o/r.git` -> `github.com:443`.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Removes the credentials embedded in a URL (`https://x:y@host/..`).
pub fn strip_userinfo(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else { return url.to_string() };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    match rest[..end].rfind('@') {
        Some(at) => format!("{scheme}://{}", &rest[at + 1..]),
        None => url.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Token encryption
// ---------------------------------------------------------------------------

/// Encrypts and decrypts the saved tokens. On Android a Keystore key does it,
/// which never leaves the device; here it is a swappable piece so it can be tested.
pub trait SecretBox: Send + Sync {
    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, String>;
    fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, String>;
}

static CIPHER: Mutex<Option<Arc<dyn SecretBox>>> = Mutex::new(None);
static REQUIRE_ENCRYPTION: AtomicBool = AtomicBool::new(false);

/// Prefix (and version) of an encrypted token in the file.
const ENC_PREFIX: &str = "enc1:";

/// Installs (or removes) the token cipher.
pub fn set_secret_box(cipher: Option<Arc<dyn SecretBox>>) {
    *CIPHER.lock().unwrap_or_else(|e| e.into_inner()) = cipher;
}

/// With `true`, a token is never saved in plain text: without a cipher available, saving fails.
pub fn require_encryption(required: bool) {
    REQUIRE_ENCRYPTION.store(required, Ordering::SeqCst);
}

fn cipher() -> Option<Arc<dyn SecretBox>> {
    CIPHER.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn encode_token(token: &str) -> Result<String, String> {
    match cipher() {
        Some(c) => {
            let sealed = c.seal(token.as_bytes()).map_err(|e| format!("git: could not encrypt the token: {e}"))?;
            Ok(format!("{ENC_PREFIX}{}", B64.encode(sealed)))
        }
        None if REQUIRE_ENCRYPTION.load(Ordering::SeqCst) => {
            Err("git: credential encryption is not available; the token is not stored in plain text".into())
        }
        None => Ok(token.to_string()),
    }
}

fn decode_token(raw: &str) -> Result<String, String> {
    let Some(encoded) = raw.strip_prefix(ENC_PREFIX) else { return Ok(raw.to_string()) };
    let cipher = cipher().ok_or("encryption is not available")?;
    let sealed = B64.decode(encoded).map_err(|_| "the credentials file is damaged".to_string())?;
    let plain = cipher.open(&sealed)?;
    String::from_utf8(plain).map_err(|_| "the decrypted token is not valid".to_string())
}

// ---------------------------------------------------------------------------
// Credentials file
// ---------------------------------------------------------------------------

/// A line of the file. `raw` is the token as it is on disk (encrypted or not): the
/// entries that cannot be decrypted are kept intact when the file is rewritten,
/// so they are not lost because of a transient cipher failure.
struct Entry {
    host: String,
    user: String,
    raw: String,
}

fn read_entries() -> Vec<Entry> {
    let Some(path) = store_path() else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            Some(Entry {
                host: parts.next()?.to_string(),
                user: parts.next()?.to_string(),
                raw: parts.next()?.to_string(),
            })
        })
        .collect()
}

fn write_entries(entries: &[Entry]) -> Result<(), String> {
    let path = store_path().ok_or("git: nowhere to store credentials")?;
    let text: String = entries.iter().map(|e| format!("{}\t{}\t{}\n", e.host, e.user, e.raw)).collect();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("git: credentials: {e}"))?;
    }
    // It is written to a temporary file and renamed: it is never left half-written.
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("git: credentials: {e}"))?;
    restrict_permissions(&tmp);
    std::fs::rename(&tmp, &path).map_err(|e| format!("git: credentials: {e}"))
}

/// Only the owner can read it (on Android the folder is already private to the app).
fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Usable (decrypted) credentials.
pub fn load() -> Vec<Credential> {
    read_entries()
        .into_iter()
        .filter_map(|e| Some(Credential { token: decode_token(&e.raw).ok()?, host: e.host, user: e.user }))
        .collect()
}

/// Hosts whose token is saved but cannot be decrypted right now.
pub fn unreadable_hosts() -> Vec<String> {
    read_entries().into_iter().filter(|e| decode_token(&e.raw).is_err()).map(|e| e.host).collect()
}

pub fn lookup(host: &str) -> Option<Credential> {
    let host = host.to_ascii_lowercase();
    load().into_iter().find(|c| c.host == host)
}

/// Saves (or replaces) a host's credentials.
pub fn save(host: &str, user: &str, token: &str) -> Result<(), String> {
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() || host.contains(char::is_whitespace) {
        return Err("git: invalid host".into());
    }
    if token.is_empty() || token.contains(|c: char| c.is_control() || c.is_whitespace()) {
        return Err("git: the token cannot be empty or contain spaces".into());
    }
    if user.is_empty() || user.contains(|c: char| c.is_control() || c.is_whitespace() || c == ':') {
        return Err("git: invalid user".into());
    }
    let raw = encode_token(token)?;
    let mut entries = read_entries();
    entries.retain(|e| e.host != host);
    entries.push(Entry { host, user: user.to_string(), raw });
    write_entries(&entries)
}

/// Forgets a host (or all of them if `host` is `None`). Returns how many entries it removed.
pub fn remove(host: Option<&str>) -> Result<usize, String> {
    let entries = read_entries();
    let before = entries.len();
    let keep: Vec<Entry> = match host {
        Some(h) => {
            let h = h.trim().to_ascii_lowercase();
            entries.into_iter().filter(|e| e.host != h).collect()
        }
        None => Vec::new(),
    };
    write_entries(&keep)?;
    Ok(before - keep.len())
}

/// Encrypts the tokens left in plain text by earlier versions. Returns how many.
/// Without a cipher available it does nothing.
pub fn migrate_plaintext() -> Result<usize, String> {
    if cipher().is_none() {
        return Ok(0);
    }
    let mut entries = read_entries();
    let mut migrated = 0;
    for entry in &mut entries {
        if !entry.raw.starts_with(ENC_PREFIX) {
            entry.raw = encode_token(&entry.raw)?;
            migrated += 1;
        }
    }
    if migrated > 0 {
        write_entries(&entries)?;
    }
    Ok(migrated)
}

/// Replaces any saved token and the credentials embedded in URLs with `***`,
/// so that no error message can show them.
pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    for c in load() {
        if c.token.len() >= 4 {
            out = out.replace(&c.token, "***");
        }
    }
    // `https://usuario:clave@host` -> `https://host`
    let mut result = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        result.push_str(head);
        let end = tail.find(|c: char| c == '/' || c.is_whitespace()).unwrap_or(tail.len());
        match tail[..end].rfind('@') {
            Some(at) => {
                result.push_str("***@");
                rest = &tail[at + 1..];
            }
            None => rest = tail,
        }
    }
    result.push_str(rest);
    result
}

// ---------------------------------------------------------------------------
// HTTP transport for libgit2
// ---------------------------------------------------------------------------

static INSTALL: Once = Once::new();

/// Registers the transport for `http://` and `https://`. It is idempotent.
///
/// **Each scheme must be registered separately**: libgit2 compares the prefix with the
/// exact scheme, so registering only `http` lets `https://` be served by its
/// internal transport, which in this build has no TLS ("no TLS stream available").
pub fn install_transport() {
    INSTALL.call_once(|| {
        for scheme in ["http", "https"] {
            // SAFETY: it is called only once per scheme and the transport keeps no
            // libgit2 pointers.
            let result = unsafe {
                transport::register(scheme, |remote: &Remote<'_>| {
                    Transport::smart(remote, true, HttpSubtransport::new()?)
                })
            };
            if let Err(e) = result {
                log_error(&format!("could not register the transport {scheme}: {e}"));
            }
        }
    });
}

fn log_error(_message: &str) {
    // atlas-git does not depend on one registration; the error will show on first use.
}

struct HttpSubtransport {
    client: reqwest::blocking::Client,
}

impl HttpSubtransport {
    fn new() -> Result<Self, Error> {
        let client = reqwest::blocking::Client::builder()
            .user_agent("git/2.43.0 (Atlas Code)")
            .connect_timeout(std::time::Duration::from_secs(20))
            // A large download can take a while; it is only cut if nothing arrives for a long time.
            .timeout(None)
            .build()
            .map_err(|e| Error::from_str(&format!("cliente HTTP: {e}")))?;
        Ok(Self { client })
    }
}

impl SmartSubtransport for HttpSubtransport {
    fn action(&self, url: &str, service: Service) -> Result<Box<dyn SmartSubtransportStream>, Error> {
        let (suffix, post, content_type, accept) = match service {
            Service::UploadPackLs => ("/info/refs?service=git-upload-pack", false, None, "*/*"),
            Service::UploadPack => (
                "/git-upload-pack",
                true,
                Some("application/x-git-upload-pack-request"),
                "application/x-git-upload-pack-result",
            ),
            Service::ReceivePackLs => ("/info/refs?service=git-receive-pack", false, None, "*/*"),
            Service::ReceivePack => (
                "/git-receive-pack",
                true,
                Some("application/x-git-receive-pack-request"),
                "application/x-git-receive-pack-result",
            ),
        };
        let clean = strip_userinfo(url);
        let credential = host_of(&clean).and_then(|h| lookup(&h));
        Ok(Box::new(HttpStream {
            client: self.client.clone(),
            url: format!("{}{suffix}", clean.trim_end_matches('/')),
            post,
            content_type,
            accept,
            credential,
            body: Vec::new(),
            response: None,
        }))
    }

    fn close(&self) -> Result<(), Error> {
        Ok(())
    }
}

/// A request: what libgit2 writes is the body; on reading it is sent and the
/// response is returned as it arrives.
struct HttpStream {
    client: reqwest::blocking::Client,
    url: String,
    post: bool,
    content_type: Option<&'static str>,
    accept: &'static str,
    credential: Option<Credential>,
    body: Vec<u8>,
    response: Option<reqwest::blocking::Response>,
}

impl HttpStream {
    fn send(&mut self) -> io::Result<()> {
        let mut request = if self.post { self.client.post(&self.url) } else { self.client.get(&self.url) };
        request = request.header("Accept", self.accept).header("Pragma", "no-cache");
        if let Some(content_type) = self.content_type {
            request = request.header("Content-Type", content_type);
        }
        if self.post {
            request = request.body(std::mem::take(&mut self.body));
        }
        let has_credential = self.credential.is_some();
        if let Some(c) = &self.credential {
            request = request.basic_auth(&c.user, Some(&c.token));
        }
        let response = request.send().map_err(|e| io::Error::other(redact(&format!("{e}"))))?;
        let status = response.status();
        if !status.is_success() {
            let host = host_of(&self.url).unwrap_or_default();
            let message = match status.as_u16() {
                401 | 403 if has_credential => {
                    format!("{host} rejected the token (HTTP {status}): check that it is valid and has permission")
                }
                401 | 403 => format!("{host} requires authentication: save a token with :git token <token>"),
                404 if !has_credential => {
                    format!("repository not found on {host} (if it is private, save a token with :git token <token>)")
                }
                404 => format!("repository not found on {host}, or the token has no access"),
                _ => format!("{host} answered HTTP {status}"),
            };
            return Err(io::Error::other(message));
        }
        self.response = Some(response);
        Ok(())
    }
}

impl Read for HttpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.response.is_none() {
            self.send()?;
        }
        self.response.as_mut().expect("respuesta creada arriba").read(buf)
    }
}

impl Write for HttpStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.body.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) static SERIAL: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    /// Temporary credentials file; it serializes the tests because the store is global.
    struct TempStore {
        _guard: std::sync::MutexGuard<'static, ()>,
        dir: PathBuf,
    }

    impl TempStore {
        fn new(tag: &str) -> Self {
            let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
            let dir = std::env::temp_dir().join(format!("atlas-cred-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            set_credentials_file(dir.join("credenciales"));
            set_secret_box(None);
            require_encryption(false);
            TempStore { _guard: guard, dir }
        }
    }

    impl Drop for TempStore {
        fn drop(&mut self) {
            set_secret_box(None);
            require_encryption(false);
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn host_is_extracted_from_urls() {
        assert_eq!(host_of("https://github.com/o/r.git").as_deref(), Some("github.com"));
        assert_eq!(host_of("https://User:Pass@GitHub.com:8443/o/r").as_deref(), Some("github.com:8443"));
        assert_eq!(host_of("http://127.0.0.1:9000/x.git").as_deref(), Some("127.0.0.1:9000"));
        assert_eq!(host_of("no-scheme"), None);
    }

    #[test]
    fn userinfo_is_stripped_from_urls() {
        assert_eq!(strip_userinfo("https://u:t@h.com/o/r.git"), "https://h.com/o/r.git");
        assert_eq!(strip_userinfo("https://h.com/o/r.git"), "https://h.com/o/r.git");
        assert_eq!(strip_userinfo("https://h.com/o@r"), "https://h.com/o@r", "an @ in the path is not userinfo");
    }

    #[test]
    fn credentials_roundtrip_replace_and_remove() {
        let _s = TempStore::new("rt");
        assert!(load().is_empty());
        save("GitHub.com", "git", "ghp_aaaa").unwrap();
        save("gitlab.com", "yo", "glpat-bbbb").unwrap();
        assert_eq!(lookup("github.com").unwrap().token, "ghp_aaaa", "the host is case insensitive");
        assert_eq!(lookup("gitlab.com").unwrap().user, "yo");

        save("github.com", "git", "ghp_nuevo").unwrap();
        assert_eq!(load().len(), 2, "saving again replaces, it does not duplicate");
        assert_eq!(lookup("github.com").unwrap().token, "ghp_nuevo");

        assert_eq!(remove(Some("github.com")).unwrap(), 1);
        assert!(lookup("github.com").is_none());
        assert!(lookup("gitlab.com").is_some());
        assert_eq!(remove(None).unwrap(), 1);
        assert!(load().is_empty());
    }

    #[test]
    fn invalid_credentials_are_rejected() {
        let _s = TempStore::new("bad");
        assert!(save("", "git", "tok").is_err());
        assert!(save("h.com", "git", "").is_err());
        assert!(save("h.com", "git", "with space").is_err());
        assert!(save("h.com", "git", "with\ttab").is_err());
        assert!(save("h.com", "", "tok").is_err());
        assert!(save("h.com", "a:b", "tok").is_err());
        assert!(load().is_empty(), "nothing must remain saved after an error");
    }

    #[test]
    fn redact_hides_stored_tokens_and_url_credentials() {
        let _s = TempStore::new("redact");
        save("h.com", "git", "ghp_secretisimo").unwrap();
        let msg = redact("failed to open https://git:ghp_secretisimo@h.com/o/r and another https://u:p@x.org/z");
        assert!(!msg.contains("secretisimo"), "{msg}");
        assert!(!msg.contains("u:p@"), "{msg}");
        assert!(msg.contains("https://***@h.com/o/r"), "{msg}");
        assert_eq!(redact("nothing to hide"), "nothing to hide");
    }

    #[test]
    fn the_credentials_file_is_written_atomically_without_leftovers() {
        let s = TempStore::new("atomic");
        save("h.com", "git", "tok-largo").unwrap();
        let names: Vec<String> = std::fs::read_dir(&s.dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["credenciales"], "the temporary file must not remain: {names:?}");
    }

    #[cfg(unix)]
    #[test]
    fn the_credentials_file_is_only_readable_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let s = TempStore::new("perm");
        save("h.com", "git", "tok-largo").unwrap();
        let mode = std::fs::metadata(s.dir.join("credenciales")).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "other users must not be able to read it (mode {mode:o})");
    }

    // ----- cifrado -----

    /// Test cipher: reverses and shifts the bytes, and adds a mark to detect
    /// tampered data. It is not secure; it only serves to check the flow.
    struct FakeCipher {
        fail_open: AtomicBool,
        fail_seal: AtomicBool,
    }

    impl FakeCipher {
        fn new() -> Arc<Self> {
            Arc::new(Self { fail_open: AtomicBool::new(false), fail_seal: AtomicBool::new(false) })
        }
    }

    impl SecretBox for FakeCipher {
        fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, String> {
            if self.fail_seal.load(Ordering::SeqCst) {
                return Err("Keystore not available".into());
            }
            let mut out: Vec<u8> = plain.iter().rev().map(|b| b.wrapping_add(7)).collect();
            out.push(0xA5);
            Ok(out)
        }

        fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, String> {
            if self.fail_open.load(Ordering::SeqCst) {
                return Err("key not available".into());
            }
            match sealed.split_last() {
                Some((0xA5, body)) => Ok(body.iter().rev().map(|b| b.wrapping_sub(7)).collect()),
                _ => Err("datos alterados".into()),
            }
        }
    }

    fn file(s: &TempStore) -> String {
        std::fs::read_to_string(s.dir.join("credenciales")).unwrap()
    }

    #[test]
    fn encrypted_tokens_are_never_written_in_clear_and_roundtrip() {
        let s = TempStore::new("enc");
        set_secret_box(Some(FakeCipher::new()));
        save("h.com", "git", "ghp_token_muy_secreto").unwrap();
        let on_disk = file(&s);
        assert!(on_disk.contains("\tenc1:"), "{on_disk}");
        assert!(!on_disk.contains("muy_secreto"), "the token is in plain text on disk: {on_disk}");
        assert_eq!(lookup("h.com").unwrap().token, "ghp_token_muy_secreto");
        assert!(unreadable_hosts().is_empty());
    }

    #[test]
    fn legacy_plaintext_tokens_are_still_readable_and_get_migrated() {
        let s = TempStore::new("migrate");
        save("viejo.com", "git", "tok-en-claro-123").unwrap(); // without a cipher: it stays in plain text
        assert!(file(&s).contains("tok-en-claro-123"));

        set_secret_box(Some(FakeCipher::new()));
        assert_eq!(lookup("viejo.com").unwrap().token, "tok-en-claro-123", "it must be read even if it is not encrypted");
        assert_eq!(migrate_plaintext().unwrap(), 1);
        assert!(!file(&s).contains("tok-en-claro-123"), "after migrating it must not stay in plain text");
        assert_eq!(lookup("viejo.com").unwrap().token, "tok-en-claro-123");
        assert_eq!(migrate_plaintext().unwrap(), 0, "migrating twice must do nothing");
    }

    #[test]
    fn migration_does_nothing_without_a_cipher() {
        let s = TempStore::new("nomigrate");
        save("h.com", "git", "tok-en-claro-123").unwrap();
        assert_eq!(migrate_plaintext().unwrap(), 0);
        assert!(file(&s).contains("tok-en-claro-123"));
    }

    #[test]
    fn saving_is_refused_when_encryption_is_required_but_unavailable() {
        let s = TempStore::new("require");
        require_encryption(true);
        let err = save("h.com", "git", "tok-secreto-123").unwrap_err();
        assert!(err.contains("not stored in plain text"), "{err}");
        assert!(!s.dir.join("credenciales").exists(), "nothing must have been written");

        // With a cipher that fails to encrypt, the same: nothing in plain text.
        let broken = FakeCipher::new();
        broken.fail_seal.store(true, Ordering::SeqCst);
        set_secret_box(Some(broken));
        let err = save("h.com", "git", "tok-secreto-123").unwrap_err();
        assert!(err.contains("could not encrypt"), "{err}");
        assert!(!s.dir.join("credenciales").exists());
    }

    #[test]
    fn unreadable_entries_are_reported_and_survive_rewrites() {
        let s = TempStore::new("unreadable");
        let cipher = FakeCipher::new();
        set_secret_box(Some(cipher.clone()));
        save("uno.com", "git", "tok-uno-1234").unwrap();
        let before = file(&s);

        // Transient cipher failure: the entry cannot be read...
        cipher.fail_open.store(true, Ordering::SeqCst);
        assert!(lookup("uno.com").is_none());
        assert_eq!(unreadable_hosts(), vec!["uno.com"]);

        // ...and saving another host must NOT delete the unreadable entry.
        save("dos.com", "git", "tok-dos-5678").unwrap();
        assert!(file(&s).contains(before.trim()), "an unreadable entry was lost:\n{}", file(&s));

        // When the cipher comes back, the old token is still there.
        cipher.fail_open.store(false, Ordering::SeqCst);
        assert_eq!(lookup("uno.com").unwrap().token, "tok-uno-1234");
        assert_eq!(lookup("dos.com").unwrap().token, "tok-dos-5678");
    }

    #[test]
    fn removing_one_host_keeps_unreadable_entries_of_others() {
        let s = TempStore::new("rmkeep");
        let cipher = FakeCipher::new();
        set_secret_box(Some(cipher.clone()));
        save("a.com", "git", "tok-aaaa-1111").unwrap();
        save("b.com", "git", "tok-bbbb-2222").unwrap();
        cipher.fail_open.store(true, Ordering::SeqCst);
        assert_eq!(remove(Some("a.com")).unwrap(), 1);
        cipher.fail_open.store(false, Ordering::SeqCst);
        assert_eq!(lookup("b.com").unwrap().token, "tok-bbbb-2222", "b.com must not be affected");
        assert!(!file(&s).contains("a.com"));
    }

    #[test]
    fn tampered_or_corrupt_ciphertext_is_unreadable_not_a_crash() {
        let s = TempStore::new("tamper");
        set_secret_box(Some(FakeCipher::new()));
        save("h.com", "git", "tok-original-99").unwrap();
        let path = s.dir.join("credenciales");
        let text = std::fs::read_to_string(&path).unwrap();
        // The base64 is corrupted.
        std::fs::write(&path, text.replace("enc1:", "enc1:!!!")).unwrap();
        assert!(lookup("h.com").is_none());
        assert_eq!(unreadable_hosts(), vec!["h.com"]);
        // A file with garbage must not cause a panic either.
        std::fs::write(&path, "line without tabs\n\t\n").unwrap();
        assert!(load().is_empty());
    }

    #[test]
    fn redaction_also_covers_encrypted_tokens() {
        let _s = TempStore::new("redact-enc");
        set_secret_box(Some(FakeCipher::new()));
        save("h.com", "git", "ghp_cifrado_secreto").unwrap();
        assert!(!redact("failure with ghp_cifrado_secreto in the message").contains("cifrado_secreto"));
    }

    // ----- transporte -----

    /// Tries to connect to `url` (a closed port) and returns the error message.
    /// What matters is WHO fails: libgit2's transport or ours.
    fn connect_error(url: &str) -> String {
        install_transport();
        let mut remote = git2::Remote::create_detached(url).unwrap();
        match remote.connect(git2::Direction::Fetch) {
            Ok(()) => "(connected)".to_string(),
            Err(e) => e.message().to_string(),
        }
    }

    #[test]
    fn both_http_and_https_urls_go_through_our_transport() {
        let _s = TempStore::new("transport");
        // libgit2, compiled without TLS, fails with this message when it uses ITS HTTPS transport.
        for url in ["http://127.0.0.1:9/x.git", "https://127.0.0.1:9/x.git", "HTTPS://127.0.0.1:9/x.git"] {
            let error = connect_error(url);
            assert!(
                !error.contains("no TLS stream"),
                "{url} was served by libgit2's internal transport (without TLS): {error}"
            );
        }
    }
}

