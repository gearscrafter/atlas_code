//! Mapping the desk with the hand: when the open palm rests still on the desk, its
//! height is recorded as the work surface.
//!
//! It only works with hand positions (the palm and the fingertips), so it is
//! tested on the PC.

use glam::Vec3;

/// Maximum height difference (m) between the palm and the tips to consider the hand flat.
pub const FLAT_SPREAD: f32 = 0.04;
/// Minimum distance (m) from the tips to the palm: a closed hand or a fist does not count.
pub const OPEN_REACH: f32 = 0.07;
/// The desk must be at least this far (m) below the eyes.
pub const MIN_BELOW_HEAD: f32 = 0.30;
/// ...and at most this far: lower is the floor or the lap in a low chair.
pub const MAX_BELOW_HEAD: f32 = 1.10;
/// How much (m) the hand can move during the wait without restarting it.
pub const STILL_RADIUS: f32 = 0.02;
/// Still frames needed (~0.8 s at 72 Hz).
pub const HOLD_FRAMES: u32 = 60;
/// Thickness (m) of the fingers: the desk is just below the lowest tip.
const FINGER_THICKNESS: f32 = 0.008;

/// The desk surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Desk {
    /// Height (m) of the desk in the reference space.
    pub height: f32,
    /// Point of the desk where the hand rested.
    pub center: Vec3,
}

/// Watches a hand until it stays flat and still on something.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeskCalibrator {
    anchor: Vec3,
    frames: u32,
}

impl DeskCalibrator {
    /// Progress toward the calibration, from 0 to 1.
    pub fn progress(&self) -> f32 {
        (self.frames as f32 / HOLD_FRAMES as f32).min(1.0)
    }

    pub fn reset(&mut self) {
        self.frames = 0;
    }

    /// `palm` and `tips` (index, middle, ring and little finger) of a hand; `head_y` is the height
    /// of the eyes. Returns the desk when the wait completes.
    pub fn update(&mut self, palm: Vec3, tips: &[Vec3; 4], head_y: f32) -> Option<Desk> {
        let (lo, hi) = tips.iter().fold((palm.y, palm.y), |(lo, hi), t| (lo.min(t.y), hi.max(t.y)));
        let below = head_y - palm.y;
        let flat = hi - lo <= FLAT_SPREAD;
        let open = tips.iter().all(|t| t.distance(palm) >= OPEN_REACH);
        let at_desk_height = (MIN_BELOW_HEAD..=MAX_BELOW_HEAD).contains(&below);
        if !(flat && open && at_desk_height) {
            self.frames = 0;
            return None;
        }
        if self.frames == 0 || palm.distance(self.anchor) > STILL_RADIUS {
            self.anchor = palm;
            self.frames = 0;
        }
        self.frames += 1;
        if self.frames < HOLD_FRAMES {
            return None;
        }
        self.frames = 0;
        let height = lo - FINGER_THICKNESS;
        Some(Desk { height, center: Vec3::new(palm.x, height, palm.z) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: f32 = 1.20;

    /// Open, flat hand with the palm at `y`, at (x, z).
    fn flat_hand(x: f32, y: f32, z: f32) -> (Vec3, [Vec3; 4]) {
        let palm = Vec3::new(x, y, z);
        let tip = |dx: f32| Vec3::new(x + dx, y - 0.008, z - 0.09);
        (palm, [tip(-0.03), tip(-0.01), tip(0.01), tip(0.03)])
    }

    fn hold(c: &mut DeskCalibrator, palm: Vec3, tips: &[Vec3; 4], frames: u32) -> Option<Desk> {
        let mut last = None;
        for _ in 0..frames {
            last = c.update(palm, tips, HEAD);
        }
        last
    }

    #[test]
    fn a_flat_still_hand_on_the_desk_maps_it() {
        let mut c = DeskCalibrator::default();
        let (palm, tips) = flat_hand(0.1, 0.55, -0.4);
        let desk = hold(&mut c, palm, &tips, HOLD_FRAMES).expect("debe calibrar");
        // The desk lies just below the lowest fingertip.
        assert!((desk.height - (0.55 - 0.008 - 0.008)).abs() < 1e-5, "{}", desk.height);
        assert!((desk.center.x - 0.1).abs() < 1e-6 && (desk.center.z + 0.4).abs() < 1e-6);
    }

    #[test]
    fn it_needs_the_full_wait() {
        let mut c = DeskCalibrator::default();
        let (palm, tips) = flat_hand(0.0, 0.55, -0.4);
        assert!(hold(&mut c, palm, &tips, HOLD_FRAMES - 1).is_none());
        assert!(c.progress() > 0.9 && c.progress() < 1.0);
    }

    #[test]
    fn moving_the_hand_restarts_the_wait() {
        let mut c = DeskCalibrator::default();
        let (palm, tips) = flat_hand(0.0, 0.55, -0.4);
        hold(&mut c, palm, &tips, HOLD_FRAMES - 5);
        let (palm2, tips2) = flat_hand(0.15, 0.55, -0.4);
        assert!(hold(&mut c, palm2, &tips2, 10).is_none(), "after moving, it starts again");
        assert!(c.progress() < 0.3);
    }

    #[test]
    fn a_tilted_hand_does_not_count() {
        let mut c = DeskCalibrator::default();
        let (palm, mut tips) = flat_hand(0.0, 0.55, -0.4);
        for t in tips.iter_mut() {
            t.y += 0.10; // fingers raised 10 cm: it is not resting
        }
        assert!(hold(&mut c, palm, &tips, HOLD_FRAMES * 2).is_none());
        assert_eq!(c.progress(), 0.0);
    }

    #[test]
    fn a_fist_does_not_count() {
        let mut c = DeskCalibrator::default();
        let palm = Vec3::new(0.0, 0.55, -0.4);
        let tips = [palm + Vec3::new(0.0, 0.0, -0.04); 4];
        assert!(hold(&mut c, palm, &tips, HOLD_FRAMES * 2).is_none());
    }

    #[test]
    fn only_surfaces_at_a_plausible_height_below_the_eyes() {
        for y in [HEAD - 0.1, HEAD - 1.5] {
            let mut c = DeskCalibrator::default();
            let (palm, tips) = flat_hand(0.0, y, -0.4);
            assert!(hold(&mut c, palm, &tips, HOLD_FRAMES * 2).is_none(), "height {y}");
        }
    }

    #[test]
    fn small_tremor_does_not_restart_it() {
        let mut c = DeskCalibrator::default();
        let (palm, tips) = flat_hand(0.0, 0.55, -0.4);
        let mut result = None;
        for i in 0..HOLD_FRAMES {
            let jitter = Vec3::new(0.003 * (i % 3) as f32, 0.0, 0.002 * (i % 2) as f32);
            result = c.update(palm + jitter, &tips.map(|t| t + jitter), HEAD);
        }
        assert!(result.is_some());
    }
}
