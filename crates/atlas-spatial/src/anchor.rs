//! Anchoring the scene to the user: placing panels and orbs **where they are and where they
//! are looking**, and not relative to the point where the headset was when the app opened.
//!
//! Only the head's position and its heading (rotation around the vertical) are kept.
//! Looking up or down and rolling are ignored, so that the scene
//! stays level even if you are looking at the floor when anchoring it.

use glam::{Quat, Vec3};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    /// Position of the head (with its height).
    pub origin: Vec3,
    /// Rotation around the vertical axis: 0 looks toward -Z, positive turns left.
    pub yaw: f32,
}

impl Anchor {
    /// Unanchored: equivalent to the origin of the reference space.
    pub const IDENTITY: Anchor = Anchor { origin: Vec3::ZERO, yaw: 0.0 };

    /// Anchors to a head at `position` with `orientation`.
    pub fn from_head(position: Vec3, orientation: Quat) -> Anchor {
        let forward = orientation * Vec3::NEG_Z;
        let mut facing = Vec3::new(forward.x, 0.0, forward.z);
        // When looking almost vertically, the forward heading is unstable: the direction the top
        // of the head points to is used instead (which points forward when looking down).
        if facing.length() < 0.2 {
            let up = orientation * Vec3::Y;
            let flat = Vec3::new(up.x, 0.0, up.z);
            facing = if forward.y < 0.0 { flat } else { -flat };
        }
        let yaw = if facing.length() < 1e-4 { 0.0 } else { (-facing.x).atan2(-facing.z) };
        Anchor { origin: position, yaw }
    }

    /// Rotation of the anchor.
    pub fn rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw)
    }

    /// World point for one expressed relative to the user: X to their right, Y up
    /// and -Z ahead.
    pub fn point(&self, relative: Vec3) -> Vec3 {
        self.origin + self.rotation() * relative
    }

    /// Orientation of a panel that faces the user, tilted `pitch` radians around
    /// its horizontal axis (negative tilts the top backward).
    pub fn orientation(&self, pitch: f32) -> Quat {
        self.rotation() * Quat::from_rotation_x(pitch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn close(a: Vec3, b: Vec3) {
        assert!((a - b).length() < 1e-4, "{a} != {b}");
    }

    #[test]
    fn a_head_looking_forward_at_the_origin_is_the_identity() {
        let a = Anchor::from_head(Vec3::ZERO, Quat::IDENTITY);
        assert!(a.yaw.abs() < 1e-6);
        close(a.point(Vec3::new(0.0, 0.0, -1.0)), Vec3::new(0.0, 0.0, -1.0));
    }

    #[test]
    fn the_scene_keeps_the_height_and_position_of_the_head() {
        let a = Anchor::from_head(Vec3::new(2.0, 1.2, -3.0), Quat::IDENTITY);
        // In front and a little below eye level.
        close(a.point(Vec3::new(0.0, -0.2, -0.5)), Vec3::new(2.0, 1.0, -3.5));
    }

    #[test]
    fn the_scene_turns_with_the_head() {
        // Head turned 90° to the left: now "ahead" is -X.
        let a = Anchor::from_head(Vec3::ZERO, Quat::from_rotation_y(FRAC_PI_2));
        close(a.point(Vec3::new(0.0, 0.0, -1.0)), Vec3::new(-1.0, 0.0, 0.0));
        // and "to my right" is... with the head looking toward -X,
        // its right is -Z.
        close(a.point(Vec3::new(1.0, 0.0, 0.0)), Vec3::new(0.0, 0.0, -1.0));
    }

    #[test]
    fn looking_up_or_down_does_not_tilt_the_scene() {
        let level = Anchor::from_head(Vec3::ZERO, Quat::IDENTITY);
        for pitch in [-1.0f32, -0.5, 0.5, 1.0] {
            let a = Anchor::from_head(Vec3::ZERO, Quat::from_rotation_x(pitch));
            assert!((a.yaw - level.yaw).abs() < 1e-5, "with pitch {pitch} the heading changed");
            close(a.point(Vec3::new(0.0, 0.0, -1.0)), Vec3::new(0.0, 0.0, -1.0));
        }
    }

    #[test]
    fn head_roll_is_ignored() {
        let a = Anchor::from_head(Vec3::ZERO, Quat::from_rotation_z(0.6));
        assert!(a.yaw.abs() < 1e-5);
    }

    #[test]
    fn looking_straight_down_still_faces_the_way_the_head_points() {
        // Head turned 90° to the left and looking all the way down.
        let q = Quat::from_rotation_y(FRAC_PI_2) * Quat::from_rotation_x(-FRAC_PI_2 + 0.01);
        let a = Anchor::from_head(Vec3::ZERO, q);
        close(a.point(Vec3::new(0.0, 0.0, -1.0)), Vec3::new(-1.0, 0.0, 0.0));
    }

    #[test]
    fn looking_straight_up_is_stable_too() {
        let q = Quat::from_rotation_y(-FRAC_PI_2) * Quat::from_rotation_x(FRAC_PI_2 - 0.01);
        let a = Anchor::from_head(Vec3::ZERO, q);
        close(a.point(Vec3::new(0.0, 0.0, -1.0)), Vec3::new(1.0, 0.0, 0.0));
    }

    #[test]
    fn panel_orientation_adds_the_pitch_on_top_of_the_heading() {
        let a = Anchor::from_head(Vec3::ZERO, Quat::from_rotation_y(FRAC_PI_2));
        // A panel's normal (+Z) faces the user.
        let normal = a.orientation(0.0) * Vec3::Z;
        close(normal, Vec3::new(1.0, 0.0, 0.0)); // the head looks toward -X, the panel faces +X (toward it)
        // Tilted backward (the top of the panel moving away): the normal points somewhat up.
        let tilted = a.orientation(-0.4) * Vec3::Z;
        assert!(tilted.y > 0.3, "{tilted}");
    }
}
