//! Link between Atlas (in the headset) and Atlas Bridge (on the laptop) over the local network.
//!
//! - **Pairing:** the headset shows a 6-digit code; the laptop types it. With SPAKE2 both agree on
//!   a key from the code **without the code traveling over the network**: whoever listens to the connection cannot
//!   try codes without connecting, and each failed attempt uses up one of the few the headset allows.
//! - **Channel:** each message is encrypted and authenticated (ChaCha20-Poly1305), with one key per direction and a
//!   counter as the nonce. An altered, repeated or out-of-order message cuts the connection.
//! - **Files:** the laptop sends the name, the size, the content in chunks and its SHA-256. The headset saves it in its
//!   inbox (`inbox/`) with a sanitized name, without overwriting anything.
//! - **The other way:** the laptop asks every so often (`Poll`) whether the headset has something for it; the headset
//!   answers with a file from its outbox or with `Idle`. That way the headset never talks unless asked and the protocol
//!   stays request-response.
//! - **Control:** if the laptop allows it (it says so in each `Poll`), the headset sends it mouse and keyboard orders
//!   (`RemoteInput`) in the replies: with each image confirmation and in each `Idle`. With them also goes the
//!   width that suits the image given how big the screen looks in the headset.
//! - **Screen:** the laptop sends its screen as JPEG images, one at a time: it waits for the headset to confirm each
//!   one before sending the next, so delay never piles up and the pace adapts to the network by itself.
//!
//! The headset listens (`serve_one`) and the laptop connects (`Client`). It does not depend on XR: tested on the PC.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spake2::{Ed25519Group, Identity, Password, Spake2};

/// Port the headset listens on.
pub const PORT: u16 = 47800;
/// First bytes of a connection: they tell Atlas apart from anything else that calls the port.
const MAGIC: &[u8; 8] = b"ATLAS-L1";
/// Largest encrypted chunk accepted (it protects memory from a message with a fake size).
const MAX_FRAME: usize = 1 << 20;
/// Size of the chunks a file is sent in.
const CHUNK: usize = 256 * 1024;
/// Screen images that can be on the way unconfirmed: with two, while the headset receives one, the next one is already
/// traveling (with only one, the network sits idle waiting for each confirmation).
const SCREEN_IN_FLIGHT: u32 = 2;
/// Largest file accepted by default.
pub const DEFAULT_MAX_FILE: u64 = 64 * 1024 * 1024;
/// Largest screen image accepted (a 1600 x 1000 JPEG usually takes a few hundred KB).
pub const MAX_SCREEN: u64 = 8 * 1024 * 1024;
/// Failed pairing attempts allowed for one code before it stops listening.
pub const MAX_ATTEMPTS: u32 = 3;

const ID_BRIDGE: &[u8] = b"atlas-bridge";
const ID_HEADSET: &[u8] = b"atlas-headset";

#[derive(Debug, Clone, PartialEq)]
pub enum LinkError {
    /// Something in the network (connection refused, cut, timed out...).
    Io(String),
    /// The other side does not speak this protocol.
    Protocol(String),
    /// The code does not match (or someone tampered with the pairing).
    WrongCode,
    /// The attempts with this code ran out: a new one must be requested.
    TooManyAttempts,
    /// The file is not accepted (too large, impossible name, damaged content...).
    Rejected(String),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkError::Io(e) => write!(f, "network: {e}"),
            LinkError::Protocol(e) => write!(f, "protocol: {e}"),
            LinkError::WrongCode => write!(f, "the pairing code does not match"),
            LinkError::TooManyAttempts => write!(f, "too many wrong codes: ask the headset for a new one (:bridge)"),
            LinkError::Rejected(e) => write!(f, "rejected: {e}"),
        }
    }
}

impl std::error::Error for LinkError {}

impl From<std::io::Error> for LinkError {
    fn from(e: std::io::Error) -> Self {
        LinkError::Io(e.to_string())
    }
}

/// A new pairing code: 6 random digits (unbiased).
pub fn pairing_code() -> String {
    loop {
        let n = OsRng.next_u32();
        // The values at the end are discarded so that every code is equally likely.
        if n < u32::MAX - (u32::MAX % 1_000_000) {
            return format!("{:06}", n % 1_000_000);
        }
    }
}

/// Control messages (as JSON, inside the encrypted channel). The content of files goes in separate chunks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Message {
    /// Key confirmation: if the other side decrypts it, both have the same code.
    Confirm { who: String },
    FileStart { name: String, size: u64 },
    FileEnd { sha256: String },
    /// A screen image (JPEG) of `size` bytes, which arrives next in chunks.
    Screen { size: u64 },
    /// The headset received the image: the next one can be sent. It carries the width that suits it now (0: whatever
    /// the laptop wants) and the pending mouse and keyboard orders.
    ScreenAck {
        #[serde(default)]
        want_width: u32,
        #[serde(default)]
        input: Vec<RemoteInput>,
    },
    Saved { name: String },
    Error { reason: String },
    /// The laptop asks whether the headset has something for it, and says whether it lets the headset control it.
    Poll {
        #[serde(default)]
        control: bool,
    },
    /// The headset has no files to send (it may have mouse and keyboard orders).
    Idle {
        #[serde(default)]
        input: Vec<RemoteInput>,
    },
}

/// An order from the headset for the laptop's mouse or keyboard. The laptop only obeys it if it allows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum RemoteInput {
    /// Move the pointer to (`u`, `v`): fraction of the width and height of the shared screen (0..1).
    Pointer { u: f32, v: f32 },
    /// Press (`down`) or release a mouse button: the left one, or the right one if `right`.
    Button { down: bool, right: bool },
    /// Turn the wheel: positive is down, in steps.
    Scroll { lines: i32 },
    /// Escribir texto.
    Text { text: String },
    /// A key: `enter`, `backspace`, `delete`, `escape`, `tab`, `left`, `right`, `up`, `down`, `home`, `end`, or a
    /// character; with Ctrl (Cmd on a Mac) if `ctrl`.
    Key { key: String, ctrl: bool },
}

/// Mouse and keyboard orders that fit in one reply (the rest wait for the next one).
const MAX_INPUT_PER_REPLY: usize = 256;

/// The headset's side, which fills the outbox.
#[derive(Clone)]
pub struct ToLaptop {
    pub files: mpsc::Sender<Outgoing>,
    pub input: mpsc::Sender<RemoteInput>,
    /// Width (px) that suits the screen image now (0: whatever the laptop wants).
    pub want_width: Arc<AtomicU32>,
}

/// What the headset has for the laptop: files, mouse and keyboard orders, and the width that suits the image.
pub struct Outbox {
    files: mpsc::Receiver<Outgoing>,
    input: mpsc::Receiver<RemoteInput>,
    want_width: Arc<AtomicU32>,
}

impl Outbox {
    fn drain_input(&self) -> Vec<RemoteInput> {
        self.input.try_iter().take(MAX_INPUT_PER_REPLY).collect()
    }
}

/// A new outbox: the headset writes to `ToLaptop` and `serve_one` reads from `Outbox`.
pub fn outbox() -> (ToLaptop, Outbox) {
    let (files_tx, files) = mpsc::channel();
    let (input_tx, input) = mpsc::channel();
    let want_width = Arc::new(AtomicU32::new(0));
    (ToLaptop { files: files_tx, input: input_tx, want_width: want_width.clone() }, Outbox { files, input, want_width })
}

/// An encrypted channel over TCP: one key and one counter per direction.
struct Channel {
    stream: TcpStream,
    send: ChaCha20Poly1305,
    recv: ChaCha20Poly1305,
    sent: u64,
    received: u64,
}

fn nonce(counter: u64) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[4..].copy_from_slice(&counter.to_be_bytes());
    n
}

impl Channel {
    fn new(stream: TcpStream, key: &[u8], is_bridge: bool) -> Result<Self, LinkError> {
        // A different key per direction: that way the counter can start at 0 on both without repeating a nonce.
        // Small messages (confirmations, questions) without delays: they go as soon as they are written.
        stream.set_nodelay(true)?;
        let hk = Hkdf::<Sha256>::new(None, key);
        let (mut to_headset, mut to_bridge) = ([0u8; 32], [0u8; 32]);
        hk.expand(b"atlas-link bridge->headset", &mut to_headset).map_err(|_| LinkError::Protocol("key".into()))?;
        hk.expand(b"atlas-link headset->bridge", &mut to_bridge).map_err(|_| LinkError::Protocol("key".into()))?;
        let (send, recv) = if is_bridge { (to_headset, to_bridge) } else { (to_bridge, to_headset) };
        Ok(Self {
            stream,
            send: ChaCha20Poly1305::new(Key::from_slice(&send)),
            recv: ChaCha20Poly1305::new(Key::from_slice(&recv)),
            sent: 0,
            received: 0,
        })
    }

    fn send_bytes(&mut self, plain: &[u8]) -> Result<(), LinkError> {
        let sealed = self.send.encrypt(Nonce::from_slice(&nonce(self.sent)), plain).map_err(|_| LinkError::Protocol("encrypt".into()))?;
        self.sent += 1;
        // Length and content in a single write: in two, the second chunk can wait for the
        // acknowledgment of the first (Nagle's algorithm) and each message loses tens of milliseconds.
        let mut frame = Vec::with_capacity(4 + sealed.len());
        frame.extend_from_slice(&(sealed.len() as u32).to_be_bytes());
        frame.extend_from_slice(&sealed);
        self.stream.write_all(&frame)?;
        Ok(())
    }

    fn recv_bytes(&mut self) -> Result<Vec<u8>, LinkError> {
        let mut len = [0u8; 4];
        self.stream.read_exact(&mut len)?;
        let len = u32::from_be_bytes(len) as usize;
        if len > MAX_FRAME + 16 {
            return Err(LinkError::Protocol(format!("frame too large ({len} bytes)")));
        }
        let mut sealed = vec![0u8; len];
        self.stream.read_exact(&mut sealed)?;
        let plain = self
            .recv
            .decrypt(Nonce::from_slice(&nonce(self.received)), sealed.as_slice())
            .map_err(|_| LinkError::Protocol("a message was altered or out of order".into()))?;
        self.received += 1;
        Ok(plain)
    }

    fn send_msg(&mut self, msg: &Message) -> Result<(), LinkError> {
        self.send_bytes(&serde_json::to_vec(msg).expect("messages can always be serialized"))
    }

    fn recv_msg(&mut self) -> Result<Message, LinkError> {
        let bytes = self.recv_bytes()?;
        serde_json::from_slice(&bytes).map_err(|e| LinkError::Protocol(format!("bad message: {e}")))
    }
}

/// SPAKE2 exchange over the `stream` (the 33 bytes from each side) and key confirmation.
fn handshake(mut stream: TcpStream, code: &str, is_bridge: bool) -> Result<Channel, LinkError> {
    let password = Password::new(code.as_bytes());
    let (state, outbound) = if is_bridge {
        Spake2::<Ed25519Group>::start_a(&password, &Identity::new(ID_BRIDGE), &Identity::new(ID_HEADSET))
    } else {
        Spake2::<Ed25519Group>::start_b(&password, &Identity::new(ID_BRIDGE), &Identity::new(ID_HEADSET))
    };
    if is_bridge {
        stream.write_all(MAGIC)?;
    } else {
        let mut magic = [0u8; 8];
        stream.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(LinkError::Protocol("not an Atlas Bridge".into()));
        }
    }
    stream.write_all(&(outbound.len() as u16).to_be_bytes())?;
    stream.write_all(&outbound)?;
    let mut len = [0u8; 2];
    stream.read_exact(&mut len)?;
    let len = u16::from_be_bytes(len) as usize;
    if len > 128 {
        return Err(LinkError::Protocol("bad key exchange".into()));
    }
    let mut inbound = vec![0u8; len];
    stream.read_exact(&mut inbound)?;
    let key = state.finish(&inbound).map_err(|_| LinkError::WrongCode)?;

    let mut channel = Channel::new(stream, &key, is_bridge)?;
    // Each side proves it has the same key. With another code, decrypting fails: wrong code.
    let me = if is_bridge { "bridge" } else { "headset" };
    channel.send_msg(&Message::Confirm { who: me.into() })?;
    match channel.recv_msg() {
        Ok(Message::Confirm { who }) if who != me => Ok(channel),
        Ok(_) => Err(LinkError::Protocol("unexpected confirmation".into())),
        Err(LinkError::Protocol(_)) => Err(LinkError::WrongCode),
        Err(e) => Err(e),
    }
}

/// A file name that can be saved safely: only the last component, without `..`, control
/// characters or separators, and of reasonable length. `None` if nothing usable is left.
pub fn sanitize_name(name: &str) -> Option<String> {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = last.chars().filter(|c| !c.is_control() && !matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|')).collect();
    let clean = clean.trim().trim_end_matches('.').to_string();
    if clean.is_empty() || clean == "." || clean == ".." {
        return None;
    }
    Some(clean.chars().take(120).collect())
}

/// Where to save `name` in `dir` without overwriting: `a.rs`, `a (2).rs`, `a (3).rs`...
fn free_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .expect("siempre hay un nombre libre")
}

/// What happens in the headset while it listens.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerEvent {
    /// Someone tried to pair with a wrong code (`left` attempts remain).
    WrongCode { left: u32 },
    /// The laptop paired.
    Paired,
    /// A file was saved in the inbox, with this name.
    Saved(String),
    /// A file was not accepted.
    Rejected(String),
    /// An image of the laptop screen (JPEG).
    Screen(Vec<u8>),
    /// The laptop saved a file the headset sent it, with this name.
    Delivered(String),
    /// The laptop did not accept a file from the headset.
    NotDelivered(String),
    /// The laptop lets (or no longer lets) the headset control its mouse and keyboard.
    Control(bool),
}

/// A file the headset wants to send to the laptop: it is delivered the next time the laptop asks.
#[derive(Debug, Clone, PartialEq)]
pub struct Outgoing {
    pub name: String,
    pub data: Vec<u8>,
}

/// The headset's limits when receiving.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_file: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_file: DEFAULT_MAX_FILE }
    }
}

/// Listens on `listener` until a laptop pairs with `code` (or the attempts run out), and receives its
/// files in `inbox` until it closes the connection. What arrives through `outbox` is sent to the laptop when it
/// asks. Reports what happens with `on_event`.
pub fn serve_one(
    listener: &TcpListener,
    code: &str,
    inbox: &Path,
    limits: Limits,
    outbox: &Outbox,
    mut on_event: impl FnMut(ServerEvent),
) -> Result<(), LinkError> {
    let mut left = MAX_ATTEMPTS;
    let mut channel = loop {
        let (stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        match handshake(stream, code, false) {
            Ok(channel) => break channel,
            Err(LinkError::WrongCode) => {
                left -= 1;
                on_event(ServerEvent::WrongCode { left });
                if left == 0 {
                    return Err(LinkError::TooManyAttempts);
                }
            }
            // Something that is not Atlas Bridge called the port: it is ignored and listening continues.
            Err(LinkError::Protocol(_)) | Err(LinkError::Io(_)) => {}
            Err(e) => return Err(e),
        }
    };
    on_event(ServerEvent::Paired);
    // Paired, it waits with no limit for the laptop to send something.
    channel.stream.set_read_timeout(None)?;
    std::fs::create_dir_all(inbox)?;
    let mut control_allowed: Option<bool> = None;
    loop {
        let msg = match channel.recv_msg() {
            Ok(m) => m,
            Err(LinkError::Io(_)) => return Ok(()), // the laptop closed
            Err(e) => return Err(e),
        };
        let (name, size) = match msg {
            Message::FileStart { name, size } => (name, size),
            Message::Screen { size } => {
                if size > MAX_SCREEN {
                    return Err(LinkError::Protocol(format!("screen image too large ({size} bytes)")));
                }
                let mut image = Vec::with_capacity(size as usize);
                while (image.len() as u64) < size {
                    image.extend_from_slice(&channel.recv_bytes()?);
                }
                if image.len() as u64 != size {
                    return Err(LinkError::Protocol("more screen data than announced".into()));
                }
                // The laptop can close without reading the last confirmation (it sends several images in a row): it is a normal
                // close, not an error.
                let ack = Message::ScreenAck { want_width: outbox.want_width.load(Ordering::Relaxed), input: outbox.drain_input() };
                let closed = matches!(channel.send_msg(&ack), Err(LinkError::Io(_)));
                on_event(ServerEvent::Screen(image));
                if closed {
                    return Ok(());
                }
                continue;
            }
            Message::Poll { control } => {
                if control_allowed != Some(control) {
                    control_allowed = Some(control);
                    on_event(ServerEvent::Control(control));
                }
                match outbox.files.try_recv() {
                    Ok(out) => {
                        send_file_data(&mut channel, &out.name, &out.data)?;
                        match channel.recv_msg()? {
                            Message::Saved { name } => on_event(ServerEvent::Delivered(name)),
                            Message::Error { reason } => on_event(ServerEvent::NotDelivered(reason)),
                            other => return Err(LinkError::Protocol(format!("unexpected answer {other:?}"))),
                        }
                    }
                    Err(_) => channel.send_msg(&Message::Idle { input: outbox.drain_input() })?,
                }
                continue;
            }
            _ => return Err(LinkError::Protocol("expected a file, a screen image or a poll".into())),
        };
        let result = receive_file(&mut channel, &name, size, inbox, limits);
        match result {
            Ok(saved) => {
                channel.send_msg(&Message::Saved { name: saved.clone() })?;
                on_event(ServerEvent::Saved(saved));
            }
            Err(LinkError::Rejected(reason)) => {
                channel.send_msg(&Message::Error { reason: reason.clone() })?;
                on_event(ServerEvent::Rejected(reason));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Receives the content of an already announced file. If it is not accepted, it is consumed anyway so as not to lose sync.
fn receive_file(channel: &mut Channel, name: &str, size: u64, inbox: &Path, limits: Limits) -> Result<String, LinkError> {
    let safe = sanitize_name(name);
    let too_big = size > limits.max_file;
    let mut data: Vec<u8> = Vec::with_capacity(if too_big { 0 } else { size as usize });
    let mut got = 0u64;
    let mut hasher = Sha256::new();
    while got < size {
        let chunk = channel.recv_bytes()?;
        got += chunk.len() as u64;
        if got > size {
            return Err(LinkError::Protocol("more data than announced".into()));
        }
        hasher.update(&chunk);
        if !too_big {
            data.extend_from_slice(&chunk);
        }
    }
    let Message::FileEnd { sha256 } = channel.recv_msg()? else {
        return Err(LinkError::Protocol("expected the end of the file".into()));
    };
    if too_big {
        return Err(LinkError::Rejected(format!("{name} is larger than {} MB", limits.max_file / (1024 * 1024))));
    }
    let Some(safe) = safe else {
        return Err(LinkError::Rejected(format!("{name:?} is not a usable file name")));
    };
    if hex(&hasher.finalize()) != sha256 {
        return Err(LinkError::Rejected(format!("{safe} arrived damaged")));
    }
    let path = free_path(inbox, &safe);
    std::fs::write(&path, &data)?;
    Ok(path.file_name().and_then(|n| n.to_str()).unwrap_or(&safe).to_string())
}

/// Sends a file through the channel: name and size, the content in chunks and its SHA-256.
fn send_file_data(channel: &mut Channel, name: &str, data: &[u8]) -> Result<(), LinkError> {
    channel.send_msg(&Message::FileStart { name: name.to_string(), size: data.len() as u64 })?;
    for chunk in data.chunks(CHUNK) {
        channel.send_bytes(chunk)?;
    }
    channel.send_msg(&Message::FileEnd { sha256: hex(&Sha256::digest(data)) })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The laptop, connected and paired with the headset.
pub struct Client {
    channel: Channel,
    /// Screen images sent whose confirmation has not been read yet.
    in_flight: u32,
    /// Mouse and keyboard orders that came from the headset and have not been collected yet.
    input: Vec<RemoteInput>,
    /// Width that suits the image now, according to the headset (0: no preference).
    want_width: u32,
}

impl Client {
    /// Connects to the headset (`192.168.1.20` or `192.168.1.20:47800`) and pairs with `code`.
    pub fn connect(addr: &str, code: &str) -> Result<Self, LinkError> {
        let target = if addr.contains(':') { addr.to_string() } else { format!("{addr}:{PORT}") };
        let socket = target
            .to_socket_addrs()
            .map_err(|e| LinkError::Io(format!("{target}: {e}")))?
            .next()
            .ok_or_else(|| LinkError::Io(format!("{target}: no address")))?;
        let stream = TcpStream::connect_timeout(&socket, Duration::from_secs(5)).map_err(|e| match e.kind() {
            std::io::ErrorKind::ConnectionRefused => {
                LinkError::Io("the headset is not listening: type :bridge on the headset and use the new code".into())
            }
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => LinkError::Io(format!(
                "the headset did not answer at {target}: check the address and that both are on the same network"
            )),
            _ => LinkError::Io(format!("{target}: {e}")),
        })?;
        stream.set_read_timeout(Some(Duration::from_secs(60)))?;
        let code: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
        Ok(Self { channel: handshake(stream, &code, true)?, in_flight: 0, input: Vec::new(), want_width: 0 })
    }

    /// Reads the pending screen image confirmations until at most `keep` remain (before another
    /// request they must all be read: the expected reply comes after them).
    fn settle_screens(&mut self, keep: u32) -> Result<(), LinkError> {
        while self.in_flight > keep {
            match self.channel.recv_msg()? {
                Message::ScreenAck { want_width, input } => {
                    self.in_flight -= 1;
                    self.want_width = want_width;
                    self.input.extend(input);
                }
                other => return Err(LinkError::Protocol(format!("unexpected answer {other:?}"))),
            }
        }
        Ok(())
    }

    /// Sends `data` with the name `name`. Returns the name it was saved with in the headset.
    pub fn send(&mut self, name: &str, data: &[u8]) -> Result<String, LinkError> {
        self.settle_screens(0)?;
        send_file_data(&mut self.channel, name, data)?;
        match self.channel.recv_msg()? {
            Message::Saved { name } => Ok(name),
            Message::Error { reason } => Err(LinkError::Rejected(reason)),
            other => Err(LinkError::Protocol(format!("unexpected answer {other:?}"))),
        }
    }

    /// Sends a screen image (JPEG). There can be `SCREEN_IN_FLIGHT` on the way: if there are already, it waits for
    /// the headset to confirm the oldest (so the delay never grows).
    pub fn send_screen(&mut self, jpeg: &[u8]) -> Result<(), LinkError> {
        self.settle_screens(SCREEN_IN_FLIGHT - 1)?;
        self.channel.send_msg(&Message::Screen { size: jpeg.len() as u64 })?;
        for chunk in jpeg.chunks(CHUNK) {
            self.channel.send_bytes(chunk)?;
        }
        self.in_flight += 1;
        Ok(())
    }

    /// Asks the headset whether it has a file for the laptop (and tells it whether it lets it control the laptop). If it does, it
    /// saves it in `dir` (with the name sanitized and without overwriting) and returns the name it ended up with; `Ok(None)` if
    /// there was nothing.
    pub fn poll(&mut self, dir: &Path, control: bool) -> Result<Option<String>, LinkError> {
        self.settle_screens(0)?;
        self.channel.send_msg(&Message::Poll { control })?;
        match self.channel.recv_msg()? {
            Message::Idle { input } => {
                self.input.extend(input);
                Ok(None)
            }
            Message::FileStart { name, size } => {
                std::fs::create_dir_all(dir)?;
                match receive_file(&mut self.channel, &name, size, dir, Limits::default()) {
                    Ok(saved) => {
                        self.channel.send_msg(&Message::Saved { name: saved.clone() })?;
                        Ok(Some(saved))
                    }
                    Err(LinkError::Rejected(reason)) => {
                        self.channel.send_msg(&Message::Error { reason: reason.clone() })?;
                        Err(LinkError::Rejected(reason))
                    }
                    Err(e) => Err(e),
                }
            }
            other => Err(LinkError::Protocol(format!("unexpected answer {other:?}"))),
        }
    }

    /// The mouse and keyboard orders the headset sent since the last time.
    pub fn take_input(&mut self) -> Vec<RemoteInput> {
        std::mem::take(&mut self.input)
    }

    /// Width that suits the screen image now, according to the headset (0: no preference).
    pub fn want_width(&self) -> u32 {
        self.want_width
    }

    /// Sends a file from disk.
    pub fn send_file(&mut self, path: &Path) -> Result<String, LinkError> {
        let data = std::fs::read(path)?;
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
        self.send(name, &data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("atlas-link-{tag}-{}-{}", std::process::id(), OsRng.next_u32()));
            std::fs::create_dir_all(&dir).unwrap();
            Tmp(dir)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A headset listening on a free localhost port, on another thread. Returns its address, its events and its result.
    fn headset(code: &str, inbox: &Path, limits: Limits) -> (String, mpsc::Receiver<ServerEvent>, std::thread::JoinHandle<Result<(), LinkError>>) {
        let (addr, events, _outbox, handle) = headset_with_outbox(code, inbox, limits);
        (addr, events, handle)
    }

    /// Like `headset`, plus the headset's outbox.
    #[allow(clippy::type_complexity)]
    fn headset_with_outbox(
        code: &str,
        inbox: &Path,
        limits: Limits,
    ) -> (String, mpsc::Receiver<ServerEvent>, ToLaptop, std::thread::JoinHandle<Result<(), LinkError>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (tx, rx) = mpsc::channel();
        let (to_laptop, out) = outbox();
        let (code, inbox) = (code.to_string(), inbox.to_path_buf());
        let handle = std::thread::spawn(move || serve_one(&listener, &code, &inbox, limits, &out, |e| tx.send(e).unwrap()));
        (addr, rx, to_laptop, handle)
    }

    #[test]
    fn mouse_and_keyboard_orders_ride_on_the_replies_and_the_laptop_says_if_it_allows_them() {
        let inbox = Tmp::new("remote-in");
        let downloads = Tmp::new("remote-out");
        let (addr, events, to_laptop, handle) = headset_with_outbox("888888", &inbox.0, Limits::default());
        let mut client = Client::connect(&addr, "888888").unwrap();
        assert_eq!(client.poll(&downloads.0, true).unwrap(), None);
        to_laptop.input.send(RemoteInput::Pointer { u: 0.5, v: 0.25 }).unwrap();
        to_laptop.input.send(RemoteInput::Button { down: true, right: false }).unwrap();
        to_laptop.want_width.store(2240, Ordering::Relaxed);
        // In the confirmation of an image...
        client.send_screen(b"img-1").unwrap();
        client.send_screen(b"img-2").unwrap();
        client.send_screen(b"img-3").unwrap();
        let got = client.take_input();
        assert_eq!(got, vec![RemoteInput::Pointer { u: 0.5, v: 0.25 }, RemoteInput::Button { down: true, right: false }]);
        assert_eq!(client.want_width(), 2240, "and the width that suits it");
        // ...and in the reply to a question.
        to_laptop.input.send(RemoteInput::Text { text: "hola".into() }).unwrap();
        to_laptop.input.send(RemoteInput::Key { key: "s".into(), ctrl: true }).unwrap();
        assert_eq!(client.poll(&downloads.0, false).unwrap(), None);
        assert_eq!(client.take_input(), vec![RemoteInput::Text { text: "hola".into() }, RemoteInput::Key { key: "s".into(), ctrl: true }]);
        assert!(client.take_input().is_empty(), "each order is delivered once");
        drop(client);
        handle.join().unwrap().unwrap();
        let control: Vec<bool> = events.try_iter().filter_map(|e| if let ServerEvent::Control(c) = e { Some(c) } else { None }).collect();
        assert_eq!(control, vec![true, false], "the headset knows when the laptop stops allowing it");
    }

    #[test]
    fn the_headset_sends_files_back_when_the_laptop_asks() {
        let inbox = Tmp::new("back-in");
        let downloads = Tmp::new("back-out");
        let (addr, events, outbox, handle) = headset_with_outbox("777777", &inbox.0, Limits::default());
        let mut client = Client::connect(&addr, "777777").unwrap();
        assert_eq!(client.poll(&downloads.0, false).unwrap(), None, "with nothing pending");
        outbox.files.send(Outgoing { name: "main.rs".into(), data: b"fn main() {}\n".to_vec() }).unwrap();
        outbox.files.send(Outgoing { name: "main.rs".into(), data: b"v2".to_vec() }).unwrap();
        outbox.files.send(Outgoing { name: "../fuera.txt".into(), data: b"x".to_vec() }).unwrap();
        assert_eq!(client.poll(&downloads.0, false).unwrap().as_deref(), Some("main.rs"));
        client.send_screen(b"imagen").unwrap();
        client.send("de-ida.txt", b"ida").unwrap();
        assert_eq!(client.poll(&downloads.0, false).unwrap().as_deref(), Some("main (2).rs"), "the previous one is not overwritten");
        assert_eq!(client.poll(&downloads.0, false).unwrap().as_deref(), Some("fuera.txt"), "the name is sanitized on the laptop too");
        assert_eq!(client.poll(&downloads.0, false).unwrap(), None);
        drop(client);
        handle.join().unwrap().unwrap();
        assert_eq!(std::fs::read(downloads.0.join("main.rs")).unwrap(), b"fn main() {}\n");
        assert_eq!(std::fs::read(downloads.0.join("main (2).rs")).unwrap(), b"v2");
        assert!(!downloads.0.parent().unwrap().join("fuera.txt").exists(), "nothing outside the folder");
        assert!(inbox.0.join("de-ida.txt").exists(), "the other direction still works");
        let all: Vec<ServerEvent> = events.try_iter().collect();
        assert!(all.contains(&ServerEvent::Delivered("main (2).rs".into())));
    }

    #[test]
    fn pairing_codes_have_six_digits_and_vary() {
        let codes: Vec<String> = (0..20).map(|_| pairing_code()).collect();
        assert!(codes.iter().all(|c| c.len() == 6 && c.chars().all(|ch| ch.is_ascii_digit())));
        assert!(codes.iter().any(|c| c != &codes[0]), "not always the same");
    }

    #[test]
    fn a_paired_laptop_sends_files_that_land_in_the_inbox() {
        let inbox = Tmp::new("ok");
        let (addr, events, handle) = headset("123456", &inbox.0, Limits::default());
        let mut client = Client::connect(&addr, "123 456").unwrap();
        assert_eq!(client.send("main.rs", b"fn main() {}\n").unwrap(), "main.rs");
        let big: Vec<u8> = (0..(CHUNK * 3 + 17)).map(|i| (i % 251) as u8).collect();
        assert_eq!(client.send("datos.bin", &big).unwrap(), "datos.bin");
        drop(client);
        handle.join().unwrap().unwrap();
        assert_eq!(std::fs::read(inbox.0.join("main.rs")).unwrap(), b"fn main() {}\n");
        assert_eq!(std::fs::read(inbox.0.join("datos.bin")).unwrap(), big, "it arrives whole, in several chunks");
        let all: Vec<ServerEvent> = events.try_iter().collect();
        assert_eq!(all[0], ServerEvent::Paired);
        assert!(all.contains(&ServerEvent::Saved("main.rs".into())));
    }

    #[test]
    fn a_wrong_code_does_not_pair_and_three_strikes_close_the_door() {
        let inbox = Tmp::new("wrong");
        let (addr, events, handle) = headset("111111", &inbox.0, Limits::default());
        for _ in 0..MAX_ATTEMPTS {
            assert_eq!(Client::connect(&addr, "222222").err(), Some(LinkError::WrongCode));
        }
        assert_eq!(handle.join().unwrap(), Err(LinkError::TooManyAttempts));
        let all: Vec<ServerEvent> = events.try_iter().collect();
        assert_eq!(all, vec![ServerEvent::WrongCode { left: 2 }, ServerEvent::WrongCode { left: 1 }, ServerEvent::WrongCode { left: 0 }]);
        assert!(std::fs::read_dir(&inbox.0).unwrap().next().is_none(), "nothing was saved");
    }

    #[test]
    fn something_that_is_not_atlas_calling_the_port_is_ignored() {
        let inbox = Tmp::new("noise");
        let (addr, _events, handle) = headset("333333", &inbox.0, Limits::default());
        {
            let mut stranger = TcpStream::connect(&addr).unwrap();
            stranger.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
        }
        let mut client = Client::connect(&addr, "333333").unwrap();
        assert_eq!(client.send("a.txt", b"hola").unwrap(), "a.txt");
        drop(client);
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn names_cannot_escape_the_inbox_and_nothing_is_overwritten() {
        let inbox = Tmp::new("names");
        let (addr, _events, handle) = headset("444444", &inbox.0, Limits::default());
        let mut client = Client::connect(&addr, "444444").unwrap();
        assert_eq!(client.send("../../secreto.txt", b"1").unwrap(), "secreto.txt");
        assert_eq!(client.send("C:\\Windows\\win.ini", b"2").unwrap(), "win.ini");
        assert_eq!(client.send("secreto.txt", b"3").unwrap(), "secreto (2).txt", "the previous one is not overwritten");
        assert!(matches!(client.send("..", b"4"), Err(LinkError::Rejected(_))));
        assert_eq!(client.send("después.md", b"5").unwrap(), "después.md", "the connection continues after a refusal");
        drop(client);
        handle.join().unwrap().unwrap();
        assert!(!inbox.0.parent().unwrap().join("secreto.txt").exists(), "nothing outside the inbox");
        assert_eq!(std::fs::read(inbox.0.join("secreto.txt")).unwrap(), b"1");
    }

    #[test]
    fn files_over_the_limit_are_refused_without_breaking_the_connection() {
        let inbox = Tmp::new("big");
        let (addr, events, handle) = headset("555555", &inbox.0, Limits { max_file: 1000 });
        let mut client = Client::connect(&addr, "555555").unwrap();
        assert!(matches!(client.send("grande.bin", &vec![7u8; 5000]), Err(LinkError::Rejected(_))));
        assert_eq!(client.send("pequeño.txt", b"ok").unwrap(), "pequeño.txt");
        drop(client);
        handle.join().unwrap().unwrap();
        assert!(!inbox.0.join("grande.bin").exists());
        assert!(events.try_iter().any(|e| matches!(e, ServerEvent::Rejected(_))));
    }

    #[test]
    fn screen_images_arrive_whole_one_at_a_time_mixed_with_files() {
        let inbox = Tmp::new("screen");
        let (addr, events, handle) = headset("666666", &inbox.0, Limits::default());
        let mut client = Client::connect(&addr, "666666").unwrap();
        let big: Vec<u8> = (0..(CHUNK * 2 + 5)).map(|i| (i % 253) as u8).collect();
        client.send_screen(b"frame-1").unwrap();
        client.send("nota.txt", b"entre imagen e imagen").unwrap();
        client.send_screen(&big).unwrap();
        drop(client);
        handle.join().unwrap().unwrap();
        let screens: Vec<Vec<u8>> = events.try_iter().filter_map(|e| if let ServerEvent::Screen(img) = e { Some(img) } else { None }).collect();
        assert_eq!(screens, vec![b"frame-1".to_vec(), big], "they arrive whole and in order");
        assert!(inbox.0.join("nota.txt").exists(), "files keep working between images");
        assert!(!inbox.0.join("frame-1").exists(), "screen images are not saved as files");
    }

    #[test]
    fn sanitizing_names() {
        assert_eq!(sanitize_name("a/b/c.rs").as_deref(), Some("c.rs"));
        assert_eq!(sanitize_name("x\\y\\z.txt").as_deref(), Some("z.txt"));
        assert_eq!(sanitize_name("..").as_deref(), None);
        assert_eq!(sanitize_name("").as_deref(), None);
        assert_eq!(sanitize_name("mal\u{0}nombre?.txt").as_deref(), Some("malnombre.txt"));
        assert_eq!(sanitize_name(".gitignore").as_deref(), Some(".gitignore"));
        assert_eq!(sanitize_name(&"x".repeat(500)).map(|s| s.len()), Some(120));
    }

    #[test]
    fn a_tampered_message_is_detected() {
        // Two channels with the same key: altering one byte of the encrypted message makes verification fail.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let sender = std::thread::spawn(move || {
            let stream = TcpStream::connect(addr).unwrap();
            let mut ch = Channel::new(stream, &[9u8; 32], true).unwrap();
            let sealed = ch.send.encrypt(Nonce::from_slice(&nonce(0)), b"hola".as_slice()).unwrap();
            let mut bad = sealed.clone();
            bad[0] ^= 1;
            ch.stream.write_all(&(bad.len() as u32).to_be_bytes()).unwrap();
            ch.stream.write_all(&bad).unwrap();
        });
        let (stream, _) = listener.accept().unwrap();
        let mut ch = Channel::new(stream, &[9u8; 32], false).unwrap();
        assert!(matches!(ch.recv_bytes(), Err(LinkError::Protocol(_))));
        sender.join().unwrap();
    }
}
