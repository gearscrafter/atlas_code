//! Interaction with the hands: detecting the pinch and the closed hand, and dragging objects with them.
//!
//! Everything works with positions already computed (thumb tip, index tip), so
//! it does not depend on OpenXR and is tested on the PC.

use glam::Vec3;

/// Distance (m) between the thumb tip and the index tip below which a pinch
/// starts.
pub const PINCH_START: f32 = 0.020;
/// Distance from which it is released. It is larger than the start one (hysteresis): without
/// it, tracking noise would make the pinch flicker when right at the threshold.
pub const PINCH_END: f32 = 0.035;
/// Slack (m) with which an object can be grabbed, in addition to its radius.
pub const GRAB_MARGIN: f32 = 0.06;
/// A grab is a "tap" (open) if the object moved less than this (m)...
pub const TAP_MOVE: f32 = 0.04;
/// ...and was released before this number of frames (~3 s at 72 Hz).
pub const TAP_FRAMES: u32 = 216;

/// Distance (m) from the four fingertips to the palm below which the hand
/// counts as a closed fist...
pub const GRIP_START: f32 = 0.065;
/// ...and from which one of the fingers opens (hysteresis, as with the pinch).
pub const GRIP_END: f32 = 0.085;

/// Grabbing an orb with the whole hand: the average distance (m) from the four fingertips to the
/// palm below which the hand closes around something (a sphere keeps it from closing fully)...
pub const GRASP_START: f32 = 0.09;
/// ...and from which it opens.
pub const GRASP_END: f32 = 0.125;
/// Slack (m) between the center of the closed hand and the orb's surface for it to be grabbable.
pub const GRASP_MARGIN: f32 = 0.06;
/// Each frame, the grabbed orb travels this fraction of the way until it is centered in the hand.
const SETTLE: f32 = 0.88;

/// Distance (m) from which a pinch in the air counts as a swipe.
pub const SWIPE_MIN: f32 = 0.06;

/// Direction of a swipe made pinching in the air.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Swipe {
    Horizontal,
    Down,
    Up,
}

/// Classifies a displacement (`dx` to the right, `dy` up, in meters) from the
/// start of the pinch. `None` while it has not moved enough. It is vertical only if
/// that clearly dominates, so that a slightly crooked sideways swipe does not open or close the editor.
pub fn classify_swipe(dx: f32, dy: f32) -> Option<Swipe> {
    if dx.abs().max(dy.abs()) < SWIPE_MIN {
        None
    } else if dy.abs() > 1.5 * dx.abs() {
        Some(if dy < 0.0 { Swipe::Down } else { Swipe::Up })
    } else {
        Some(Swipe::Horizontal)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinchEvent {
    None,
    Started,
    Ended,
}

/// Pinch state of a hand.
#[derive(Debug, Clone, Copy, Default)]
pub struct Pinch {
    active: bool,
}

impl Pinch {
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Updates with the thumb and index tips. Returns whether the pinch just
    /// started or ended.
    pub fn update(&mut self, thumb_tip: Vec3, index_tip: Vec3) -> PinchEvent {
        let distance = thumb_tip.distance(index_tip);
        match (self.active, distance) {
            (false, d) if d <= PINCH_START => {
                self.active = true;
                PinchEvent::Started
            }
            (true, d) if d >= PINCH_END => {
                self.active = false;
                PinchEvent::Ended
            }
            _ => PinchEvent::None,
        }
    }

    /// The hand is no longer visible: if it was pinching, the pinch ends.
    pub fn lost(&mut self) -> PinchEvent {
        if std::mem::take(&mut self.active) {
            PinchEvent::Ended
        } else {
            PinchEvent::None
        }
    }
}

/// Hand closed around something: looser than the fist, because an orb in the palm keeps it from closing fully.
#[derive(Debug, Clone, Copy, Default)]
pub struct Grasp {
    active: bool,
}

impl Grasp {
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// `palm` and the index, middle, ring and little fingertips. Returns whether the hand just closed
    /// or opened.
    pub fn update(&mut self, palm: Vec3, tips: &[Vec3; 4]) -> PinchEvent {
        let curl = tips.iter().map(|t| t.distance(palm)).sum::<f32>() / 4.0;
        match (self.active, curl) {
            (false, c) if c <= GRASP_START => {
                self.active = true;
                PinchEvent::Started
            }
            (true, c) if c >= GRASP_END => {
                self.active = false;
                PinchEvent::Ended
            }
            _ => PinchEvent::None,
        }
    }

    /// The hand is no longer visible: if it was closed, it opens.
    pub fn lost(&mut self) -> PinchEvent {
        if std::mem::take(&mut self.active) {
            PinchEvent::Ended
        } else {
            PinchEvent::None
        }
    }
}

/// Where the hand closes: halfway between the palm and the average of the fingertips. It is where
/// a held orb stays.
pub fn grasp_point(palm: Vec3, tips: &[Vec3; 4]) -> Vec3 {
    let mean = tips.iter().copied().sum::<Vec3>() / 4.0;
    palm + (mean - palm) * 0.5
}

/// Fist state of a hand: grabbing "as if it were an object".
#[derive(Debug, Clone, Copy, Default)]
pub struct Grip {
    active: bool,
}

impl Grip {
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// `palm` and the index, middle, ring and little fingertips. Returns whether the fist just
    /// closed or opened. It is told apart from the pinch because here the index is also
    /// bent toward the palm.
    pub fn update(&mut self, palm: Vec3, tips: &[Vec3; 4]) -> PinchEvent {
        let farthest = tips.iter().map(|t| t.distance(palm)).fold(0.0f32, f32::max);
        match (self.active, farthest) {
            (false, d) if d <= GRIP_START => {
                self.active = true;
                PinchEvent::Started
            }
            (true, d) if d >= GRIP_END => {
                self.active = false;
                PinchEvent::Ended
            }
            _ => PinchEvent::None,
        }
    }

    /// The hand is no longer visible: if it was closed, it opens.
    pub fn lost(&mut self) -> PinchEvent {
        if std::mem::take(&mut self.active) {
            PinchEvent::Ended
        } else {
            PinchEvent::None
        }
    }
}

/// Point where it "grabs": between the two tips.
pub fn pinch_point(thumb_tip: Vec3, index_tip: Vec3) -> Vec3 {
    (thumb_tip + index_tip) / 2.0
}

/// What the drag system needs to know about a hand in this frame.
#[derive(Debug, Clone, Copy)]
pub struct HandPoint {
    pub tracked: bool,
    /// Pinch point: cards and the handle are grabbed with it.
    pub point: Vec3,
    pub pinch: PinchEvent,
    pub pinching: bool,
    /// Point where the whole hand closes: orbs are grabbed with it.
    pub grasp_point: Vec3,
    pub grasp: PinchEvent,
    pub grasping: bool,
}

impl HandPoint {
    pub const UNTRACKED: HandPoint = HandPoint {
        tracked: false,
        point: Vec3::ZERO,
        pinch: PinchEvent::None,
        pinching: false,
        grasp_point: Vec3::ZERO,
        grasp: PinchEvent::None,
        grasping: false,
    };
}

/// A spherical object that can be grabbed. Orbs (`grasp`) are grabbed by closing the whole hand; the
/// rest (cards, the handle) with a pinch.
#[derive(Debug, Clone, Copy)]
pub struct Draggable {
    pub center: Vec3,
    pub radius: f32,
    pub grasp: bool,
}

/// What happened with each hand in this frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DragFrame {
    /// Object each hand is over (without pinching).
    pub hovered: [Option<usize>; 2],
    /// Object each hand is holding.
    pub grabbed: [Option<usize>; 2],
    /// `(hand, object)` grabbed in this frame.
    pub started: Vec<(usize, usize)>,
    /// `(hand, object)` released in this frame.
    pub ended: Vec<(usize, usize)>,
    /// `(hand, object)` released as a tap: grabbed and released barely moving it.
    pub tapped: Vec<(usize, usize)>,
}

/// Two-handed dragging: each hand can carry one object, and an object is carried by only one hand.
#[derive(Debug, Default)]
pub struct DragSystem {
    /// Per hand: grabbed object and its offset from the pinch point.
    grabs: [Option<(usize, Vec3)>; 2],
    /// Per hand: where the object was when grabbed and how many frames it has been held.
    starts: [Vec3; 2],
    held: [u32; 2],
}

impl DragSystem {
    /// Releases `item` from the hand carrying it (e.g. when activating it), without waiting for the pinch.
    pub fn release_item(&mut self, item: usize) {
        for grab in self.grabs.iter_mut() {
            if matches!(grab, Some((i, _)) if *i == item) {
                *grab = None;
            }
        }
    }

    /// Object each hand is carrying right now.
    pub fn grabbed(&self) -> [Option<usize>; 2] {
        [self.grabs[0].map(|(i, _)| i), self.grabs[1].map(|(i, _)| i)]
    }

    /// Processes a frame and moves the grabbed objects. `items` can change
    /// size between frames; grabs on objects that no longer exist are released.
    pub fn update(&mut self, hands: &[HandPoint; 2], items: &mut [Draggable]) -> DragFrame {
        let mut frame = DragFrame::default();

        for (hand, h) in hands.iter().enumerate() {
            // Release: grab ended, hand lost or object gone.
            if let Some((item, _)) = self.grabs[hand] {
                let gone = item >= items.len();
                let holding = if gone {
                    false
                } else if items[item].grasp {
                    h.grasping && h.grasp != PinchEvent::Ended
                } else {
                    h.pinching && h.pinch != PinchEvent::Ended
                };
                if !h.tracked || !holding {
                    self.grabs[hand] = None;
                    if !gone {
                        frame.ended.push((hand, item));
                        let moved = items[item].center.distance(self.starts[hand]);
                        if moved <= TAP_MOVE && self.held[hand] <= TAP_FRAMES {
                            frame.tapped.push((hand, item));
                        }
                    }
                }
            }

            // Grab: when the pinch starts (cards) or the hand closes (orbs), the nearest free object.
            if h.tracked && self.grabs[hand].is_none() {
                let taken: Vec<usize> = self.grabs.iter().flatten().map(|(i, _)| *i).collect();
                let found = if h.pinch == PinchEvent::Started {
                    nearest_of(items, h.point, GRAB_MARGIN, &taken, false).map(|i| (i, h.point))
                } else if h.grasp == PinchEvent::Started {
                    nearest_of(items, h.grasp_point, GRASP_MARGIN, &taken, true).map(|i| (i, h.grasp_point))
                } else {
                    None
                };
                if let Some((item, at)) = found {
                    self.grabs[hand] = Some((item, items[item].center - at));
                    self.starts[hand] = items[item].center;
                    self.held[hand] = 0;
                    frame.started.push((hand, item));
                }
            }

            // Arrastrar.
            if let Some((item, offset)) = self.grabs[hand] {
                self.held[hand] = self.held[hand].saturating_add(1);
                // An orb grabbed with the whole hand centers in it; the rest keep where they were grabbed.
                let offset = if items[item].grasp { offset * SETTLE } else { offset };
                self.grabs[hand] = Some((item, offset));
                if h.tracked {
                    let at = if items[item].grasp { h.grasp_point } else { h.point };
                    items[item].center = at + offset;
                }
                frame.grabbed[hand] = Some(item);
            } else if h.tracked && !h.pinching && !h.grasping {
                let taken: Vec<usize> = self.grabs.iter().flatten().map(|(i, _)| *i).collect();
                frame.hovered[hand] = nearest_of(items, h.grasp_point, GRASP_MARGIN, &taken, true)
                    .or_else(|| nearest_of(items, h.point, GRAB_MARGIN, &taken, false));
            }
        }
        frame
    }
}

/// Nearest object to `p` whose surface is closer than `margin`, ignoring `exclude`.
pub fn nearest(items: &[Draggable], p: Vec3, margin: f32, exclude: &[usize]) -> Option<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(i, it)| !exclude.contains(i) && p.distance(it.center) <= it.radius + margin)
        .min_by(|(_, a), (_, b)| p.distance(a.center).total_cmp(&p.distance(b.center)))
        .map(|(i, _)| i)
}

/// Like `nearest`, but only among the objects grabbed with the whole hand (`grasp`) or only among
/// the ones grabbed with a pinch.
fn nearest_of(items: &[Draggable], p: Vec3, margin: f32, exclude: &[usize], grasp: bool) -> Option<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(i, it)| it.grasp == grasp && !exclude.contains(i) && p.distance(it.center) <= it.radius + margin)
        .min_by(|(_, a), (_, b)| p.distance(a.center).total_cmp(&p.distance(b.center)))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3::new(x, y, z)
    }

    /// Tips `gap` meters apart horizontally.
    fn tips(gap: f32) -> (Vec3, Vec3) {
        (v(0.0, 0.0, -0.4), v(gap, 0.0, -0.4))
    }

    // ----- pellizco -----

    fn fingers(palm: Vec3, reach: f32) -> [Vec3; 4] {
        [palm + Vec3::new(0.0, 0.0, -reach); 4]
    }

    #[test]
    fn a_closed_fist_grips_and_an_open_hand_releases() {
        let palm = v(0.0, 1.0, -0.4);
        let mut grip = Grip::default();
        assert_eq!(grip.update(palm, &fingers(palm, 0.10)), PinchEvent::None, "open hand");
        assert_eq!(grip.update(palm, &fingers(palm, 0.04)), PinchEvent::Started);
        assert!(grip.is_active());
        assert_eq!(grip.update(palm, &fingers(palm, 0.04)), PinchEvent::None, "without repeating");
        assert_eq!(grip.update(palm, &fingers(palm, 0.10)), PinchEvent::Ended);
    }

    #[test]
    fn grip_has_hysteresis() {
        let palm = Vec3::ZERO;
        let mut grip = Grip::default();
        grip.update(palm, &fingers(palm, 0.04));
        // Between the two thresholds (6.5 and 8.5 cm) it stays closed.
        for reach in [0.07, 0.08, 0.075] {
            assert_eq!(grip.update(palm, &fingers(palm, reach)), PinchEvent::None);
            assert!(grip.is_active());
        }
        let mut open = Grip::default();
        assert_eq!(open.update(palm, &fingers(palm, 0.075)), PinchEvent::None);
        assert!(!open.is_active());
    }

    #[test]
    fn a_pinch_with_the_other_fingers_out_is_not_a_grip() {
        let palm = Vec3::ZERO;
        // Index and thumb together, but the middle, ring and little fingers still extended.
        let tips = [palm + Vec3::new(0.0, 0.0, -0.10), palm + Vec3::new(0.0, 0.0, -0.04), palm + Vec3::new(0.0, 0.0, -0.04), palm + Vec3::new(0.0, 0.0, -0.04)];
        assert_eq!(Grip::default().update(palm, &tips), PinchEvent::None);
    }

    #[test]
    fn losing_the_hand_opens_the_grip_once() {
        let mut grip = Grip::default();
        assert_eq!(grip.lost(), PinchEvent::None);
        grip.update(Vec3::ZERO, &fingers(Vec3::ZERO, 0.03));
        assert_eq!(grip.lost(), PinchEvent::Ended);
        assert_eq!(grip.lost(), PinchEvent::None);
    }

    #[test]
    fn swipes_are_classified_by_their_dominant_direction() {
        assert_eq!(classify_swipe(0.01, -0.02), None, "poco recorrido");
        assert_eq!(classify_swipe(0.0, -0.10), Some(Swipe::Down));
        assert_eq!(classify_swipe(0.02, 0.12), Some(Swipe::Up));
        assert_eq!(classify_swipe(0.10, -0.02), Some(Swipe::Horizontal));
        assert_eq!(classify_swipe(-0.12, 0.03), Some(Swipe::Horizontal));
        // A diagonal does not count as vertical.
        assert_eq!(classify_swipe(0.08, -0.10), Some(Swipe::Horizontal));
    }

    #[test]
    fn pinch_starts_once_and_ends_once() {
        let mut pinch = Pinch::default();
        let (t, i) = tips(0.10);
        assert_eq!(pinch.update(t, i), PinchEvent::None);
        let (t, i) = tips(0.015);
        assert_eq!(pinch.update(t, i), PinchEvent::Started);
        assert!(pinch.is_active());
        assert_eq!(pinch.update(t, i), PinchEvent::None, "it must not repeat the event");
        let (t, i) = tips(0.06);
        assert_eq!(pinch.update(t, i), PinchEvent::Ended);
        assert!(!pinch.is_active());
        assert_eq!(pinch.update(t, i), PinchEvent::None);
    }

    #[test]
    fn hysteresis_prevents_flicker_around_the_threshold() {
        let mut pinch = Pinch::default();
        let (t, i) = tips(0.018);
        assert_eq!(pinch.update(t, i), PinchEvent::Started);
        // Between the two thresholds (2.0 cm and 3.5 cm) the pinch holds...
        for gap in [0.022, 0.030, 0.026, 0.034, 0.021] {
            let (t, i) = tips(gap);
            assert_eq!(pinch.update(t, i), PinchEvent::None, "at {gap} m it must not release");
            assert!(pinch.is_active());
        }
        // ...and no new one starts in that band either.
        let mut open = Pinch::default();
        let (t, i) = tips(0.028);
        assert_eq!(open.update(t, i), PinchEvent::None);
        assert!(!open.is_active());
    }

    #[test]
    fn losing_the_hand_ends_an_active_pinch_only() {
        let mut pinch = Pinch::default();
        assert_eq!(pinch.lost(), PinchEvent::None);
        let (t, i) = tips(0.01);
        pinch.update(t, i);
        assert_eq!(pinch.lost(), PinchEvent::Ended);
        assert_eq!(pinch.lost(), PinchEvent::None);
    }

    #[test]
    fn pinch_point_is_the_midpoint() {
        let p = pinch_point(v(0.0, 0.0, 0.0), v(0.02, 0.04, -0.02));
        assert!((p - v(0.01, 0.02, -0.01)).length() < 1e-6);
    }

    // ----- arrastre -----

    fn hand(point: Vec3, pinch: PinchEvent, pinching: bool) -> HandPoint {
        HandPoint { tracked: true, point, pinch, pinching, ..HandPoint::UNTRACKED }
    }

    fn orbs() -> Vec<Draggable> {
        vec![
            Draggable { center: v(0.0, 0.0, -1.0), radius: 0.05, grasp: false },
            Draggable { center: v(0.5, 0.0, -1.0), radius: 0.05, grasp: false },
        ]
    }

    #[test]
    fn grabbing_moves_the_object_with_the_hand_keeping_the_offset() {
        let mut sys = DragSystem::default();
        let mut items = orbs();
        // Pinch 3 cm to the right of the first orb's center.
        let start = v(0.03, 0.0, -1.0);
        let f = sys.update(&[hand(start, PinchEvent::Started, true), HandPoint::UNTRACKED], &mut items);
        assert_eq!(f.started, vec![(0, 0)]);
        assert_eq!(f.grabbed[0], Some(0));

        // The hand moves 20 cm up: the orb follows it keeping the 3 cm offset.
        let f = sys.update(&[hand(v(0.03, 0.2, -1.0), PinchEvent::None, true), HandPoint::UNTRACKED], &mut items);
        assert_eq!(f.grabbed[0], Some(0));
        assert!((items[0].center - v(0.0, 0.2, -1.0)).length() < 1e-5, "{:?}", items[0].center);
        assert_eq!(items[1].center, v(0.5, 0.0, -1.0), "the other orb must not move");
    }

    #[test]
    fn releasing_leaves_the_object_where_it_was_dropped() {
        let mut sys = DragSystem::default();
        let mut items = orbs();
        sys.update(&[hand(v(0.0, 0.0, -1.0), PinchEvent::Started, true), HandPoint::UNTRACKED], &mut items);
        sys.update(&[hand(v(0.3, 0.1, -0.9), PinchEvent::None, true), HandPoint::UNTRACKED], &mut items);
        let f = sys.update(&[hand(v(0.3, 0.1, -0.9), PinchEvent::Ended, false), HandPoint::UNTRACKED], &mut items);
        assert_eq!(f.ended, vec![(0, 0)]);
        assert_eq!(f.grabbed[0], None);
        let dropped = items[0].center;
        // Further hand movements no longer drag it.
        sys.update(&[hand(v(0.9, 0.9, -0.5), PinchEvent::None, false), HandPoint::UNTRACKED], &mut items);
        assert_eq!(items[0].center, dropped);
    }

    #[test]
    fn a_pinch_far_from_everything_grabs_nothing() {
        let mut sys = DragSystem::default();
        let mut items = orbs();
        let f = sys.update(&[hand(v(0.0, 0.5, -1.0), PinchEvent::Started, true), HandPoint::UNTRACKED], &mut items);
        assert!(f.started.is_empty());
        assert_eq!(f.grabbed, [None, None]);
        // And moving the pinching hand does not drag anything that was not grabbed at the start.
        let f = sys.update(&[hand(v(0.0, 0.0, -1.0), PinchEvent::None, true), HandPoint::UNTRACKED], &mut items);
        assert!(f.started.is_empty(), "it only grabs when the pinch starts, not when passing over");
    }

    #[test]
    fn grab_margin_allows_a_bit_of_slack_but_not_too_much() {
        let mut sys = DragSystem::default();
        let mut items = orbs();
        // radio 5 cm + holgura 6 cm = 11 cm.
        let f = sys.update(&[hand(v(0.10, 0.0, -1.0), PinchEvent::Started, true), HandPoint::UNTRACKED], &mut items);
        assert_eq!(f.started, vec![(0, 0)]);
        let mut sys = DragSystem::default();
        let mut items = orbs();
        let f = sys.update(&[hand(v(0.13, 0.0, -1.0), PinchEvent::Started, true), HandPoint::UNTRACKED], &mut items);
        assert!(f.started.is_empty());
    }

    #[test]
    fn the_closest_object_wins_when_two_are_in_reach() {
        let mut sys = DragSystem::default();
        let mut items = vec![
            Draggable { center: v(0.00, 0.0, -1.0), radius: 0.05, grasp: false },
            Draggable { center: v(0.06, 0.0, -1.0), radius: 0.05, grasp: false },
        ];
        let f = sys.update(&[hand(v(0.05, 0.0, -1.0), PinchEvent::Started, true), HandPoint::UNTRACKED], &mut items);
        assert_eq!(f.started, vec![(0, 1)], "it must pick the orb whose center is closest");
    }

    #[test]
    fn each_hand_carries_its_own_object_and_an_object_has_one_owner() {
        let mut sys = DragSystem::default();
        let mut items = orbs();
        let f = sys.update(
            &[
                hand(v(0.0, 0.0, -1.0), PinchEvent::Started, true),
                hand(v(0.5, 0.0, -1.0), PinchEvent::Started, true),
            ],
            &mut items,
        );
        assert_eq!(f.grabbed, [Some(0), Some(1)]);

        // The right hand tries to take the orb the left one is carrying: it cannot.
        let mut sys = DragSystem::default();
        let mut items = orbs();
        sys.update(&[hand(v(0.0, 0.0, -1.0), PinchEvent::Started, true), HandPoint::UNTRACKED], &mut items);
        let f = sys.update(
            &[hand(v(0.0, 0.0, -1.0), PinchEvent::None, true), hand(v(0.01, 0.0, -1.0), PinchEvent::Started, true)],
            &mut items,
        );
        assert_eq!(f.grabbed[0], Some(0));
        assert_eq!(f.grabbed[1], None, "an object cannot have two owners");
    }

    #[test]
    fn losing_tracking_drops_the_object() {
        let mut sys = DragSystem::default();
        let mut items = orbs();
        sys.update(&[hand(v(0.0, 0.0, -1.0), PinchEvent::Started, true), HandPoint::UNTRACKED], &mut items);
        let before = items[0].center;
        let f = sys.update(&[HandPoint::UNTRACKED, HandPoint::UNTRACKED], &mut items);
        assert_eq!(f.ended, vec![(0, 0)]);
        assert_eq!(f.grabbed, [None, None]);
        assert_eq!(items[0].center, before, "when the hand is lost the object does not jump anywhere");
    }

    #[test]
    fn a_quick_grab_and_release_without_moving_is_a_tap() {
        let mut sys = DragSystem::default();
        let mut items = orbs();
        sys.update(&[hand(v(0.0, 0.0, -1.0), PinchEvent::Started, true), HandPoint::UNTRACKED], &mut items);
        sys.update(&[hand(v(0.01, 0.0, -1.0), PinchEvent::None, true), HandPoint::UNTRACKED], &mut items);
        let f = sys.update(&[hand(v(0.01, 0.0, -1.0), PinchEvent::Ended, false), HandPoint::UNTRACKED], &mut items);
        assert_eq!(f.tapped, vec![(0, 0)]);
    }

    #[test]
    fn dragging_far_or_holding_long_is_not_a_tap() {
        let mut sys = DragSystem::default();
        let mut items = orbs();
        sys.update(&[hand(v(0.0, 0.0, -1.0), PinchEvent::Started, true), HandPoint::UNTRACKED], &mut items);
        sys.update(&[hand(v(0.2, 0.0, -1.0), PinchEvent::None, true), HandPoint::UNTRACKED], &mut items);
        let f = sys.update(&[hand(v(0.2, 0.0, -1.0), PinchEvent::Ended, false), HandPoint::UNTRACKED], &mut items);
        assert!(f.tapped.is_empty() && f.ended == vec![(0, 0)], "moved 20 cm is not a tap");

        let mut sys = DragSystem::default();
        let mut items = orbs();
        sys.update(&[hand(v(0.0, 0.0, -1.0), PinchEvent::Started, true), HandPoint::UNTRACKED], &mut items);
        for _ in 0..TAP_FRAMES + 5 {
            sys.update(&[hand(v(0.0, 0.0, -1.0), PinchEvent::None, true), HandPoint::UNTRACKED], &mut items);
        }
        let f = sys.update(&[hand(v(0.0, 0.0, -1.0), PinchEvent::Ended, false), HandPoint::UNTRACKED], &mut items);
        assert!(f.tapped.is_empty(), "held more than three seconds is not a tap");
    }

    #[test]
    fn a_released_item_stops_following_the_hand() {
        let mut sys = DragSystem::default();
        let mut items = orbs();
        sys.update(&[hand(v(0.0, 0.0, -1.0), PinchEvent::Started, true), HandPoint::UNTRACKED], &mut items);
        sys.release_item(0);
        let before = items[0].center;
        let f = sys.update(&[hand(v(0.4, 0.4, -1.0), PinchEvent::None, true), HandPoint::UNTRACKED], &mut items);
        assert_eq!(f.grabbed, [None, None]);
        assert_eq!(items[0].center, before);
    }

    #[test]
    fn hover_is_reported_only_when_not_pinching() {
        let mut sys = DragSystem::default();
        let mut items = orbs();
        let f = sys.update(&[hand(v(0.02, 0.0, -1.0), PinchEvent::None, false), HandPoint::UNTRACKED], &mut items);
        assert_eq!(f.hovered, [Some(0), None]);
        let f = sys.update(&[hand(v(0.9, 0.9, -1.0), PinchEvent::None, false), HandPoint::UNTRACKED], &mut items);
        assert_eq!(f.hovered, [None, None]);
    }

    #[test]
    fn a_grab_survives_the_list_shrinking_without_panicking() {
        let mut sys = DragSystem::default();
        let mut items = orbs();
        sys.update(&[hand(v(0.5, 0.0, -1.0), PinchEvent::Started, true), HandPoint::UNTRACKED], &mut items);
        let mut fewer = vec![items[0]]; // orb 1 disappears
        let f = sys.update(&[hand(v(0.5, 0.0, -1.0), PinchEvent::None, true), HandPoint::UNTRACKED], &mut fewer);
        assert_eq!(f.grabbed, [None, None]);
        assert!(f.ended.is_empty(), "there is nothing to report about an object that no longer exists");
    }

    // ----- hand closed around an orb -----

    fn orb(center: Vec3) -> Draggable {
        Draggable { center, radius: 0.05, grasp: true }
    }

    /// A closed hand with the grab center at `point`.
    fn closed(point: Vec3, grasp: PinchEvent, grasping: bool) -> HandPoint {
        HandPoint { tracked: true, grasp_point: point, grasp, grasping, ..HandPoint::UNTRACKED }
    }

    #[test]
    fn grasp_closes_on_a_ball_and_opens_with_hysteresis() {
        let palm = v(0.0, 1.0, -0.4);
        let tips_at = |d: f32| [palm + Vec3::new(0.0, 0.0, -d); 4];
        let mut g = Grasp::default();
        assert_eq!(g.update(palm, &tips_at(0.16)), PinchEvent::None, "open hand");
        assert_eq!(g.update(palm, &tips_at(0.08)), PinchEvent::Started, "closed around something");
        assert_eq!(g.update(palm, &tips_at(0.10)), PinchEvent::None, "between the two thresholds it stays closed");
        assert!(g.is_active());
        assert_eq!(g.update(palm, &tips_at(0.13)), PinchEvent::Ended);
        assert_eq!(g.update(palm, &tips_at(0.08)), PinchEvent::Started);
        assert_eq!(g.lost(), PinchEvent::Ended);
        assert_eq!(g.lost(), PinchEvent::None);
    }

    #[test]
    fn the_grasp_point_is_half_way_to_the_fingertips() {
        let palm = v(0.0, 1.0, -0.4);
        let tips = [palm + Vec3::new(0.0, 0.0, -0.08); 4];
        assert!((grasp_point(palm, &tips) - (palm + Vec3::new(0.0, 0.0, -0.04))).length() < 1e-6);
    }

    #[test]
    fn closing_the_hand_around_an_orb_grabs_and_carries_it_and_opening_drops_it() {
        let mut sys = DragSystem::default();
        let mut items = [orb(v(0.0, 0.0, -1.0))];
        let f = sys.update(&[closed(v(0.0, 0.01, -1.0), PinchEvent::Started, true), HandPoint::UNTRACKED], &mut items);
        assert_eq!(f.started, vec![(0, 0)]);
        sys.update(&[closed(v(0.3, 0.2, -0.8), PinchEvent::None, true), HandPoint::UNTRACKED], &mut items);
        assert!(items[0].center.distance(v(0.3, 0.2, -0.8)) < 0.012, "the orb follows the hand");
        for _ in 0..60 {
            sys.update(&[closed(v(0.3, 0.2, -0.8), PinchEvent::None, true), HandPoint::UNTRACKED], &mut items);
        }
        assert!(items[0].center.distance(v(0.3, 0.2, -0.8)) < 1e-4, "and ends up centered in the hand");
        let f = sys.update(&[closed(v(0.3, 0.2, -0.8), PinchEvent::Ended, false), HandPoint::UNTRACKED], &mut items);
        assert_eq!(f.ended, vec![(0, 0)]);
        assert_eq!(sys.grabbed(), [None, None]);
    }

    #[test]
    fn a_pinch_no_longer_grabs_an_orb_but_still_grabs_a_card() {
        let mut sys = DragSystem::default();
        let mut items = [orb(v(0.0, 0.0, -1.0)), Draggable { center: v(0.5, 0.0, -1.0), radius: 0.05, grasp: false }];
        let f = sys.update(&[hand(v(0.0, 0.0, -1.0), PinchEvent::Started, true), HandPoint::UNTRACKED], &mut items);
        assert!(f.started.is_empty(), "pinching an orb does not grab it");
        let f = sys.update(&[hand(v(0.5, 0.0, -1.0), PinchEvent::Started, true), HandPoint::UNTRACKED], &mut items);
        assert_eq!(f.started, vec![(0, 1)], "a card does");
    }

    #[test]
    fn a_closed_hand_far_from_the_orb_or_closing_on_a_card_grabs_nothing() {
        let mut sys = DragSystem::default();
        let mut items = [orb(v(0.0, 0.0, -1.0)), Draggable { center: v(0.5, 0.0, -1.0), radius: 0.05, grasp: false }];
        let f = sys.update(&[closed(v(0.3, 0.0, -1.0), PinchEvent::Started, true), HandPoint::UNTRACKED], &mut items);
        assert!(f.started.is_empty(), "far from the orb");
        let mut sys = DragSystem::default();
        let f = sys.update(&[closed(v(0.5, 0.0, -1.0), PinchEvent::Started, true), HandPoint::UNTRACKED], &mut items);
        assert!(f.started.is_empty(), "cards are not grabbed with the closed hand");
    }

    #[test]
    fn a_held_orb_is_not_dropped_by_a_pinch_ending_and_two_hands_hold_two_orbs() {
        let mut sys = DragSystem::default();
        let mut items = [orb(v(0.0, 0.0, -1.0)), orb(v(0.4, 0.0, -1.0))];
        let both = |a: HandPoint, b: HandPoint| [a, b];
        sys.update(&both(closed(v(0.0, 0.0, -1.0), PinchEvent::Started, true), closed(v(0.4, 0.0, -1.0), PinchEvent::Started, true)), &mut items);
        assert_eq!(sys.grabbed(), [Some(0), Some(1)]);
        // A pinch ending on hand 0 does not release the orb: it is held with the closed hand.
        let mut a = closed(v(0.0, 0.0, -1.0), PinchEvent::None, true);
        a.pinch = PinchEvent::Ended;
        sys.update(&both(a, closed(v(0.4, 0.0, -1.0), PinchEvent::None, true)), &mut items);
        assert_eq!(sys.grabbed(), [Some(0), Some(1)]);
    }

    #[test]
    fn a_hand_hovering_an_orb_is_reported_without_grabbing() {
        let mut sys = DragSystem::default();
        let mut items = [orb(v(0.0, 0.0, -1.0))];
        let f = sys.update(&[closed(v(0.0, 0.05, -1.0), PinchEvent::None, false), HandPoint::UNTRACKED], &mut items);
        assert_eq!(f.hovered[0], Some(0));
        assert!(f.started.is_empty());
    }
}
