//! Mapping a wall with the hand: when the open palm rests still against it, the plane of
//! the hand is recorded as the wall. It is the vertical version of `desk`.

use glam::Vec3;

use crate::desk::{HOLD_FRAMES, OPEN_REACH, STILL_RADIUS};

/// How far (m) a finger can stray from the hand's plane and still count as flat.
pub const PLANE_TOLERANCE: f32 = 0.025;
/// Maximum tilt of the plane from vertical (vertical component of the normal).
pub const MAX_NORMAL_Y: f32 = 0.35;
/// The index and little fingers must spread enough to define a plane (sine of the angle).
const MIN_SPREAD: f32 = 0.15;
/// Minimum distance (m) between the palm and the head: a hand against the face is not a wall.
const MIN_FROM_HEAD: f32 = 0.45;
/// Thickness (m) of the hand between the palm and the surface.
const PALM_THICKNESS: f32 = 0.015;

/// A wall: a vertical plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wall {
    /// A point on the wall.
    pub point: Vec3,
    /// Horizontal unit normal, toward the user's side.
    pub normal: Vec3,
}

impl Wall {
    /// Signed distance from `p` to the wall: positive on the user's side.
    pub fn distance(&self, p: Vec3) -> f32 {
        (p - self.point).dot(self.normal)
    }
}

/// Watches a hand until it stays open, still and flat against a wall.
#[derive(Debug, Clone, Copy, Default)]
pub struct WallCalibrator {
    anchor: Vec3,
    frames: u32,
}

impl WallCalibrator {
    /// Progress toward the calibration, from 0 to 1.
    pub fn progress(&self) -> f32 {
        (self.frames as f32 / HOLD_FRAMES as f32).min(1.0)
    }

    pub fn reset(&mut self) {
        self.frames = 0;
    }

    /// `palm` and the index, middle, ring and little fingertips; `head` is the head.
    pub fn update(&mut self, palm: Vec3, tips: &[Vec3; 4], head: Vec3) -> Option<Wall> {
        match Self::plane_of(palm, tips, head) {
            Some(wall) => self.hold(palm, wall),
            None => {
                self.frames = 0;
                None
            }
        }
    }

    fn hold(&mut self, palm: Vec3, wall: Wall) -> Option<Wall> {
        if self.frames == 0 || palm.distance(self.anchor) > STILL_RADIUS {
            self.anchor = palm;
            self.frames = 0;
        }
        self.frames += 1;
        if self.frames < HOLD_FRAMES {
            return None;
        }
        self.frames = 0;
        Some(wall)
    }

    /// The plane of an open, flat hand, if it is vertical and faces the user.
    pub fn plane_of(palm: Vec3, tips: &[Vec3; 4], head: Vec3) -> Option<Wall> {
        if tips.iter().any(|t| t.distance(palm) < OPEN_REACH) {
            return None;
        }
        let (index, little) = (tips[0] - palm, tips[3] - palm);
        let cross = index.cross(little);
        if cross.length() < MIN_SPREAD * index.length() * little.length() {
            return None;
        }
        let mut n = cross.normalize();
        if tips[1..3].iter().any(|t| n.dot(*t - palm).abs() > PLANE_TOLERANCE) {
            return None;
        }
        if n.y.abs() > MAX_NORMAL_Y {
            return None;
        }
        n.y = 0.0;
        let mut normal = n.try_normalize()?;
        if normal.dot(head - palm) < 0.0 {
            normal = -normal;
        }
        let from_head = Vec3::new(palm.x - head.x, 0.0, palm.z - head.z).length();
        if from_head < MIN_FROM_HEAD {
            return None;
        }
        Some(Wall { point: palm - normal * PALM_THICKNESS, normal })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: Vec3 = Vec3::new(0.0, 1.2, 0.0);

    /// Hand with the fingers up, flat against the wall x = `wall_x`.
    fn hand_on_wall(wall_x: f32) -> (Vec3, [Vec3; 4]) {
        let palm = Vec3::new(wall_x - 0.015, 1.1, -0.5);
        let tip = |dz: f32| palm + Vec3::new(0.0, 0.09, dz);
        (palm, [tip(0.03), tip(0.01), tip(-0.01), tip(-0.03)])
    }

    fn hold(c: &mut WallCalibrator, palm: Vec3, tips: &[Vec3; 4], frames: u32) -> Option<Wall> {
        let mut last = None;
        for _ in 0..frames {
            last = c.update(palm, tips, HEAD);
        }
        last
    }

    #[test]
    fn a_flat_still_hand_against_a_wall_maps_it() {
        let (palm, tips) = hand_on_wall(1.0);
        let wall = hold(&mut WallCalibrator::default(), palm, &tips, HOLD_FRAMES).expect("debe calibrar");
        // The normal is horizontal and points toward the user (who is at x = 0, on the left).
        assert!((wall.normal - Vec3::new(-1.0, 0.0, 0.0)).length() < 1e-4, "{}", wall.normal);
        assert!((wall.distance(Vec3::new(1.0, 1.0, 0.0))).abs() < 0.02, "the wall goes through x = 1");
        assert!(wall.distance(HEAD) > 0.9, "the user is on the positive side");
    }

    #[test]
    fn the_normal_always_faces_the_user() {
        // The same wall seen from the other side.
        let (palm, tips) = hand_on_wall(-1.0);
        let wall = hold(&mut WallCalibrator::default(), palm, &tips, HOLD_FRAMES).unwrap();
        assert!((wall.normal - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-4, "{}", wall.normal);
    }

    #[test]
    fn it_needs_the_full_wait_and_stillness() {
        let (palm, tips) = hand_on_wall(1.0);
        let mut c = WallCalibrator::default();
        assert!(hold(&mut c, palm, &tips, HOLD_FRAMES - 1).is_none());
        let moved = palm + Vec3::new(0.0, 0.0, 0.2);
        let moved_tips = tips.map(|t| t + Vec3::new(0.0, 0.0, 0.2));
        assert!(hold(&mut c, moved, &moved_tips, 5).is_none(), "after moving, it starts again");
        assert!(c.progress() < 0.2);
    }

    #[test]
    fn a_hand_flat_on_a_desk_is_not_a_wall() {
        let palm = Vec3::new(0.0, 0.55, -0.4);
        let tip = |dx: f32| palm + Vec3::new(dx, -0.008, -0.09);
        let tips = [tip(-0.03), tip(-0.01), tip(0.01), tip(0.03)];
        assert!(hold(&mut WallCalibrator::default(), palm, &tips, HOLD_FRAMES * 2).is_none());
    }

    #[test]
    fn a_fist_or_curled_fingers_are_not_a_wall() {
        let (palm, _) = hand_on_wall(1.0);
        let tips = [palm + Vec3::new(0.0, 0.03, 0.0); 4];
        assert!(hold(&mut WallCalibrator::default(), palm, &tips, HOLD_FRAMES * 2).is_none());
        // Fingers spread but bent: the middle finger leaves the plane.
        let (palm, mut tips) = hand_on_wall(1.0);
        tips[1] += Vec3::new(-0.06, 0.0, 0.0);
        assert!(hold(&mut WallCalibrator::default(), palm, &tips, HOLD_FRAMES * 2).is_none());
    }

    #[test]
    fn a_hand_at_the_face_is_not_a_wall() {
        let palm = Vec3::new(0.1, 1.2, -0.1);
        let tip = |dz: f32| palm + Vec3::new(0.0, 0.09, dz);
        let tips = [tip(0.03), tip(0.01), tip(-0.01), tip(-0.03)];
        assert!(hold(&mut WallCalibrator::default(), palm, &tips, HOLD_FRAMES * 2).is_none());
    }
}
