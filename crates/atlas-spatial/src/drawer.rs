//! The file drawer: a handle on the desk that you pull toward you. When it opens, the cards come out of the
//! drawer and stand on the desk, in a tidy fan in front of you (like a desk card file),
//! below the editor. A few are visible at a time and you page through them sweeping with your palm. Above them goes
//! a title card with the current folder. Data only: tested on the PC.

use glam::{Quat, Vec3};
use std::f32::consts::PI;

/// How much (m) you have to pull to open the drawer fully.
pub const TRAVEL: f32 = 0.28;
/// If more than this fraction was pulled on release, the drawer finishes opening; otherwise it closes.
pub const SNAP: f32 = 0.45;
/// Speed (m/s) at which the drawer opens or closes by itself.
const SLIDE_SPEED: f32 = 0.9;
/// Cards visible at a time in the card file; with more, you page one by one sweeping with your palm.
pub const PAGE: usize = 5;
/// Height over width of a card (each card's texture is 512 x 384).
pub const CARD_ASPECT: f32 = 0.75;
/// Spacing between adjacent cards, in card widths: a little less than 1, they overlap a bit like a fan.
const SPACING: f32 = 0.92;
/// Gap (m) between the bottom edge of a card and the desk.
const ABOVE_DESK: f32 = 0.02;
/// The chosen card moves forward (m), rises (m) and grows a little.
const SELECTED_FORWARD: f32 = 0.03;
const SELECTED_RISE: f32 = 0.015;
const SELECTED_SCALE: f32 = 1.12;
/// The title card goes above the fan, somewhat smaller.
const TITLE_SCALE: f32 = 0.8;
/// How much the cards' exit is staggered (0 = all at once).
const STAGGER: f32 = 0.6;
/// How much (m) the cards rise halfway through their flight, so they come out in an arc.
const ARC_LIFT: f32 = 0.14;

/// The drawer: its closed place, the direction it is pulled and how open it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drawer {
    /// Position of the handle with the drawer closed.
    pub closed: Vec3,
    /// Horizontal unit direction moving away from you: when pulling, the handle goes against it.
    pub forward: Vec3,
    pull: f32,
    target: f32,
    held: bool,
}

impl Drawer {
    pub fn new(closed: Vec3, forward: Vec3) -> Self {
        let forward = Vec3::new(forward.x, 0.0, forward.z).try_normalize().unwrap_or(Vec3::NEG_Z);
        Self { closed, forward, pull: 0.0, target: 0.0, held: false }
    }

    /// Where the handle is now.
    pub fn handle(&self) -> Vec3 {
        self.closed - self.forward * self.pull
    }

    /// How open it is, from 0 to 1.
    pub fn progress(&self) -> f32 {
        self.pull / TRAVEL
    }

    pub fn is_held(&self) -> bool {
        self.held
    }

    /// The drawer is (or will be) open.
    pub fn is_open(&self) -> bool {
        self.target > 0.0 || self.pull > 0.0
    }

    /// Starts pulling the handle.
    pub fn grab(&mut self) {
        self.held = true;
    }

    /// With the handle in hand: it follows `hand` only along the drawer's rail.
    pub fn hold(&mut self, hand: Vec3) {
        if self.held {
            self.pull = (self.closed - hand).dot(self.forward).clamp(0.0, TRAVEL);
            self.target = self.pull;
        }
    }

    /// Releases the handle: it either finishes opening or closes, depending on how much was pulled.
    pub fn release(&mut self) {
        if self.held {
            self.held = false;
            self.target = if self.pull > TRAVEL * SNAP { TRAVEL } else { 0.0 };
        }
    }

    /// Closes it by itself (for example, when opening a file from a card), even if nobody holds the handle.
    pub fn close(&mut self) {
        self.held = false;
        self.target = 0.0;
    }

    /// Advances the animation of opening or closing by itself.
    pub fn update(&mut self, dt: f32) {
        if self.held {
            return;
        }
        let step = SLIDE_SPEED * dt.max(0.0);
        if self.pull < self.target {
            self.pull = (self.pull + step).min(self.target);
        } else {
            self.pull = (self.pull - step).max(self.target);
        }
    }
}

/// Where each card is and how far it has come out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardPose {
    pub position: Vec3,
    /// Relative size: small inside the drawer, full on the desk (somewhat larger if it is the chosen one).
    pub scale: f32,
    /// Progress of this card, from 0 (in the drawer) to 1 (in its place).
    pub t: f32,
}

/// Where the card file opens: in an arc in front of you, with the cards standing on the desk.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fan {
    /// Your head: the arc is centered on it (horizontally).
    pub head: Vec3,
    /// The direction it opens toward (only its horizontal part is used).
    pub forward: Vec3,
    /// Horizontal distance (m) from the head to the cards.
    pub radius: f32,
    /// Height (m) of the desk.
    pub desk_y: f32,
    /// Width (m) of a card.
    pub card_width: f32,
}

impl Fan {
    fn dir(&self, angle: f32) -> Vec3 {
        let f = Vec3::new(self.forward.x, 0.0, self.forward.z).try_normalize().unwrap_or(Vec3::NEG_Z);
        Quat::from_rotation_y(-angle) * f
    }

    fn base(&self) -> Vec3 {
        Vec3::new(self.head.x, self.desk_y, self.head.z)
    }

    /// Angle (rad) between adjacent cards.
    pub fn step(&self) -> f32 {
        SPACING * self.card_width / self.radius.max(0.05)
    }

    /// Where the card at position `slot` of `visible` goes (`slot` can be fractional: that is how it slides).
    pub fn slot_position(&self, slot: f32, visible: usize) -> Vec3 {
        let angle = (slot - (visible.max(1) as f32 - 1.0) / 2.0) * self.step();
        let height = self.card_width * CARD_ASPECT;
        self.base() + self.dir(angle) * self.radius + Vec3::Y * (ABOVE_DESK + height / 2.0)
    }

    /// Where the title card goes: above the center of the fan, a little farther back.
    pub fn title_position(&self) -> Vec3 {
        let height = self.card_width * CARD_ASPECT;
        self.base() + self.dir(0.0) * (self.radius + 0.04) + Vec3::Y * (ABOVE_DESK + height * 1.5 + 0.03)
    }
}

/// What part of a folder with `total` entries is visible with `selected` chosen: (first, how many). The chosen one
/// stays in the center except near the ends.
pub fn window(total: usize, selected: usize) -> (usize, usize) {
    let visible = total.min(PAGE);
    let start = selected.min(total.saturating_sub(1)).saturating_sub(PAGE / 2).min(total - visible);
    (start, visible)
}

/// Card `i` of `n` coming out of the drawer toward `target`: the first ones leave earlier (cascading) and rise in an arc.
fn emerge(target: Vec3, i: usize, n: usize, drawer: &Drawer, progress: f32, full_scale: f32) -> CardPose {
    let progress = progress.clamp(0.0, 1.0);
    let t = (progress * (1.0 + STAGGER) - i as f32 / n.max(1) as f32 * STAGGER).clamp(0.0, 1.0);
    let ease = t * t * (3.0 - 2.0 * t);
    let stack = drawer.handle() + Vec3::Y * (0.04 + 0.006 * i as f32);
    let arc = Vec3::Y * (PI * ease).sin() * ARC_LIFT;
    CardPose { position: stack.lerp(target, ease) + arc, scale: (0.15 + 0.85 * ease) * full_scale, t }
}

/// The visible cards of the card file according to how open the drawer is. `selected` is the chosen one among the
/// visible; `shift` slides the fan (in cards) while paging from one to another.
pub fn fan_poses(fan: &Fan, visible: usize, selected: usize, shift: f32, drawer: &Drawer, progress: f32) -> Vec<CardPose> {
    (0..visible)
        .map(|k| {
            let mut target = fan.slot_position(k as f32 + shift, visible);
            let mut scale = 1.0;
            if k == selected {
                let toward = Vec3::new(fan.head.x - target.x, 0.0, fan.head.z - target.z).try_normalize().unwrap_or(Vec3::ZERO);
                target += toward * SELECTED_FORWARD + Vec3::Y * SELECTED_RISE;
                scale = SELECTED_SCALE;
            }
            emerge(target, k + 1, visible + 1, drawer, progress, scale)
        })
        .collect()
}

/// The title card: it shows the current folder and, when tapped, goes up to the one above. It is the first to come out.
pub fn title_pose(fan: &Fan, drawer: &Drawer, progress: f32) -> CardPose {
    emerge(fan.title_position(), 0, 1, drawer, progress, TITLE_SCALE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawer() -> Drawer {
        // Drawer 0.5 m in front of you (toward -Z), on the desk.
        Drawer::new(Vec3::new(0.0, 0.72, -0.5), Vec3::NEG_Z)
    }

    #[test]
    fn pulling_the_handle_toward_you_opens_it() {
        let mut d = drawer();
        d.grab();
        d.hold(Vec3::new(0.0, 0.72, -0.5 + 0.14)); // 14 cm hacia ti
        assert!((d.progress() - 0.5).abs() < 1e-5);
        assert!((d.handle().z - (-0.5 + 0.14)).abs() < 1e-5, "the handle follows the hand");
    }

    #[test]
    fn the_handle_stays_on_its_rail() {
        let mut d = drawer();
        d.grab();
        d.hold(Vec3::new(0.3, 1.5, -0.1)); // the hand goes to one side and up
        assert_eq!((d.handle().x, d.handle().y), (0.0, 0.72), "it only moves along the rail");
        d.hold(Vec3::new(0.0, 0.72, 0.9)); // pulling too far
        assert_eq!(d.progress(), 1.0);
        d.hold(Vec3::new(0.0, 0.72, -1.5)); // empujar hacia dentro
        assert_eq!(d.progress(), 0.0);
    }

    #[test]
    fn releasing_past_the_snap_point_finishes_opening_and_before_it_closes() {
        let mut d = drawer();
        d.grab();
        d.hold(Vec3::new(0.0, 0.72, -0.5 + TRAVEL * 0.6));
        d.release();
        for _ in 0..200 {
            d.update(1.0 / 72.0);
        }
        assert_eq!(d.progress(), 1.0);
        assert!(d.is_open());

        let mut d = drawer();
        d.grab();
        d.hold(Vec3::new(0.0, 0.72, -0.5 + TRAVEL * 0.3));
        d.release();
        for _ in 0..200 {
            d.update(1.0 / 72.0);
        }
        assert_eq!(d.progress(), 0.0);
        assert!(!d.is_open());
    }

    #[test]
    fn pushing_an_open_drawer_back_closes_it() {
        let mut d = drawer();
        d.grab();
        d.hold(Vec3::new(0.0, 0.72, 0.5));
        d.release();
        d.update(1.0);
        assert_eq!(d.progress(), 1.0);
        d.grab();
        d.hold(Vec3::new(0.0, 0.72, -0.5 + TRAVEL * 0.2));
        d.release();
        d.update(1.0);
        assert_eq!(d.progress(), 0.0);
    }

    #[test]
    fn it_does_not_move_by_itself_while_held_and_ignores_a_hand_that_is_not_holding() {
        let mut d = drawer();
        d.hold(Vec3::new(0.0, 0.72, 0.5));
        assert_eq!(d.progress(), 0.0, "without grabbing it, it does not move");
        d.grab();
        d.hold(Vec3::new(0.0, 0.72, -0.5 + 0.1));
        let p = d.progress();
        d.update(5.0);
        assert_eq!(d.progress(), p);
    }

    #[test]
    fn a_drawer_can_close_by_itself_even_when_open() {
        let mut d = drawer();
        d.grab();
        d.hold(Vec3::new(0.0, 0.72, 0.5));
        d.release();
        d.update(1.0);
        assert_eq!(d.progress(), 1.0);
        d.close();
        assert!(!d.is_held());
        for _ in 0..100 {
            d.update(1.0 / 72.0);
        }
        assert_eq!(d.progress(), 0.0);
        assert!(!d.is_open());
    }

    fn fan() -> Fan {
        // Desk 40 cm below the eyes; cards 62 cm in front of you (toward -Z).
        Fan { head: Vec3::new(0.0, 1.2, 0.0), forward: Vec3::NEG_Z, radius: 0.62, desk_y: 0.8, card_width: 0.21 }
    }

    #[test]
    fn the_window_keeps_the_selected_card_in_the_middle_except_at_the_ends() {
        assert_eq!(window(0, 0), (0, 0));
        assert_eq!(window(3, 2), (0, 3), "less than a page: all are visible");
        assert_eq!(window(20, 0), (0, PAGE));
        assert_eq!(window(20, 10), (8, PAGE));
        assert_eq!(window(20, 19), (15, PAGE));
        assert_eq!(window(20, 99), (15, PAGE), "an out-of-range choice does not break it");
    }

    #[test]
    fn the_cards_stand_on_the_desk_in_an_ordered_arc_in_front_of_you() {
        let (f, d) = (fan(), drawer());
        let poses = fan_poses(&f, PAGE, 99, 0.0, &d, 1.0); // nothing chosen, to measure the clean arc
        let height = f.card_width * CARD_ASPECT;
        for (k, p) in poses.iter().enumerate() {
            assert_eq!(p.t, 1.0, "card {k}");
            let flat = Vec3::new(p.position.x, 0.0, p.position.z).length();
            assert!((flat - f.radius).abs() < 1e-4, "card {k} on the arc: {flat}");
            let bottom = p.position.y - height / 2.0;
            assert!((bottom - (f.desk_y + 0.02)).abs() < 1e-4, "standing on the desk: {bottom}");
            assert!(p.position.z < 0.0, "in front of you");
            assert!(p.position.y < f.head.y - 0.2, "below the eyes, without covering the editor");
        }
        let xs: Vec<f32> = poses.iter().map(|p| p.position.x).collect();
        assert!(xs.windows(2).all(|w| w[0] < w[1]), "from left to right: {xs:?}");
        assert!((xs.iter().sum::<f32>() / xs.len() as f32).abs() < 1e-4, "centrado");
        for w in poses.windows(2) {
            let gap = w[0].position.distance(w[1].position);
            assert!(gap > 0.85 * f.card_width && gap < f.card_width, "they overlap a bit, without piling up: {gap}");
        }
        let widest = (xs[PAGE - 1] / f.radius).asin().to_degrees();
        assert!(widest < 45.0, "the whole fan fits in view without turning the head: {widest:.0}°");
    }

    #[test]
    fn the_selected_card_comes_forward_rises_and_grows() {
        let (f, d) = (fan(), drawer());
        let poses = fan_poses(&f, PAGE, 2, 0.0, &d, 1.0);
        let flat = |p: &CardPose| Vec3::new(p.position.x, 0.0, p.position.z).length();
        assert!(flat(&poses[2]) < flat(&poses[1]) - 0.02, "closer to you");
        assert!(poses[2].position.y > poses[1].position.y);
        assert!(poses[2].scale > poses[1].scale && poses[1].scale == 1.0);
    }

    #[test]
    fn the_title_card_sits_above_the_middle_and_comes_out_first() {
        let (f, d) = (fan(), drawer());
        let title = title_pose(&f, &d, 1.0);
        let middle = fan_poses(&f, PAGE, 99, 0.0, &d, 1.0)[PAGE / 2];
        assert!(title.position.x.abs() < 1e-4, "centrada");
        assert!(title.position.y > middle.position.y + f.card_width * CARD_ASPECT * 0.9, "above the fan");
        assert!(title.scale < 1.0);
        let half_title = title_pose(&f, &d, 0.4);
        let half_cards = fan_poses(&f, PAGE, 0, 0.0, &d, 0.4);
        assert!(half_cards.iter().all(|c| c.t <= half_title.t), "the title card comes out first");
    }

    #[test]
    fn shifting_slides_the_cards_along_the_arc() {
        let f = fan();
        let half = f.slot_position(1.5, PAGE);
        let (a, b) = (f.slot_position(1.0, PAGE), f.slot_position(2.0, PAGE));
        assert!(half.x > a.x && half.x < b.x, "a medio camino entre dos sitios");
        assert!((Vec3::new(half.x, 0.0, half.z).length() - f.radius).abs() < 1e-4, "without leaving the arc");
    }

    #[test]
    fn a_closed_drawer_keeps_every_card_inside_and_they_come_out_in_a_cascade() {
        let (f, d) = (fan(), drawer());
        for p in fan_poses(&f, PAGE, 0, 0.0, &d, 0.0) {
            assert_eq!(p.t, 0.0);
            assert!(p.position.distance(d.closed) < 0.1, "inside the drawer: {}", p.position);
            assert!(p.scale < 0.2);
        }
        let mid = fan_poses(&f, PAGE, 99, 0.0, &d, 0.5);
        assert!(mid.windows(2).all(|w| w[0].t >= w[1].t), "the first ones go ahead");
        let more = fan_poses(&f, PAGE, 99, 0.0, &d, 0.8);
        assert!(more.iter().zip(&mid).all(|(a, b)| a.t >= b.t), "more open, farther out");
    }

    #[test]
    fn an_empty_folder_has_no_cards_but_still_a_title() {
        let (f, d) = (fan(), drawer());
        assert!(fan_poses(&f, 0, 0, 0.0, &d, 1.0).is_empty());
        assert_eq!(title_pose(&f, &d, 1.0).t, 1.0);
    }
}
