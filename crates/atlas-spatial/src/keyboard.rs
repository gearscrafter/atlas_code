//! Atlas's own keyboard: a flat panel on the desk touched with the index fingertip. It has a
//! row of Vim keys (Esc, Tab, Ctrl, `:`, `/` and arrows), letters, a symbols layer and, on the
//! right, a trackpad with a scroll strip. Data only: tested on the PC.

use glam::{Quat, Vec3};

/// Spacing (m) between key centers and the height of a row.
const PITCH: f32 = 0.040;
/// Hueco (m) entre teclas contiguas.
const GAP: f32 = 0.004;
/// How much (rad) the panel is tilted from vertical: a quarter turn leaves it horizontal, parallel
/// to the desk, with the Vim row farthest from you.
pub const TILT: f32 = std::f32::consts::FRAC_PI_2;
/// Distance (m) in front of the panel from which a finger is "armed" to press.
const ARM: f32 = 0.02;
/// Distance (m) at which the finger presses, and how far behind is allowed (beyond that, it came from behind).
const PRESS: f32 = 0.006;
const BEHIND: f32 = -0.03;
/// Fingers tracked at once (the index fingers of both hands and a margin).
pub const FINGERS: usize = 4;
/// How much (m) the keys are shifted left of the panel's center: the trackpad goes on the right.
pub const KEYS_SHIFT: f32 = -0.095;
/// Trackpad: center and size (m) on the panel, and the width of its scroll strip (the right edge).
pub const PAD_X: f32 = 0.25;
pub const PAD_Y: f32 = 0.0;
pub const PAD_W: f32 = 0.17;
pub const PAD_H: f32 = 0.14;
pub const SCROLL_W: f32 = 0.03;
/// Margin (m) of the plate around the keys and the trackpad (the same the panel draws).
pub const PLATE_PAD: f32 = 0.014;
/// Distance (m) to a corner of the plate at which a pinch grabs the keyboard. Only the corners move it:
/// that way a pinch near the keys or the trackpad does not drag it by accident.
pub const GRIP_RADIUS: f32 = 0.04;
/// Distance (m) at which the corner lights up as the hand approaches, to show that it can be pinched.
pub const GRIP_NEAR: f32 = 0.09;
/// Distance (m) at which the finger touches the trackpad, and at which it releases it (hysteresis: hand
/// tracking trembles a couple of centimeters).
const PAD_ENTER: f32 = 0.010;
const PAD_LEAVE: f32 = 0.025;
/// A tap lasts at most `TAP_TIME` seconds and moves at most `TAP_MOVE` meters.
const TAP_TIME: f32 = 0.30;
const TAP_MOVE: f32 = 0.012;

/// What the keyboard presses, with Shift and Ctrl already applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Char(char),
    /// Ctrl + letter (lowercase).
    Ctrl(char),
    Enter,
    Backspace,
    Escape,
    Tab,
    Left,
    Right,
    Up,
    Down,
}

/// What the finger does on the trackpad, in meters on the panel (X right, Y up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PadEvent {
    /// Sliding the finger: moves the cursor.
    Move { dx: f32, dy: f32 },
    /// Sliding along the strip on the right edge: scrolls.
    Scroll { dy: f32 },
    /// Touching and releasing without moving.
    Tap,
}

#[derive(Debug, Clone, Copy, Default)]
struct PadFinger {
    down: bool,
    scroll: bool,
    last: (f32, f32),
    time: f32,
    moved: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Letters,
    Symbols,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cap {
    Char(char),
    Act(KeyAction),
    Shift,
    Ctrl,
    Layer,
}

/// A drawable key: its position on the panel (X right, Y up, from the center) and size.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyRect {
    pub label: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// The key is active (Shift or Ctrl armed).
    pub lit: bool,
    cap: Cap,
}

pub struct Keyboard {
    pub center: Vec3,
    right: Vec3,
    up: Vec3,
    normal: Vec3,
    layer: Layer,
    shift: bool,
    ctrl: bool,
    armed: [bool; FINGERS],
    pad: [PadFinger; FINGERS],
}

impl Keyboard {
    /// `center`: center of the panel. `toward_user`: (almost horizontal) direction toward you.
    pub fn new(center: Vec3, toward_user: Vec3) -> Self {
        let flat = Vec3::new(toward_user.x, 0.0, toward_user.z).try_normalize().unwrap_or(Vec3::Z);
        let right = Vec3::Y.cross(flat);
        let tilt = Quat::from_axis_angle(right, -TILT);
        Self {
            center,
            right,
            up: tilt * flat.cross(right),
            normal: tilt * flat,
            layer: Layer::Letters,
            shift: false,
            ctrl: false,
            armed: [false; FINGERS],
            pad: [PadFinger::default(); FINGERS],
        }
    }

    pub fn layer(&self) -> Layer {
        self.layer
    }

    pub fn shift(&self) -> bool {
        self.shift
    }

    pub fn ctrl(&self) -> bool {
        self.ctrl
    }

    /// Axes of the panel in the world: right, up and normal (toward you).
    pub fn axes(&self) -> (Vec3, Vec3, Vec3) {
        (self.right, self.up, self.normal)
    }

    /// The keys of the current layer.
    pub fn keys(&self) -> Vec<KeyRect> {
        let letters = self.layer == Layer::Letters;
        let mut keys = Vec::new();
        let mut row = |y: f32, items: Vec<(Cap, String, f32)>| {
            let total: f32 = items.iter().map(|i| i.2).sum::<f32>() * PITCH;
            let mut x = -total / 2.0;
            for (cap, label, units) in items {
                let w = units * PITCH;
                let lit = (cap == Cap::Shift && self.shift) || (cap == Cap::Ctrl && self.ctrl);
                keys.push(KeyRect { label, x: x + w / 2.0 + KEYS_SHIFT, y, w: w - GAP, h: PITCH - GAP, lit, cap });
                x += w;
            }
        };
        let ch = |c: char| (Cap::Char(c), c.to_string(), 1.0);
        let act = |a: KeyAction, l: &str, u: f32| (Cap::Act(a), l.to_string(), u);
        let chars = |s: &str| -> Vec<(Cap, String, f32)> {
            s.chars().map(|c| if letters && self.shift { ch(c.to_ascii_uppercase()) } else { ch(c) }).collect()
        };

        // Vim row, the same in both layers.
        row(
            2.0 * PITCH,
            vec![
                act(KeyAction::Escape, "Esc", 1.5),
                act(KeyAction::Tab, "Tab", 1.5),
                (Cap::Ctrl, "Ctrl".into(), 1.5),
                ch(':'),
                ch('/'),
                act(KeyAction::Left, "<", 1.0),
                act(KeyAction::Down, "v", 1.0),
                act(KeyAction::Up, "^", 1.0),
                act(KeyAction::Right, ">", 1.0),
                act(KeyAction::Backspace, "Bksp", 1.5),
            ],
        );
        if letters {
            row(PITCH, chars("qwertyuiop"));
            row(0.0, chars("asdfghjkl;"));
            let mut r3 = vec![(Cap::Shift, "Shift".into(), 1.5)];
            r3.extend(chars("zxcvbnm,."));
            row(-PITCH, r3);
        } else {
            row(PITCH, chars("1234567890"));
            row(0.0, chars("!@#$%^&*()"));
            row(-PITCH, chars("'\"[]{}<>=+"));
        }
        row(
            -2.0 * PITCH,
            vec![
                (Cap::Layer, if letters { "123".into() } else { "abc".into() }, 1.5),
                ch('-'),
                ch('_'),
                (Cap::Char(' '), "Space".into(), 3.5),
                ch('|'),
                act(KeyAction::Enter, "Enter", 2.0),
            ],
        );
        keys
    }

    /// Position of a world point on the panel: (x right, y up, z in front of the panel).
    fn local(&self, p: Vec3) -> Vec3 {
        let d = p - self.center;
        Vec3::new(d.dot(self.right), d.dot(self.up), d.dot(self.normal))
    }

    /// Is `p` over the keyboard's plate (to grab it and move it with a pinch)?
    pub fn on_plate(&self, p: Vec3) -> bool {
        let l = self.local(p);
        l.x.abs() <= 0.37 && l.y.abs() <= 0.15 && l.z.abs() <= 0.06
    }

    /// The corners of the plate (top left, top right, bottom left, bottom right), in panel
    /// meters (X right, Y up, from its center).
    pub fn plate_corners(&self) -> [(f32, f32); 4] {
        let (mut l, mut r, mut t, mut b) = (PAD_X - PAD_W / 2.0, PAD_X + PAD_W / 2.0, PAD_Y + PAD_H / 2.0, PAD_Y - PAD_H / 2.0);
        for k in self.keys() {
            l = l.min(k.x - k.w / 2.0);
            r = r.max(k.x + k.w / 2.0);
            t = t.max(k.y + k.h / 2.0);
            b = b.min(k.y - k.h / 2.0);
        }
        let (l, r, t, b) = (l - PLATE_PAD, r + PLATE_PAD, t + PLATE_PAD, b - PLATE_PAD);
        [(l, t), (r, t), (l, b), (r, b)]
    }

    /// The plate corner within `radius` of `p` (measured over the keyboard, with `p` a few cm from it), the
    /// closest one if there are several.
    pub fn corner_near(&self, p: Vec3, radius: f32) -> Option<usize> {
        let l = self.local(p);
        if l.z.abs() > 0.08 {
            return None;
        }
        self.plate_corners()
            .iter()
            .enumerate()
            .map(|(i, (x, y))| (i, ((l.x - x).powi(2) + (l.y - y).powi(2)).sqrt()))
            .filter(|(_, d)| *d <= radius)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    fn hit(keys: &[KeyRect], x: f32, y: f32) -> Option<usize> {
        keys.iter().position(|k| (x - k.x).abs() <= k.w / 2.0 + GAP / 2.0 && (y - k.y).abs() <= k.h / 2.0 + GAP / 2.0)
    }

    /// The key under the fingertip, if it is near the panel (to light it up on approach).
    pub fn hover(&self, tip: Vec3) -> Option<usize> {
        let l = self.local(tip);
        if !(BEHIND..=0.06).contains(&l.z) {
            return None;
        }
        Self::hit(&self.keys(), l.x, l.y)
    }

    /// A finger (`finger`, 0..FINGERS) with its tip at `tip`. It presses a key when crossing the panel
    /// from the front; it has to move away again to press again.
    pub fn touch(&mut self, finger: usize, tip: Vec3) -> Option<KeyAction> {
        let finger = finger.min(FINGERS - 1);
        let l = self.local(tip);
        if l.z >= ARM {
            self.armed[finger] = true;
            return None;
        }
        if l.z < BEHIND {
            self.armed[finger] = false;
            return None;
        }
        if l.z > PRESS || !self.armed[finger] {
            return None;
        }
        let keys = self.keys();
        let index = Self::hit(&keys, l.x, l.y)?;
        self.armed[finger] = false;
        self.press(keys[index].cap)
    }

    /// A finger with its tip at `tip` over the trackpad. Each frame it returns what it did: sliding
    /// moves, the strip on the right edge scrolls, and resting and lifting without moving is a tap.
    pub fn pad_touch(&mut self, finger: usize, tip: Vec3, dt: f32) -> Vec<PadEvent> {
        let finger = finger.min(FINGERS - 1);
        let l = self.local(tip);
        let inside = (l.x - PAD_X).abs() <= PAD_W / 2.0 && (l.y - PAD_Y).abs() <= PAD_H / 2.0;
        let st = &mut self.pad[finger];
        let reach = if st.down { PAD_LEAVE } else { PAD_ENTER };
        let contact = inside && (BEHIND..=reach).contains(&l.z);
        let mut out = Vec::new();
        match (st.down, contact) {
            (false, true) => {
                *st = PadFinger { down: true, scroll: l.x >= PAD_X + PAD_W / 2.0 - SCROLL_W, last: (l.x, l.y), time: 0.0, moved: 0.0 };
            }
            (true, true) => {
                let (dx, dy) = (l.x - st.last.0, l.y - st.last.1);
                st.last = (l.x, l.y);
                st.time += dt.max(0.0);
                st.moved += (dx * dx + dy * dy).sqrt();
                out.push(if st.scroll { PadEvent::Scroll { dy } } else { PadEvent::Move { dx, dy } });
            }
            (true, false) => {
                if !st.scroll && st.time <= TAP_TIME && st.moved <= TAP_MOVE {
                    out.push(PadEvent::Tap);
                }
                st.down = false;
            }
            (false, false) => {}
        }
        out
    }

    /// The hand of `finger` is no longer visible: whatever it was doing on the trackpad is cancelled.
    pub fn pad_lost(&mut self, finger: usize) {
        self.pad[finger.min(FINGERS - 1)].down = false;
    }

    /// Where the fingers touch the trackpad now (X right, Y up, on the panel), to draw them.
    pub fn pad_contacts(&self) -> Vec<(f32, f32)> {
        self.pad.iter().filter(|f| f.down).map(|f| f.last).collect()
    }

    fn press(&mut self, cap: Cap) -> Option<KeyAction> {
        let action = match cap {
            Cap::Shift => {
                self.shift = !self.shift;
                return None;
            }
            Cap::Ctrl => {
                self.ctrl = !self.ctrl;
                return None;
            }
            Cap::Layer => {
                self.layer = if self.layer == Layer::Letters { Layer::Symbols } else { Layer::Letters };
                return None;
            }
            Cap::Char(c) if self.ctrl && c.is_ascii_alphabetic() => KeyAction::Ctrl(c.to_ascii_lowercase()),
            Cap::Char(c) if self.shift && self.layer == Layer::Letters => KeyAction::Char(c.to_ascii_uppercase()),
            Cap::Char(c) => KeyAction::Char(c),
            Cap::Act(a) => a,
        };
        // Shift and Ctrl count for a single key press.
        self.shift = false;
        self.ctrl = false;
        Some(action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_corners_of_the_plate_grab_the_keyboard() {
        let kb = Keyboard::new(Vec3::new(0.0, 0.75, -0.4), Vec3::Z);
        let (right, up, normal) = kb.axes();
        let at = |x: f32, y: f32, z: f32| kb.center + right * x + up * y + normal * z;
        let corners = kb.plate_corners();
        for (i, (x, y)) in corners.iter().enumerate() {
            assert_eq!(kb.corner_near(at(*x, *y, 0.02), GRIP_RADIUS), Some(i), "right at corner {i}");
            // A little inside the plate, and a little outside, too.
            assert_eq!(kb.corner_near(at(x - x.signum() * 0.02, y - y.signum() * 0.02, 0.0), GRIP_RADIUS), Some(i));
            assert_eq!(kb.corner_near(at(x + x.signum() * 0.02, *y, 0.0), GRIP_RADIUS), Some(i));
            assert_eq!(kb.corner_near(at(*x, *y, 0.15), GRIP_RADIUS), None, "far above the keyboard, no");
        }
        assert_eq!(kb.corner_near(kb.center, GRIP_RADIUS), None, "the center of the plate no longer grabs it");
        let (tx, ty) = corners[1];
        assert_eq!(kb.corner_near(at(tx * 0.5, ty, 0.0), GRIP_RADIUS), None, "nor does the edge, far from the corners");
        assert!(kb.on_plate(kb.center), "the plate is still the plate (so the drawer does not open over it)");
    }

    #[test]
    fn the_corner_grips_stay_clear_of_the_corner_keys() {
        // Esc is at the top-left corner and Enter at the bottom-right of the letter block:
        // their centers stay outside the grab zone, so that pressing near them does not move the keyboard.
        let kb = Keyboard::new(Vec3::ZERO, Vec3::Z);
        let (right, up, _) = kb.axes();
        for key in kb.keys() {
            let c = right * key.x + up * key.y;
            assert_eq!(kb.corner_near(c, GRIP_RADIUS), None, "key {} falls on a corner", key.label);
        }
    }

    fn keyboard() -> Keyboard {
        // On the desk 0.45 m in front of you (toward -Z), facing you (+Z).
        Keyboard::new(Vec3::new(0.0, 0.75, -0.45), Vec3::Z)
    }

    /// Fingertip over the key `label`, `z` meters in front of the panel.
    fn over(kb: &Keyboard, label: &str, z: f32) -> Vec3 {
        let k = kb.keys().into_iter().find(|k| k.label == label).unwrap_or_else(|| panic!("no key {label}"));
        let (r, u, n) = kb.axes();
        kb.center + r * k.x + u * k.y + n * z
    }

    /// Brings the finger closer, crosses the panel and returns what was pressed.
    fn tap(kb: &mut Keyboard, finger: usize, label: &str) -> Option<KeyAction> {
        assert_eq!(kb.touch(finger, over(kb, label, 0.05)), None);
        kb.touch(finger, over(kb, label, 0.0))
    }

    #[test]
    fn axes_are_orthonormal_and_the_panel_lies_flat() {
        let kb = keyboard();
        let (r, u, n) = kb.axes();
        assert!((r.length() - 1.0).abs() < 1e-5 && (u.length() - 1.0).abs() < 1e-5 && (n.length() - 1.0).abs() < 1e-5);
        assert!(r.dot(u).abs() < 1e-5 && r.dot(n).abs() < 1e-5 && u.dot(n).abs() < 1e-5);
        assert!(r.x > 0.9, "the right of the panel is your right");
        assert!(n.y > 0.999, "parallel to the desk: the normal points up");
        assert!(u.z < -0.999, "the top edge (Vim row) is farthest from you");
    }

    #[test]
    fn keys_do_not_overlap_and_cover_the_alphabet() {
        let kb = keyboard();
        let keys = kb.keys();
        for (i, a) in keys.iter().enumerate() {
            for b in &keys[i + 1..] {
                let apart = (a.x - b.x).abs() >= (a.w + b.w) / 2.0 || (a.y - b.y).abs() >= (a.h + b.h) / 2.0;
                assert!(apart, "{} overlaps {}", a.label, b.label);
            }
        }
        for c in 'a'..='z' {
            assert!(keys.iter().any(|k| k.label == c.to_string()), "falta {c}");
        }
        for l in ["Esc", "Tab", "Ctrl", ":", "/", "Space", "Enter", "Bksp"] {
            assert!(keys.iter().any(|k| k.label == l), "falta {l}");
        }
        let right = keys.iter().map(|k| k.x + k.w / 2.0).fold(f32::MIN, f32::max);
        assert!(right < PAD_X - PAD_W / 2.0, "the keys do not intrude on the trackpad ({right} m)");
        let left = keys.iter().map(|k| k.x - k.w / 2.0).fold(f32::MAX, f32::min);
        assert!(left > -0.37, "everything fits on the panel ({left} m)");
    }

    #[test]
    fn tapping_a_key_types_it_once() {
        let mut kb = keyboard();
        assert_eq!(tap(&mut kb, 0, "a"), Some(KeyAction::Char('a')));
        // With the finger inside it does not repeat, not even over another key.
        assert_eq!(kb.touch(0, over(&kb, "a", -0.004)), None);
        assert_eq!(kb.touch(0, over(&kb, "s", 0.0)), None);
        // After moving away and coming back, it does.
        assert_eq!(tap(&mut kb, 0, "s"), Some(KeyAction::Char('s')));
    }

    #[test]
    fn a_finger_that_comes_from_behind_or_starts_inside_does_not_press() {
        let mut kb = keyboard();
        assert_eq!(kb.touch(0, over(&kb, "a", 0.0)), None, "it starts inside: it is not armed");
        assert_eq!(kb.touch(0, over(&kb, "a", -0.06)), None);
        assert_eq!(kb.touch(0, over(&kb, "a", 0.0)), None, "it came from behind");
    }

    #[test]
    fn fingers_are_independent() {
        let mut kb = keyboard();
        assert_eq!(kb.touch(0, over(&kb, "j", 0.05)), None);
        assert_eq!(kb.touch(1, over(&kb, "k", 0.05)), None);
        assert_eq!(kb.touch(1, over(&kb, "k", 0.0)), Some(KeyAction::Char('k')));
        assert_eq!(kb.touch(0, over(&kb, "j", 0.0)), Some(KeyAction::Char('j')));
    }

    #[test]
    fn shift_and_ctrl_apply_to_one_key() {
        let mut kb = keyboard();
        assert_eq!(tap(&mut kb, 0, "Shift"), None);
        assert!(kb.shift() && kb.keys().iter().any(|k| k.label == "Q"), "the letters show in upper case");
        assert_eq!(tap(&mut kb, 0, "Q"), Some(KeyAction::Char('Q')));
        assert!(!kb.shift());
        assert_eq!(tap(&mut kb, 0, "w"), Some(KeyAction::Char('w')));

        assert_eq!(tap(&mut kb, 0, "Ctrl"), None);
        assert_eq!(tap(&mut kb, 0, "r"), Some(KeyAction::Ctrl('r')));
        assert!(!kb.ctrl());
        assert_eq!(tap(&mut kb, 0, "r"), Some(KeyAction::Char('r')));
    }

    #[test]
    fn vim_row_and_editing_keys() {
        let mut kb = keyboard();
        assert_eq!(tap(&mut kb, 0, "Esc"), Some(KeyAction::Escape));
        assert_eq!(tap(&mut kb, 0, ":"), Some(KeyAction::Char(':')));
        assert_eq!(tap(&mut kb, 0, "Bksp"), Some(KeyAction::Backspace));
        assert_eq!(tap(&mut kb, 0, "Enter"), Some(KeyAction::Enter));
        assert_eq!(tap(&mut kb, 0, "Space"), Some(KeyAction::Char(' ')));
        assert_eq!(tap(&mut kb, 0, "<"), Some(KeyAction::Left));
        assert_eq!(tap(&mut kb, 0, "^"), Some(KeyAction::Up));
    }

    #[test]
    fn the_symbol_layer_has_digits_and_brackets() {
        let mut kb = keyboard();
        assert_eq!(tap(&mut kb, 0, "123"), None);
        assert_eq!(kb.layer(), Layer::Symbols);
        assert_eq!(tap(&mut kb, 0, "5"), Some(KeyAction::Char('5')));
        assert_eq!(tap(&mut kb, 0, "{"), Some(KeyAction::Char('{')));
        assert_eq!(tap(&mut kb, 0, "Esc"), Some(KeyAction::Escape), "the Vim row stays");
        assert_eq!(tap(&mut kb, 0, "abc"), None);
        assert_eq!(kb.layer(), Layer::Letters);
    }

    #[test]
    fn the_plate_can_be_grabbed_where_the_keys_are_but_not_far_away() {
        let kb = keyboard();
        assert!(kb.on_plate(over(&kb, "g", 0.01)));
        assert!(!kb.on_plate(over(&kb, "g", 0.3)), "far from the panel");
        assert!(!kb.on_plate(kb.center + Vec3::X * 0.5), "outside the plate");
    }

    #[test]
    fn hover_only_near_the_panel_and_on_a_key() {
        let kb = keyboard();
        let near = over(&kb, "g", 0.03);
        let index = kb.hover(near).expect("over the g");
        assert_eq!(kb.keys()[index].label, "g");
        assert_eq!(kb.hover(over(&kb, "g", 0.4)), None, "demasiado lejos");
        assert_eq!(kb.hover(near + kb.axes().0 * 0.5), None, "outside the keyboard");
    }

    /// Fingertip over the trackpad at (x, y) of the panel, `z` meters in front.
    fn on_pad(kb: &Keyboard, x: f32, y: f32, z: f32) -> Vec3 {
        let (r, u, n) = kb.axes();
        kb.center + r * x + u * y + n * z
    }

    #[test]
    fn sliding_on_the_pad_moves_and_tapping_taps() {
        let mut kb = keyboard();
        assert!(kb.pad_touch(0, on_pad(&kb, PAD_X, 0.0, 0.05), 0.011).is_empty(), "in the air it does nothing");
        assert!(kb.pad_touch(0, on_pad(&kb, PAD_X, 0.0, 0.004), 0.011).is_empty(), "resting it does not move");
        let ev = kb.pad_touch(0, on_pad(&kb, PAD_X + 0.02, 0.01, 0.004), 0.011);
        assert_eq!(ev.len(), 1);
        match ev[0] {
            PadEvent::Move { dx, dy } => assert!((dx - 0.02).abs() < 1e-5 && (dy - 0.01).abs() < 1e-5, "{dx} {dy}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(kb.pad_contacts().len(), 1);
        // Lifting after moving a lot is not a tap.
        assert!(kb.pad_touch(0, on_pad(&kb, PAD_X + 0.02, 0.01, 0.05), 0.011).is_empty());
        assert!(kb.pad_contacts().is_empty());

        // A tap: resting and lifting right away without moving.
        let mut kb = keyboard();
        kb.pad_touch(0, on_pad(&kb, PAD_X, 0.0, 0.004), 0.011);
        kb.pad_touch(0, on_pad(&kb, PAD_X, 0.0, 0.004), 0.011);
        assert_eq!(kb.pad_touch(0, on_pad(&kb, PAD_X, 0.0, 0.05), 0.011), vec![PadEvent::Tap]);
    }

    #[test]
    fn a_long_press_is_not_a_tap_and_the_scroll_strip_scrolls() {
        let mut kb = keyboard();
        for _ in 0..40 {
            kb.pad_touch(0, on_pad(&kb, PAD_X, 0.0, 0.004), 0.011);
        }
        assert!(kb.pad_touch(0, on_pad(&kb, PAD_X, 0.0, 0.05), 0.011).is_empty(), "demasiado largo");

        let edge = PAD_X + PAD_W / 2.0 - SCROLL_W / 2.0;
        let mut kb = keyboard();
        kb.pad_touch(0, on_pad(&kb, edge, 0.0, 0.004), 0.011);
        let ev = kb.pad_touch(0, on_pad(&kb, edge, -0.02, 0.004), 0.011);
        assert!(matches!(ev[..], [PadEvent::Scroll { dy }] if (dy + 0.02).abs() < 1e-5), "{ev:?}");
        assert!(kb.pad_touch(0, on_pad(&kb, edge, -0.02, 0.05), 0.011).is_empty(), "the strip gives no taps");
    }

    #[test]
    fn the_pad_has_hysteresis_ignores_other_places_and_a_lost_hand_cancels() {
        let mut kb = keyboard();
        kb.pad_touch(0, on_pad(&kb, PAD_X, 0.0, 0.004), 0.011);
        // At 2 cm it is still resting (tracking jitter); at 3 cm it releases.
        assert_eq!(kb.pad_touch(0, on_pad(&kb, PAD_X + 0.01, 0.0, 0.02), 0.011).len(), 1);
        kb.pad_touch(0, on_pad(&kb, PAD_X + 0.01, 0.0, 0.03), 0.011);
        assert!(kb.pad_contacts().is_empty());
        // Over the keys or outside the trackpad it does not count.
        assert!(kb.pad_touch(0, over(&kb, "g", 0.004), 0.011).is_empty());
        assert!(kb.pad_touch(0, on_pad(&kb, PAD_X + 0.3, 0.0, 0.004), 0.011).is_empty());
        // Neither does a finger from behind the panel.
        assert!(kb.pad_touch(0, on_pad(&kb, PAD_X, 0.0, -0.1), 0.011).is_empty());
        // Hand lost: no tap.
        kb.pad_touch(1, on_pad(&kb, PAD_X, 0.0, 0.004), 0.011);
        kb.pad_lost(1);
        assert!(kb.pad_contacts().is_empty());
        assert!(kb.pad_touch(1, on_pad(&kb, PAD_X, 0.0, 0.05), 0.011).is_empty());
    }
}
