//! Per-eye cameras from what OpenXR delivers (a pose and a field of view).
//!
//! Conventions: OpenXR axes (X right, Y up, -Z forward) and, for Vulkan,
//! clip space with Y down and depth in `0..1`.

use glam::{Mat4, Quat, Vec3};

/// Field of view of an eye, in radians, as OpenXR gives it: `left` and `down` are
/// negative, `right` and `up` positive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fov {
    pub left: f32,
    pub right: f32,
    pub up: f32,
    pub down: f32,
}

impl Fov {
    /// Symmetric field of `fov_y` radians vertically and the given aspect ratio.
    pub fn symmetric(fov_y: f32, aspect: f32) -> Self {
        let half_y = fov_y / 2.0;
        let half_x = (half_y.tan() * aspect).atan();
        Self { left: -half_x, right: half_x, up: half_y, down: -half_y }
    }
}

/// Projection for Vulkan. With `far <= near` the far plane is at infinity.
///
/// It is OpenXR's asymmetric formulation (the optical center need not be in
/// the middle of the image), like Khronos's `XrMatrix4x4f_CreateProjectionFov`.
pub fn projection(fov: Fov, near: f32, far: f32) -> Mat4 {
    let (tan_l, tan_r) = (fov.left.tan(), fov.right.tan());
    let (tan_u, tan_d) = (fov.up.tan(), fov.down.tan());
    let width = tan_r - tan_l;
    // Vulkan's clip space has Y pointing down.
    let height = tan_d - tan_u;

    let (z_scale, z_offset) = if far <= near {
        (-1.0, -near)
    } else {
        (-far / (far - near), -(far * near) / (far - near))
    };

    // Columns, as in GLSL.
    Mat4::from_cols_array(&[
        2.0 / width, 0.0, 0.0, 0.0, //
        0.0, 2.0 / height, 0.0, 0.0, //
        (tan_r + tan_l) / width, (tan_u + tan_d) / height, z_scale, -1.0, //
        0.0, 0.0, z_offset, 0.0,
    ])
}

/// View matrix (from the world to the eye) for an eye at `position` with `orientation`.
pub fn view_matrix(position: Vec3, orientation: Quat) -> Mat4 {
    Mat4::from_rotation_translation(orientation, position).inverse()
}

/// What each eye's shader needs.
#[derive(Debug, Clone, Copy)]
pub struct EyeCamera {
    pub view_proj: Mat4,
    /// Position of the eye in the world (to compute the view direction).
    pub position: Vec3,
}

pub fn eye_camera(position: Vec3, orientation: Quat, fov: Fov, near: f32, far: f32) -> EyeCamera {
    EyeCamera { view_proj: projection(fov, near, far) * view_matrix(position, orientation), position }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec4;

    /// Normalized coordinates (x, y, z) of a world point.
    fn ndc(cam: &EyeCamera, p: Vec3) -> Vec3 {
        let c = cam.view_proj * Vec4::new(p.x, p.y, p.z, 1.0);
        Vec3::new(c.x / c.w, c.y / c.w, c.z / c.w)
    }

    fn assert_close(a: f32, b: f32, what: &str) {
        assert!((a - b).abs() < 1e-4, "{what}: {a} != {b}");
    }

    fn square_fov() -> Fov {
        // 90° on each axis: the edges are at tan(45°) = 1.
        Fov { left: -std::f32::consts::FRAC_PI_4, right: std::f32::consts::FRAC_PI_4, up: std::f32::consts::FRAC_PI_4, down: -std::f32::consts::FRAC_PI_4 }
    }

    #[test]
    fn the_point_straight_ahead_lands_in_the_center() {
        let cam = eye_camera(Vec3::ZERO, Quat::IDENTITY, square_fov(), 0.1, 100.0);
        let p = ndc(&cam, Vec3::new(0.0, 0.0, -2.0));
        assert_close(p.x, 0.0, "x");
        assert_close(p.y, 0.0, "y");
    }

    #[test]
    fn screen_edges_match_the_field_of_view() {
        let cam = eye_camera(Vec3::ZERO, Quat::IDENTITY, square_fov(), 0.1, 100.0);
        assert_close(ndc(&cam, Vec3::new(1.0, 0.0, -1.0)).x, 1.0, "borde derecho");
        assert_close(ndc(&cam, Vec3::new(-1.0, 0.0, -1.0)).x, -1.0, "borde izquierdo");
        // Vulkan: Y points down, so what is above comes out with a negative Y.
        assert_close(ndc(&cam, Vec3::new(0.0, 1.0, -1.0)).y, -1.0, "borde superior");
        assert_close(ndc(&cam, Vec3::new(0.0, -1.0, -1.0)).y, 1.0, "borde inferior");
    }

    #[test]
    fn depth_runs_from_zero_at_the_near_plane_to_one_at_the_far_plane() {
        let cam = eye_camera(Vec3::ZERO, Quat::IDENTITY, square_fov(), 0.5, 50.0);
        assert_close(ndc(&cam, Vec3::new(0.0, 0.0, -0.5)).z, 0.0, "plano cercano");
        assert_close(ndc(&cam, Vec3::new(0.0, 0.0, -50.0)).z, 1.0, "plano lejano");
        let mid = ndc(&cam, Vec3::new(0.0, 0.0, -5.0)).z;
        assert!(mid > 0.0 && mid < 1.0, "intermediate depth out of range: {mid}");
    }

    #[test]
    fn infinite_far_plane_keeps_depth_below_one() {
        let cam = eye_camera(Vec3::ZERO, Quat::IDENTITY, square_fov(), 0.1, 0.0);
        let far = ndc(&cam, Vec3::new(0.0, 0.0, -100_000.0)).z;
        assert!(far < 1.0 && far > 0.99, "{far}");
    }

    #[test]
    fn asymmetric_fov_shifts_the_optical_center() {
        // The eye sees more to the right than to the left.
        let fov = Fov { left: -0.5, right: 0.9, up: 0.7, down: -0.7 };
        let cam = eye_camera(Vec3::ZERO, Quat::IDENTITY, fov, 0.1, 100.0);
        assert_close(ndc(&cam, Vec3::new(fov.right.tan(), 0.0, -1.0)).x, 1.0, "asymmetric right edge");
        assert_close(ndc(&cam, Vec3::new(fov.left.tan(), 0.0, -1.0)).x, -1.0, "asymmetric left edge");
        assert!(ndc(&cam, Vec3::new(0.0, 0.0, -1.0)).x < 0.0, "the center must be shifted");
    }

    #[test]
    fn view_matrix_follows_the_head_position() {
        let v = view_matrix(Vec3::new(1.0, 0.5, 0.0), Quat::IDENTITY);
        let p = v.transform_point3(Vec3::new(1.0, 0.5, -3.0));
        assert!((p - Vec3::new(0.0, 0.0, -3.0)).length() < 1e-5, "{p}");
    }

    #[test]
    fn view_matrix_follows_the_head_rotation() {
        // The head turns 90° to the left (around Y): what was on its left
        // (-X) pasa a verse justo delante (-Z).
        let q = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let v = view_matrix(Vec3::ZERO, q);
        let p = v.transform_point3(Vec3::new(-2.0, 0.0, 0.0));
        assert!((p - Vec3::new(0.0, 0.0, -2.0)).length() < 1e-5, "{p}");
    }

    #[test]
    fn what_is_behind_the_eye_has_negative_w() {
        let cam = eye_camera(Vec3::ZERO, Quat::IDENTITY, square_fov(), 0.1, 100.0);
        let c = cam.view_proj * Vec4::new(0.0, 0.0, 2.0, 1.0);
        assert!(c.w < 0.0, "points behind the eye must be clipped");
    }

    #[test]
    fn symmetric_fov_is_centered() {
        let f = Fov::symmetric(1.0, 1.5);
        assert_close(f.left, -f.right, "horizontal symmetry");
        assert_close(f.up, -f.down, "vertical symmetry");
        assert_close(f.right.tan() / f.up.tan(), 1.5, "aspect ratio");
    }
}
