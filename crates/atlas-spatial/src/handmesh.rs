//! The mesh of a hand (the one the runtime delivers) and its skinning with each frame's
//! joints (skinning on the CPU). Data only: tested on the PC.

use glam::{Mat4, Quat, Vec3};

/// Joints of a hand in OpenXR.
pub const JOINTS: usize = 26;
/// With fewer reliable joints than this the hand is not drawn (it would come out deformed).
const MIN_JOINTS: u32 = 20;

/// Vertex ready to draw: position and normal in the reference space.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
}

/// The mesh of a hand in its rest pose, with each vertex's weights per joint.
#[derive(Debug, Clone)]
pub struct HandMeshData {
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    blend_indices: Vec<[u16; 4]>,
    blend_weights: Vec<[f32; 4]>,
    pub indices: Vec<u16>,
    /// Inverse of each joint's rest pose.
    bind_inverse: [Mat4; JOINTS],
}

impl HandMeshData {
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// Builds the mesh from its parts (for tests and for `fetch`).
    pub fn from_parts(
        positions: Vec<Vec3>,
        normals: Vec<Vec3>,
        blend_indices: Vec<[u16; 4]>,
        blend_weights: Vec<[f32; 4]>,
        indices: Vec<u16>,
        bind_poses: &[Mat4; JOINTS],
    ) -> Self {
        let bind_inverse = bind_poses.map(|m| m.inverse());
        Self { positions, normals, blend_indices, blend_weights, indices, bind_inverse }
    }

    /// Skins the mesh with this frame's joints. `None` if too many
    /// reliable joints are missing for the hand to come out right.
    pub fn skin(&self, joints: &[Vec3; JOINTS], rotations: &[Quat; JOINTS], mask: u32) -> Option<Vec<HandVertex>> {
        if mask.count_ones() < MIN_JOINTS {
            return None;
        }
        let mut skin_matrix = [Mat4::IDENTITY; JOINTS];
        for i in 0..JOINTS {
            if mask & (1 << i) != 0 {
                skin_matrix[i] = Mat4::from_rotation_translation(rotations[i], joints[i]) * self.bind_inverse[i];
            }
        }
        Some(
            (0..self.positions.len())
                .map(|v| {
                    let (mut pos, mut normal, mut total) = (Vec3::ZERO, Vec3::ZERO, 0.0f32);
                    for k in 0..4 {
                        let (j, w) = (self.blend_indices[v][k] as usize, self.blend_weights[v][k]);
                        if w > 0.0 && j < JOINTS && mask & (1 << j) != 0 {
                            pos += skin_matrix[j].transform_point3(self.positions[v]) * w;
                            normal += skin_matrix[j].transform_vector3(self.normals[v]) * w;
                            total += w;
                        }
                    }
                    if total > 0.0 {
                        pos /= total;
                    }
                    let n = normal.try_normalize().unwrap_or(Vec3::Y);
                    HandVertex { pos: pos.to_array(), normal: n.to_array() }
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> u32 {
        (1u32 << JOINTS) - 1
    }

    fn rest() -> [Vec3; JOINTS] {
        [Vec3::ZERO; JOINTS]
    }

    fn quats() -> [Quat; JOINTS] {
        [Quat::IDENTITY; JOINTS]
    }

    /// A triangle weighted fully to joint 3 (and another to joint 4).
    fn mesh() -> HandMeshData {
        HandMeshData::from_parts(
            vec![Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0)],
            vec![Vec3::Y; 3],
            vec![[3, 0, 0, 0], [3, 0, 0, 0], [3, 4, 0, 0]],
            vec![[1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [0.5, 0.5, 0.0, 0.0]],
            vec![0, 1, 2],
            &[Mat4::IDENTITY; JOINTS],
        )
    }

    #[test]
    fn a_resting_hand_leaves_the_mesh_where_it_is() {
        let v = mesh().skin(&rest(), &quats(), all()).unwrap();
        assert!((Vec3::from(v[0].pos) - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-6);
        assert!((Vec3::from(v[2].pos) - Vec3::new(0.0, 0.0, 1.0)).length() < 1e-6);
    }

    #[test]
    fn a_moved_joint_drags_its_vertices() {
        let mut j = rest();
        j[3] = Vec3::new(0.0, 0.0, -2.0);
        let v = mesh().skin(&j, &quats(), all()).unwrap();
        assert!((Vec3::from(v[0].pos) - Vec3::new(1.0, 0.0, -2.0)).length() < 1e-5);
    }

    #[test]
    fn shared_weights_blend_between_joints() {
        let mut j = rest();
        j[3] = Vec3::new(0.0, 2.0, 0.0);
        j[4] = Vec3::new(0.0, -2.0, 0.0);
        let v = mesh().skin(&j, &quats(), all()).unwrap();
        // Half and half between +2 and -2 in Y: the vertex does not move in Y.
        assert!((Vec3::from(v[2].pos) - Vec3::new(0.0, 0.0, 1.0)).length() < 1e-5, "{:?}", v[2].pos);
    }

    #[test]
    fn rotating_a_joint_rotates_positions_and_normals() {
        let mut r = quats();
        r[3] = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let v = mesh().skin(&rest(), &r, all()).unwrap();
        assert!((Vec3::from(v[0].pos) - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-5, "{:?}", v[0].pos);
        assert!((Vec3::from(v[0].normal) - Vec3::new(-1.0, 0.0, 0.0)).length() < 1e-5, "{:?}", v[0].normal);
    }

    #[test]
    fn the_bind_pose_is_undone_before_moving() {
        // If the joint at rest was at (5,0,0) and is still there, the vertex does not move.
        let mut bind = [Mat4::IDENTITY; JOINTS];
        bind[3] = Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0));
        let m = HandMeshData::from_parts(vec![Vec3::new(6.0, 0.0, 0.0)], vec![Vec3::Y], vec![[3, 0, 0, 0]], vec![[1.0, 0.0, 0.0, 0.0]], vec![0], &bind);
        let mut j = rest();
        j[3] = Vec3::new(5.0, 0.0, 0.0);
        let v = m.skin(&j, &quats(), all()).unwrap();
        assert!((Vec3::from(v[0].pos) - Vec3::new(6.0, 0.0, 0.0)).length() < 1e-5);
    }

    #[test]
    fn a_hand_with_too_few_valid_joints_is_not_drawn() {
        assert!(mesh().skin(&rest(), &quats(), 0b111).is_none());
        assert!(mesh().skin(&rest(), &quats(), 0).is_none());
    }

    #[test]
    fn joints_that_are_not_valid_are_ignored_not_zeroed() {
        let mut j = rest();
        j[3] = Vec3::new(9.0, 9.0, 9.0);
        let without_3 = all() & !(1 << 3);
        let v = mesh().skin(&j, &quats(), without_3).unwrap();
        // Joint 3 is not reliable: vertex 0 (joint 3 only) is left without weights and does not fly off to (9,9,9).
        assert!(Vec3::from(v[0].pos).length() < 1.0 + 1e-5 || Vec3::from(v[0].pos) == Vec3::ZERO);
    }
}
