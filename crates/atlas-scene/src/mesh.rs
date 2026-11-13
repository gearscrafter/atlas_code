//! The meshes of the basic shapes, generated from their parameters. They all face outward with
//! counterclockwise faces (as glTF requires), in meters and with Y up.

use glam::Vec3;
use std::f32::consts::{PI, TAU};

/// A mesh ready to draw or export.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

impl MeshData {
    fn push(&mut self, p: Vec3, n: Vec3) -> u32 {
        self.positions.push(p.to_array());
        self.normals.push(n.to_array());
        (self.positions.len() - 1) as u32
    }

    /// Minimum and maximum corners, which glTF requires for the vertices.
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for p in &self.positions {
            for k in 0..3 {
                min[k] = min[k].min(p[k]);
                max[k] = max[k].max(p[k]);
            }
        }
        (min, max)
    }
}

/// A cube (or box) with sides `size`, centered at the origin.
pub fn cube(size: [f32; 3]) -> MeshData {
    let half = Vec3::from(size) * 0.5;
    let mut mesh = MeshData::default();
    // (normal, u, v) with u x v = normal: that way each face is counterclockwise seen from outside.
    let faces = [
        (Vec3::X, Vec3::Y, Vec3::Z),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::Z, Vec3::X),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::Y, Vec3::X),
    ];
    for (n, u, v) in faces {
        let (hn, hu, hv) = (n.abs().dot(half), u.abs().dot(half), v.abs().dot(half));
        let at = |su: f32, sv: f32| n * hn + u * (su * hu) + v * (sv * hv);
        let a = mesh.push(at(-1.0, -1.0), n);
        let b = mesh.push(at(1.0, -1.0), n);
        let c = mesh.push(at(1.0, 1.0), n);
        let d = mesh.push(at(-1.0, 1.0), n);
        mesh.indices.extend([a, b, c, a, c, d]);
    }
    mesh
}

/// A plane of `size` (width, height) in XY, facing +Z.
pub fn plane(size: [f32; 2]) -> MeshData {
    let (hw, hh) = (size[0] * 0.5, size[1] * 0.5);
    let mut mesh = MeshData::default();
    let a = mesh.push(Vec3::new(-hw, -hh, 0.0), Vec3::Z);
    let b = mesh.push(Vec3::new(hw, -hh, 0.0), Vec3::Z);
    let c = mesh.push(Vec3::new(hw, hh, 0.0), Vec3::Z);
    let d = mesh.push(Vec3::new(-hw, hh, 0.0), Vec3::Z);
    mesh.indices.extend([a, b, c, a, c, d]);
    mesh
}

/// A sphere of `radius`, with enough meridians and parallels to look round.
pub fn sphere(radius: f32) -> MeshData {
    const SEGMENTS: u32 = 24;
    const RINGS: u32 = 16;
    let mut mesh = MeshData::default();
    for i in 0..=RINGS {
        let theta = PI * i as f32 / RINGS as f32;
        for j in 0..=SEGMENTS {
            let phi = TAU * j as f32 / SEGMENTS as f32;
            let n = Vec3::new(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin());
            mesh.push(n * radius, n);
        }
    }
    for i in 0..RINGS {
        for j in 0..SEGMENTS {
            let a = i * (SEGMENTS + 1) + j;
            let b = a + SEGMENTS + 1;
            mesh.indices.extend([a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    mesh
}

/// A cylinder of `radius` and `height` along Y, centered at the origin.
pub fn cylinder(radius: f32, height: f32) -> MeshData {
    const SEGMENTS: u32 = 24;
    let half = height * 0.5;
    let mut mesh = MeshData::default();
    // Side: a column of vertices at the bottom and another at the top for each meridian.
    for j in 0..=SEGMENTS {
        let phi = TAU * j as f32 / SEGMENTS as f32;
        let n = Vec3::new(phi.cos(), 0.0, phi.sin());
        mesh.push(n * radius - Vec3::Y * half, n);
        mesh.push(n * radius + Vec3::Y * half, n);
    }
    for j in 0..SEGMENTS {
        let (bottom, top) = (2 * j, 2 * j + 1);
        let (next_bottom, next_top) = (2 * j + 2, 2 * j + 3);
        mesh.indices.extend([bottom, next_top, next_bottom, bottom, top, next_top]);
    }
    // Tapas.
    for (y, n) in [(half, Vec3::Y), (-half, Vec3::NEG_Y)] {
        let center = mesh.push(Vec3::Y * y, n);
        let ring: Vec<u32> = (0..=SEGMENTS)
            .map(|j| {
                let phi = TAU * j as f32 / SEGMENTS as f32;
                mesh.push(Vec3::new(phi.cos() * radius, y, phi.sin() * radius), n)
            })
            .collect();
        for j in 0..SEGMENTS as usize {
            if n.y > 0.0 {
                mesh.indices.extend([center, ring[j + 1], ring[j]]);
            } else {
                mesh.indices.extend([center, ring[j], ring[j + 1]]);
            }
        }
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<(&'static str, MeshData)> {
        vec![
            ("cubo", cube([0.2, 0.4, 0.6])),
            ("plano", plane([0.5, 0.3])),
            ("esfera", sphere(0.1)),
            ("cilindro", cylinder(0.05, 0.3)),
        ]
    }

    #[test]
    fn indices_are_in_range_and_triangles_come_in_threes() {
        for (name, m) in all() {
            assert_eq!(m.positions.len(), m.normals.len(), "{name}");
            assert_eq!(m.indices.len() % 3, 0, "{name}");
            assert!(m.indices.iter().all(|&i| (i as usize) < m.positions.len()), "{name}: index out of range");
        }
    }

    #[test]
    fn normals_are_unit_length() {
        for (name, m) in all() {
            for n in &m.normals {
                assert!((Vec3::from(*n).length() - 1.0).abs() < 1e-4, "{name}: {n:?}");
            }
        }
    }

    #[test]
    fn every_face_looks_outward_in_counter_clockwise_order() {
        for (name, m) in all() {
            for tri in m.indices.chunks(3) {
                let p = |i: u32| Vec3::from(m.positions[i as usize]);
                let n = |i: u32| Vec3::from(m.normals[i as usize]);
                let face = (p(tri[1]) - p(tri[0])).cross(p(tri[2]) - p(tri[0]));
                if face.length() < 1e-9 {
                    continue; // degenerate triangle at a pole of the sphere
                }
                let vertex_normal = n(tri[0]) + n(tri[1]) + n(tri[2]);
                assert!(face.dot(vertex_normal) > 0.0, "{name}: a face points inward");
            }
        }
    }

    #[test]
    fn the_bounds_match_the_requested_size() {
        let (min, max) = cube([0.2, 0.4, 0.6]).bounds();
        for (k, half) in [0.1, 0.2, 0.3].into_iter().enumerate() {
            assert!((min[k] + half).abs() < 1e-6 && (max[k] - half).abs() < 1e-6, "eje {k}");
        }
        let (min, max) = cylinder(0.05, 0.3).bounds();
        assert!((max[1] - 0.15).abs() < 1e-6 && (min[1] + 0.15).abs() < 1e-6);
        assert!((max[0] - 0.05).abs() < 1e-5);
        let (_, max) = sphere(0.1).bounds();
        assert!((max[1] - 0.1).abs() < 1e-6);
        let (min, max) = plane([0.5, 0.3]).bounds();
        assert_eq!((min[2], max[2]), (0.0, 0.0));
    }
}
