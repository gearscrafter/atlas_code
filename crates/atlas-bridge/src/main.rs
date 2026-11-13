//! Atlas Bridge: connects your laptop (Windows or Mac) with Atlas in the headset, over your local network. It sends files,
//! shares your screen to see it inside Atlas, and lets the headset use the mouse and keyboard if you allow it.
//!
//! On the headset type `:bridge`: it shows an address and a 6-digit code. Enter them here, press Connect and
//! drop files on the window, or press Share screen. The files land in Atlas's `inbox/` folder. What
//! you send from the headset with `:send` lands in `Downloads/Atlas` on the laptop.
//!
//! Without a window: `atlas-bridge send <address> <code> <file>...`, `atlas-bridge screen <address> <code>
//! [seconds]` and `atlas-bridge session <address> <code> <seconds> [--screen] [file...]` (the same as the
//! window does, with the same thread, printing what happens to the console).
//! On Windows you can also drop files on the `.exe`: they are sent as soon as you connect.

mod control;
mod screen;

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

use atlas_link::{Client, LinkError, RemoteInput};
use eframe::egui;

/// How often the headset is asked whether it has something for the laptop (more often if you let it control the
/// laptop, so the mouse responds without sharing the screen).
const POLL_EVERY: Duration = Duration::from_millis(300);
const POLL_EVERY_CONTROL: Duration = Duration::from_millis(60);

/// Where what the headset sends is saved: `Downloads/Atlas` (on Windows and on Mac).
fn downloads_dir() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join("Downloads").join("Atlas")
}

/// Shows a file in the file explorer, selected (that way it shows up on the laptop screen).
fn reveal_file(path: &std::path::Path) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Some(dir) = path.parent() {
            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
        }
    }
}

/// Opens the folder in the file explorer.
fn open_folder(dir: &std::path::Path) {
    let _ = std::fs::create_dir_all(dir);
    let program = if cfg!(target_os = "windows") { "explorer" } else { "open" };
    let _ = std::process::Command::new(program).arg(dir).spawn();
}

/// What the window asks the thread that talks to the headset (so the window never freezes).
enum Cmd {
    Connect { address: String, code: String },
    Send(Vec<PathBuf>),
    Share(bool),
    /// Let (or not) the headset control the laptop's mouse and keyboard.
    Control(bool),
}

/// What the thread tells the window.
enum Note {
    Connected,
    Sent { file: String, saved: String },
    Failed(String),
    Disconnected(String),
    Sharing { fps: f32 },
    StoppedSharing(String),
    /// A file arrived from the headset, saved with this name in `Downloads/Atlas`.
    Received(String),
    /// The headset is using the mouse or the keyboard (how many orders in the last second).
    Controlled(u32),
}

impl Note {
    /// A line for the console (`atlas-bridge session`).
    fn line(&self) -> String {
        match self {
            Note::Connected => "connected".into(),
            Note::Sent { file, saved } => format!("sent {file} -> inbox/{saved}"),
            Note::Failed(e) => format!("failed: {e}"),
            Note::Disconnected(e) => format!("disconnected: {e}"),
            Note::Sharing { fps } => format!("sharing screen: {fps:.1} images per second"),
            Note::StoppedSharing(e) => format!("screen sharing {e}"),
            Note::Received(name) => format!("received from the headset: {}", downloads_dir().join(name).display()),
            Note::Controlled(n) => format!("the headset is controlling this laptop ({n} actions)"),
        }
    }
}

fn worker(cmds: Receiver<Cmd>, notes: Sender<Note>) {
    let mut client: Option<Client> = None;
    // Sharing the screen: the thread that captures, one image ahead.
    let mut grabber: Option<screen::Grabber> = None;
    let (mut frames, mut since) = (0u32, Instant::now());
    let mut last_poll = Instant::now();
    let downloads = downloads_dir();
    // Control from the headset: only if the user allows it. The one that moves the mouse is created when used.
    let mut control = false;
    let mut controller: Option<control::Controller> = None;
    let (mut actions, mut actions_since) = (0u32, Instant::now());
    loop {
        let poll_every = if control { POLL_EVERY_CONTROL } else { POLL_EVERY };
        // While sharing the screen, requests are checked without waiting; connected, it waits at most until the next
        // question to the headset; without a connection, until the next request.
        let cmd = if grabber.is_some() {
            match cmds.try_recv() {
                Ok(c) => Some(c),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        } else if client.is_some() {
            match cmds.recv_timeout(poll_every) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match cmds.recv() {
                Ok(c) => Some(c),
                Err(_) => return,
            }
        };
        // A network failure closes the connection (and stops sharing).
        let mut lost: Option<String> = None;
        match cmd {
            Some(Cmd::Connect { address, code }) => match Client::connect(&address, &code) {
                Ok(c) => {
                    client = Some(c);
                    last_poll = Instant::now();
                    let _ = notes.send(Note::Connected);
                }
                Err(e) => {
                    let _ = notes.send(Note::Failed(e.to_string()));
                }
            },
            Some(Cmd::Send(paths)) => {
                for path in paths {
                    let Some(c) = client.as_mut() else {
                        let _ = notes.send(Note::Failed("connect to the headset first".into()));
                        break;
                    };
                    let file = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    match c.send_file(&path) {
                        Ok(saved) => {
                            let _ = notes.send(Note::Sent { file, saved });
                        }
                        // A refused file does not cut the connection; a network failure does.
                        Err(LinkError::Rejected(reason)) => {
                            let _ = notes.send(Note::Failed(format!("{file}: {reason}")));
                        }
                        Err(e) => {
                            lost = Some(format!("{file}: {e}"));
                            break;
                        }
                    }
                }
            }
            Some(Cmd::Control(on)) => {
                control = on;
                if !on {
                    controller = None;
                }
            }
            Some(Cmd::Share(on)) => {
                (frames, since) = (0, Instant::now());
                if on && client.is_some() {
                    grabber = Some(screen::Grabber::start());
                } else {
                    grabber = None;
                    let _ = notes.send(Note::StoppedSharing(if on { "needs a connection first".into() } else { "stopped".into() }));
                }
            }
            None => {}
        }
        if let (Some(g), Some(c), None) = (&grabber, client.as_mut(), &lost) {
            match g.next(Duration::from_millis(500)) {
                None => {}
                Some(Ok(jpeg)) => match c.send_screen(&jpeg) {
                    Ok(()) => {
                        // The headset asks for the resolution depending on how big it sees the screen.
                        g.set_width(c.want_width());
                        frames += 1;
                        let elapsed = since.elapsed().as_secs_f32();
                        if elapsed >= 1.0 {
                            let _ = notes.send(Note::Sharing { fps: frames as f32 / elapsed });
                            (frames, since) = (0, Instant::now());
                        }
                    }
                    Err(e) => lost = Some(e.to_string()),
                },
                // Capture failure: sharing stops but the connection stays.
                Some(Err(e)) => {
                    grabber = None;
                    let _ = notes.send(Note::StoppedSharing(e));
                }
            }
        }
        // Ask the headset whether it has something for the laptop (what is sent with `:send`).
        if let (Some(c), None) = (client.as_mut(), &lost) {
            if last_poll.elapsed() >= poll_every {
                last_poll = Instant::now();
                match c.poll(&downloads, control) {
                    Ok(Some(name)) => {
                        let _ = notes.send(Note::Received(name));
                    }
                    Ok(None) => {}
                    Err(LinkError::Rejected(reason)) => {
                        let _ = notes.send(Note::Failed(format!("from the headset: {reason}")));
                    }
                    Err(e) => lost = Some(e.to_string()),
                }
            }
        }
        // The mouse and keyboard orders that came with the replies: they are obeyed only if allowed.
        if let Some(c) = client.as_mut() {
            let inputs = c.take_input();
            if control && !inputs.is_empty() {
                obey(&mut controller, &inputs, &notes);
                actions += inputs.len() as u32;
            }
            if actions > 0 && actions_since.elapsed() >= Duration::from_secs(1) {
                let _ = notes.send(Note::Controlled(actions));
                (actions, actions_since) = (0, Instant::now());
            }
        }
        if let Some(e) = lost {
            client = None;
            grabber = None;
            controller = None;
            let _ = notes.send(Note::Disconnected(e));
        }
    }
}

/// Does what the headset asks with the mouse and the keyboard.
fn obey(controller: &mut Option<control::Controller>, inputs: &[RemoteInput], notes: &Sender<Note>) {
    if controller.is_none() {
        match control::Controller::new() {
            Ok(c) => *controller = Some(c),
            Err(e) => {
                let _ = notes.send(Note::Failed(e));
                return;
            }
        }
    }
    let Some(c) = controller.as_mut() else { return };
    for input in inputs {
        if let Err(e) = c.apply(input) {
            let _ = notes.send(Note::Failed(e));
        }
    }
}

struct BridgeApp {
    address: String,
    code: String,
    connected: bool,
    busy: bool,
    sharing: bool,
    /// Show in the file explorer what arrives from the headset.
    reveal: bool,
    /// Let the headset control the mouse and the keyboard (off at the start).
    control: bool,
    status: String,
    history: Vec<String>,
    /// Files dropped on the `.exe`: they are sent on connecting.
    pending: Vec<PathBuf>,
    cmds: Sender<Cmd>,
    notes: Receiver<Note>,
}

impl BridgeApp {
    fn new(pending: Vec<PathBuf>) -> Self {
        let (cmds, cmd_rx) = channel();
        let (note_tx, notes) = channel();
        std::thread::spawn(move || worker(cmd_rx, note_tx));
        let status = if pending.is_empty() {
            "On the headset, type :bridge. It shows an address and a 6-digit code.".into()
        } else {
            format!("{} file(s) ready. Type :bridge on the headset and connect.", pending.len())
        };
        Self { address: String::new(), code: String::new(), connected: false, busy: false, sharing: false, reveal: true, control: false, status, history: Vec::new(), pending, cmds, notes }
    }

    fn send(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        self.busy = true;
        self.status = format!("Sending {} file(s)...", paths.len());
        let _ = self.cmds.send(Cmd::Send(paths));
    }
}

impl eframe::App for BridgeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(note) = self.notes.try_recv() {
            match note {
                Note::Connected => {
                    self.connected = true;
                    self.busy = false;
                    self.status = "Connected. Drop files here, or share your screen.".into();
                    let pending = std::mem::take(&mut self.pending);
                    self.send(pending);
                }
                Note::Sent { file, saved } => {
                    self.busy = false;
                    self.status = format!("Sent {file}");
                    self.history.insert(0, format!("{file}  →  inbox/{saved}"));
                    self.history.truncate(10);
                }
                Note::Failed(e) => {
                    self.busy = false;
                    self.status = e;
                }
                Note::Disconnected(e) => {
                    self.busy = false;
                    self.connected = false;
                    self.sharing = false;
                    self.status = format!("Disconnected: {e}. Type :bridge on the headset for a new code.");
                }
                Note::Sharing { fps } => {
                    self.status = format!("Sharing your screen · {fps:.1} images per second");
                }
                Note::StoppedSharing(why) => {
                    self.sharing = false;
                    self.status = format!("Screen sharing {why}");
                }
                Note::Controlled(_) => {
                    self.status = "The headset is controlling this laptop (untick the box to stop it)".into();
                }
                Note::Received(name) => {
                    if self.reveal {
                        reveal_file(&downloads_dir().join(&name));
                    }
                    self.status = format!("Received {name} from the headset");
                    self.history.insert(0, format!("headset  →  Downloads/Atlas/{name}"));
                    self.history.truncate(10);
                }
            }
        }

        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() {
            if self.connected {
                self.send(dropped);
            } else {
                self.status = "Connect to the headset first, then drop the files again.".into();
            }
        }
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Atlas Bridge");
            ui.add_space(6.0);
            ui.add_enabled_ui(!self.connected && !self.busy, |ui| {
                egui::Grid::new("connect").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label("Headset address");
                    ui.text_edit_singleline(&mut self.address);
                    ui.end_row();
                    ui.label("Code");
                    ui.text_edit_singleline(&mut self.code);
                    ui.end_row();
                });
                if ui.button("Connect").clicked() {
                    self.busy = true;
                    self.status = format!("Connecting to {}...", self.address.trim());
                    let _ = self.cmds.send(Cmd::Connect { address: self.address.trim().to_string(), code: self.code.trim().to_string() });
                }
            });
            ui.add_space(6.0);
            ui.add_enabled_ui(self.connected, |ui| {
                let label = if self.sharing { "Stop sharing screen" } else { "Share screen" };
                if ui.button(label).clicked() {
                    self.sharing = !self.sharing;
                    if self.sharing {
                        self.status = "Starting screen sharing...".into();
                    }
                    let _ = self.cmds.send(Cmd::Share(self.sharing));
                }
            });
            ui.add_space(8.0);
            ui.label(&self.status);
            ui.add_space(8.0);

            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 110.0), egui::Sense::hover());
            let color = if hovering && self.connected { egui::Color32::from_rgb(255, 150, 70) } else { egui::Color32::from_rgb(90, 200, 230) };
            ui.painter().rect_stroke(rect, 8.0, egui::Stroke::new(2.0_f32, color));
            let text = if self.connected { "Drop files here" } else { "Connect first, then drop files here" };
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(18.0), color);

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Files sent from the headset (:send) go to Downloads/Atlas.");
                if ui.button("Open folder").clicked() {
                    open_folder(&downloads_dir());
                }
            });
            ui.checkbox(&mut self.reveal, "Show each file from the headset in the file explorer");
            if ui.checkbox(&mut self.control, "Let the headset control this laptop's mouse and keyboard").changed() {
                let _ = self.cmds.send(Cmd::Control(self.control));
            }
            ui.add_space(8.0);
            for line in &self.history {
                ui.label(line);
            }
        });
        // The window refreshes by itself to show what the thread reports.
        ctx.request_repaint_after(Duration::from_millis(200));
    }
}

fn usage() -> i32 {
    eprintln!("usage: atlas-bridge send <headset address> <code> <file>...");
    eprintln!("       atlas-bridge screen <headset address> <code> [seconds]");
    eprintln!("       atlas-bridge session <headset address> <code> <seconds> [--screen] [--reveal] [--control] [file...]");
    2
}

/// `atlas-bridge send <address> <code> <file>...`: sends files without opening a window.
fn send_from_command_line(args: &[String]) -> i32 {
    let [address, code, files @ ..] = args else { return usage() };
    if files.is_empty() {
        return usage();
    }
    let mut client = match Client::connect(address, code) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cannot connect: {e}");
            return 1;
        }
    };
    let mut failed = false;
    for file in files {
        match client.send_file(std::path::Path::new(file)) {
            Ok(saved) => println!("{file} -> inbox/{saved}"),
            Err(e) => {
                eprintln!("{file}: {e}");
                failed = true;
            }
        }
    }
    i32::from(failed)
}

/// `atlas-bridge screen <address> <code> [seconds]`: shares the screen for a while without opening a window.
fn screen_from_command_line(args: &[String]) -> i32 {
    match args {
        [a, c] => session(a, c, 30.0, true, false, false, Vec::new()),
        [a, c, s] => session(a, c, s.parse::<f32>().unwrap_or(30.0), true, false, false, Vec::new()),
        _ => usage(),
    }
}

/// `atlas-bridge session <address> <code> <seconds> [--screen] [file...]`: connects, sends the files,
/// shares the screen if asked and receives what the headset sends, for those seconds. It uses the same thread as the
/// window and prints to the console what happens.
fn session_from_command_line(args: &[String]) -> i32 {
    let [address, code, seconds, rest @ ..] = args else { return usage() };
    let Ok(seconds) = seconds.parse::<f32>() else { return usage() };
    let share = rest.iter().any(|a| a == "--screen");
    let reveal = rest.iter().any(|a| a == "--reveal");
    let control = rest.iter().any(|a| a == "--control");
    let files = rest.iter().filter(|a| !a.starts_with("--")).map(PathBuf::from).collect();
    session(address, code, seconds, share, reveal, control, files)
}

#[allow(clippy::too_many_arguments)]
fn session(address: &str, code: &str, seconds: f32, share: bool, reveal: bool, control: bool, files: Vec<PathBuf>) -> i32 {
    let (cmds, cmd_rx) = channel();
    let (note_tx, notes) = channel();
    std::thread::spawn(move || worker(cmd_rx, note_tx));
    let _ = cmds.send(Cmd::Connect { address: address.to_string(), code: code.to_string() });
    if control {
        let _ = cmds.send(Cmd::Control(true));
    }
    let start = Instant::now();
    let (mut connected, mut failed) = (false, false);
    let mut files = Some(files);
    while start.elapsed().as_secs_f32() < seconds {
        let Ok(note) = notes.recv_timeout(Duration::from_millis(100)) else { continue };
        println!("[{:5.1} s] {}", start.elapsed().as_secs_f32(), note.line());
        match note {
            Note::Connected => {
                connected = true;
                if share {
                    let _ = cmds.send(Cmd::Share(true));
                }
                if let Some(files) = files.take().filter(|f| !f.is_empty()) {
                    let _ = cmds.send(Cmd::Send(files));
                }
            }
            Note::Received(name) if reveal => reveal_file(&downloads_dir().join(name)),
            Note::Failed(_) if !connected => return 1,
            Note::Failed(_) => failed = true,
            Note::Disconnected(_) => return 1,
            _ => {}
        }
    }
    i32::from(failed || !connected)
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("send") => std::process::exit(send_from_command_line(&args[1..])),
        Some("screen") => std::process::exit(screen_from_command_line(&args[1..])),
        Some("session") => std::process::exit(session_from_command_line(&args[1..])),
        _ => {}
    }
    // Files dropped on the executable (Windows) or passed as arguments.
    let pending: Vec<PathBuf> = args.iter().map(PathBuf::from).filter(|p| p.is_file()).collect();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([520.0, 460.0]).with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native("Atlas Bridge", options, Box::new(move |_cc| Ok(Box::new(BridgeApp::new(pending)))))
}
