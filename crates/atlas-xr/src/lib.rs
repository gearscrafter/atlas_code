//! Atlas Code, the XR app for Meta Quest.
//!
//! OpenXR session over Vulkan with passthrough and floating panels: the code
//! editor and, below it, an output panel with git's progress.

mod input;
mod handmesh;
mod hands;
mod keystore;
mod orbs;
mod panel_slot;
mod selftest;
mod sysinput;
mod vk_ctx;

use android_activity::input::{InputEvent, KeyAction};
use android_activity::{AndroidApp, InputStatus, MainEvent, PollEvent};
use ash::vk::{self, Handle};
use log::{error, info};
use openxr as xr;
use std::sync::mpsc;
use std::time::Duration;

use atlas_editor::{parent_of, perform, Action, Editor, Entry, FileInfo, Key, Mode, GitRequest, Log, Progress, Workspace};
use atlas_git::GitOutcome;
use atlas_script::{CardRelease, Command as ScriptCommand, KeyboardKind, EventArg, Runtime as Script, ScriptError, Snapshot};
use atlas_ui::{AlertPanel, AlertView, ALERT_H, ALERT_W, ALERT_W_M, CardsPanel, CardsView, CodePanel, KeyView, KeyboardPanel, KeyboardView, PadView, CARD_COLS, CARD_ROWS, KB_H, KB_W, KB_W_M};
use atlas_spatial::World as PhysWorld;
use atlas_spatial::{KeyAction as KbKey, Keyboard, Mat3, PadEvent, GRIP_NEAR, GRIP_RADIUS, PAD_H, PAD_W, PAD_X, PAD_Y, SCROLL_W};
use atlas_spatial::{eye_camera, Anchor, fan_poses, title_pose, window, CardPose, Drawer, Fan, PalmSwipe, TRAVEL, Body, Desk, Hit, VelocityTracker, Wall, WallCalibrator, DeskCalibrator, classify_swipe, Swipe, DragSystem, Draggable, Fov, PinchEvent, Quat, Vec3};
use hands::{shoulder, HandFrame, HandInput};
use orbs::{env_orb, ENVIRONMENTS, fingertip_marker, highlight, HandDraw, OrbInstance, OrbRenderer, Roll, Surface};
use panel_slot::PanelSlot;
use vk_ctx::{QuadTexture, VkContext, COLOR_FORMAT, PIPELINE_DEPTH, VIEW_COUNT};

const VIEW_TYPE: xr::ViewConfigurationType = xr::ViewConfigurationType::PRIMARY_STEREO;

/// Texture of the code panel. 2048 px over 1.2 m at 0.9 m give about 30 px/degree, above
/// the Quest 3's density (~25): that way the panel does not lose sharpness when resampled to the headset's display.
const PANEL_W: u32 = 2048;
const PANEL_H: u32 = 1365;
/// Default width (m) of the panel (`config.panel_width`).
const PANEL_W_M: f32 = 1.2;
/// Height over width of the texture.
const PANEL_ASPECT: f32 = PANEL_H as f32 / PANEL_W as f32;
/// Font size (px) with `config.font_scale` = 1. It is the usual one (30 px over 1536) scaled to 2048.
const FONT_BASE_PX: f32 = 40.0;
/// Output panel, below the code one.
const LOG_W: u32 = 1536;
const LOG_H: u32 = 400;
const LOG_W_M: f32 = 1.2;
/// Upload slots of each panel (see `vk_ctx::UPLOAD_SLOTS`).
const CODE_SLOT: usize = 0;
const LOG_SLOT: usize = 1;
const CARDS_SLOT: usize = 2;
const KB_SLOT: usize = 3;
const ALERT_SLOT: usize = 4;
const SCREEN_SLOT: usize = 5;
/// The laptop screen: texture (the laptop reduces it to at most this) and width in the headset (m).
const SCREEN_W: u32 = 2560;
const SCREEN_H: u32 = 1600;
const SCREEN_W_M: f32 = 1.0;
/// Minimum and maximum width (m) when resizing the screen by its corners.
const SCREEN_MIN_W: f32 = 0.4;
const SCREEN_MAX_W: f32 = 3.0;
/// User configuration, at the root of the workspace.
const INIT_FILE: &str = "init.lua";
/// Template with everything commented out, created the first time.
const INIT_TEMPLATE: &str = include_str!("../assets/init.lua");
/// Texture of the cards: a grid of CARD_COLS x CARD_ROWS cards of 512 x 384.
const CARDS_W: u32 = 2048;
const CARDS_H: u32 = 1152;
/// Cards that fit in the grid.
/// Working file in the app's storage.
const SCRATCH_FILE: &str = "scratch.rs";
const WELCOME: &str = include_str!("../assets/welcome.rs");
/// Alpha 0: lets the passthrough show under the projection layer.
const CLEAR_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 0.0];

/// What a git thread tells the main loop.
enum GitEvent {
    Progress(Progress),
    Done(GitOutcome),
}

#[no_mangle]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("atlas"),
    );
    std::panic::set_hook(Box::new(|p| error!("panic: {p}")));
    if let Err(e) = run(&app) {
        error!("fallo fatal: {e}");
    }
    info!("saliendo");
}

/// What a script sees of the editor's state.
fn snapshot_of(editor: &Editor) -> Snapshot {
    let c = editor.buffer.cursor();
    Snapshot {
        text: editor.buffer.to_string(),
        file: editor.file.clone(),
        mode: format!("{:?}", editor.mode).to_uppercase(),
        cursor: (c.line + 1, c.col + 1),
    }
}

/// Notifies the scripts of an event; errors go to the output panel.
fn fire(script: &mut Script, editor: &mut Editor, name: &str, args: &[EventArg]) {
    for e in script.emit(name, args) {
        editor.log.push(format!("ERROR gesture.on('{name}'): {e}"));
        editor.log.set_visible(true);
    }
}

/// Dumps to the output panel what the script printed and its errors.
fn report_script(editor: &mut Editor, script: &mut Script, errors: &[ScriptError], ok: &str) {
    for line in script.take_output() {
        editor.log.push(line);
    }
    for e in errors {
        editor.log.push(format!("ERROR {e}"));
    }
    if errors.is_empty() {
        editor.log.push(format!("OK {ok} ({} orbes)", script.orbs().len()));
    } else {
        editor.message = format!("script: {}", errors[0]);
    }
    editor.log.set_visible(true);
}

/// Runs a `:` order a script requested, as if you had typed it (only in Normal or
/// Browser, so as not to type inside your text; and without git, scripts or quitting the app).
fn run_ex(editor: &mut Editor, workspace: &Workspace, command: &str) {
    if !matches!(editor.mode, Mode::Normal | Mode::Browser) {
        editor.message = format!("editor.command('{command}') only works in Normal mode");
        return;
    }
    let keys = std::iter::once(Key::Char(':')).chain(command.chars().map(Key::Char)).chain(std::iter::once(Key::Enter));
    for key in keys {
        let action = editor.handle_key(key);
        if matches!(action, Action::Git(_) | Action::Run | Action::Reload | Action::Set { .. } | Action::Bridge | Action::Send(_) | Action::SelfTest(_) | Action::Remote(_) | Action::Caption(_) | Action::Quit | Action::SaveAndQuit) {
            editor.message = "that command can only be typed on the keyboard".into();
            return;
        }
        perform(editor, &action, workspace);
    }
}

/// The bridge with the laptop while it listens: the code to type there, the headset's address and what
/// its thread reports.
struct Bridge {
    code: String,
    address: String,
    events: mpsc::Receiver<BridgeEvent>,
    /// The last image of the laptop screen, already decoded. Only the last one is kept: if the headset is
    /// slow (or idle), the old ones are discarded instead of piling up in memory.
    screen: std::sync::Arc<std::sync::Mutex<Option<ScreenFrame>>>,
    /// What goes to the laptop: files (`:send`), mouse and keyboard orders (`:remote`) and the width that suits
    /// the image. It is delivered in the replies.
    to_laptop: atlas_link::ToLaptop,
}

/// An image of the laptop screen, in RGBA.
#[derive(Default)]
struct ScreenFrame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// Decodes a JPEG from the laptop. `None` if it is not a valid image.
fn decode_screen(jpeg: &[u8]) -> Option<ScreenFrame> {
    // With limits: a small JPEG can declare a huge size and exhaust memory when decompressed.
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(SCREEN_W * 2);
    limits.max_image_height = Some(SCREEN_H * 2);
    limits.max_alloc = Some(64 * 1024 * 1024);
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(jpeg), image::ImageFormat::Jpeg);
    reader.limits(limits);
    let image = reader.decode().ok()?.to_rgba8();
    Some(ScreenFrame { width: image.width(), height: image.height(), rgba: image.into_raw() })
}

/// What is drawn on top of the laptop screen: the ray's cursor, the outline and the corner grips.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct ScreenUi {
    /// Where the ray points, in canvas pixels.
    cursor: Option<(f32, f32)>,
    /// 0 nothing, 1 pointed at, 2 grabbed (moving it or resizing it).
    state: u8,
    /// The corner pointed at or grabbed (0 top left, 1 top right, 2 bottom left, 3 bottom right).
    corner: Option<usize>,
    /// The keyboard types on the laptop (`:remote`, after pinching its screen): orange outline.
    focus: bool,
}

/// The screen while a hand's ray holds it.
#[derive(Debug, Clone, Copy)]
enum ScreenRay {
    /// Moving it: the point where it was grabbed (on the screen) and the distances from the shoulder to that point and to the hand.
    Move { hand: usize, local_hit: Vec3, hit_dist: f32, hand_dist: f32 },
    /// Resizing it by a corner: the width and the distance to the center at the start.
    Resize { hand: usize, corner: usize, width: f32, reach: f32 },
}

impl ScreenRay {
    fn hand(&self) -> usize {
        match self {
            ScreenRay::Move { hand, .. } | ScreenRay::Resize { hand, .. } => *hand,
        }
    }
}

/// Center of the screen image and its height. The layer shows exactly the image (its rectangle inside the
/// texture), so `pose` is already its center; `aspect` is the image's height / width.
fn visible_rect(pose: Vec3, _rot: Quat, width: f32, aspect: f32) -> (Vec3, f32) {
    (pose, width * aspect)
}

/// Where the screen goes so that its image is centered on `center`.
fn screen_pose_for(center: Vec3, rot: Quat, _width: f32, _aspect: f32) -> (Vec3, Quat) {
    (center, rot)
}

/// The width (px) that suits the laptop image to look sharp: the angle the screen takes in your view
/// times the Quest 3's ~26 pixels per degree, rounded to multiples of 320 between 960 and 2560.
fn wanted_width(screen_w: f32, distance: f32) -> u32 {
    let degrees = 2.0 * (screen_w / 2.0 / distance.max(0.2)).atan().to_degrees();
    (((degrees * 26.0 / 320.0).ceil() as u32) * 320).clamp(960, 2560)
}

/// The keys that go to the laptop, as orders: consecutive characters are joined into a single text.
fn keys_to_remote(keys: &[Key]) -> Vec<atlas_link::RemoteInput> {
    use atlas_link::RemoteInput;
    let mut out = Vec::new();
    let mut text = String::new();
    let key = |name: &str, ctrl: bool| RemoteInput::Key { key: name.to_string(), ctrl };
    for k in keys {
        let special = match k {
            Key::Char('\t') => Some(key("tab", false)),
            Key::Char(c) => {
                text.push(*c);
                None
            }
            Key::Ctrl(c) => Some(key(&c.to_string(), true)),
            Key::Enter => Some(key("enter", false)),
            Key::Backspace => Some(key("backspace", false)),
            Key::Delete => Some(key("delete", false)),
            Key::Escape => Some(key("escape", false)),
            Key::Left => Some(key("left", false)),
            Key::Right => Some(key("right", false)),
            Key::Up => Some(key("up", false)),
            Key::Down => Some(key("down", false)),
            Key::Home => Some(key("home", false)),
            Key::End => Some(key("end", false)),
        };
        if let Some(special) = special {
            if !text.is_empty() {
                out.push(RemoteInput::Text { text: std::mem::take(&mut text) });
            }
            out.push(special);
        }
    }
    if !text.is_empty() {
        out.push(RemoteInput::Text { text });
    }
    out
}

/// Where the line from `origin` in direction `dir` cuts the plane of a panel centered at `center`: the point in
/// panel coordinates (X right, Y up, Z = 0) and the distance from `origin`. `None` if it does not cut it in front.
fn ray_plane(origin: Vec3, dir: Vec3, center: Vec3, rotation: Quat) -> Option<(Vec3, f32)> {
    let normal = rotation * Vec3::Z;
    let denom = dir.dot(normal);
    if denom.abs() < 1e-4 {
        return None;
    }
    let t = (center - origin).dot(normal) / denom;
    if t <= 0.0 {
        return None;
    }
    let local = rotation.inverse() * (origin + dir * t - center);
    Some((Vec3::new(local.x, local.y, 0.0), t))
}

/// Puts the image at the top left of a canvas for the headset's screen, over a dark background (if it is larger, it is
/// cropped). At the top left, because the layer shows only the rectangle the image takes: a wide screen leaves no bars.
/// On top, the L-shaped corner grips (where it is resized), the outline if you point at it or carry it, and the
/// ray's cursor.
fn screen_canvas(frame: Option<&ScreenFrame>, ui: &ScreenUi) -> atlas_ui::Canvas {
    let mut canvas = atlas_ui::Canvas::new(SCREEN_W, SCREEN_H);
    canvas.clear([6, 10, 14], 255);
    let Some(frame) = frame.filter(|f| f.rgba.len() == (f.width * f.height * 4) as usize) else {
        return canvas;
    };
    let (w, h) = (frame.width.min(SCREEN_W), frame.height.min(SCREEN_H));
    // Top left: the layer shows only that rectangle, the exact size of the image.
    let (ox, oy) = (0, 0);
    for y in 0..h {
        let src = (y * frame.width * 4) as usize;
        let dst = (((oy + y) * SCREEN_W + ox) * 4) as usize;
        canvas.rgba[dst..dst + (w * 4) as usize].copy_from_slice(&frame.rgba[src..src + (w * 4) as usize]);
    }
    const CYAN: [u8; 3] = [120, 225, 255];
    const ORANGE: [u8; 3] = [255, 205, 160];
    const WHITE: [u8; 3] = [255, 245, 235];
    let (x0, x1, y1) = (ox as i32, (ox + w) as i32, h as i32);
    if ui.state > 0 || ui.focus {
        let (color, t) = if ui.state == 2 { (WHITE, 6) } else if ui.focus { ([255, 150, 70], 8) } else { (CYAN, 4) };
        canvas.fill_rect(x0, 0, w as i32, t, color, 255);
        canvas.fill_rect(x0, y1 - t, w as i32, t, color, 255);
        canvas.fill_rect(x0, 0, t, y1, color, 255);
        canvas.fill_rect(x1 - t, 0, t, y1, color, 255);
    }
    let arm = (w.min(h) as f32 * 0.09) as i32;
    let thick = 12;
    for (i, (cx, cy, right, down)) in [(x0, 0, true, true), (x1, 0, false, true), (x0, y1, true, false), (x1, y1, false, false)].into_iter().enumerate() {
        let (color, alpha) = match (ui.corner == Some(i), ui.state) {
            (true, 2) => (WHITE, 255),
            (true, _) => (ORANGE, 255),
            (false, s) if s > 0 => (CYAN, 235),
            _ => (CYAN, 150),
        };
        let hx = if right { cx } else { cx - arm };
        let hy = if down { cy } else { cy - thick };
        canvas.fill_rect(hx, hy, arm, thick, color, alpha);
        let vx = if right { cx } else { cx - thick };
        let vy = if down { cy } else { cy - arm };
        canvas.fill_rect(vx, vy, thick, arm, color, alpha);
    }
    if let Some((cx, cy)) = ui.cursor {
        for (r, color, alpha) in [(16.0f32, [10, 10, 10], 170u8), (10.0, WHITE, 255)] {
            for y in (cy - r) as i32..=(cy + r) as i32 {
                for x in (cx - r) as i32..=(cx + r) as i32 {
                    let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                    if d <= r {
                        canvas.blend_pixel(x, y, color, alpha);
                    }
                }
            }
        }
    }
    canvas
}

enum BridgeEvent {
    Server(atlas_link::ServerEvent),
    Done(Result<(), atlas_link::LinkError>),
}

/// The pairing code with a space in the middle, to read it at a glance: `482 913`.
fn spaced_code(code: &str) -> String {
    if code.len() == 6 { format!("{} {}", &code[..3], &code[3..]) } else { code.to_string() }
}

/// The alert that stays while the bridge waits for the laptop.
fn bridge_waiting(b: &Bridge, note: &str) -> AlertView {
    AlertView { title: "Atlas Bridge".into(), big: spaced_code(&b.code), body: format!("On your laptop: {} · {note}", b.address) }
}

/// This headset's address on the local network. It sends nothing: it only asks which way a packet would go out.
fn local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

/// Starts listening for Atlas Bridge with a new code. What arrives is saved in `inbox`.
fn start_bridge(inbox: std::path::PathBuf) -> Result<Bridge, String> {
    let listener = std::net::TcpListener::bind(("0.0.0.0", atlas_link::PORT))
        .map_err(|e| format!("bridge: cannot listen on port {}: {e}", atlas_link::PORT))?;
    let code = atlas_link::pairing_code();
    let address = local_ip().unwrap_or_else(|| "(no network)".into());
    let (tx, events) = mpsc::channel();
    let thread_code = code.clone();
    let screen = std::sync::Arc::new(std::sync::Mutex::new(None));
    let latest = screen.clone();
    let (to_laptop, out) = atlas_link::outbox();
    std::thread::Builder::new()
        .name("bridge".into())
        .spawn(move || {
            let events_tx = tx.clone();
            let (mut decoded, mut decode_time) = (0u32, Duration::ZERO);
            let result = atlas_link::serve_one(&listener, &thread_code, &inbox, atlas_link::Limits::default(), &out, |e| match e {
                // The screen is decoded here, on the bridge thread, so as not to slow down the headset's loop.
                atlas_link::ServerEvent::Screen(jpeg) => {
                    let t = std::time::Instant::now();
                    let frame = decode_screen(&jpeg);
                    decode_time += t.elapsed();
                    decoded += 1;
                    if decoded % 60 == 0 {
                        let size = frame.as_ref().map_or((0, 0), |f| (f.width, f.height));
                        info!(
                            "bridge: screen, {decoded} images, arriving at {}x{}, decoding {:?} on average",
                            size.0,
                            size.1,
                            decode_time / 60
                        );
                        decode_time = Duration::ZERO;
                    }
                    if let Some(frame) = frame {
                        if let Ok(mut slot) = latest.lock() {
                            *slot = Some(frame);
                        }
                    }
                }
                other => {
                    let _ = events_tx.send(BridgeEvent::Server(other));
                }
            });
            let _ = tx.send(BridgeEvent::Done(result));
        })
        .map_err(|e| format!("bridge: {e}"))?;
    Ok(Bridge { code, address, events, screen, to_laptop })
}

/// The editor key that corresponds to a press on the virtual keyboard.
fn key_from_virtual(action: KbKey) -> Key {
    match action {
        KbKey::Char(c) => Key::Char(c),
        KbKey::Ctrl(c) => Key::Ctrl(c),
        KbKey::Enter => Key::Enter,
        KbKey::Backspace => Key::Backspace,
        KbKey::Escape => Key::Escape,
        KbKey::Tab => Key::Char('\t'),
        KbKey::Left => Key::Left,
        KbKey::Right => Key::Right,
        KbKey::Up => Key::Up,
        KbKey::Down => Key::Down,
    }
}

/// Moves to the next environment and announces it.
fn next_environment(environment: &mut u32, editor: &mut Editor) {
    *environment = (*environment + 1) % 4;
    editor.message = format!("environment: {}", ENVIRONMENTS[*environment as usize]);
    info!("environment {}: {}", *environment, ENVIRONMENTS[*environment as usize]);
}

fn quat_pose(position: Vec3, rotation: Quat) -> xr::Posef {
    xr::Posef {
        orientation: xr::Quaternionf { x: rotation.x, y: rotation.y, z: rotation.z, w: rotation.w },
        position: xr::Vector3f { x: position.x, y: position.y, z: position.z },
    }
}

/// Orientation of a panel placed at `position` so that it faces `head`.
fn face(position: Vec3, head: Vec3) -> Quat {
    let d = head - position;
    let horizontal = (d.x * d.x + d.z * d.z).sqrt();
    let yaw = d.x.atan2(d.z);
    let pitch = (-d.y.atan2(horizontal)).clamp(-0.7, 0.7);
    Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch)
}

/// Is `point` over the editor panel (with some slack)?
fn hits_panel(position: Vec3, rotation: Quat, point: Vec3, width: f32) -> bool {
    hits_quad(position, rotation, point, width, width * PANEL_ASPECT)
}

/// Puts a file in the outbox toward the laptop (`:send`, or dropping a card in front of its screen).
/// `bridge` is the bridge if the laptop is paired. Returns the message for the status bar.
fn send_to_laptop(bridge: Option<&Bridge>, name: &str, data: Option<Vec<u8>>) -> Result<String, String> {
    let short = name.rsplit('/').next().unwrap_or(name).to_string();
    match (bridge, data) {
        (None, _) => Err("send: connect the laptop first (type :bridge and enter the code on Atlas Bridge)".into()),
        (Some(_), None) if name.is_empty() => Err("send: open a file first, or :send <file>".into()),
        (Some(_), None) => Err(format!("send: cannot read {name}")),
        (Some(b), Some(data)) => b
            .to_laptop
            .files
            .send(atlas_link::Outgoing { name: short.clone(), data })
            .map(|()| format!("sending {short} to the laptop..."))
            .map_err(|_| "send: the bridge is closed (type :bridge)".into()),
    }
}

/// Does the line from `origin` through `through` cross the `width` x `height` rectangle centered at
/// `center`? (From your eyes: does `through` cover the rectangle?)
fn ray_hits_quad(origin: Vec3, through: Vec3, center: Vec3, rotation: Quat, width: f32, height: f32) -> bool {
    let dir = (through - origin).normalize_or_zero();
    let normal = rotation * Vec3::Z;
    let denom = dir.dot(normal);
    if denom.abs() < 1e-4 {
        return false;
    }
    let t = (center - origin).dot(normal) / denom;
    if t <= 0.0 {
        return false;
    }
    let local = rotation.inverse() * (origin + dir * t - center);
    local.x.abs() <= width / 2.0 && local.y.abs() <= height / 2.0
}

/// Is `point` over a `width` x `height` rectangle centered at `position` (with some slack)?
fn hits_quad(position: Vec3, rotation: Quat, point: Vec3, width: f32, height: f32) -> bool {
    let local = rotation.inverse() * (point - position);
    local.x.abs() <= width / 2.0 + 0.05 && local.y.abs() <= height / 2.0 + 0.05 && local.z.abs() <= 0.10
}

/// A pinch made in the air: where it started and what is being done with it.
#[derive(Debug, Clone, Copy)]
struct AirGesture {
    lateral: f32,
    height: f32,
    kind: Option<Swipe>,
    base: f32,
    applied: i32,
}

/// Quad layer that shows only the top-left `rect_w` x `rect_h` px rectangle of the texture, with
/// `width_m` of width and the height that matches (the laptop screen, which changes size without changing texture).
fn quad_layer_rect<'a>(
    space: &'a xr::Space,
    texture: &'a QuadTexture,
    pose: xr::Posef,
    width_m: f32,
    rect_w: i32,
    rect_h: i32,
) -> xr::CompositionLayerQuad<'a, xr::Vulkan> {
    let (rect_w, rect_h) = (rect_w.clamp(1, texture.width as i32), rect_h.clamp(1, texture.height as i32));
    let rect = xr::Rect2Di { offset: xr::Offset2Di { x: 0, y: 0 }, extent: xr::Extent2Di { width: rect_w, height: rect_h } };
    xr::CompositionLayerQuad::new()
        .layer_flags(xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA | xr::CompositionLayerFlags::UNPREMULTIPLIED_ALPHA)
        .space(space)
        .eye_visibility(xr::EyeVisibility::BOTH)
        .sub_image(xr::SwapchainSubImage::new().swapchain(&texture.handle).image_array_index(0).image_rect(rect))
        .pose(pose)
        .size(xr::Extent2Df { width: width_m, height: width_m * rect_h as f32 / rect_w as f32 })
}

/// Quad layer with non-premultiplied alpha for a panel texture.
///
/// `reveal` (0..1) shows only that top fraction of the panel, like a half-unrolled
/// scroll; `pose` is the whole panel's and here the center is lowered so that the top edge
/// stays fixed.
fn quad_layer<'a>(
    space: &'a xr::Space,
    texture: &'a QuadTexture,
    pose: xr::Posef,
    width_m: f32,
    reveal: f32,
) -> xr::CompositionLayerQuad<'a, xr::Vulkan> {
    let reveal = reveal.clamp(0.0, 1.0);
    let rows = ((texture.height as f32 * reveal).round() as i32).clamp(1, texture.height as i32);
    let visible = rows as f32 / texture.height as f32;
    let full_h = width_m * texture.height as f32 / texture.width as f32;
    let q = pose.orientation;
    let rot = Quat::from_xyzw(q.x, q.y, q.z, q.w);
    let shift = rot * Vec3::new(0.0, full_h / 2.0 * (1.0 - visible), 0.0);
    let pose = xr::Posef {
        orientation: pose.orientation,
        position: xr::Vector3f { x: pose.position.x + shift.x, y: pose.position.y + shift.y, z: pose.position.z + shift.z },
    };
    let rect = xr::Rect2Di {
        offset: xr::Offset2Di { x: 0, y: 0 },
        extent: xr::Extent2Di { width: texture.width as _, height: rows },
    };
    xr::CompositionLayerQuad::new()
        .layer_flags(
            xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA
                | xr::CompositionLayerFlags::UNPREMULTIPLIED_ALPHA,
        )
        .space(space)
        .eye_visibility(xr::EyeVisibility::BOTH)
        .sub_image(xr::SwapchainSubImage::new().swapchain(&texture.handle).image_array_index(0).image_rect(rect))
        .pose(pose)
        .size(xr::Extent2Df { width: width_m, height: full_h * visible })
}

/// Quad layer of a card: a crop of the cards texture (`cell` = column and row).
fn card_layer<'a>(
    space: &'a xr::Space,
    texture: &'a QuadTexture,
    pose: xr::Posef,
    width_m: f32,
    cell: (u32, u32),
) -> xr::CompositionLayerQuad<'a, xr::Vulkan> {
    let (cw, ch) = (CARDS_W / CARD_COLS, CARDS_H / CARD_ROWS);
    let rect = xr::Rect2Di {
        offset: xr::Offset2Di { x: (cell.0 * cw) as i32, y: (cell.1 * ch) as i32 },
        extent: xr::Extent2Di { width: cw as i32, height: ch as i32 },
    };
    xr::CompositionLayerQuad::new()
        .layer_flags(xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA | xr::CompositionLayerFlags::UNPREMULTIPLIED_ALPHA)
        .space(space)
        .eye_visibility(xr::EyeVisibility::BOTH)
        .sub_image(xr::SwapchainSubImage::new().swapchain(&texture.handle).image_array_index(0).image_rect(rect))
        .pose(pose)
        .size(xr::Extent2Df { width: width_m, height: width_m * ch as f32 / cw as f32 })
}

/// Short description of a git operation for the log.
fn describe(request: &GitRequest) -> String {
    match request {
        GitRequest::Clone { url, .. } => format!("git clone {url}"),
        GitRequest::Status => "git status".into(),
        GitRequest::Diff => "git diff".into(),
        GitRequest::Log => "git log".into(),
        GitRequest::Commit(message) => format!("git commit -m {message:?}"),
        GitRequest::Fetch => "git fetch".into(),
        GitRequest::Pull => "git pull".into(),
        GitRequest::Push => "git push".into(),
        // The token (or the user) is never written to the visible log.
        GitRequest::SetToken { host, .. } => match host {
            Some(host) => format!("git token {host} (oculto)"),
            None => "git token (oculto)".into(),
        },
        GitRequest::Logout(host) => format!("git logout {}", host.as_deref().unwrap_or("(todos)")),
        GitRequest::Credentials => "git creds".into(),
    }
}

/// Shows the result of a git operation: notes it in the log, opens the
/// long text in a buffer or leaves a message, and after a clone opens the browser
/// in the new folder.
fn apply_git_outcome(editor: &mut Editor, workspace: &Workspace, outcome: GitOutcome) {
    info!(
        "git -> ok={} mensaje={:?} vista={:?}",
        outcome.ok,
        outcome.message,
        outcome.view.as_ref().map(|(n, t)| (n.as_str(), t.lines().take(6).collect::<Vec<_>>().join(" | ")))
    );
    let prefix = if outcome.ok { "OK" } else { "ERROR" };
    editor.log.push(format!("{prefix} {}", outcome.message));

    match outcome.view {
        // Opening the view would replace the buffer: it is not done if there are unsaved changes.
        Some((name, _)) if editor.modified => {
            editor.message = format!("E37: save first (:w) to view {name}");
        }
        Some((name, text)) => {
            editor.load(&name, &text);
            editor.message = format!("{name}   q: back to the file");
        }
        None => editor.message = outcome.message,
    }
    if let Some(dir) = outcome.browse {
        let message = std::mem::take(&mut editor.message);
        perform(editor, &Action::Browse(dir), workspace);
        // The browser must not erase the notice that the clone finished.
        if editor.message.is_empty() {
            editor.message = message;
        }
    }
}

/// Sets up where and how tokens are saved:
///
/// * In `no_backup`, the folder Android always excludes from backups,
///   and outside the workspace (which can be read with `adb pull`).
/// * Encrypted with a Keystore key that never leaves the device. If the Keystore does not
///   work, no token is saved in plain text.
fn setup_credentials(app: &AndroidApp) {
    atlas_git::require_encryption(true);
    let Some(files) = app.internal_data_path() else {
        info!("no private folder: tokens cannot be saved");
        return;
    };
    let dir = files.parent().map(|p| p.join("no_backup")).unwrap_or_else(|| files.clone());
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("credenciales");

    // Earlier versions saved it in `files/` (which is included in backups).
    let old = files.join("credenciales");
    if old.exists() && !path.exists() {
        match std::fs::rename(&old, &path) {
            Ok(()) => info!("credenciales movidas a no_backup"),
            Err(e) => error!("the credentials could not be moved: {e}"),
        }
    }
    atlas_git::set_credentials_file(path);

    match keystore::self_test() {
        Ok(()) => {
            atlas_git::set_secret_box(Some(std::sync::Arc::new(keystore::AndroidKeystore)));
            match atlas_git::migrate_plaintext() {
                Ok(0) => {}
                Ok(n) => info!("{n} plain-text token(s) encrypted with the Keystore"),
                Err(e) => error!("could not migrate to encrypted tokens: {}", atlas_git::redact(&e)),
            }
            info!("Keystore working: tokens are saved encrypted");
        }
        Err(e) => error!("Keystore NOT available ({e}): tokens will not be saved"),
    }
}

fn run(app: &AndroidApp) -> Result<(), Box<dyn std::error::Error>> {
    let platform = unsafe {
        xr::AndroidPlatformInfo::new(app.vm_as_ptr().cast(), app.activity_as_ptr().cast())
    };
    let entry = unsafe { xr::Entry::load(&platform)? };

    let available = entry.enumerate_extensions()?;
    if !available.khr_vulkan_enable2 {
        return Err("the runtime does not offer XR_KHR_vulkan_enable2".into());
    }
    let mut exts = xr::ExtensionSet::default();
    exts.khr_vulkan_enable2 = true;
    exts.khr_android_create_instance = true;
    let has_hands = available.ext_hand_tracking;
    exts.ext_hand_tracking = has_hands;
    exts.fb_hand_tracking_mesh = has_hands && available.fb_hand_tracking_mesh;
    let has_passthrough = available.fb_passthrough;
    exts.fb_passthrough = has_passthrough;

    let instance = entry.create_instance(
        &xr::ApplicationInfo {
            application_name: "Atlas Code",
            application_version: 1,
            engine_name: "atlas",
            engine_version: 1,
            api_version: xr::Version::new(1, 0, 0),
        },
        &exts,
        &[],
        &platform,
    )?;
    let props = instance.properties()?;
    info!("runtime OpenXR: {} {}", props.runtime_name, props.runtime_version);

    let system = instance.system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)?;
    let blend_mode = instance.enumerate_environment_blend_modes(system, VIEW_TYPE)?[0];
    info!("blend mode: {blend_mode:?}");

    let vk = unsafe { VkContext::new(&instance, system)? };

    let (session, mut frame_wait, mut frame_stream) = unsafe {
        instance.create_session::<xr::Vulkan>(
            system,
            &xr::vulkan::SessionCreateInfo {
                instance: vk.instance.handle().as_raw() as _,
                physical_device: vk.physical_device.as_raw() as _,
                device: vk.device.handle().as_raw() as _,
                queue_family_index: vk.queue_family,
                queue_index: 0,
            },
        )?
    };
    let space = session.create_reference_space(xr::ReferenceSpaceType::LOCAL, xr::Posef::IDENTITY)?;

    // Passthrough: the layer starts active, so there is no need to call `start`
    // (in openxr 0.22 `Passthrough::start` calls `pause` by mistake).
    let passthrough = if has_passthrough {
        let pt = session.create_passthrough(xr::PassthroughFlagsFB::IS_RUNNING_AT_CREATION)?;
        let layer = session.create_passthrough_layer(
            &pt,
            xr::PassthroughFlagsFB::IS_RUNNING_AT_CREATION,
            xr::PassthroughLayerPurposeFB::RECONSTRUCTION,
        )?;
        info!("passthrough activo");
        Some((pt, layer))
    } else {
        info!("passthrough not available");
        None
    };

    let mut swapchain: Option<vk_ctx::XrSwapchain> = None;
    let mut orb_renderer = unsafe { OrbRenderer::new(&vk)? };
    // scene_orbs[0] is the central orb; the rest, one per browser entry.
    let mut scene_orbs: Vec<OrbInstance> = Vec::new();
    let mut orb_key = String::new();
    // Entry each orb of scene_orbs[1..] represents.
    let mut orb_entry: Vec<usize> = Vec::new();
    // Swipe with a pinch in the air: starting lateral position and steps already applied.
    let mut air: [Option<AirGesture>; 2] = [None, None];
    // The editor starts hidden: it appears when pulling down in the air (or when typing) and it
    // moves by grabbing it. Position and orientation in the world.
    let mut editor_panel: Option<(Vec3, Quat)> = None;
    let mut panel_grab: Option<(usize, Vec3)> = None;
    // Scroll: the editor unrolls when opening and rolls up when closing. `panel_open` is what
    // is requested; `panel_p` (0..1) how unrolled it is now.
    let mut panel_open = false;
    let mut panel_p = 0.0f32;
    let mut summon = false;
    // The scene is anchored to the head the first time the headset is worn.
    let mut anchor: Option<Anchor> = None;
    let mut hand_input = HandInput::new(&session, has_hands)?;
    info!("hand tracking: {}", if hand_input.is_some() { "available" } else { "not available" });
    let mut drag = DragSystem::default();
    // The hand as a mesh from Meta's own runtime, if it offers it.
    let hand_meshes = hand_input.as_ref().and_then(|h| match h.meshes(&instance) {
        Ok(m) => Some(m),
        Err(e) => {
            info!("hand mesh not available: {e}");
            None
        }
    });
    if let Some(m) = &hand_meshes {
        unsafe { orb_renderer.set_hand_meshes(&vk, m)? };
        info!("hand mesh: {} and {} vertices", m[0].vertex_count(), m[1].vertex_count());
    }
    // Frames with both hands pinching in the air (the gesture to re-place the scene).
    let mut recenter_hold = 0u32;
    // Desk mapped with the resting palm, and each hand's calibration.
    let mut desk: Option<Desk> = None;
    // Current environment (0 = passthrough). It changes by crushing the environment orb against the desk.
    let mut environment = 0u32;
    // Walls mapped with the palm against them (up to 4) and when they were mapped.
    let mut walls: Vec<(Wall, std::time::Instant)> = Vec::new();
    let mut wall_calibrators = [WallCalibrator::default(); 2];
    // Throwing the environment orb: each hand's speed and the orb in flight.
    let mut velocity = [VelocityTracker::default(), VelocityTracker::default()];
    let mut env_body: Option<Body> = None;
    let mut env_flight = 0.0f32;
    // The orb released without throwing it floats back to its place.
    let mut env_returning = false;
    // Seconds the orb released by the desk waits there before going back (so it can be crushed with the palm).
    let mut env_hold = 0.0f32;
    // How much of its real size each orb is drawn at: when grabbed it shrinks to fit in the hand.
    let mut orb_fit: Vec<f32> = Vec::new();
    // Height of each hand's palm in the previous frame, to know whether it is going down.
    let mut prev_palm_y = [0.0f32; 2];
    let mut env_rest = 0.0f32;
    let mut last_tick = std::time::Instant::now();
    let mut desk_mapped_at: Option<std::time::Instant> = None;
    let mut desk_calibrators = [DeskCalibrator::default(); 2];
    // Workspace: the app's external folder, to which code can be copied
    // from the PC with `adb push` (without storage permissions).
    let root = app
        .external_data_path()
        .or_else(|| app.internal_data_path())
        .ok_or("no storage for the app")?;
    setup_credentials(app);
    atlas_git::install_transport();
    let workspace = Workspace::new(root);
    info!("workspace: {}", workspace.root().display());
    let mut editor = match workspace.read(SCRATCH_FILE) {
        Ok(Some(text)) => Editor::new(&text),
        _ => Editor::new(WELCOME),
    };
    editor.file = SCRATCH_FILE.to_string();

    // Customization: `init.lua` at the root (a commented template is created the first time).
    let mut script = Script::new();
    let mut script_request: Option<bool> = None; // Some(true) = :run, Some(false) = :reload
    let mut script_time = 0.0f32;
    let mut snapshot_dirty = true;
    match workspace.read(INIT_FILE) {
        Ok(Some(text)) => {
            let errors = script.set_init(Some(text));
            report_script(&mut editor, &mut script, &errors, "init.lua cargado");
        }
        Ok(None) => {
            let _ = workspace.write(INIT_FILE, INIT_TEMPLATE);
        }
        Err(e) => editor.log.push(format!("ERROR init.lua: {e}")),
    }

    // Each panel is drawn on the CPU on its own thread (~30 ms on the Quest) so that
    // the render loop never waits and no frames are lost while typing.
    let mut code_panel: PanelSlot<(Editor, f32)> = PanelSlot::spawn("panel", PANEL_W, PANEL_H, {
        let mut painter = CodePanel::new(FONT_BASE_PX);
        move |(snapshot, font_px): (Editor, f32)| {
            let t = std::time::Instant::now();
            painter.set_px(font_px);
            let canvas = painter.render(&snapshot, PANEL_W, PANEL_H);
            info!("panel: render {:.1} ms (separate thread)", t.elapsed().as_secs_f32() * 1000.0);
            canvas
        }
    })?;
    let mut log_panel: PanelSlot<Log> = PanelSlot::spawn("salida", LOG_W, LOG_H, {
        let mut painter = CodePanel::new(24.0);
        move |log: Log| painter.render_log(&log, LOG_W, LOG_H)
    })?;
    let mut cards_panel: PanelSlot<CardsView> = PanelSlot::spawn("fichas", CARDS_W, CARDS_H, {
        let mut painter = CardsPanel::new();
        move |view: CardsView| painter.render(&view, CARDS_W, CARDS_H)
    })?;
    let mut kb_panel: PanelSlot<KeyboardView> = PanelSlot::spawn("keyboard", KB_W, KB_H, {
        let mut painter = KeyboardPanel::new();
        move |view: KeyboardView| painter.render(&view, KB_W, KB_H)
    })?;
    // Floating alert (e.g. the Atlas Bridge code): in front of you, following your gaze lazily. `until` is
    // when it leaves by itself; `None` keeps it until something changes it.
    let mut alert_panel: PanelSlot<AlertView> = PanelSlot::spawn("alerta", ALERT_W, ALERT_H, {
        let mut painter = AlertPanel::new();
        move |view: AlertView| painter.render(&view, ALERT_W, ALERT_H)
    })?;
    // The laptop screen, shared by Atlas Bridge: the last image not yet drawn, when the last one arrived and
    // where it floats (it is placed on your left with the first image and stays there).
    // It is redrawn with the last image every time what is on top changes (cursor, outline, grips).
    type ScreenRequest = (Option<std::sync::Arc<ScreenFrame>>, ScreenUi);
    let mut screen_panel: PanelSlot<ScreenRequest> =
        PanelSlot::spawn("screen", SCREEN_W, SCREEN_H, |(frame, ui): ScreenRequest| screen_canvas(frame.as_deref(), &ui))?;
    let mut screen_frame: Option<std::sync::Arc<ScreenFrame>> = None;
    let mut screen_ui = ScreenUi::default();
    // Width of the screen in the headset (m): it changes by its corners or with `:screen <width>`.
    let mut screen_w = SCREEN_W_M;
    // Width of the image in canvas pixels (a screen narrower than the texture leaves margins on the sides).
    let mut screen_img: (f32, f32) = (1600.0, 1000.0);
    // The screen grabbed from afar, with a hand's ray.
    let mut screen_ray: Option<ScreenRay> = None;
    // `:screen ...` pending (it needs this frame's head).
    let mut screen_cmd: Option<String> = None;
    let mut screen_at: Option<std::time::Instant> = None;
    let mut screen_pose: Option<(Vec3, Quat)> = None;
    // Rows of the texture the image takes (a wide screen uses less than the whole texture).
    // Height / width of the image.
    let mut screen_aspect = 1000.0 / 1600.0;
    // Width last requested from the laptop (to log it when it changes).
    let mut screen_want = 0u32;
    // `:remote`: the ray drives the laptop's mouse and, after pinching its screen, the keyboard types on it. Only
    // if the laptop allows it (a box in Atlas Bridge).
    let mut remote_on = false;
    let mut remote_allowed = false;
    let mut remote_focus = false;
    let mut remote_keys: Vec<Key> = Vec::new();
    // The hand that has the laptop's mouse button pressed, and where the pointer moved last.
    let mut remote_click: Option<usize> = None;
    let mut remote_uv: Option<(f32, f32)> = None;
    // The hand that has grabbed the screen (with the closed hand, like the editor) and its offset from the center.
    let mut screen_grab: Option<(usize, Vec3)> = None;
    // The laptop is paired (`:send` needs it to be).
    let mut bridge_paired = false;
    // `:selftest drop`: the test hand and what it needs to know from the previous frame.
    let mut selftest: Option<selftest::SelfTest> = None;
    let mut st_card_pos: Option<Vec3> = None;
    let mut st_card_grabbed = false;
    let mut st_sent: Option<String> = None;
    // Where the test hand is and whether it pinches: it is drawn as a sphere of light (in a video you see what moves things).
    let mut st_marker: Option<(Vec3, bool)> = None;
    // The grabbed card that is in front of the laptop screen (releasing it sends it there).
    let mut card_over_screen: Option<usize> = None;
    let mut alert: Option<(AlertView, Option<std::time::Instant>)> = None;
    let mut alert_drawn = AlertView::default();
    let mut alert_pos: Option<Vec3> = None;
    // Atlas's own virtual keyboard: it exists while the editor is open and is touched with the index finger.
    let mut keyboard: Option<Keyboard> = None;
    let mut kb_desk = false;
    let mut kb_view = KeyboardView::default();
    let mut kb_sig = None;
    let mut kb_pressed: Option<(usize, std::time::Instant)> = None;
    let mut virtual_keys: Vec<Key> = Vec::new();
    // Pinching the keyboard moves it; where you leave it is remembered for the session.
    let mut kb_grab: Option<(usize, Vec3)> = None;
    // The corner the keyboard was grabbed by.
    let mut kb_corner: Option<usize> = None;
    // Meta's system keyboard (overlay): requested while the editor is open.
    let mut sys_keyboard = false;
    let mut sys_input: Option<sysinput::SysInput> = None;
    let mut sys_input_tried = false;
    let mut kb_home: Option<Vec3> = None;
    // Font size the panel was last drawn with (to redraw it if it changes).
    let mut last_font_scale = 1.0f32;
    // What the finger has traveled on the trackpad and has not yet turned into keys.
    let mut pad_acc = (0.0f32, 0.0f32);
    let mut pad_scroll = 0.0f32;
    // Cards for the files of the current folder, which come out of the desk drawer.
    let mut cards: Vec<FileInfo> = Vec::new();
    // Folder the cards show and its content: its own, it does not depend on the editor's mode.
    let mut cards_dir = parent_of(SCRATCH_FILE);
    let mut card_entries: Vec<Entry> = Vec::new();
    let mut cards_reload = true;
    let mut card_hover: Option<usize> = None;
    // Card file: all the entries of the folder, the chosen one, the slide when paging, the window
    // currently drawn, the card to choose on reload (when going up a folder, the folder you came from),
    // each palm's sweep and what the title card says.
    let mut dir_entries: Vec<Entry> = Vec::new();
    let mut card_selected: usize = 0;
    let mut card_shift = 0.0f32;
    let mut card_window = (usize::MAX, 0usize);
    let mut pending_select: Option<String> = None;
    let mut palm_swipes = [PalmSwipe::default(); 2];
    let mut card_folder_label = String::new();
    // Current (smoothed) position of each card.
    let mut card_smooth: Vec<Vec3> = Vec::new();
    let mut card_moved: Vec<bool> = Vec::new();
    let mut prev_drawer_open = false;
    let mut prev_panel_open = false;
    let mut drawer: Option<Drawer> = None;
    // Hand that is pulling the imaginary drawer.
    let mut pull_hand: Option<usize> = None;
    let mut env_trail: std::collections::VecDeque<Vec3> = std::collections::VecDeque::new();
    let mut log_revision = u64::MAX; // forces the first draw

    // Git runs on separate threads: a clone can take a while and must not freeze the headset.
    let (git_tx, git_rx) = mpsc::channel::<GitEvent>();
    // Bridge with the laptop (`:bridge`) and the last file received, to open the drawer right there.
    let mut bridge: Option<Bridge> = None;
    let mut inbox_fresh: Option<String> = None;
    let mut git_busy = false;
    let mut last_progress_log: Option<(String, Option<u8>)> = None;
    let mut last_repo: Option<std::path::PathBuf> = None;

    let mut events = xr::EventDataBuffer::new();
    let mut running = false;
    let mut destroyed = false;
    let mut frame = 0usize;
    let mut frame_n = 0u64;
    let mut last_frame = std::time::Instant::now();
    let (mut dt_max, mut dt_sum) = (0.0f32, 0.0f32);

    // Handling a key (physical or from the virtual keyboard) is the same: it goes through the modal editor.
    macro_rules! process_key {
        ($quit:ident, $key:expr) => {{
        summon = true;
        let action = editor.handle_key($key);
        $quit |= perform(&mut editor, &action, &workspace);
        snapshot_dirty = true;
        // `:bridge` starts listening for the laptop (or reminds the code if it is already listening).
        if action == Action::Bridge {
            if bridge.is_none() {
                match start_bridge(workspace.root().join("inbox")) {
                    Ok(b) => {
                        info!("bridge: listening on {}:{}", b.address, atlas_link::PORT);
                        bridge = Some(b);
                    }
                    Err(e) => {
                        editor.message = e.clone();
                        alert = Some((AlertView { title: "Atlas Bridge".into(), big: "Error".into(), body: e }, Some(std::time::Instant::now() + Duration::from_secs(6))));
                    }
                }
            }
            if let Some(b) = &bridge {
                editor.message = format!("bridge: on your laptop, open Atlas Bridge and enter {} and code {}", b.address, b.code);
                // The code, in an alert that stays until the laptop connects.
                alert = Some((bridge_waiting(b, "waiting..."), None));
            }
        }
        // `:send` sends the open file to the laptop (as it is in the editor, even unsaved) or the named one.
        if let Action::Send(arg) = &action {
            let name = if arg.is_empty() { editor.file.clone() } else { arg.clone() };
            let data = if name.is_empty() {
                None
            } else if name == editor.file {
                Some(editor.buffer.to_string().into_bytes())
            } else {
                workspace.resolve(&name).ok().and_then(|p| std::fs::read(p).ok())
            };
            editor.message = send_to_laptop(bridge.as_ref().filter(|_| bridge_paired), &name, data).unwrap_or_else(|e| e);
        }
        if let Action::Remote(arg) = &action {
            let want = match arg.trim() {
                "" => Some(!remote_on),
                "on" => Some(true),
                "off" => Some(false),
                _ => None,
            };
            editor.message = match want {
                None => "remote: use :remote, :remote on or :remote off".into(),
                Some(true) if !(bridge_paired && remote_allowed) => {
                    "remote: on Atlas Bridge, tick \"Let the headset control this laptop\" first".into()
                }
                Some(true) if screen_at.is_none() => "remote: share the laptop screen first (Atlas Bridge > Share screen)".into(),
                Some(on) => {
                    remote_on = on;
                    if !on {
                        remote_focus = false;
                    }
                    info!("laptop control: {}", if on { "on" } else { "off" });
                    if on {
                        "remote: on. Point at the laptop screen and pinch to click; then the keyboard types there (pinch the editor to type here again)".into()
                    } else {
                        "remote: off".into()
                    }
                }
            };
        }
        // `:caption`: a caption for demos and videos (the large text, and after `|` a small one).
        if let Action::Caption(arg) = &action {
            let (big, body) = arg.split_once('|').map_or((arg.trim(), ""), |(a, b)| (a.trim(), b.trim()));
            alert = Some((
                AlertView { title: "Atlas Code".into(), big: big.to_string(), body: body.to_string() },
                Some(std::time::Instant::now() + Duration::from_secs(7)),
            ));
            editor.message.clear();
        }
        if let Action::Screen(arg) = &action {
            screen_cmd = Some(arg.clone());
        }
        if let Action::SelfTest(arg) = &action {
            if arg == "drop" {
                selftest = Some(selftest::SelfTest::drop_on_screen());
                st_sent = None;
                editor.message = "selftest: a test hand opens the drawer and drops the card on the laptop screen".into();
            } else if arg == "screen" {
                selftest = Some(selftest::SelfTest::far_screen());
                editor.message = "selftest: a test hand pulls the laptop screen closer from afar, then stretches a corner".into();
            } else if arg == "remote" {
                selftest = Some(selftest::SelfTest::remote());
                editor.message = "selftest: a test hand clicks the laptop screen, waits for typing, then pinches the editor".into();
            } else if arg == "keyboard" {
                selftest = Some(selftest::SelfTest::keyboard());
                editor.message = "selftest: a test hand pinches the middle of the keyboard, then a corner".into();
            } else {
                editor.message = "selftest: use :selftest drop, :selftest keyboard, :selftest screen or :selftest remote".into();
            }
        }
        // `:font`, `:panel` and `:distance` change a reading setting right away (not saved: it goes in init.lua).
        if let Action::Set { name, value } = &action {
            let key = match name.as_str() {
                "font" => "font_scale",
                "panel" => "panel_width",
                _ => "panel_distance",
            };
            let relative = value.starts_with('+') || value.starts_with('-');
            editor.message = match value.parse::<f32>() {
                Ok(v) => match script.adjust_config(key, v, relative) {
                    Ok(now) => format!("{key} = {now:.2} (add `config.{key} = {now:.2}` to init.lua to keep it)"),
                    Err(e) => e,
                },
                Err(_) => format!("E474: {value} is not a number"),
            };
        }
        match action {
            Action::Run => script_request = Some(true),
            Action::Reload => script_request = Some(false),
            _ => {}
        }
        if let Action::Git(request) = &action {
            if git_busy {
                editor.message = "git: busy, wait for the previous operation to finish".into();
            } else {
                git_busy = true;
                editor.message = "git: working…".into();
                editor.log.set_visible(true);
                editor.log.push(format!("> {}", describe(request)));
                let (tx, ws) = (git_tx.clone(), workspace.clone());
                let (request, file, repo) = (request.clone(), editor.file.clone(), last_repo.clone());
                std::thread::spawn(move || {
                    let report = |p: Progress| {
                        let _ = tx.send(GitEvent::Progress(p));
                    };
                    let outcome = atlas_git::execute(&request, &ws, &file, repo.as_deref(), &report);
                    let _ = tx.send(GitEvent::Done(outcome));
                });
            }
        }
        if action != Action::None {
            code_panel.mark_dirty();
        }
        }};
    }

    'main: loop {
        app.poll_events(Some(Duration::ZERO), |ev| {
            if let PollEvent::Main(MainEvent::Destroy) = ev {
                destroyed = true;
            }
        });
        // Keyboard: each press (and repeat) goes through the modal editor.
        let mut quit = false;
        if let Ok(mut events) = app.input_events_iter() {
            while events.next(|ev| {
                let InputEvent::KeyEvent(k) = ev else { return InputStatus::Unhandled };
                if k.action() != KeyAction::Down {
                    return InputStatus::Unhandled;
                }
                let mapped = input::map_key(app, k);
                // Not logged at Info level: what is typed can be a token or a password.
                log::debug!("tecla {:?} dispositivo {} -> {:?}", k.key_code(), k.device_id(), mapped);
                let Some(key) = mapped else { return InputStatus::Unhandled };
                if remote_focus {
                    remote_keys.push(key);
                } else {
                    process_key!(quit, key);
                }
                InputStatus::Handled
            }) {}
        }
        for key in std::mem::take(&mut virtual_keys) {
            if remote_focus {
                remote_keys.push(key);
            } else {
                process_key!(quit, key);
            }
        }
        // With the focus on the laptop, what is typed (own or physical keyboard) is written there.
        if !remote_keys.is_empty() {
            let keys = std::mem::take(&mut remote_keys);
            if let Some(b) = bridge.as_ref().filter(|_| remote_on && remote_allowed) {
                for input in keys_to_remote(&keys) {
                    let _ = b.to_laptop.input.send(input);
                }
            }
        }
        if destroyed || quit {
            let _ = session.request_exit();
        }
        // `:run` runs the open file as a script; `:reload` reloads `init.lua` and the script.
        if let Some(is_run) = script_request.take() {
            let errors = if is_run {
                script.run_script(&editor.buffer.to_string())
            } else {
                match workspace.read(INIT_FILE) {
                    Ok(text) => script.set_init(text),
                    Err(e) => vec![ScriptError { chunk: "init".into(), line: None, message: e }],
                }
            };
            report_script(&mut editor, &mut script, &errors, if is_run { "script executed" } else { "init.lua reloaded" });
            code_panel.mark_dirty();
        }
        if snapshot_dirty && script.is_running() {
            script.set_snapshot(snapshot_of(&editor));
            snapshot_dirty = false;
        }

        while let Some(event) = instance.poll_event(&mut events)? {
            use xr::Event::*;
            match event {
                SessionStateChanged(e) => {
                    info!("session: {:?}", e.state());
                    match e.state() {
                        xr::SessionState::READY => {
                            session.begin(VIEW_TYPE)?;
                            running = true;
                        }
                        xr::SessionState::STOPPING => {
                            session.end()?;
                            running = false;
                        }
                        xr::SessionState::EXITING | xr::SessionState::LOSS_PENDING => break 'main,
                        _ => {}
                    }
                }
                InstanceLossPending(_) => break 'main,
                // Holding the Meta button re-places the origin: the scene is anchored again.
                ReferenceSpaceChangePending(_) => {
                    anchor = None;
                    editor_panel = None;
                    panel_open = false;
                    panel_p = 0.0;
                    panel_grab = None;
                    desk = None;
                    keyboard = None;
                    desk_mapped_at = None;
                    drawer = None;
                    walls.clear();
                    orb_key.clear();
                }
                _ => {}
            }
        }

        if !running {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }

        let state = frame_wait.wait()?;
        let dt = last_frame.elapsed().as_secs_f32() * 1000.0;
        last_frame = std::time::Instant::now();
        dt_sum += dt;
        dt_max = dt_max.max(dt);
        if frame_n % 300 == 299 {
            info!("frames: average {:.1} ms, worst {:.1} ms (last 300)", dt_sum / 300.0, dt_max);
            dt_sum = 0.0;
            dt_max = 0.0;
        }
        frame_stream.begin()?;
        if !state.should_render {
            frame_stream.end(state.predicted_display_time, blend_mode, &[])?;
            continue;
        }

        let sc = match &mut swapchain {
            Some(s) => s,
            None => {
                let views = instance.enumerate_view_configuration_views(system, VIEW_TYPE)?;
                let res = vk::Extent2D {
                    width: views[0].recommended_image_rect_width,
                    height: views[0].recommended_image_rect_height,
                };
                info!("resolution per eye: {}x{}", res.width, res.height);
                let handle = session.create_swapchain(&xr::SwapchainCreateInfo {
                    create_flags: xr::SwapchainCreateFlags::EMPTY,
                    usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT
                        | xr::SwapchainUsageFlags::SAMPLED,
                    format: COLOR_FORMAT.as_raw() as _,
                    sample_count: 1,
                    width: res.width,
                    height: res.height,
                    face_count: 1,
                    array_size: VIEW_COUNT,
                    mip_count: 1,
                })?;
                swapchain.insert(unsafe { vk.make_swapchain(handle, res)? })
            }
        };

        // What the git threads report: progress and final result.
        while let Ok(event) = git_rx.try_recv() {
            match event {
                GitEvent::Progress(p) => {
                    // A log entry every 10 % (or on a phase change) to be able to follow it with logcat.
                    let step = (p.label.clone(), p.percent().map(|x| x / 10));
                    if last_progress_log.as_ref() != Some(&step) {
                        info!("git progreso: {} {:?}% ({}/{:?})", p.label, p.percent(), p.done, p.total);
                        last_progress_log = Some(step);
                    }
                    editor.log.set_progress(Some(p));
                }
                GitEvent::Done(outcome) => {
                    git_busy = false;
                    editor.log.set_progress(None);
                    if outcome.repo.is_some() {
                        last_repo = outcome.repo.clone();
                    }
                    apply_git_outcome(&mut editor, &workspace, outcome);
                    code_panel.mark_dirty();
                }
            }
        }
        // What the bridge with the laptop reports.
        let mut bridge_done = false;
        if let Some(b) = &bridge {
            while let Ok(event) = b.events.try_recv() {
                use atlas_link::ServerEvent;
                match event {
                    BridgeEvent::Server(ServerEvent::Paired) => {
                        info!("bridge: laptop connected");
                        bridge_paired = true;
                        editor.message = "bridge: laptop connected. Drop files on Atlas Bridge to send them here".into();
                        alert = Some((
                            AlertView { title: "Atlas Bridge".into(), big: "Connected".into(), body: "Drop files on the laptop window".into() },
                            Some(std::time::Instant::now() + Duration::from_secs(4)),
                        ));
                    }
                    BridgeEvent::Server(ServerEvent::WrongCode { left }) => {
                        editor.message = format!("bridge: wrong code ({left} tries left)");
                        if left > 0 {
                            alert = Some((bridge_waiting(b, &format!("wrong code, {left} tries left")), None));
                        }
                    }
                    BridgeEvent::Server(ServerEvent::Saved(name)) => {
                        info!("bridge: received inbox/{name}");
                        editor.log.push(format!("bridge: received inbox/{name}"));
                        editor.message = format!("received inbox/{name}: pull the drawer to see it");
                        if cards_dir == "inbox" {
                            pending_select = Some(name.clone());
                            cards_reload = true;
                        }
                        alert = Some((
                            AlertView { title: "Received".into(), big: name.clone(), body: "Pull the drawer to see it in inbox/".into() },
                            Some(std::time::Instant::now() + Duration::from_secs(5)),
                        ));
                        inbox_fresh = Some(name);
                    }
                    BridgeEvent::Server(ServerEvent::Rejected(reason)) => {
                        editor.log.push(format!("bridge: {reason}"));
                        editor.message = format!("bridge: {reason}");
                        alert = Some((
                            AlertView { title: "Atlas Bridge".into(), big: "Not saved".into(), body: reason },
                            Some(std::time::Instant::now() + Duration::from_secs(6)),
                        ));
                    }
                    BridgeEvent::Server(ServerEvent::Delivered(name)) => {
                        info!("bridge: delivered to the laptop {name}");
                        editor.log.push(format!("bridge: sent {name} to the laptop"));
                        editor.message = format!("sent {name}: it is in Downloads/Atlas on your laptop");
                        alert = Some((
                            AlertView { title: "Sent to laptop".into(), big: name, body: "In Downloads/Atlas on your laptop".into() },
                            Some(std::time::Instant::now() + Duration::from_secs(4)),
                        ));
                    }
                    BridgeEvent::Server(ServerEvent::NotDelivered(reason)) => {
                        editor.message = format!("send: the laptop refused it: {reason}");
                        alert = Some((
                            AlertView { title: "Atlas Bridge".into(), big: "Not sent".into(), body: reason },
                            Some(std::time::Instant::now() + Duration::from_secs(6)),
                        ));
                    }
                    BridgeEvent::Server(ServerEvent::Control(allowed)) => {
                        info!("bridge: the laptop {} control", if allowed { "allows" } else { "no longer allows" });
                        remote_allowed = allowed;
                        if allowed {
                            editor.message = "bridge: the laptop lets you control it: type :remote".into();
                        } else if remote_on {
                            remote_on = false;
                            remote_focus = false;
                            editor.message = "remote: off (the laptop no longer allows it)".into();
                        }
                    }
                    // Screen images do not go through here: the bridge thread leaves them in `Bridge::screen`.
                    BridgeEvent::Server(ServerEvent::Screen(_)) => {}
                    BridgeEvent::Done(result) => {
                        bridge_done = true;
                        let (big, body) = match &result {
                            Ok(()) => ("Disconnected".to_string(), "Type :bridge to connect again".to_string()),
                            Err(e) => ("Closed".to_string(), e.to_string()),
                        };
                        editor.message = format!("bridge: {body}");
                        // If a file just arrived, its notice stays (the laptop usually disconnects right after).
                        match alert.as_mut().filter(|(view, _)| view.title == "Received" && result.is_ok()) {
                            Some((view, until)) => {
                                view.body = "Pull the drawer to see it in inbox/ · laptop disconnected".into();
                                *until = Some(std::time::Instant::now() + Duration::from_secs(5));
                            }
                            None => {
                                alert = Some((AlertView { title: "Atlas Bridge".into(), big, body }, Some(std::time::Instant::now() + Duration::from_secs(5))));
                            }
                        }
                    }
                }
                code_panel.mark_dirty();
            }
        }
        if let Some(frame) = bridge.as_ref().and_then(|b| b.screen.lock().ok().and_then(|mut slot| slot.take())) {
            if screen_at.is_none() {
                alert = Some((
                    AlertView { title: "Atlas Bridge".into(), big: "Screen shared".into(), body: "Your laptop screen is on your left".into() },
                    Some(std::time::Instant::now() + Duration::from_secs(4)),
                ));
            }
            screen_img = (frame.width.clamp(1, SCREEN_W) as f32, frame.height.clamp(1, SCREEN_H) as f32);
            screen_aspect = screen_img.1 / screen_img.0;
            screen_frame = Some(std::sync::Arc::new(frame));
            screen_at = Some(std::time::Instant::now());
            screen_panel.mark_dirty();
        }
        if bridge_done {
            bridge = None;
            bridge_paired = false;
            remote_on = false;
            remote_allowed = false;
            remote_focus = false;
            remote_click = None;
            screen_frame = None;
            screen_at = None;
            screen_grab = None;
            screen_ray = None;
        }
        if editor.log.revision() != log_revision {
            log_revision = editor.log.revision();
            log_panel.mark_dirty();
        }
        let font_scale = script.config().font_scale;
        if (font_scale - last_font_scale).abs() > 1e-4 {
            last_font_scale = font_scale;
            code_panel.mark_dirty();
        }
        code_panel.pump(&session, &vk, || (editor.clone(), FONT_BASE_PX * font_scale))?;
        log_panel.pump(&session, &vk, || editor.log.clone())?;
        cards_panel.pump(&session, &vk, || CardsView { cards: cards.clone(), highlighted: card_hover, folder: card_folder_label.clone() })?;
        kb_panel.pump(&session, &vk, || kb_view.clone())?;
        if alert.as_ref().is_some_and(|(_, until)| until.is_some_and(|t| std::time::Instant::now() >= t)) {
            alert = None;
        }
        let alert_view = alert.as_ref().map(|(view, _)| view.clone()).unwrap_or_default();
        if alert_view != alert_drawn {
            alert_drawn = alert_view.clone();
            alert_panel.mark_dirty();
        }
        alert_panel.pump(&session, &vk, || alert_view.clone())?;
        screen_panel.pump(&session, &vk, || (screen_frame.clone(), screen_ui))?;

        // The views are needed BEFORE recording: the orbs' cameras come from them.
        let (view_flags, views) =
            session.locate_views(VIEW_TYPE, state.predicted_display_time, &space)?;
        // Without the headset on, tracking returns null poses, which OpenXR rejects.
        let views_ok = view_flags.contains(xr::ViewStateFlags::ORIENTATION_VALID)
            && view_flags.contains(xr::ViewStateFlags::POSITION_VALID);

        if anchor.is_none() && views_ok {
            let (p, q) = (views[0].pose.position, views[0].pose.orientation);
            let a = Anchor::from_head(Vec3::new(p.x, p.y, p.z), Quat::from_xyzw(q.x, q.y, q.z, q.w));
            info!("scene anchored to the head: ({:.2}, {:.2}, {:.2}) heading {:.0}°", a.origin.x, a.origin.y, a.origin.z, a.yaw.to_degrees());
            editor.log.push("rest your open palm flat and still on your desk to map it".to_string());
            editor.log.push("to see your files: pinch in the air and pull toward you, like a drawer".to_string());
            editor.log.push("to throw the environment orb, press your palm against a wall (optional)".to_string());
            anchor = Some(a);
        }
        let anchor_now = anchor.unwrap_or(Anchor::IDENTITY);
        let head = if views_ok {
            Vec3::new(views[0].pose.position.x, views[0].pose.position.y, views[0].pose.position.z)
        } else {
            anchor_now.origin
        };
        if summon && !panel_open {
            let pos = anchor_now.point(Vec3::new(0.0, 0.0, -script.config().panel_distance));
            editor_panel = Some((pos, face(pos, head)));
            panel_open = true;
        }
        summon = false;

        // Orbs are only for changing the environment or the platform: one, for now.
        let key = format!("environment{environment}");
        if anchor.is_some() && key != orb_key {
            scene_orbs.clear();
            orb_entry.clear();
            env_body = None;
            // Environment orb, at the top right.
            scene_orbs.push(env_orb(anchor_now.point(Vec3::new(0.30, 0.04, -0.42)), environment));
            orb_key = key;
        }

        let tick = std::time::Instant::now();
        let frame_dt = tick.duration_since(last_tick).as_secs_f32().clamp(0.001, 0.1);
        last_tick = tick;

        // Scripts: on_frame, what they print and the orders they request.
        script_time += frame_dt;
        if let Err(e) = script.frame(script_time, frame_dt) {
            editor.log.push(format!("ERROR on_frame: {e}"));
            editor.log.set_visible(true);
        }
        for line in script.take_output() {
            editor.log.push(line);
        }
        for command in script.take_commands() {
            match command {
                ScriptCommand::Message(text) => editor.message = text,
                ScriptCommand::Ex(text) => run_ex(&mut editor, &workspace, &text),
            }
            code_panel.mark_dirty();
        }
        let cfg = script.config();

        // Hands: pinching an orb grabs it and carries it with the hand.
        let mut hand_frames: [HandFrame; 2] = match (&mut hand_input, views_ok) {
            (Some(input), true) => input.update(&space, state.predicted_display_time),
            _ => [HandFrame::LOST_FRAME; 2],
        };
        // `:selftest`: while it lasts, the right hand is the test hand (everything else works the same).
        if let Some(st) = selftest.as_mut() {
            let forward = if views_ok {
                let q = views[0].pose.orientation;
                let f = Quat::from_xyzw(q.x, q.y, q.z, q.w) * Vec3::NEG_Z;
                Vec3::new(f.x, 0.0, f.z).try_normalize().unwrap_or(Vec3::NEG_Z)
            } else {
                Vec3::NEG_Z
            };
            let screen_center = screen_pose.filter(|_| screen_at.is_some_and(|t| t.elapsed() < Duration::from_secs(4))).map(|(p, r)| {
                let _ = r;
                p
            });
            let world = selftest::World {
                head,
                forward,
                desk_y: desk.map_or(head.y - 0.40, |d| d.height),
                drawer: drawer.as_ref().map(|d| (d.progress(), d.is_held())),
                card: st_card_pos,
                card_grabbed: st_card_grabbed,
                screen: screen_center,
                sent: st_sent.clone(),
                far_screen: screen_pose.filter(|_| screen_at.is_some_and(|t| t.elapsed() < Duration::from_secs(4))).map(|(p, r)| {
                    let (center, height) = visible_rect(p, r, screen_w, screen_aspect);
                    selftest::ScreenState {
                        center,
                        rot: r,
                        width: screen_w,
                        image_half_w: screen_w / 2.0,
                        height,
                        mode: match screen_ray {
                            None => 0,
                            Some(ScreenRay::Move { .. }) => 1,
                            Some(ScreenRay::Resize { .. }) => 2,
                        },
                    }
                }),
                remote: (remote_on, remote_allowed, remote_focus),
                editor: editor_panel.filter(|_| panel_open).map(|(p, _)| p),
                keyboard: keyboard.as_ref().map(|kb| {
                    let (right, up, normal) = kb.axes();
                    let (x, y) = kb.plate_corners()[0];
                    selftest::KeyboardState { center: kb.center, normal, corner: kb.center + right * x + up * y, grabbed: kb_corner.is_some() }
                }),
            };
            match st.step(&world, frame_dt) {
                Some(hand) => {
                    st_marker = Some((hand.pinch_point, hand.pinching));
                    hand_frames[1] = hand;
                }
                None => {
                    st_marker = None;
                    editor.message = match &st.result {
                        Some(Ok(m)) => format!("selftest OK: {m}"),
                        Some(Err(e)) => format!("selftest FAILED: {e}"),
                        None => "selftest: stopped".into(),
                    };
                    code_panel.mark_dirty();
                    selftest = None;
                }
            }
        }
        for (hand, f) in hand_frames.iter().enumerate() {
            if !f.tracked || f.pinch == PinchEvent::Started || f.grasp == PinchEvent::Started {
                velocity[hand].clear();
            }
            // In the frame it is released it is not cleared: it is needed to throw the orb.
            if f.tracked && (f.grasping || f.grasp == PinchEvent::Ended) {
                velocity[hand].push(f.grasp_point, frame_dt);
            } else if f.tracked && (f.pinching || f.pinch == PinchEvent::Ended) {
                velocity[hand].push(f.pinch_point, frame_dt);
            }
        }
        // The laptop screen: with the first image it is placed next to the editor, on its left and at the same height;
        // with no editor open, 1 m away and about 35° to your left. After that it stays where you leave it (also if you
        // reconnect), until you move it with your hand.
        if screen_at.is_some() && screen_pose.is_none() && views_ok {
            let lift = Vec3::ZERO;
            let pos = match editor_panel.filter(|_| panel_open) {
                Some((p, r)) => p + r * Vec3::NEG_X * (cfg.panel_width / 2.0 + screen_w / 2.0 + 0.06) + lift,
                None => {
                    let q = views[0].pose.orientation;
                    let gaze = Quat::from_xyzw(q.x, q.y, q.z, q.w) * Vec3::NEG_Z;
                    let flat = Vec3::new(gaze.x, 0.0, gaze.z).try_normalize().unwrap_or(Vec3::NEG_Z);
                    head + Quat::from_rotation_y(0.61) * flat * 1.0 + Vec3::Y * 0.05 + lift
                }
            };
            screen_pose = Some((pos, face(pos, head)));
            match editor_panel.filter(|_| panel_open) {
                Some((p, _)) => info!("screen placed next to the editor: center {pos:.2}, editor {p:.2}, head {head:.2}"),
                None => info!("screen placed on your left: center {pos:.2}, head {head:.2}"),
            }
        }
        if let Some(arg) = screen_cmd.take() {
            let arg = arg.trim();
            editor.message = match (screen_pose, arg.parse::<f32>()) {
                (None, _) => "screen: share the laptop screen first (Atlas Bridge > Share screen)".into(),
                (Some(_), _) if (arg.is_empty() || arg == "here") && views_ok => {
                    // In front of where you look, 70 cm away, a little below the eyes and at a comfortable size.
                    let q = views[0].pose.orientation;
                    let gaze = Quat::from_xyzw(q.x, q.y, q.z, q.w) * Vec3::NEG_Z;
                    let flat = Vec3::new(gaze.x, 0.0, gaze.z).try_normalize().unwrap_or(Vec3::NEG_Z);
                    screen_w = 0.8;
                    let center = head + flat * 0.70 - Vec3::Y * 0.05;
                    screen_pose = Some(screen_pose_for(center, face(center, head), screen_w, screen_aspect));
                    screen_ray = None;
                    screen_grab = None;
                    info!("screen brought here: {:.2} m from the eyes, {:.2} m wide", center.distance(head), screen_w);
                    "screen: here (pinch it from afar to move it, or pinch a corner to resize it)".into()
                }
                (Some((p, r)), Ok(w)) if w.is_finite() => {
                    let (center, _) = visible_rect(p, r, screen_w, screen_aspect);
                    screen_w = w.clamp(SCREEN_MIN_W, SCREEN_MAX_W);
                    screen_pose = Some(screen_pose_for(center, r, screen_w, screen_aspect));
                    format!("screen: {:.2} m wide", screen_w)
                }
                _ => "screen: use :screen here, or :screen <width in metres> (for example :screen 1.4)".into(),
            };
            code_panel.mark_dirty();
        }
        // The alert floats in front of you, a little below the eyes, and follows your gaze lazily.
        if alert.is_some() && views_ok {
            let q = views[0].pose.orientation;
            let gaze = Quat::from_xyzw(q.x, q.y, q.z, q.w) * Vec3::NEG_Z;
            let flat = Vec3::new(gaze.x, 0.0, gaze.z).try_normalize().unwrap_or(Vec3::NEG_Z);
            let target = head + flat * 0.65 - Vec3::Y * 0.10;
            alert_pos = Some(match alert_pos {
                Some(p) if p.distance(target) < 1.5 => p.lerp(target, 1.0 - (-2.5 * frame_dt).exp()),
                _ => target,
            });
        } else if alert.is_none() {
            alert_pos = None;
        }
        // Virtual keyboard: it appears on the desk with the editor open and is touched with the index finger.
        let kb_wanted = panel_open && panel_p > 0.9;
        // The keyboard is chosen with `config.keyboard`: Atlas's own (the default) or Meta's system one, which
        // is requested through a Java helper. If the system one cannot be prepared, the own one is used.
        let want_system = cfg.keyboard == KeyboardKind::System;
        if kb_wanted && want_system && !sys_input_tried {
            sys_input_tried = true;
            match sysinput::SysInput::new(app.activity_as_ptr()) {
                Ok(si) => sys_input = Some(si),
                Err(e) => error!("system keyboard: could not prepare it: {e}"),
            }
        }
        let sys_wanted = kb_wanted && want_system;
        if sys_wanted != sys_keyboard {
            sys_keyboard = sys_wanted;
            if let Some(si) = &sys_input {
                let result = if sys_wanted { si.show(app.activity_as_ptr()) } else { si.hide(app.activity_as_ptr()) };
                match result {
                    Ok(()) => info!("system keyboard: {}", if sys_wanted { "requested" } else { "hidden" }),
                    Err(e) => error!("{e}"),
                }
            }
        }
        if let (Some(si), true) = (&sys_input, frame_n % 4 == 0) {
            match si.poll() {
                Ok(text) => virtual_keys.extend(text.chars().map(|c| match c {
                    '\u{8}' => Key::Backspace,
                    '\n' => Key::Enter,
                    c => Key::Char(c),
                })),
                Err(e) => error!("{e}"),
            }
        }
        // Atlas's own keyboard is shown unless the system one is chosen and ready.
        if !(kb_wanted && (!want_system || sys_input.is_none())) {
            keyboard = None;
            kb_grab = None;
            kb_corner = None;
        } else if keyboard.is_none() || kb_desk != desk.is_some() {
            // In front of where you are looking now (not where you were at startup), resting on the desk.
            let gaze = if views_ok {
                let q = views[0].pose.orientation;
                Quat::from_xyzw(q.x, q.y, q.z, q.w) * Vec3::NEG_Z
            } else {
                anchor_now.rotation() * Vec3::NEG_Z
            };
            let flat = Vec3::new(gaze.x, 0.0, gaze.z).try_normalize().unwrap_or(Vec3::NEG_Z);
            let at = head + flat * 0.40;
            let y = desk.map_or(head.y - 0.40, |d| d.height + 0.02);
            let center = kb_home.map_or(Vec3::new(at.x, y, at.z), |home| desk.map_or(home, |d| Vec3::new(home.x, d.height + 0.02, home.z)));
            keyboard = Some(Keyboard::new(center, -flat));
            if desk.is_none() && kb_home.is_none() {
                editor.message = "pinch a corner of the keyboard and drag it onto your desk".into();
            }
            kb_desk = desk.is_some();
            kb_sig = None;
        }
        if let Some(kb) = keyboard.as_mut() {
            // Pinching a corner of the plate (the L-shaped grips) grabs the keyboard; when released it stays (stuck to the desk
            // if mapped). The rest of the plate does not move it: that is where you type.
            if let Some((h, offset)) = kb_grab {
                let f = &hand_frames[h];
                if f.tracked && f.pinching {
                    kb.center = f.pinch_point + offset;
                } else {
                    if let Some(d) = &desk {
                        kb.center.y = d.height + 0.02;
                    }
                    kb_home = Some(kb.center);
                    kb_grab = None;
                    kb_corner = None;
                    editor.message = "keyboard placed".into();
                }
            } else if let Some((h, f, corner)) = hand_frames.iter().enumerate().find_map(|(h, f)| {
                (f.tracked && f.pinch == PinchEvent::Started).then(|| kb.corner_near(f.pinch_point, GRIP_RADIUS)).flatten().map(|c| (h, f, c))
            }) {
                info!("keyboard grabbed by corner {corner}");
                kb_grab = Some((h, kb.center - f.pinch_point));
                kb_corner = Some(corner);
            }
            // The grips light up with the hand close (a hint that you pinch there) and turn white while you carry it.
            let mut grips = [0u8; 4];
            for f in hand_frames.iter().filter(|f| f.tracked) {
                if let Some(c) = kb.corner_near(f.pinch_point, GRIP_NEAR) {
                    grips[c] = grips[c].max(1);
                }
            }
            if let Some(c) = kb_corner {
                grips[c] = 2;
            }
            let mut hover = None;
            for (hand, f) in hand_frames.iter().enumerate() {
                if !f.tracked {
                    kb.pad_lost(hand);
                    continue;
                }
                // Trackpad: sliding moves the cursor, the edge strip scrolls and a tap is Enter.
                for event in kb.pad_touch(hand, f.index_tip, frame_dt) {
                    match event {
                        PadEvent::Move { dx, dy } => {
                            pad_acc.0 += dx;
                            pad_acc.1 += dy;
                            let (cols, rows) = ((pad_acc.0 / 0.008) as i32, (pad_acc.1 / 0.012) as i32);
                            pad_acc.0 -= cols as f32 * 0.008;
                            pad_acc.1 -= rows as f32 * 0.012;
                            virtual_keys.extend(std::iter::repeat_n(if cols > 0 { Key::Right } else { Key::Left }, cols.unsigned_abs().min(6) as usize));
                            virtual_keys.extend(std::iter::repeat_n(if rows > 0 { Key::Up } else { Key::Down }, rows.unsigned_abs().min(6) as usize));
                        }
                        PadEvent::Scroll { dy } => {
                            pad_scroll += dy;
                            let lines = (pad_scroll / 0.003) as i32;
                            pad_scroll -= lines as f32 * 0.003;
                            // Like on a laptop: moving the finger up brings what is below.
                            virtual_keys.extend(std::iter::repeat_n(if lines > 0 { Key::Down } else { Key::Up }, lines.unsigned_abs().min(8) as usize));
                        }
                        PadEvent::Tap => virtual_keys.push(Key::Enter),
                    }
                }
                let near = kb.hover(f.index_tip);
                hover = hover.or(near);
                let before = (kb.shift(), kb.ctrl());
                let action = kb.touch(hand, f.index_tip);
                if action.is_some() || before != (kb.shift(), kb.ctrl()) {
                    kb_pressed = near.map(|i| (i, std::time::Instant::now()));
                }
                if let Some(a) = action {
                    virtual_keys.push(key_from_virtual(a));
                }
            }
            if kb_pressed.is_some_and(|(_, t)| t.elapsed() > Duration::from_millis(150)) {
                kb_pressed = None;
            }
            let pressed = kb_pressed.map(|(i, _)| i);
            let contacts = kb.pad_contacts();
            if contacts.is_empty() {
                pad_acc = (0.0, 0.0);
                pad_scroll = 0.0;
            }
            let sig = (kb.layer(), kb.shift(), kb.ctrl(), hover, pressed, contacts.clone(), grips);
            if kb_sig.as_ref() != Some(&sig) {
                kb_sig = Some(sig);
                kb_view = KeyboardView {
                    keys: kb
                        .keys()
                        .into_iter()
                        .map(|k| KeyView { label: k.label, x: k.x, y: k.y, w: k.w, h: k.h, lit: k.lit })
                        .collect(),
                    pad: Some(PadView { x: PAD_X, y: PAD_Y, w: PAD_W, h: PAD_H, scroll_w: SCROLL_W, contacts }),
                    hover,
                    pressed,
                    grips,
                };
                kb_panel.mark_dirty();
            }
        }
        let points = [hand_frames[0].as_point(), hand_frames[1].as_point()];
        // Both hands pinching for ~1 s without grabbing anything: re-place the scene in front of you.
        if hand_frames.iter().all(|f| f.tracked && f.pinching) && drag.grabbed().iter().all(|g| g.is_none()) {
            recenter_hold += 1;
            if recenter_hold == 72 {
                info!("re-placing the scene");
                anchor = None;
                orb_key.clear();
            }
        } else {
            recenter_hold = 0;
        }
        // Desk: open palm resting still on the desk for ~1 s.
        let mut desk_progress: Option<(Vec3, f32)> = None; // (center under the hand, progress)
        if desk.is_none() && views_ok {
            let head_y = views[0].pose.position.y;
            for (hand, f) in hand_frames.iter().enumerate() {
                if !f.tracked || f.pinching {
                    desk_calibrators[hand].reset();
                    continue;
                }
                if let Some(found) = desk_calibrators[hand].update(f.palm, &f.finger_tips, head_y) {
                    info!("desk mapped: height {:.2} m ({:.0} cm below the eyes)", found.height, (head_y - found.height) * 100.0);
                    editor.message = format!("desk mapped, {:.0} cm below your eyes", (head_y - found.height) * 100.0);
                    code_panel.mark_dirty();
                    desk = Some(found);
                    desk_mapped_at = Some(std::time::Instant::now());
                    fire(&mut script, &mut editor, "desk_mapped", &[]);
                } else if desk_calibrators[hand].progress() > 0.0 {
                    let low = f.finger_tips.iter().fold(f.palm.y, |m, t| m.min(t.y)) - 0.008;
                    desk_progress = Some((Vec3::new(f.palm.x, low, f.palm.z), desk_calibrators[hand].progress()));
                }
            }
        }
        // Walls: open palm still against a wall for ~1 s.
        let mut wall_progress: Option<(Wall, f32)> = None;
        if views_ok && desk_progress.is_none() {
            for (hand, f) in hand_frames.iter().enumerate() {
                if !f.tracked || f.pinching || f.gripping {
                    wall_calibrators[hand].reset();
                    continue;
                }
                if let Some(found) = wall_calibrators[hand].update(f.palm, &f.finger_tips, head) {
                    // If there is already a wall almost in the same plane, it is replaced instead of duplicated.
                    walls.retain(|(w, _)| !(w.normal.dot(found.normal) > 0.9 && w.distance(found.point).abs() < 0.15));
                    walls.truncate(3);
                    walls.push((found, std::time::Instant::now()));
                    editor.message = format!("wall mapped ({} in total)", walls.len());
                    fire(&mut script, &mut editor, "wall_mapped", &[]);
                    info!("wall mapped: normal ({:.2}, {:.2}), {:.2} m away", found.normal.x, found.normal.z, found.distance(head));
                    code_panel.mark_dirty();
                } else if wall_calibrators[hand].progress() > 0.0 {
                    if let Some(w) = WallCalibrator::plane_of(f.palm, &f.finger_tips, head) {
                        wall_progress = Some((w, wall_calibrators[hand].progress()));
                    }
                }
            }
        }
        // Closing the fist in the air: over the editor it grabs it; otherwise, a swipe. Down opens the
        // editor, up closes it and sideways moves from one orb to another.
        if let Some((h, _)) = panel_grab {
            if !(hand_frames[h].tracked && hand_frames[h].gripping) {
                panel_grab = None;
            }
        }
        if let Some((h, _)) = screen_grab {
            if !(hand_frames[h].tracked && hand_frames[h].gripping) {
                screen_grab = None;
            }
        }
        let screen_visible = screen_at.is_some_and(|t| t.elapsed() < Duration::from_secs(4));
        let right = anchor_now.rotation() * Vec3::X;
        let forward = anchor_now.rotation() * Vec3::NEG_Z;
        for (hand, f) in hand_frames.iter().enumerate() {
            let on_orb = drag.grabbed()[hand].is_some();
            let holding_panel = matches!(panel_grab, Some((h, _)) if h == hand) || matches!(screen_grab, Some((h, _)) if h == hand);
            if !(f.tracked && f.gripping) || on_orb || holding_panel {
                air[hand] = None;
                continue;
            }
            let rel = f.palm - anchor_now.origin;
            let (lateral, height) = (rel.dot(right), rel.y);
            if f.grip == PinchEvent::Started && air[hand].is_none() {
                // The laptop screen is grabbed like the editor: with the hand closed on it.
                let vis_h = screen_w * screen_aspect;
                let on_screen = screen_pose.filter(|(p, r)| {
                    let center = *p;
                    screen_visible && hits_quad(center, *r, f.palm, screen_w, vis_h)
                });
                match editor_panel.filter(|(p, r)| panel_open && panel_p > 0.9 && hits_panel(*p, *r, f.palm, cfg.panel_width)) {
                    Some((pos, _)) => panel_grab = Some((hand, pos - f.palm)),
                    None => match on_screen {
                        Some((pos, _)) => {
                            info!("screen grabbed");
                            screen_grab = Some((hand, pos - f.palm));
                        }
                        None => air[hand] = Some(AirGesture { lateral, height, kind: None, base: lateral, applied: 0 }),
                    },
                }
                continue;
            }
            let Some(g) = air[hand].as_mut() else { continue };
            match g.kind {
                None => {
                    g.kind = classify_swipe(lateral - g.lateral, height - g.height);
                    g.base = lateral;
                    match g.kind {
                        Some(Swipe::Down) if !panel_open => {
                            // The editor appears in front of you, at hand height, and the hand carries it.
                            let pos = anchor_now.origin + forward * (cfg.panel_distance - 0.05) + right * lateral + Vec3::Y * (height + 0.1);
                            editor_panel = Some((pos, face(pos, head)));
                            panel_open = true;
                            panel_grab = Some((hand, pos - f.palm));
                            info!("editor opened");
                        }
                        Some(Swipe::Up) if panel_open => {
                            panel_open = false;
                            panel_grab = None;
                            info!("editor closed");
                        }
                        _ => {}
                    }
                }
                Some(Swipe::Horizontal) => {
                    let steps = ((lateral - g.base) / 0.05) as i32;
                    if steps != g.applied {
                        // The hand to the right brings the orbs from the left.
                        editor.browser.move_selection((g.applied - steps) as isize);
                        if let Some(e) = editor.browser.selected_entry() {
                            editor.message = e.name.clone();
                        }
                        code_panel.mark_dirty();
                        g.applied = steps;
                    }
                }
                Some(_) => {}
            }
        }
        // Scroll animation: it unrolls in ~0.8 s and rolls up in ~0.5 s.
        let dt = frame_dt;
        panel_p = if panel_open { (panel_p + dt / 0.8).min(1.0) } else { (panel_p - dt / 0.5).max(0.0) };
        if !panel_open && panel_p <= 0.0 {
            editor_panel = None;
        }
        let reveal = panel_p * panel_p * (3.0 - 2.0 * panel_p);
        if panel_open != prev_panel_open {
            prev_panel_open = panel_open;
            fire(&mut script, &mut editor, if panel_open { "editor_open" } else { "editor_close" }, &[]);
        }
        // The grabbed editor follows the hand and turns toward you.
        if let (Some((h, offset)), Some((pos, rot))) = (panel_grab, editor_panel.as_mut()) {
            if hand_frames[h].tracked {
                *pos = hand_frames[h].palm + offset;
                *rot = face(*pos, head);
            }
        }
        // The laptop screen, the same: it follows the hand and stays where you release it.
        if let (Some((h, offset)), Some((pos, rot))) = (screen_grab, screen_pose.as_mut()) {
            if hand_frames[h].tracked {
                *pos = hand_frames[h].palm + offset;
                *rot = face(*pos, head);
            }
        }
        // Card file of the current folder: all the entries are read, but only the visible window is drawn
        // (PAGE cards around the chosen one) plus the title card, which goes last.
        if cards_reload {
            dir_entries = workspace.list_dir(&cards_dir).unwrap_or_default();
            card_selected = pending_select
                .take()
                .and_then(|name| dir_entries.iter().position(|e| e.name == name))
                .unwrap_or(0);
            card_window = (usize::MAX, 0);
            card_shift = 0.0;
            cards_reload = false;
        }
        let (win_start, win_len) = window(dir_entries.len(), card_selected);
        if (win_start, win_len) != card_window {
            card_window = (win_start, win_len);
            card_entries = dir_entries[win_start..win_start + win_len].to_vec();
            cards = workspace.file_infos(&cards_dir, &card_entries);
            // The title card: `..` goes up a folder; `/` is the root, which you cannot go up from.
            let title = if cards_dir.is_empty() { "/" } else { ".." };
            card_entries.push(Entry { name: title.into(), is_dir: true });
            cards.push(FileInfo::named(title, true));
            card_hover = None;
            cards_panel.mark_dirty();
        }
        let folder_label = format!(
            "/{} · {}/{}",
            cards_dir,
            if dir_entries.is_empty() { 0 } else { card_selected + 1 },
            dir_entries.len()
        );
        if folder_label != card_folder_label {
            card_folder_label = folder_label;
            cards_panel.mark_dirty();
        }
        if let Some(d) = drawer.as_mut() {
            d.update(frame_dt);
        }
        let drawer_open_now = drawer.as_ref().is_some_and(|d| d.progress() > 0.99);
        let drawer_closed_now = drawer.as_ref().map_or(true, |d| d.progress() < 0.05);
        if drawer_open_now && !prev_drawer_open {
            prev_drawer_open = true;
            fire(&mut script, &mut editor, "drawer_open", &[]);
        } else if drawer_closed_now && prev_drawer_open {
            prev_drawer_open = false;
            fire(&mut script, &mut editor, "drawer_close", &[]);
        }
        // The cards stand on the desk (or, without a mapped desk, about 40 cm below the eyes), in an arc in front of you.
        let desk_y = desk.map_or(head.y - 0.40, |d| d.height);
        let poses: Vec<CardPose> = match &drawer {
            Some(d) => {
                let fan = Fan { head, forward: d.forward, radius: cfg.fan_radius, desk_y, card_width: cfg.card_width };
                let mut p = fan_poses(&fan, win_len, card_selected.saturating_sub(win_start), card_shift, d, d.progress());
                p.push(title_pose(&fan, d, d.progress()));
                p
            }
            None => Vec::new(),
        };
        // What can be grabbed: the orbs and the cards that already came out of the drawer.
        let handle_present = drawer
            .as_ref()
            .is_some_and(|d| d.is_open() && pull_hand.is_none() && (d.progress() > 0.85 || d.is_held()));
        let handle_index = scene_orbs.len();
        let card_base = scene_orbs.len() + usize::from(handle_present);
        let mut items: Vec<Draggable> =
            scene_orbs.iter().map(|o| Draggable { center: o.center, radius: o.radius, grasp: true }).collect();
        if let (true, Some(d)) = (handle_present, &drawer) {
            items.push(Draggable { center: d.handle(), radius: 0.05, grasp: false });
        }
        if drawer.is_some() {
            for p in &poses {
                let out = p.t > 0.6;
                items.push(Draggable { center: if out { p.position } else { Vec3::new(0.0, -100.0, 0.0) }, radius: if out { 0.11 * cfg.card_width / 0.21 } else { 0.01 }, grasp: false });
            }
        }
        let drag_frame = drag.update(&points, &mut items);
        // Crushing the orb needs to know where the desk is: without a mapped desk, the user is told.
        if desk.is_none() && drag_frame.started.iter().any(|(_, i)| *i < scene_orbs.len()) {
            editor.message = "map your desk first: rest your open palm flat and still on it".into();
        }
        for (orb, item) in scene_orbs.iter_mut().zip(&items) {
            orb.center = item.center;
        }
        // The imaginary drawer: pinch in the air from anywhere and pull toward you. Pushing
        // the other way closes it. It does not depend on any object, so as not to get confused with the orbs.
        let head_forward = if views_ok {
            let q = views[0].pose.orientation;
            let f = Quat::from_xyzw(q.x, q.y, q.z, q.w) * Vec3::NEG_Z;
            Vec3::new(f.x, 0.0, f.z).try_normalize().unwrap_or(forward)
        } else {
            forward
        };
        // The handle: with the drawer open, pinching it and pushing it closes it.
        if let Some(d) = drawer.as_mut() {
            if handle_present {
                if drag_frame.started.iter().any(|(_, i)| *i == handle_index) {
                    d.grab();
                }
                if drag_frame.grabbed.contains(&Some(handle_index)) {
                    d.hold(items[handle_index].center);
                }
                if drag_frame.ended.iter().any(|(_, i)| *i == handle_index) {
                    d.release();
                }
            }
        }
        // The laptop screen from afar: a ray comes out of each hand (from the shoulder, through the pinch). Pointing at
        // it shows a cursor and lights up its outline and grips. Pinching inside grabs it: bringing the hand closer
        // brings it and moving it away takes it away (amplified, to bring it from afar without stretching the arm). Pinching a corner
        // resizes it. What is within reach of the hand (cards, keyboard, drawer) takes priority.
        let mut ui = ScreenUi::default();
        let to_laptop = bridge.as_ref().filter(|_| remote_on && remote_allowed).map(|b| b.to_laptop.clone());
        // With the keyboard on the laptop, pinching the Atlas editor gives the keyboard back to Atlas (like clicking another
        // window). That pinch neither opens the drawer nor clicks on the laptop.
        let mut took_focus: Option<usize> = None;
        if remote_focus {
            if let Some((p, r)) = editor_panel.filter(|_| panel_open) {
                for (hand, f) in hand_frames.iter().enumerate() {
                    if !(f.tracked && f.pinch == PinchEvent::Started) {
                        continue;
                    }
                    let s = shoulder(head, head_forward, hand);
                    let on_editor = ray_plane(s, (f.pinch_point - s).normalize_or_zero(), p, r)
                        .is_some_and(|(l, _)| l.x.abs() <= cfg.panel_width / 2.0 && l.y.abs() <= cfg.panel_width * PANEL_ASPECT / 2.0);
                    if on_editor {
                        remote_focus = false;
                        took_focus = Some(hand);
                        info!("keyboard back in Atlas");
                        editor.message = "keyboard: back in Atlas".into();
                        code_panel.mark_dirty();
                        break;
                    }
                }
            }
        }
        // The laptop's mouse button is released when the pinch is released.
        if let Some(h) = remote_click {
            let f = &hand_frames[h];
            if !(f.tracked && f.pinching) || !remote_on {
                if let Some(tx) = &to_laptop {
                    let _ = tx.input.send(atlas_link::RemoteInput::Button { down: false, right: false });
                }
                remote_click = None;
            }
        }
        if let Some((pose, rot)) = screen_pose.filter(|_| screen_at.is_some_and(|t| t.elapsed() < Duration::from_secs(4))) {
            let (center, vis_h) = visible_rect(pose, rot, screen_w, screen_aspect);
            let img_hw = screen_w / 2.0;
            if let Some(g) = screen_ray {
                let f = &hand_frames[g.hand()];
                if !(f.tracked && f.pinching) {
                    info!("screen released: {:.2} m away, {:.2} m wide", center.distance(head), screen_w);
                    screen_ray = None;
                }
            }
            for (hand, f) in hand_frames.iter().enumerate() {
                // Both hands can grab it: one pointing at it does not stop the other from pinching it.
                if !f.tracked {
                    continue;
                }
                let mine = screen_ray.is_some_and(|g| g.hand() == hand);
                let busy = drag.grabbed()[hand].is_some()
                    || pull_hand == Some(hand)
                    || kb_grab.is_some_and(|(h, _)| h == hand)
                    || screen_grab.is_some_and(|(h, _)| h == hand)
                    || keyboard.as_ref().is_some_and(|kb| kb.on_plate(f.pinch_point) || kb.corner_near(f.pinch_point, GRIP_NEAR).is_some());
                if (busy && !mine) || took_focus == Some(hand) {
                    continue;
                }
                let s = shoulder(head, head_forward, hand);
                let dir = (f.pinch_point - s).normalize_or_zero();
                let Some((local, t)) = ray_plane(s, dir, center, rot) else { continue };
                // Where it points, as a fraction of the image (0..1, from the top left).
                let uv = ((local.x / screen_w + 0.5).clamp(0.0, 1.0), (0.5 - local.y / vis_h).clamp(0.0, 1.0));
                let cursor_px = (uv.0 * screen_img.0, uv.1 * screen_img.1);
                // `:remote`: dragging with the button pressed, the pointer follows the hand.
                if remote_click == Some(hand) {
                    if let Some(tx) = &to_laptop {
                        if remote_uv.is_none_or(|(u, v)| (u - uv.0).abs() + (v - uv.1).abs() > 0.0015) {
                            let _ = tx.input.send(atlas_link::RemoteInput::Pointer { u: uv.0, v: uv.1 });
                            remote_uv = Some(uv);
                        }
                    }
                    ui = ScreenUi { cursor: Some(cursor_px), state: 2, corner: None, focus: remote_focus };
                    continue;
                }
                match screen_ray {
                    Some(ScreenRay::Move { local_hit, hit_dist, hand_dist, .. }) if mine => {
                        let reach = (f.pinch_point - s).length().max(0.05);
                        let dist = (hit_dist * (reach / hand_dist).powf(1.5)).clamp(0.35, 4.0);
                        let face_rot = face(center, head);
                        let new_center = s + dir * dist - face_rot * local_hit;
                        screen_pose = Some(screen_pose_for(new_center, face(new_center, head), screen_w, screen_aspect));
                        ui = ScreenUi { cursor: None, state: 2, corner: None, focus: remote_focus };
                    }
                    Some(ScreenRay::Resize { corner, width, reach, .. }) if mine => {
                        let w = (width * local.length() / reach).clamp(SCREEN_MIN_W, SCREEN_MAX_W);
                        if (w - screen_w).abs() > 1e-4 {
                            screen_w = w;
                            screen_pose = Some(screen_pose_for(center, rot, screen_w, screen_aspect));
                        }
                        ui = ScreenUi { cursor: None, state: 2, corner: Some(corner), focus: remote_focus };
                    }
                    Some(_) => {}
                    None => {
                        let zone = (0.08 * screen_w).max(0.04);
                        let corners = [(-img_hw, vis_h / 2.0), (img_hw, vis_h / 2.0), (-img_hw, -vis_h / 2.0), (img_hw, -vis_h / 2.0)];
                        let corner = corners
                            .iter()
                            .enumerate()
                            .map(|(i, (x, y))| (i, ((local.x - x).powi(2) + (local.y - y).powi(2)).sqrt()))
                            .filter(|(_, d)| *d <= zone)
                            .min_by(|a, b| a.1.total_cmp(&b.1))
                            .map(|(i, _)| i);
                        let inside = local.x.abs() <= img_hw && local.y.abs() <= vis_h / 2.0;
                        if !(inside || corner.is_some()) {
                            continue;
                        }
                        let started = f.pinch == PinchEvent::Started && !drag_frame.started.iter().any(|(h, _)| *h == hand);
                        // `:remote`: inside the image, the ray is the laptop's mouse and pinching is a click (the
                        // corners still resize it; to move it, the closed hand or `:screen here`).
                        if let (Some(tx), None) = (&to_laptop, corner) {
                            // One hand drives the mouse: the one clicking, or the first one pointing (if both
                            // point, they do not fight over it).
                            if ui.state > 0 && !started {
                                continue;
                            }
                            if remote_uv.is_none_or(|(u, v)| (u - uv.0).abs() + (v - uv.1).abs() > 0.0015) {
                                let _ = tx.input.send(atlas_link::RemoteInput::Pointer { u: uv.0, v: uv.1 });
                                remote_uv = Some(uv);
                            }
                            if started {
                                let _ = tx.input.send(atlas_link::RemoteInput::Pointer { u: uv.0, v: uv.1 });
                                let _ = tx.input.send(atlas_link::RemoteInput::Button { down: true, right: false });
                                remote_click = Some(hand);
                                if !remote_focus {
                                    remote_focus = true;
                                    info!("keyboard on the laptop");
                                    editor.message = "keyboard: typing on your laptop (pinch the Atlas editor to type here)".into();
                                    code_panel.mark_dirty();
                                }
                                info!("click on the laptop at ({:.3}, {:.3})", uv.0, uv.1);
                            }
                            ui = ScreenUi { cursor: Some(cursor_px), state: if started { 2 } else { 1 }, corner: None, focus: remote_focus };
                            continue;
                        }
                        if started {
                            screen_ray = Some(match corner {
                                Some(corner) => {
                                    info!("screen: resize by corner {corner}");
                                    ScreenRay::Resize { hand, corner, width: screen_w, reach: local.length().max(0.05) }
                                }
                                None => {
                                    info!("screen grabbed from afar ({t:.2} m from the shoulder)");
                                    ScreenRay::Move { hand, local_hit: local, hit_dist: t, hand_dist: (f.pinch_point - s).length().max(0.05) }
                                }
                            });
                        }
                        // The cursor belongs to one hand: the one grabbing, or the first one pointing.
                        if ui.state > 0 && !started {
                            continue;
                        }
                        ui = ScreenUi { cursor: (!started && corner.is_none()).then_some(cursor_px), state: if started { 2 } else { 1 }, corner, focus: remote_focus };
                    }
                }
            }
        } else {
            screen_ray = None;
        }
        ui.focus = remote_focus;
        if ui != screen_ui {
            screen_ui = ui;
            screen_panel.mark_dirty();
        }
        // The resolution that suits the screen as you see it now: it is requested from the laptop.
        if let (Some(b), Some((p, _))) = (&bridge, screen_pose) {
            // It goes up as soon as needed; it goes down only if it would still be too much even if you were 12 % closer (otherwise,
            // moving the head at the threshold would oscillate between two sizes).
            let d = p.distance(head);
            let up = wanted_width(screen_w, d);
            let want = if up > screen_want || wanted_width(screen_w, d * 0.88) < screen_want { up } else { screen_want };
            if want != screen_want {
                screen_want = want;
                b.to_laptop.want_width.store(want, std::sync::atomic::Ordering::Relaxed);
                info!("screen: asking the laptop for {want} px wide ({:.2} m at {:.2} m)", screen_w, p.distance(head));
            }
        }
        // The drawer opens by pinching above keyboard height (15 cm above the desk), so as not to be confused
        // with typing, and never with a pinch that starts over the keyboard or while the keyboard is moving. With the
        // drawer open, a pinch anywhere in that zone (that is not a card or the handle) grabs it as
        // it is, and pushing it closes it: there is no need to hit the handle.
        let above_keyboard = |p: Vec3| p.y >= desk_y + 0.15;
        let on_keyboard = |p: Vec3| kb_grab.is_some() || keyboard.as_ref().is_some_and(|kb| kb.on_plate(p) || kb.corner_near(p, GRIP_NEAR).is_some());
        for (hand, f) in hand_frames.iter().enumerate() {
            let free = f.tracked && f.pinching && drag.grabbed()[hand].is_none();
            let drawer_spot = above_keyboard(f.pinch_point) && !on_keyboard(f.pinch_point);
            let on_screen = screen_ray.is_some_and(|g| g.hand() == hand)
                || remote_click == Some(hand)
                || took_focus == Some(hand)
                || (screen_ui.state > 0 && f.pinch == PinchEvent::Started);
            if free && drawer_spot && !on_screen && f.pinch == PinchEvent::Started && pull_hand.is_none() && !drag_frame.started.iter().any(|(h, _)| *h == hand) {
                pull_hand = Some(hand);
                // A new drawer starts in `inbox/` if something just arrived from the laptop (with that chosen); otherwise, in
                // the folder of the file you are editing, with that file chosen.
                if drawer.is_none() && inbox_fresh.is_some() {
                    cards_dir = "inbox".into();
                    pending_select = inbox_fresh.take();
                    cards_reload = true;
                } else if drawer.is_none() && !editor.file.is_empty() {
                    cards_dir = parent_of(&editor.file);
                    pending_select = editor.file.rsplit('/').next().map(str::to_string);
                    cards_reload = true;
                }
                let reach = drawer.as_ref().map_or(0.0, |d| d.progress() * TRAVEL);
                let mut d = drawer.take().unwrap_or_else(|| Drawer::new(f.pinch_point, head_forward));
                // If it was already open, it is grabbed as it is: pushing closes it.
                d.forward = if reach > 0.0 { d.forward } else { head_forward };
                d.closed = f.pinch_point + d.forward * reach;
                d.grab();
                drawer = Some(d);
            } else if pull_hand == Some(hand) {
                if free || (f.tracked && f.pinching) {
                    if let Some(d) = drawer.as_mut() {
                        d.hold(f.pinch_point);
                    }
                } else {
                    if let Some(d) = drawer.as_mut() {
                        d.release();
                    }
                    pull_hand = None;
                }
            }
        }
        // Paging cards: with the card file open, sweep with the open palm above the cards. To the
        // left, the next one; to the right, the previous one (like turning pages).
        if let Some(d) = drawer.as_ref().filter(|d| d.progress() > 0.9 && !d.is_held()) {
            let right = Vec3::new(d.forward.x, 0.0, d.forward.z).try_normalize().unwrap_or(Vec3::NEG_Z).cross(Vec3::Y);
            for (hand, f) in hand_frames.iter().enumerate() {
                let curl = f.finger_tips.iter().map(|t| t.distance(f.palm)).sum::<f32>() / 4.0;
                let open_palm = f.tracked && !f.pinching && !f.gripping && !f.grasping && curl >= 0.12;
                let reach = Vec3::new(f.palm.x - head.x, 0.0, f.palm.z - head.z).length();
                let over_cards = (cfg.fan_radius - 0.15..=cfg.fan_radius + 0.25).contains(&reach)
                    && (desk_y - 0.05..=desk_y + 0.45).contains(&f.palm.y);
                if let Some(step) = palm_swipes[hand].update(f.palm, open_palm && over_cards, right, frame_dt) {
                    let last = dir_entries.len().saturating_sub(1);
                    let next = (card_selected as i64 + i64::from(step)).clamp(0, last as i64) as usize;
                    if next != card_selected {
                        let (old_start, _) = window(dir_entries.len(), card_selected);
                        let (new_start, _) = window(dir_entries.len(), next);
                        card_selected = next;
                        // The cards slide one position instead of changing all at once.
                        card_shift += new_start as f32 - old_start as f32;
                    }
                }
            }
        }
        card_shift *= (-12.0 * frame_dt).exp();
        if card_shift.abs() < 0.005 {
            card_shift = 0.0;
        }
        // Cards: the grabbed one follows the hand; the others, their place in the fan.
        card_smooth.resize(poses.len(), Vec3::ZERO);
        card_moved.resize(poses.len(), false);
        let stay = cfg.card_release == CardRelease::Stay;
        let drawer_progress = drawer.as_ref().map_or(0.0, |d| d.progress());
        for (k, p) in poses.iter().enumerate() {
            let cur = &mut card_smooth[k];
            if drag_frame.grabbed.contains(&Some(card_base + k)) {
                *cur = items[card_base + k].center;
                card_moved[k] = true;
            } else if stay && card_moved[k] && drawer_progress > 0.3 {
                // `config.card_release = "stay"`: it stays where you dropped it, until the drawer closes.
            } else {
                card_moved[k] = false;
                if cur.distance(p.position) > 0.9 || *cur == Vec3::ZERO {
                    *cur = p.position;
                } else {
                    // Released, the card floats back to its place in the fan.
                    *cur = cur.lerp(p.position, 1.0 - (-9.0 * frame_dt).exp());
                }
            }
        }
        let card_positions: Vec<Vec3> = card_smooth.clone();
        // Dropping a card in front of the laptop screen (so that it covers it from your eyes, even if the screen is
        // far) sends the file to the laptop. The card must have left its place in the fan.
        let screen_target = screen_pose.filter(|_| screen_at.is_some_and(|t| t.elapsed() < Duration::from_secs(4))).map(|(p, r)| {
            (p, r, screen_w * screen_aspect)
        });
        let in_front_of_screen = |k: usize| {
            screen_target.is_some_and(|(c, r, h)| {
                poses.get(k).is_some_and(|p| card_smooth[k].distance(p.position) > 0.12)
                    && ray_hits_quad(head, card_smooth[k], c, r, screen_w, h)
            })
        };
        let over = (0..card_entries.len()).find(|&k| drag_frame.grabbed.contains(&Some(card_base + k)) && in_front_of_screen(k));
        if over != card_over_screen {
            card_over_screen = over;
            if let Some(entry) = over.and_then(|k| card_entries.get(k)) {
                editor.message = format!("release to send {} to the laptop", entry.name);
                code_panel.mark_dirty();
            }
        }
        for (_, item) in &drag_frame.ended {
            let Some(k) = item.checked_sub(card_base) else { continue };
            let Some(entry) = card_entries.get(k).cloned() else { continue };
            if !in_front_of_screen(k) {
                continue;
            }
            let path = if cards_dir.is_empty() { entry.name.clone() } else { format!("{cards_dir}/{}", entry.name) };
            editor.message = if entry.is_dir || entry.name == ".." || entry.name == "/" {
                "folders cannot be sent to the laptop yet: drop a file".into()
            } else {
                let data = if path == editor.file {
                    Some(editor.buffer.to_string().into_bytes())
                } else {
                    workspace.resolve(&path).ok().and_then(|p| std::fs::read(p).ok())
                };
                match send_to_laptop(bridge.as_ref().filter(|_| bridge_paired), &path, data) {
                    Ok(m) => {
                        info!("card dropped in front of the laptop screen: {path} goes to the laptop");
                        st_sent = Some(entry.name.clone());
                        m
                    }
                    Err(e) => e,
                }
            };
            card_over_screen = None;
            code_panel.mark_dirty();
        }
        // For the test hand: where the chosen card is and whether the right hand is holding one.
        st_card_pos = card_selected.checked_sub(win_start).and_then(|k| card_smooth.get(k).copied()).filter(|_| drawer.is_some());
        st_card_grabbed = drag.grabbed()[1].is_some_and(|i| i >= card_base && i < card_base + card_entries.len());
        let hover_now = drag_frame.hovered.iter().flatten().find(|&&i| i >= card_base).map(|i| i - card_base);
        if hover_now != card_hover {
            card_hover = hover_now;
            cards_panel.mark_dirty();
        }
        // A tap on a card opens the file (and with it the editor) or enters the folder.
        for (_, item) in &drag_frame.tapped {
            if *item >= card_base {
                if let Some(entry) = card_entries.get(*item - card_base).cloned() {
                    let path = if cards_dir.is_empty() { entry.name.clone() } else { format!("{cards_dir}/{}", entry.name) };
                    info!("tap on card {}: {path}", *item - card_base);
                    fire(&mut script, &mut editor, "card_tap", &[EventArg::Text(entry.name.clone()), EventArg::Num(f64::from(u8::from(entry.is_dir)))]);
                    if entry.name == ".." {
                        // When going up, the folder you came from stays chosen.
                        pending_select = cards_dir.rsplit('/').next().map(str::to_string);
                        cards_dir = parent_of(&cards_dir);
                        cards_reload = true;
                    } else if entry.name == "/" {
                        // The root: there is nowhere to go up.
                    } else if entry.is_dir {
                        cards_dir = path;
                        cards_reload = true;
                    } else if editor.modified {
                        editor.message = "E37: unsaved changes (use :w first)".into();
                        code_panel.mark_dirty();
                    } else {
                        summon = true;
                        perform(&mut editor, &Action::Open(path), &workspace);
                        code_panel.mark_dirty();
                        // With the file open, the cards go back into the drawer and the editor is left clear.
                        if let Some(d) = drawer.as_mut() {
                            d.close();
                        }
                    }
                }
            }
        }
        for (hand, orb) in &drag_frame.started {
            info!("{} hand grabs orb {orb}", if *hand == 0 { "left" } else { "right" });
        }
        for (hand, orb) in &drag_frame.ended {
            info!("{} hand releases orb {orb}", if *hand == 0 { "left" } else { "right" });
        }
        // The environment orb changes the environment in two ways: crushed against the desk
        // while you carry it, or thrown against a wall.
        let env_index = 0usize;
        let env_valid = scene_orbs.len() == env_index + 1;
        if env_valid {
            let grabbed = drag_frame.grabbed.contains(&Some(env_index));
            if grabbed {
                env_body = None;
            }
            if grabbed {
                env_hold = 0.0;
            }
            if let Some(d) = &desk {
                let orb = &scene_orbs[env_index];
                // Crushing: bringing it against the desk with the closed hand, or lowering the open palm onto it
                // when it is on the desk. A moment after it was released, so as not to crush it when opening the hand.
                let by_hand = grabbed && orb.center.y - orb.radius <= d.height + 0.01;
                let by_palm = !grabbed
                    && env_hold > 0.0
                    && env_hold < 1.25
                    && hand_frames.iter().enumerate().any(|(hand, f)| {
                        let across = Vec3::new(f.palm.x - orb.center.x, 0.0, f.palm.z - orb.center.z).length();
                        f.tracked
                            && !f.grasping
                            && across <= orb.radius + 0.05
                            && f.palm.y >= orb.center.y + 0.02
                            && f.palm.y <= orb.center.y + orb.radius + 0.05
                            && prev_palm_y[hand] - f.palm.y > 0.002
                    });
                if by_hand || by_palm {
                    next_environment(&mut environment, &mut editor);
                    fire(&mut script, &mut editor, "environment", &[EventArg::Text(ENVIRONMENTS[environment as usize].into())]);
                    code_panel.mark_dirty();
                    drag.release_item(env_index);
                    orb_key.clear();
                    env_hold = 0.0;
                }
            }
            // Releasing it with the hand moving throws it (it draws a curve because of gravity); releasing it
            // slowly makes it float back to its place.
            for (hand, item) in &drag_frame.ended {
                if *item == env_index {
                    let v = velocity[*hand].velocity();
                    let orb = &scene_orbs[env_index];
                    if v.length() >= cfg.throw_min_speed {
                        env_body = Some(Body::thrown(orb.center, v, orb.radius));
                        env_flight = 0.0;
                        env_rest = 0.0;
                        info!("orb thrown at {:.1} m/s", v.length());
                        fire(&mut script, &mut editor, "orb_thrown", &[EventArg::Num(f64::from(v.length()))]);
                    } else if desk.is_some_and(|d| orb.center.y - orb.radius <= d.height + 0.05) {
                        // Released by the desk: it stays there for a moment, in case you crush it with the palm.
                        env_hold = 1.5;
                    } else {
                        env_returning = true;
                    }
                    velocity[*hand].clear();
                }
            }
            if grabbed {
                env_returning = false;
            }
            if env_hold > 0.0 {
                env_hold -= frame_dt;
                if env_hold <= 0.0 {
                    env_returning = true;
                }
            }
            if env_returning && env_body.is_none() && !grabbed {
                let home = anchor_now.point(Vec3::new(0.30, 0.04, -0.42));
                let orb = &mut scene_orbs[env_index];
                orb.center = orb.center.lerp(home, 1.0 - (-6.0 * frame_dt).exp());
                if orb.center.distance(home) < 0.01 {
                    env_returning = false;
                }
            }
        }
        if let (Some(body), true) = (env_body.as_mut(), env_valid) {
            let world = PhysWorld { desk_height: desk.map(|d| d.height), walls: walls.iter().map(|(w, _)| *w).collect(), gravity: cfg.gravity };
            let hits = body.step(frame_dt, &world);
            scene_orbs[env_index].center = body.pos;
            env_flight += frame_dt;
            env_rest = if body.resting { env_rest + frame_dt } else { 0.0 };
            let wall_hit = hits.iter().any(|h| matches!(h, Hit::Wall { speed, .. } if *speed >= 0.8));
            if wall_hit {
                let hit_speed = hits.iter().filter_map(|h| if let Hit::Wall { speed, .. } = h { Some(*speed) } else { None }).fold(0.0f32, f32::max);
                next_environment(&mut environment, &mut editor);
                fire(&mut script, &mut editor, "wall_hit", &[EventArg::Num(f64::from(hit_speed))]);
                fire(&mut script, &mut editor, "environment", &[EventArg::Text(ENVIRONMENTS[environment as usize].into())]);
                code_panel.mark_dirty();
                env_body = None;
                orb_key.clear();
            } else if env_flight > 8.0 || env_rest > 1.5 || body.pos.y < head.y - 2.0 {
                // It did not reach any wall: it goes back to its place.
                env_body = None;
                orb_key.clear();
            }
        }
        for (hand, f) in hand_frames.iter().enumerate() {
            if f.pinch != PinchEvent::None {
                info!("{} hand: pinch {:?}", if hand == 0 { "left" } else { "right" }, f.pinch);
            }
        }
        // What is drawn: the orbs (highlighted if the hand touches or carries them) and the
        // fingertips, to see where the hand is.
        let mut draw_orbs: Vec<OrbInstance> = scene_orbs.clone();
        // The test hand, visible: a cyan sphere that turns orange when pinching.
        if let (Some((p, pinching)), true) = (st_marker, selftest.is_some()) {
            let mut dot = fingertip_marker(p, pinching);
            dot.radius = if pinching { 0.022 } else { 0.016 };
            dot.intensity = 2.2;
            draw_orbs.push(dot);
        }
        for o in script.orbs() {
            draw_orbs.push(OrbInstance {
                center: anchor_now.point(Vec3::from(o.position)),
                radius: o.radius,
                core: o.core,
                rim: o.rim,
                intensity: o.intensity,
                pulse: o.pulse,
                speed: o.speed,
                phase: o.phase,
            });
        }
        if let Some(d) = drawer.as_ref().filter(|d| d.is_held()) {
            // While you pull, a thread of glowing dots joins the point where you started with your hand. It fades
            // as soon as the cards start coming out, so that once open only the cards and the handle remain.
            let t = ((d.progress() - 0.10) / 0.40).clamp(0.0, 1.0);
            let dots = 1.0 - t * t * (3.0 - 2.0 * t);
            for k in (1..=7).filter(|_| dots > 0.05) {
                let mut dot = fingertip_marker(d.closed.lerp(d.handle(), k as f32 / 8.0), true);
                dot.radius = (0.004 + 0.0015 * k as f32) * (0.5 + 0.5 * dots);
                dot.intensity = (0.4 + 0.1 * k as f32) * dots;
                dot.core = [1.0, 0.55, 0.22];
                dot.rim = [1.0, 0.8, 0.5];
                draw_orbs.push(dot);
            }
        }
        // The grabbed orb shrinks to fit in the hollow of the fingers and returns to its size when released.
        orb_fit.resize(scene_orbs.len(), 1.0);
        for (i, fit) in orb_fit.iter_mut().enumerate() {
            let target = match drag_frame.grabbed.iter().position(|g| *g == Some(i)) {
                Some(h) if hand_frames[h].tracked => {
                    let f = &hand_frames[h];
                    let cavity = f.finger_tips.iter().map(|t| t.distance(f.grasp_point)).sum::<f32>() / 4.0;
                    (cavity - 0.010).clamp(0.022, scene_orbs[i].radius) / scene_orbs[i].radius
                }
                _ => 1.0,
            };
            *fit += (target - *fit) * 0.25;
        }
        for (hand, f) in hand_frames.iter().enumerate() {
            prev_palm_y[hand] = f.palm.y;
        }
        for (i, orb) in draw_orbs.iter_mut().enumerate().take(scene_orbs.len()) {
            orb.radius *= orb_fit[i];
            let grabbed = drag_frame.grabbed.contains(&Some(i));
            let hovered = drag_frame.hovered.contains(&Some(i));
            if grabbed || hovered {
                highlight(orb, grabbed);
            }
        }
        // Trail of the thrown orb: you see the curve it draws.
        match env_body {
            Some(body) => {
                env_trail.push_back(body.pos);
                while env_trail.len() > 26 {
                    env_trail.pop_front();
                }
            }
            None => env_trail.clear(),
        }
        let trail_len = env_trail.len().max(1) as f32;
        for (k, p) in env_trail.iter().enumerate() {
            let t = k as f32 / trail_len;
            let mut dot = fingertip_marker(*p, true);
            dot.radius = 0.004 + 0.016 * t;
            dot.intensity = 0.3 + 0.9 * t;
            dot.core = [1.0, 0.55, 0.22];
            dot.rim = [1.0, 0.8, 0.5];
            draw_orbs.push(dot);
        }
        // The roll of the scroll goes on the bottom edge of the unrolled part.
        let mut rolls: Vec<Roll> = Vec::new();
        if let Some((pos, rot)) = editor_panel.filter(|_| panel_p > 0.0 && reveal < 0.99) {
            let full_h = cfg.panel_width * PANEL_ASPECT;
            let radius = 0.018 + 0.035 * (1.0 - reveal);
            let edge = pos + rot * Vec3::new(0.0, full_h / 2.0 - full_h * reveal, 0.0);
            rolls.push(Roll {
                center: edge - rot * Vec3::new(0.0, radius, 0.0),
                axis: rot * Vec3::X,
                half_length: cfg.panel_width / 2.0 + 0.02,
                radius,
                tint: [0.18, 0.55, 0.70],
            });
        }
        // Desk: a wave that charges under the hand and, when mapped, the trace of the desk.
        let mut surfaces: Vec<Surface> = Vec::new();
        if let (Some(d), Some(at)) = (&desk, desk_mapped_at) {
            surfaces.push(Surface { center: d.center, half: 0.6, color: [0.45, 0.85, 1.0], progress: 1.0, age: at.elapsed().as_secs_f32(), normal: Vec3::Y, yaw: 0.0 });
        } else if let Some((center, progress)) = desk_progress {
            surfaces.push(Surface { center, half: 0.25, color: [1.0, 0.65, 0.3], progress, age: -1.0, normal: Vec3::Y, yaw: 0.0 });
        }
        // The drawer appears instantly when pulling as a glowing tray, which fades while the
        // cards come out; only the handle remains (to close it and put the files away).
        if let Some(d) = drawer.as_ref().filter(|d| d.progress() > 0.01) {
            let p = d.progress();
            let right = d.forward.cross(Vec3::Y);
            // The tray shows when starting to pull and fades soon (between 15 % and 55 %), when the
            // cards are already coming out: once open only the cards and the handle remain.
            let t = ((p - 0.15) / 0.40).clamp(0.0, 1.0);
            let fade = 1.0 - t * t * (3.0 - 2.0 * t);
            if fade > 0.0 {
                let centre = d.closed.lerp(d.handle(), 0.5) - Vec3::Y * 0.07;
                surfaces.push(Surface {
                    center: centre,
                    half: 0.16 + 0.14 * p,
                    color: [1.0, 0.65, 0.3],
                    progress: fade,
                    age: 6.0,
                    normal: Vec3::Y,
                    yaw: (-right.z).atan2(right.x),
                });
            }
            rolls.push(Roll { center: d.handle(), axis: right, half_length: 0.09, radius: 0.012, tint: [0.95, 0.6, 0.3] });
        }
        for (wall, at) in &walls {
            surfaces.push(Surface { center: wall.point, half: 0.9, color: [0.45, 0.85, 1.0], progress: 1.0, age: at.elapsed().as_secs_f32(), normal: wall.normal, yaw: 0.0 });
        }
        if let Some((wall, progress)) = wall_progress {
            surfaces.push(Surface { center: wall.point, half: 0.25, color: [1.0, 0.65, 0.3], progress, age: -1.0, normal: wall.normal, yaw: 0.0 });
        }
        // The hand silhouette. While typing, or with the hand against the desk, it is hidden so as not to
        // cover the keyboard and the text; with a virtual environment active it always shows (the real hand does not).
        let typing = matches!(editor.mode, Mode::Insert | Mode::Command);
        let near_desk = |p: Vec3| desk.is_some_and(|d| p.y < d.height + 0.15);
        let mut hand_draws: Vec<HandDraw> = Vec::new();
        if let Some(meshes) = &hand_meshes {
            for (hand, f) in hand_frames.iter().enumerate().filter(|(_, f)| f.tracked) {
                // If the hand holds an orb, the silhouette always shows (to see the grip).
                let holding = drag.grabbed()[hand].is_some_and(|i| i < scene_orbs.len());
                if !holding && environment == 0 && (typing || near_desk(f.pinch_point)) {
                    continue;
                }
                if let Some(vertices) = meshes[hand].skin(&f.joints, &f.joint_rot, f.joint_mask) {
                    let tint = if f.pinching || f.gripping || f.grasping { [1.0, 0.65, 0.3] } else { [0.45, 0.85, 1.0] };
                    hand_draws.push(HandDraw { hand, vertices, tint, alpha: 1.0 });
                }
            }
        }

        let image = sc.handle.acquire_image()?;
        let seconds = ((state.predicted_display_time.as_nanos() % 3_600_000_000_000) as f64 / 1e9) as f32;
        let eyes = views_ok.then(|| {
            let eye = |v: &xr::View| {
                let (p, q, f) = (v.pose.position, v.pose.orientation, v.fov);
                eye_camera(
                    Vec3::new(p.x, p.y, p.z),
                    Quat::from_xyzw(q.x, q.y, q.z, q.w),
                    Fov { left: f.angle_left, right: f.angle_right, up: f.angle_up, down: f.angle_down },
                    0.05,
                    50.0,
                )
            };
            [eye(&views[0]), eye(&views[1])]
        });
        let resolution = sc.resolution;
        unsafe {
            vk.record_clear(frame, sc, image as usize, CLEAR_COLOR, |device, cmd| {
                if let Some(eyes) = &eyes {
                    orb_renderer.record(device, cmd, frame, resolution, eyes, seconds, &draw_orbs, &surfaces, environment, &rolls, &hand_draws);
                }
            })?
        };
        sc.handle.wait_image(xr::Duration::INFINITE)?;
        let mut uploads = Vec::new();
        if code_panel.record_upload(&vk, frame, CODE_SLOT)? {
            uploads.push(CODE_SLOT);
        }
        if log_panel.record_upload(&vk, frame, LOG_SLOT)? {
            uploads.push(LOG_SLOT);
        }
        if cards_panel.record_upload(&vk, frame, CARDS_SLOT)? {
            uploads.push(CARDS_SLOT);
        }
        if kb_panel.record_upload(&vk, frame, KB_SLOT)? {
            uploads.push(KB_SLOT);
        }
        if alert_panel.record_upload(&vk, frame, ALERT_SLOT)? {
            uploads.push(ALERT_SLOT);
        }
        if screen_panel.record_upload(&vk, frame, SCREEN_SLOT)? {
            uploads.push(SCREEN_SLOT);
        }
        unsafe { vk.submit(frame, &uploads)? };
        code_panel.finish_upload()?;
        log_panel.finish_upload()?;
        cards_panel.finish_upload()?;
        kb_panel.finish_upload()?;
        alert_panel.finish_upload()?;
        screen_panel.finish_upload()?;
        sc.handle.release_image()?;

        if views_ok && frame_n % 300 == 0 {
            let p = views[0].pose.position;
            info!("head (left eye): {:.2} {:.2} {:.2}", p.x, p.y, p.z);
        }

        let rect = xr::Rect2Di {
            offset: xr::Offset2Di { x: 0, y: 0 },
            extent: xr::Extent2Di {
                width: sc.resolution.width as _,
                height: sc.resolution.height as _,
            },
        };
        let view = |i: usize| {
            xr::CompositionLayerProjectionView::new()
                .pose(views[i].pose)
                .fov(views[i].fov)
                .sub_image(
                    xr::SwapchainSubImage::new()
                        .swapchain(&sc.handle)
                        .image_array_index(i as u32)
                        .image_rect(rect),
                )
        };
        let proj_views = [view(0), view(1)];
        let projection = xr::CompositionLayerProjection::new()
            .layer_flags(xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA)
            .space(&space)
            .views(&proj_views);

        // Quad layers are only sent after their first pixel upload.
        let code_layer = editor_panel.filter(|_| panel_p > 0.0).and_then(|(pos, rot)| code_panel
            .texture()
            .filter(|_| code_panel.is_ready())
            .map(|t| quad_layer(&space, t, quat_pose(pos, rot), cfg.panel_width, reveal)));
        // The output panel follows the editor's width and moves aside while the keyboard is on screen.
        let panel_k = cfg.panel_width / PANEL_W_M;
        let log_layer = editor_panel.filter(|_| reveal > 0.98 && keyboard.is_none()).and_then(|(pos, rot)| log_panel
            .texture()
            .filter(|_| log_panel.is_ready() && editor.log.is_visible())
            .map(|t| quad_layer(&space, t, quat_pose(pos + rot * Vec3::new(0.0, -0.58 * panel_k, 0.03), rot), LOG_W_M * panel_k, 1.0)));

        let kb_layer = keyboard.as_ref().and_then(|kb| {
            kb_panel.texture().filter(|_| kb_panel.is_ready()).map(|t| {
                let (r, u, n) = kb.axes();
                let rot = Quat::from_mat3(&Mat3::from_cols(r, u, n));
                quad_layer(&space, t, quat_pose(kb.center, rot), KB_W_M, 1.0)
            })
        });

        // The cards that came out of the drawer, each one facing you.
        let mut card_layers = Vec::new();
        if let Some(texture) = cards_panel.texture().filter(|_| cards_panel.is_ready()) {
            for (k, pose) in poses.iter().enumerate() {
                if pose.t < 0.02 {
                    continue;
                }
                let position = card_positions[k];
                card_layers.push(card_layer(
                    &space,
                    texture,
                    quat_pose(position, face(position, head)),
                    cfg.card_width * pose.scale,
                    (k as u32 % CARD_COLS, k as u32 / CARD_COLS),
                ));
            }
        }

        let passthrough_layer = passthrough.as_ref().map(|(_, layer)| {
            xr::sys::CompositionLayerPassthroughFB {
                ty: xr::sys::CompositionLayerPassthroughFB::TYPE,
                next: std::ptr::null(),
                flags: xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA,
                space: <xr::sys::Space as xr::sys::Handle>::NULL,
                layer_handle: layer.as_raw(),
            }
        });
        // Composition order: passthrough at the back, then projection and panels.
        let mut layers: Vec<&xr::CompositionLayerBase<xr::Vulkan>> = Vec::new();
        if let Some(raw) = &passthrough_layer {
            // SAFETY: `CompositionLayerBase` is a transparent view of the
            // common header (`type`, `next`) of any OpenXR layer.
            layers.push(unsafe {
                &*(raw as *const xr::sys::CompositionLayerPassthroughFB)
                    .cast::<xr::CompositionLayerBase<xr::Vulkan>>()
            });
        }
        if views_ok {
            layers.push(&projection);
        }
        // The laptop screen, while images keep arriving (if sharing stops, it hides). Layers
        // have no depth, only order: if it is farther than the editor it goes first (below); if not, after. The
        // keyboard, which is on the desk in front of you, always on top.
        let screen_live = screen_at.is_some_and(|t| t.elapsed() < Duration::from_secs(4));
        let screen_layer = screen_pose.filter(|_| screen_live).and_then(|(pos, rot)| {
            screen_panel.texture().filter(|_| screen_panel.is_ready()).map(|t| quad_layer_rect(&space, t, quat_pose(pos, rot), screen_w, screen_img.0 as i32, screen_img.1 as i32))
        });
        let screen_behind = match (screen_pose, editor_panel) {
            (Some((s, r)), Some((e, _))) => visible_rect(s, r, screen_w, screen_aspect).0.distance(head) > e.distance(head),
            _ => false,
        };
        if let (true, Some(layer)) = (screen_behind, &screen_layer) {
            layers.push(layer);
        }
        if let Some(layer) = &code_layer {
            layers.push(layer);
        }
        if let Some(layer) = &log_layer {
            layers.push(layer);
        }
        if let (false, Some(layer)) = (screen_behind, &screen_layer) {
            layers.push(layer);
        }
        if let Some(layer) = &kb_layer {
            layers.push(layer);
        }
        // The alert, on top of everything.
        let alert_layer = alert_pos.filter(|_| alert.is_some()).and_then(|pos| {
            alert_panel.texture().filter(|_| alert_panel.is_ready()).map(|t| quad_layer(&space, t, quat_pose(pos, face(pos, head)), ALERT_W_M, 1.0))
        });
        if let Some(layer) = &alert_layer {
            layers.push(layer);
        }
        for layer in &card_layers {
            layers.push(layer);
        }
        frame_stream.end(state.predicted_display_time, blend_mode, &layers)?;
        frame = (frame + 1) % PIPELINE_DEPTH as usize;
        frame_n += 1;
    }

    // OpenXR must release its handles before the Vulkan resources are destroyed.
    drop((passthrough, session, frame_wait, frame_stream, space));
    let textures: Vec<QuadTexture> =
        [code_panel.into_texture(), log_panel.into_texture(), cards_panel.into_texture(), kb_panel.into_texture(), alert_panel.into_texture(), screen_panel.into_texture()].into_iter().flatten().collect();
    unsafe {
        let _ = vk.device.device_wait_idle();
        orb_renderer.destroy(&vk.device);
        vk.destroy(swapchain, textures);
    }
    Ok(())
}
