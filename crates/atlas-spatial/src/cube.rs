//! The cube of worlds: a box with glowing edges on the desk. A push down
//! opens it (the four side faces fold outward, the lid rises and fades) and the
//! orbs come out and form a ring above it: the selected one in front, large, and the others
//! around. A sweep with the open palm turns the ring. Data only: tested on the PC.

use glam::{Quat, Vec3};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// Edge (m) of the cube.
pub const SIZE: f32 = 0.16;
/// Minimum downward speed (m/s) of the hand for it to count as a push.
pub const PUSH_SPEED: f32 = 0.45;
/// Up to what height (m) above the lid the push is detected.
const PUSH_REACH: f32 = 0.14;
/// Seconds it takes to open or close.
const OPEN_TIME: f32 = 0.9;
/// Minimum seconds between two pushes.
const PUSH_COOLDOWN: f32 = 0.6;
/// Radius (m) of the orb ring and height (m) of the ring above the lid.
const RING_RADIUS: f32 = 0.17;
const RING_LIFT: f32 = 0.15;
/// How much the orbs' exit is staggered (0 = all at once).
const STAGGER: f32 = 0.5;
/// How much (m) the orbs rise halfway through their exit, so they come out in an arc.
const ARC_LIFT: f32 = 0.06;
/// How much (m) the lid rises when opening.
const LID_LIFT: f32 = 0.22;
/// How quickly the ring reaches its place when turning (higher = snappier).
const SPIN_RATE: f32 = 9.0;

/// One of the six faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Face {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
}

/// Where a face is. `rotation` applies to the orientation the face has with the cube closed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FacePose {
    pub face: Face,
    pub center: Vec3,
    pub rotation: Quat,
    /// Opacity of the face, from 0 to 1.
    pub alpha: f32,
}

/// Where an orb of the ring is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrbPose {
    pub index: usize,
    pub position: Vec3,
    /// Relative size: 1 for the front one, smaller for the ones behind.
    pub scale: f32,
    /// Progress of this orb's exit, from 0 (inside the cube) to 1 (on the ring).
    pub t: f32,
}

/// The cube: its place, its orientation and how open it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Cube {
    /// Center of the bottom face, on the desk.
    pub base: Vec3,
    /// Horizontal unit direction moving away from you.
    pub forward: Vec3,
    count: usize,
    selected: usize,
    /// Continuous position of the ring (fractional index), to turn smoothly.
    angle: f32,
    open: f32,
    target: f32,
    cooldown: f32,
}

impl Cube {
    pub fn new(base: Vec3, forward: Vec3, count: usize) -> Self {
        let forward = Vec3::new(forward.x, 0.0, forward.z).try_normalize().unwrap_or(Vec3::NEG_Z);
        Self { base, forward, count, selected: 0, angle: 0.0, open: 0.0, target: 0.0, cooldown: 0.0 }
    }

    /// Axis toward your right, seen from the cube toward `forward`.
    fn right(&self) -> Vec3 {
        self.forward.cross(Vec3::Y)
    }

    /// How open it is, from 0 to 1.
    pub fn progress(&self) -> f32 {
        self.open
    }

    /// The cube is (or will be) open.
    pub fn is_open(&self) -> bool {
        self.target > 0.0 || self.open > 0.0
    }

    /// It is already fully open and the ring can be turned.
    pub fn is_ready(&self) -> bool {
        self.target > 0.0 && self.open >= 1.0
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn count(&self) -> usize {
        self.count
    }

    /// Changes how many orbs there are (for example when loading the profiles).
    pub fn set_count(&mut self, count: usize) {
        self.count = count;
        self.selected = self.selected.min(count.saturating_sub(1));
        self.angle = self.angle.rem_euclid(count.max(1) as f32);
    }

    /// Height (m) of the lid above the desk.
    pub fn top(&self) -> f32 {
        self.base.y + SIZE
    }

    /// A hand at `hand` moving at `velocity` pushes the cube down: it opens or
    /// closes it. Returns whether the push counted.
    pub fn push(&mut self, hand: Vec3, velocity: Vec3) -> bool {
        let horizontal = Vec3::new(hand.x - self.base.x, 0.0, hand.z - self.base.z).length();
        let above = hand.y - self.top();
        let hit = horizontal <= SIZE * 0.75 && (-0.03..=PUSH_REACH).contains(&above) && velocity.y <= -PUSH_SPEED;
        if !hit || self.cooldown > 0.0 {
            return false;
        }
        self.target = if self.target > 0.0 { 0.0 } else { 1.0 };
        self.cooldown = PUSH_COOLDOWN;
        true
    }

    /// Moves `steps` orbs (positive = next) and returns the selected one. The ring is circular.
    pub fn step(&mut self, steps: i32) -> usize {
        if self.count > 0 {
            self.selected = (self.selected as i64 + steps as i64).rem_euclid(self.count as i64) as usize;
        }
        self.selected
    }

    /// Advances the animations.
    pub fn update(&mut self, dt: f32) {
        let dt = dt.max(0.0);
        self.cooldown = (self.cooldown - dt).max(0.0);
        let step = dt / OPEN_TIME;
        self.open = if self.open < self.target { (self.open + step).min(self.target) } else { (self.open - step).max(self.target) };
        if self.count > 0 {
            let n = self.count as f32;
            // By the shortest way: from n-1 to 0 the ring advances one step, not n-1 backward.
            let diff = (self.selected as f32 - self.angle + n / 2.0).rem_euclid(n) - n / 2.0;
            self.angle = (self.angle + diff * (1.0 - (-SPIN_RATE * dt).exp())).rem_euclid(n);
        }
    }

    /// The six faces according to how open the cube is.
    pub fn faces(&self) -> [FacePose; 6] {
        let e = smooth(self.open);
        let (f, r) = (self.forward, self.right());
        let half = SIZE / 2.0;
        let side = |face: Face, n: Vec3| {
            // Hinge on the bottom edge: the face turns until it lies flat outward.
            let hinge = self.base + n * half;
            let rotation = Quat::from_axis_angle(Vec3::Y.cross(n), e * FRAC_PI_2);
            FacePose { face, center: hinge + rotation * (Vec3::Y * half), rotation, alpha: 1.0 }
        };
        let lift = Vec3::Y * (SIZE + LID_LIFT * e);
        let tilt = Quat::from_axis_angle(r, -0.5 * e);
        [
            side(Face::Front, -f),
            side(Face::Back, f),
            side(Face::Left, -r),
            side(Face::Right, r),
            FacePose { face: Face::Top, center: self.base + lift, rotation: tilt, alpha: 1.0 - e },
            FacePose { face: Face::Bottom, center: self.base, rotation: Quat::IDENTITY, alpha: 1.0 },
        ]
    }

    /// The orbs: inside the cube at first, then on a ring above the lid with the
    /// selected one in front (the side facing you).
    pub fn orbs(&self) -> Vec<OrbPose> {
        let n = self.count;
        let (f, r) = (self.forward, self.right());
        let inside = self.base + Vec3::Y * (SIZE * 0.5);
        let centre = self.base + Vec3::Y * (SIZE + RING_LIFT);
        (0..n)
            .map(|i| {
                let t = (self.open * (1.0 + STAGGER) - i as f32 / n as f32 * STAGGER).clamp(0.0, 1.0);
                let ease = smooth(t);
                // 0 = front; the ring turns according to `angle`.
                let phi = (i as f32 - self.angle) / n as f32 * TAU;
                let ring = centre + (-f * phi.cos() + r * phi.sin()) * RING_RADIUS;
                let depth = (1.0 + phi.cos()) / 2.0;
                let arc = Vec3::Y * (PI * ease).sin() * ARC_LIFT;
                let scale = 0.12 + (0.5 + 0.5 * depth - 0.12) * ease;
                OrbPose { index: i, position: inside.lerp(ring, ease) + arc, scale, t }
            })
            .collect()
    }
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Detects the open-palm sweep that moves to the next orb.
#[derive(Debug, Clone, Copy, Default)]
pub struct PalmSwipe {
    last: Option<Vec3>,
    travel: f32,
    cooldown: f32,
}

/// Minimum lateral speed (m/s) to add to the sweep, distance (m) that completes it and pause (s).
const SWIPE_SPEED: f32 = 0.6;
const SWIPE_TRAVEL: f32 = 0.12;
const SWIPE_COOLDOWN: f32 = 0.45;

impl PalmSwipe {
    /// `palm` is the position of the palm, `open` whether the hand is open and extended, and `right`
    /// your right. Returns `Some(1)` if you sweep to the left (next, like turning a
    /// page) and `Some(-1)` to the right (previous).
    pub fn update(&mut self, palm: Vec3, open: bool, right: Vec3, dt: f32) -> Option<i32> {
        let dt = dt.max(1e-4);
        self.cooldown = (self.cooldown - dt).max(0.0);
        let last = self.last.replace(palm);
        let Some(last) = last.filter(|_| open) else {
            self.travel = 0.0;
            if !open {
                self.last = None;
            }
            return None;
        };
        let dx = (palm - last).dot(right);
        let speed = dx / dt;
        if speed.abs() < SWIPE_SPEED || (self.travel != 0.0 && dx.signum() != self.travel.signum()) {
            self.travel = if speed.abs() >= SWIPE_SPEED { dx } else { 0.0 };
            return None;
        }
        self.travel += dx;
        if self.travel.abs() >= SWIPE_TRAVEL && self.cooldown <= 0.0 {
            let dir = if self.travel < 0.0 { 1 } else { -1 };
            self.travel = 0.0;
            self.cooldown = SWIPE_COOLDOWN;
            return Some(dir);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(n: usize) -> Cube {
        // On the desk 0.5 m in front of you (toward -Z).
        Cube::new(Vec3::new(0.0, 0.72, -0.5), Vec3::NEG_Z, n)
    }

    fn open_fully(c: &mut Cube) {
        assert!(c.push(Vec3::new(0.0, c.top() + 0.05, -0.5), Vec3::new(0.0, -1.0, 0.0)));
        for _ in 0..200 {
            c.update(0.01);
        }
    }

    #[test]
    fn a_push_down_opens_and_a_second_one_closes() {
        let mut c = cube(4);
        assert!(!c.is_open());
        open_fully(&mut c);
        assert!(c.is_ready() && c.progress() == 1.0);
        assert!(c.push(Vec3::new(0.0, c.top() + 0.05, -0.5), Vec3::new(0.0, -1.0, 0.0)));
        for _ in 0..200 {
            c.update(0.01);
        }
        assert!(!c.is_open() && c.progress() == 0.0);
    }

    #[test]
    fn slow_off_target_or_upward_hands_do_not_push() {
        let mut c = cube(4);
        let on = Vec3::new(0.0, c.top() + 0.05, -0.5);
        assert!(!c.push(on, Vec3::new(0.0, -0.1, 0.0)), "demasiado lenta");
        assert!(!c.push(on, Vec3::new(0.0, 1.0, 0.0)), "hacia arriba");
        assert!(!c.push(Vec3::new(0.4, c.top() + 0.05, -0.5), Vec3::new(0.0, -1.0, 0.0)), "outside the cube");
        assert!(!c.push(Vec3::new(0.0, c.top() + 0.6, -0.5), Vec3::new(0.0, -1.0, 0.0)), "demasiado alta");
        assert!(!c.is_open());
    }

    #[test]
    fn pushes_have_a_cooldown() {
        let mut c = cube(4);
        let on = Vec3::new(0.0, c.top() + 0.05, -0.5);
        let down = Vec3::new(0.0, -1.0, 0.0);
        assert!(c.push(on, down));
        assert!(!c.push(on, down), "the same hit does not count twice");
        c.update(PUSH_COOLDOWN + 0.01);
        assert!(c.push(on, down));
    }

    #[test]
    fn closed_faces_form_a_box_and_open_ones_lie_flat() {
        let mut c = cube(4);
        let closed = c.faces();
        let front = closed[0];
        assert_eq!(front.face, Face::Front);
        // Closed: the front face (toward you, +Z) is in the middle of its side.
        assert!((front.center - (c.base + Vec3::new(0.0, SIZE / 2.0, SIZE / 2.0))).length() < 1e-5);
        assert_eq!(closed[4].alpha, 1.0, "the lid is visible");

        open_fully(&mut c);
        let open = c.faces();
        for p in &open[..4] {
            assert!(p.center.y - c.base.y < 1e-4, "{:?} must lie flat on the desk", p.face);
        }
        // Lying flat outward: farther from the center than before.
        let dist = |p: &FacePose| Vec3::new(p.center.x - c.base.x, 0.0, p.center.z - c.base.z).length();
        assert!(dist(&open[0]) > dist(&closed[0]) + SIZE * 0.4);
        assert_eq!(open[4].alpha, 0.0, "the lid fades");
        assert!(open[4].center.y > closed[4].center.y);
        assert_eq!(open[5].alpha, 1.0, "the base stays");
    }

    #[test]
    fn orbs_start_inside_and_end_in_a_ring_with_the_selected_in_front() {
        let mut c = cube(5);
        let inside = c.orbs();
        assert!(inside.iter().all(|o| o.t == 0.0 && o.scale < 0.2));
        assert!(inside.iter().all(|o| o.position.y < c.top()), "inside the cube");

        open_fully(&mut c);
        let ring = c.orbs();
        assert!(ring.iter().all(|o| o.t == 1.0));
        let front = ring.iter().find(|o| o.index == c.selected()).unwrap();
        assert!((front.scale - 1.0).abs() < 1e-4);
        // In front = the side toward you (+Z, since the cube faces -Z) and closer than the others.
        assert!(ring.iter().filter(|o| o.index != front.index).all(|o| front.position.z > o.position.z - 1e-5));
        assert!(ring.iter().filter(|o| o.index != front.index).all(|o| o.scale < front.scale));
        assert!(front.position.y > c.top());
    }

    #[test]
    fn stepping_wraps_and_the_ring_turns_by_the_short_way() {
        let mut c = cube(5);
        open_fully(&mut c);
        assert_eq!(c.step(-1), 4, "from the first it goes to the last");
        for _ in 0..100 {
            c.update(0.02);
        }
        let front = c.orbs().into_iter().find(|o| o.index == 4).unwrap();
        assert!((front.scale - 1.0).abs() < 1e-3, "the newly selected one has reached the front");
        assert_eq!(c.step(1), 0);
        assert_eq!(c.step(7), 2);
    }

    #[test]
    fn no_orbs_is_safe() {
        let mut c = cube(0);
        c.update(0.1);
        assert!(c.orbs().is_empty());
        assert_eq!(c.step(1), 0);
        c.set_count(3);
        assert_eq!(c.count(), 3);
    }

    fn sweep(swipe: &mut PalmSwipe, from: f32, to: f32, secs: f32, open: bool) -> Vec<Option<i32>> {
        let frames = (secs / 0.011) as usize;
        (0..=frames)
            .map(|k| {
                let x = from + (to - from) * k as f32 / frames as f32;
                swipe.update(Vec3::new(x, 1.0, -0.4), open, Vec3::X, 0.011)
            })
            .collect()
    }

    #[test]
    fn an_open_palm_swipe_left_goes_next_and_right_goes_previous() {
        let mut s = PalmSwipe::default();
        let left = sweep(&mut s, 0.1, -0.15, 0.25, true);
        assert_eq!(left.iter().flatten().collect::<Vec<_>>(), vec![&1]);
        let mut s = PalmSwipe::default();
        let right = sweep(&mut s, -0.15, 0.1, 0.25, true);
        assert_eq!(right.iter().flatten().collect::<Vec<_>>(), vec![&-1]);
    }

    #[test]
    fn slow_drifts_and_closed_hands_do_not_swipe() {
        let mut s = PalmSwipe::default();
        assert!(sweep(&mut s, 0.1, -0.2, 3.0, true).iter().all(Option::is_none), "movimiento lento");
        let mut s = PalmSwipe::default();
        assert!(sweep(&mut s, 0.1, -0.2, 0.25, false).iter().all(Option::is_none), "hand not open");
    }
}
