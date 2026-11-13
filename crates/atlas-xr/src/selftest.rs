//! Test hand: it makes real gestures to validate without hands (from the PC, through adb). What it does goes through the
//! same path as your hands: the drag system, the drawer, the cards... only where the right hand's data comes from
//! changes while the test lasts.
//!
//! `:selftest drop` opens the drawer by pulling in the air, grabs the chosen card with a pinch, carries it in front of the
//! laptop screen (without reaching it: it is enough that it covers it from your eyes) and releases it. It reports each step in the
//! log with `selftest:`.
//!
//! `:selftest screen` points at the laptop screen from afar with the ray, pinches it and brings the hand closer (the
//! screen must come toward you), and then pinches a corner and stretches it (it must grow).
//!
//! `:selftest remote` (with `:remote` on) points at the center of the laptop screen and clicks (the keyboard
//! goes to the laptop), waits 12 s for typing from outside, and pinches the editor (the keyboard goes back to Atlas).
//!
//! `:selftest keyboard` pinches the middle of the keyboard (it must not move), then a corner and drags it 6 cm toward
//! you (it must move) and puts it back in its place.

use atlas_spatial::{PinchEvent, Vec3};
use log::info;

use crate::hands::{shoulder, HandFrame};

/// What the test hand needs to know about the world in each frame.
pub struct World {
    pub head: Vec3,
    /// Where you are looking, horizontally.
    pub forward: Vec3,
    pub desk_y: f32,
    /// Progress of the drawer (0 closed, 1 open) and whether someone is holding it.
    pub drawer: Option<(f32, bool)>,
    /// Where the chosen card is.
    pub card: Option<Vec3>,
    /// The right hand is holding a card.
    pub card_grabbed: bool,
    /// Center of the laptop screen, if it is showing.
    pub screen: Option<Vec3>,
    /// The released card was already sent to the laptop.
    pub sent: Option<String>,
    /// Atlas's own keyboard, if it is there.
    pub keyboard: Option<KeyboardState>,
    /// The laptop screen, if it is showing.
    pub far_screen: Option<ScreenState>,
    /// `:remote`: on, allowed by the laptop, and the keyboard typing on the laptop.
    pub remote: (bool, bool, bool),
    /// Center of the editor, if it is open.
    pub editor: Option<Vec3>,
}

/// What the test hand knows about the laptop screen.
#[derive(Debug, Clone, Copy)]
pub struct ScreenState {
    /// Center of the visible image.
    pub center: Vec3,
    pub rot: atlas_spatial::Quat,
    pub width: f32,
    /// Half the width of the image (it can be less than half of `width`).
    pub image_half_w: f32,
    pub height: f32,
    /// 0 released, 1 grabbed with the ray, 2 resizing.
    pub mode: u8,
}

/// What the test hand knows about the keyboard.
#[derive(Debug, Clone, Copy)]
pub struct KeyboardState {
    pub center: Vec3,
    /// Out of the plate.
    pub normal: Vec3,
    /// The top-left corner of the plate.
    pub corner: Vec3,
    /// It is grabbed by a corner.
    pub grabbed: bool,
}

/// Does the ray from `origin` through `through` touch the screen image (with a 4 cm margin)?
fn ray_hits_screen(sc: &ScreenState, origin: Vec3, through: Vec3) -> bool {
    let dir = (through - origin).normalize_or_zero();
    let normal = sc.rot * Vec3::Z;
    let denom = dir.dot(normal);
    if denom.abs() < 1e-4 {
        return false;
    }
    let t = (sc.center - origin).dot(normal) / denom;
    if t <= 0.0 {
        return false;
    }
    let local = sc.rot.inverse() * (origin + dir * t - sc.center);
    local.x.abs() <= sc.image_half_w + 0.04 && local.y.abs() <= sc.height / 2.0 + 0.04
}

/// Horizontal distance (the keyboard settles back on the desk when released).
fn flat_distance(a: Vec3, b: Vec3) -> f32 {
    Vec3::new(a.x - b.x, 0.0, a.z - b.z).length()
}

#[derive(Debug, Clone, Copy)]
enum Phase {
    Start,
    Pull { t: f32, from: Vec3 },
    WaitOpen { t: f32, open_for: f32 },
    Approach { t: f32 },
    Grab { t: f32, from: Vec3 },
    Carry { t: f32, from: Vec3, to: Vec3 },
    Release { t: f32 },
    // `:selftest keyboard`
    KbStart,
    KbMiddle { t: f32, start: Vec3, grabbed: bool },
    KbGrab { t: f32, at: Vec3, start: Vec3, back: bool },
    KbCarry { t: f32, from: Vec3, to: Vec3, start: Vec3, back: bool },
    KbCheck { t: f32, start: Vec3, back: bool },
    // `:selftest remote`
    RmStart,
    RmClick { t: f32, at: Vec3 },
    RmWait { t: f32, at: Vec3 },
    RmEditor { t: f32, at: Vec3 },
    // `:selftest screen`
    ScStart,
    ScAim { t: f32, at: Vec3, resize: bool, before: f32 },
    ScPull { t: f32, from: Vec3, to: Vec3, resize: bool, before: f32 },
    ScCheck { t: f32, resize: bool, before: f32 },
}

pub struct SelfTest {
    phase: Phase,
    point: Vec3,
    pinching: bool,
    was_pinching: bool,
    /// At the end: what happened (or why it failed).
    pub result: Option<Result<String, String>>,
    /// `:selftest screen`: where the screen came from and where it ended.
    moved: Option<(f32, f32)>,
}

/// Time the hand stays still without pinching before pinching (so that the pinch really starts).
const SETTLE: f32 = 0.2;

impl SelfTest {
    pub fn drop_on_screen() -> Self {
        info!("selftest: start (open the drawer, grab the card, drop it in front of the laptop screen)");
        Self { phase: Phase::Start, point: Vec3::ZERO, pinching: false, was_pinching: false, result: None, moved: None }
    }

    pub fn remote() -> Self {
        info!("selftest: start (click on the laptop screen, wait for typing, pinch the editor)");
        Self { phase: Phase::RmStart, point: Vec3::ZERO, pinching: false, was_pinching: false, result: None, moved: None }
    }

    pub fn far_screen() -> Self {
        info!("selftest: start (pull the laptop screen closer from afar with the ray, then stretch a corner)");
        Self { phase: Phase::ScStart, point: Vec3::ZERO, pinching: false, was_pinching: false, result: None, moved: None }
    }

    pub fn keyboard() -> Self {
        info!("selftest: start (pinch the middle of the keyboard, then a corner and drag it)");
        Self { phase: Phase::KbStart, point: Vec3::ZERO, pinching: false, was_pinching: false, result: None, moved: None }
    }

    fn finish(&mut self, result: Result<String, String>) {
        match &result {
            Ok(m) => info!("selftest: OK: {m}"),
            Err(e) => info!("selftest: FAILED: {e}"),
        }
        self.result = Some(result);
    }

    /// Advances `dt` seconds. Returns the test hand for this frame, or `None` if it already finished.
    pub fn step(&mut self, w: &World, dt: f32) -> Option<HandFrame> {
        if self.result.is_some() {
            return None;
        }
        self.was_pinching = self.pinching;
        match self.phase {
            Phase::Start => {
                if w.screen.is_none() {
                    self.finish(Err("share the laptop screen first (Atlas Bridge > Share screen)".into()));
                    return None;
                }
                if w.drawer.is_some_and(|(p, _)| p > 0.99) {
                    self.phase = Phase::WaitOpen { t: 0.0, open_for: 0.0 };
                } else {
                    // Above keyboard height and in front of you, where a pinch opens the drawer. If from there
                    // the hand's ray points at the laptop screen, the pinch would be for it (you see the cursor
                    // on it): as a person would, it looks for a spot to the side or lower where it does not point at it.
                    let base = (w.desk_y + 0.25).max(w.head.y - 0.25);
                    let side = w.forward.cross(Vec3::Y).normalize_or_zero();
                    let candidates = [(0.0, 0.0), (0.0, -0.08), (0.22, 0.0), (-0.22, 0.0), (0.22, -0.08), (-0.22, -0.08)];
                    let spot = |(lateral, down): (f32, f32)| {
                        Vec3::new(w.head.x, (base + down).max(w.desk_y + 0.16), w.head.z) + w.forward * 0.42 + side * lateral
                    };
                    let free = |p: Vec3| w.far_screen.map_or(true, |sc| !ray_hits_screen(&sc, shoulder(w.head, w.forward, 1), p));
                    let from = candidates.iter().copied().map(spot).find(|p| free(*p)).unwrap_or_else(|| spot(candidates[0]));
                    self.point = from;
                    self.pinching = false;
                    info!("selftest: pull the drawer from {from:.2}");
                    self.phase = Phase::Pull { t: 0.0, from };
                }
            }
            Phase::Pull { t, from } => {
                let t = t + dt;
                self.pinching = (SETTLE..0.85).contains(&t);
                let pull = ((t - SETTLE) / 0.5).clamp(0.0, 1.0);
                self.point = from - w.forward * 0.25 * pull;
                self.phase = if t > 1.0 { Phase::WaitOpen { t: 0.0, open_for: 0.0 } } else { Phase::Pull { t, from } };
            }
            Phase::WaitOpen { t, open_for } => {
                let t = t + dt;
                self.pinching = false;
                let open = w.drawer.is_some_and(|(p, held)| p > 0.99 && !held);
                let open_for = if open { open_for + dt } else { 0.0 };
                if open_for > 1.2 {
                    info!("selftest: drawer open");
                    self.phase = Phase::Approach { t: 0.0 };
                } else if t > 6.0 {
                    self.finish(Err(format!("the drawer did not open (drawer: {:?})", w.drawer)));
                    return None;
                } else {
                    self.phase = Phase::WaitOpen { t, open_for };
                }
            }
            Phase::Approach { t } => {
                let Some(card) = w.card else {
                    self.finish(Err("there is no card to grab".into()));
                    return None;
                };
                let t = t + dt;
                self.point = card;
                self.pinching = false;
                if t > SETTLE {
                    info!("selftest: pinch the card at {card:.2}");
                    self.pinching = true;
                    self.phase = Phase::Grab { t: 0.0, from: card };
                } else {
                    self.phase = Phase::Approach { t };
                }
            }
            Phase::Grab { t, from } => {
                let t = t + dt;
                self.point = from;
                self.pinching = true;
                if t > 0.15 {
                    if !w.card_grabbed {
                        self.pinching = false;
                        self.finish(Err("the pinch did not grab the card".into()));
                        return None;
                    }
                    let Some(screen) = w.screen else {
                        self.finish(Err("the laptop screen disappeared".into()));
                        return None;
                    };
                    // 55 cm from the eyes, on the line toward the center of the screen: within reach of the hand.
                    let to = w.head + (screen - w.head).normalize_or_zero() * 0.55;
                    info!("selftest: card grabbed; carry it to {to:.2}, in front of the screen (center {screen:.2})");
                    self.phase = Phase::Carry { t: 0.0, from, to };
                } else {
                    self.phase = Phase::Grab { t, from };
                }
            }
            Phase::Carry { t, from, to } => {
                let t = t + dt;
                let k = (t / 1.0).clamp(0.0, 1.0);
                self.point = from.lerp(to, k * k * (3.0 - 2.0 * k));
                self.pinching = true;
                if t > 1.3 {
                    info!("selftest: soltar");
                    self.pinching = false;
                    self.phase = Phase::Release { t: 0.0 };
                } else {
                    self.phase = Phase::Carry { t, from, to };
                }
            }
            Phase::RmStart => {
                let (on, allowed, _) = w.remote;
                let Some(sc) = w.far_screen else {
                    self.finish(Err("share the laptop screen first (Atlas Bridge > Share screen)".into()));
                    return None;
                };
                if !allowed {
                    self.finish(Err("the laptop does not allow control (tick the box on Atlas Bridge)".into()));
                    return None;
                }
                if !on {
                    self.finish(Err("type :remote first".into()));
                    return None;
                }
                let s = shoulder(w.head, w.forward, 1);
                let at = s + (sc.center - s).normalize_or_zero() * 0.40;
                info!("selftest: point at the center of the laptop screen and click");
                self.point = at;
                self.phase = Phase::RmClick { t: 0.0, at };
            }
            Phase::RmClick { t, at } => {
                let t = t + dt;
                self.point = at;
                // Half a second pointing (the laptop pointer goes to the center), a short click and release.
                self.pinching = (0.5..0.65).contains(&t);
                if t > 1.0 {
                    if !w.remote.2 {
                        self.finish(Err("the click did not give the keyboard to the laptop".into()));
                        return None;
                    }
                    info!("selftest: click done; the keyboard types on the laptop. Waiting 12 s for typing");
                    self.phase = Phase::RmWait { t: 0.0, at };
                } else {
                    self.phase = Phase::RmClick { t, at };
                }
            }
            Phase::RmWait { t, at } => {
                let t = t + dt;
                self.point = at;
                self.pinching = false;
                if t > 12.0 {
                    let Some(editor) = w.editor else {
                        self.finish(Ok("clicked the laptop screen and typed there (the editor is closed, so the keyboard stays on the laptop)".into()));
                        return None;
                    };
                    let s = shoulder(w.head, w.forward, 1);
                    let at = s + (editor - s).normalize_or_zero() * 0.40;
                    info!("selftest: pinch the editor to give the keyboard back to Atlas");
                    self.phase = Phase::RmEditor { t: 0.0, at };
                } else {
                    self.phase = Phase::RmWait { t, at };
                }
            }
            Phase::RmEditor { t, at } => {
                let t = t + dt;
                self.point = at;
                self.pinching = (SETTLE..SETTLE + 0.15).contains(&t);
                if t > SETTLE + 0.5 {
                    if w.remote.2 {
                        self.finish(Err("pinching the editor did not bring the keyboard back to Atlas".into()));
                        return None;
                    }
                    self.finish(Ok("clicked the centre of the laptop screen, the keyboard typed there, and pinching the editor brought it back".into()));
                    return None;
                }
                self.phase = Phase::RmEditor { t, at };
            }
            Phase::ScStart => {
                let Some(sc) = w.far_screen else {
                    self.finish(Err("share the laptop screen first (Atlas Bridge > Share screen)".into()));
                    return None;
                };
                // The hand 40 cm from the shoulder, on the line toward the center of the screen.
                let s = shoulder(w.head, w.forward, 1);
                let at = s + (sc.center - s).normalize_or_zero() * 0.40;
                let before = sc.center.distance(w.head);
                info!("selftest: point at the screen ({before:.2} m from the eyes) and pinch");
                self.point = at;
                self.phase = Phase::ScAim { t: 0.0, at, resize: false, before };
            }
            Phase::ScAim { t, at, resize, before } => {
                let t = t + dt;
                self.point = at;
                self.pinching = t > SETTLE;
                if t > SETTLE + 0.25 {
                    let Some(sc) = w.far_screen else {
                        self.finish(Err("the laptop screen disappeared".into()));
                        return None;
                    };
                    let want = if resize { 2 } else { 1 };
                    if sc.mode != want {
                        self.pinching = false;
                        self.finish(Err(if resize { "pinching the corner did not start resizing".into() } else { "pinching the screen from afar did not grab it".into() }));
                        return None;
                    }
                    let s = shoulder(w.head, w.forward, 1);
                    let to = if resize {
                        // Stretch: so that the ray ends 35 % beyond the corner.
                        let target = sc.center + sc.rot * Vec3::new(sc.image_half_w * 1.35, sc.height / 2.0 * 1.35, 0.0);
                        s + (target - s).normalize_or_zero() * 0.40
                    } else {
                        // Bring the hand toward the shoulder: from 40 to 22 cm.
                        s + (at - s).normalize_or_zero() * 0.22
                    };
                    info!("selftest: {}", if resize { "corner grabbed; stretch it" } else { "screen grabbed; bring the hand closer" });
                    self.phase = Phase::ScPull { t: 0.0, from: at, to, resize, before };
                } else {
                    self.phase = Phase::ScAim { t, at, resize, before };
                }
            }
            Phase::ScPull { t, from, to, resize, before } => {
                let t = t + dt;
                let k = (t / 0.8).clamp(0.0, 1.0);
                self.point = from.lerp(to, k * k * (3.0 - 2.0 * k));
                self.pinching = t < 1.0;
                self.phase = if t > 1.0 { Phase::ScCheck { t: 0.0, resize, before } } else { Phase::ScPull { t, from, to, resize, before } };
            }
            Phase::ScCheck { t, resize, before } => {
                let t = t + dt;
                self.pinching = false;
                let Some(sc) = w.far_screen else {
                    self.finish(Err("the laptop screen disappeared".into()));
                    return None;
                };
                if t > 0.4 {
                    if !resize {
                        let now = sc.center.distance(w.head);
                        if now > before * 0.75 {
                            self.finish(Err(format!("the screen did not come closer ({before:.2} m -> {now:.2} m)")));
                            return None;
                        }
                        info!("selftest: the screen came from {before:.2} m to {now:.2} m; now stretch a corner");
                        let s = shoulder(w.head, w.forward, 1);
                        let corner = sc.center + sc.rot * Vec3::new(sc.image_half_w - 0.02, sc.height / 2.0 - 0.02, 0.0);
                        let at = s + (corner - s).normalize_or_zero() * 0.40;
                        self.phase = Phase::ScAim { t: 0.0, at, resize: true, before: sc.width };
                        self.result = None;
                        // Distance kept for the final message.
                        self.moved = Some((before, now));
                    } else {
                        if sc.width < before * 1.2 {
                            self.finish(Err(format!("the screen did not grow ({before:.2} m -> {:.2} m wide)", sc.width)));
                            return None;
                        }
                        let (d0, d1) = self.moved.unwrap_or((0.0, 0.0));
                        self.finish(Ok(format!(
                            "pulled from {d0:.2} m to {d1:.2} m with the ray, and stretched from {before:.2} m to {:.2} m wide",
                            sc.width
                        )));
                        return None;
                    }
                } else {
                    self.phase = Phase::ScCheck { t, resize, before };
                }
            }
            Phase::KbStart => {
                let Some(kb) = w.keyboard else {
                    self.finish(Err("the keyboard is not showing (press i to type)".into()));
                    return None;
                };
                info!("selftest: pinch the middle of the keyboard at {:.2}", kb.center);
                self.phase = Phase::KbMiddle { t: 0.0, start: kb.center, grabbed: false };
            }
            Phase::KbMiddle { t, start, grabbed } => {
                let Some(kb) = w.keyboard else {
                    self.finish(Err("the keyboard disappeared".into()));
                    return None;
                };
                let t = t + dt;
                // 3 cm above the plate: it is a pinch, not a key press.
                self.point = start + kb.normal * 0.03;
                self.pinching = (SETTLE..0.7).contains(&t);
                let grabbed = grabbed || kb.grabbed;
                if t > 1.0 {
                    let moved = kb.center.distance(start);
                    if grabbed || moved > 0.003 {
                        self.finish(Err(format!("pinching the middle of the keyboard moved it ({:.0} mm)", moved * 1000.0)));
                        return None;
                    }
                    info!("selftest: the middle does not move it; now the corner at {:.2}", kb.corner);
                    self.phase = Phase::KbGrab { t: 0.0, at: kb.corner + kb.normal * 0.03, start, back: false };
                } else {
                    self.phase = Phase::KbMiddle { t, start, grabbed };
                }
            }
            Phase::KbGrab { t, at, start, back } => {
                let t = t + dt;
                self.point = at;
                self.pinching = t > SETTLE;
                if t > SETTLE + 0.2 {
                    if !w.keyboard.is_some_and(|k| k.grabbed) {
                        self.pinching = false;
                        self.finish(Err("pinching the corner did not grab the keyboard".into()));
                        return None;
                    }
                    // Toward you, and back.
                    let to = if back { at + w.forward * 0.06 } else { at - w.forward * 0.06 };
                    info!("selftest: corner grabbed; drag it {}", if back { "back" } else { "6 cm toward you" });
                    self.phase = Phase::KbCarry { t: 0.0, from: at, to, start, back };
                } else {
                    self.phase = Phase::KbGrab { t, at, start, back };
                }
            }
            Phase::KbCarry { t, from, to, start, back } => {
                let t = t + dt;
                let k = (t / 0.6).clamp(0.0, 1.0);
                self.point = from.lerp(to, k * k * (3.0 - 2.0 * k));
                self.pinching = t < 0.8;
                self.phase = if t > 0.8 { Phase::KbCheck { t: 0.0, start, back } } else { Phase::KbCarry { t, from, to, start, back } };
            }
            Phase::KbCheck { t, start, back } => {
                let t = t + dt;
                self.pinching = false;
                let Some(kb) = w.keyboard else {
                    self.finish(Err("the keyboard disappeared".into()));
                    return None;
                };
                if t > 0.4 {
                    let moved = flat_distance(kb.center, start);
                    if !back {
                        if moved < 0.04 {
                            self.finish(Err(format!("dragging the corner moved it only {:.0} mm", moved * 1000.0)));
                            return None;
                        }
                        info!("selftest: it moved {:.0} mm; put it back", moved * 1000.0);
                        self.phase = Phase::KbGrab { t: 0.0, at: kb.corner + kb.normal * 0.03, start, back: true };
                    } else {
                        if moved > 0.01 {
                            self.finish(Err(format!("it did not come back to its place ({:.0} mm off)", moved * 1000.0)));
                            return None;
                        }
                        self.finish(Ok(format!(
                            "pinching the middle does nothing; a corner drags it 6 cm and back ({:.0} mm off)",
                            moved * 1000.0
                        )));
                        return None;
                    }
                } else {
                    self.phase = Phase::KbCheck { t, start, back };
                }
            }
            Phase::Release { t } => {
                let t = t + dt;
                self.pinching = false;
                if let Some(name) = &w.sent {
                    self.finish(Ok(format!("{name} dropped on the laptop screen and queued for the laptop")));
                    return None;
                }
                if t > 1.0 {
                    self.finish(Err("the card was released but nothing was sent".into()));
                    return None;
                }
                self.phase = Phase::Release { t };
            }
        }
        Some(self.hand())
    }

    /// The test hand: pinch at `point`, fingers curled (neither a fist nor an open palm).
    fn hand(&self) -> HandFrame {
        let p = self.point;
        let pinch = match (self.was_pinching, self.pinching) {
            (false, true) => PinchEvent::Started,
            (true, false) => PinchEvent::Ended,
            _ => PinchEvent::None,
        };
        let palm = p + Vec3::new(0.0, -0.03, 0.08);
        HandFrame {
            tracked: true,
            index_tip: p,
            pinch_point: p,
            pinch,
            pinching: self.pinching,
            palm,
            finger_tips: [p; 4],
            grasp_point: p,
            ..HandFrame::LOST_FRAME
        }
    }
}
