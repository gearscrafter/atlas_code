//! Simple meshes, ready to upload to the GPU.

use std::collections::HashMap;

/// Triangle mesh: positions and a list of indices.
#[derive(Debug, Clone)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

/// Sphere of radius 1 built by subdividing an icosahedron. Each subdivision multiplies
/// the triangles by four: 0 -> 20, 1 -> 80, 2 -> 320, 3 -> 1280.
pub fn icosphere(subdivisions: u32) -> Mesh {
    let t = (1.0 + 5.0f32.sqrt()) / 2.0;
    let mut positions: Vec<[f32; 3]> = [
        [-1.0, t, 0.0], [1.0, t, 0.0], [-1.0, -t, 0.0], [1.0, -t, 0.0],
        [0.0, -1.0, t], [0.0, 1.0, t], [0.0, -1.0, -t], [0.0, 1.0, -t],
        [t, 0.0, -1.0], [t, 0.0, 1.0], [-t, 0.0, -1.0], [-t, 0.0, 1.0],
    ]
    .iter()
    .map(|p| normalize(*p))
    .collect();

    let mut indices: Vec<u32> = vec![
        0, 11, 5, 0, 5, 1, 0, 1, 7, 0, 7, 10, 0, 10, 11, //
        1, 5, 9, 5, 11, 4, 11, 10, 2, 10, 7, 6, 7, 1, 8, //
        3, 9, 4, 3, 4, 2, 3, 2, 6, 3, 6, 8, 3, 8, 9, //
        4, 9, 5, 2, 4, 11, 6, 2, 10, 8, 6, 7, 9, 8, 1,
    ];

    for _ in 0..subdivisions {
        // Each edge is split only once, sharing the midpoint between triangles.
        let mut midpoints: HashMap<(u32, u32), u32> = HashMap::new();
        let mut next = Vec::with_capacity(indices.len() * 4);
        let mut midpoint = |a: u32, b: u32, positions: &mut Vec<[f32; 3]>| -> u32 {
            let key = (a.min(b), a.max(b));
            *midpoints.entry(key).or_insert_with(|| {
                let (pa, pb) = (positions[a as usize], positions[b as usize]);
                positions.push(normalize([(pa[0] + pb[0]) / 2.0, (pa[1] + pb[1]) / 2.0, (pa[2] + pb[2]) / 2.0]));
                (positions.len() - 1) as u32
            })
        };
        for tri in indices.chunks_exact(3) {
            let (a, b, c) = (tri[0], tri[1], tri[2]);
            let ab = midpoint(a, b, &mut positions);
            let bc = midpoint(b, c, &mut positions);
            let ca = midpoint(c, a, &mut positions);
            next.extend_from_slice(&[a, ab, ca, b, bc, ab, c, ca, bc, ab, bc, ca]);
        }
        indices = next;
    }
    Mesh { positions, indices }
}

fn normalize(p: [f32; 3]) -> [f32; 3] {
    let len = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    [p[0] / len, p[1] / len, p[2] / len]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangle_and_vertex_counts_follow_the_subdivision_formula() {
        for n in 0..=4u32 {
            let m = icosphere(n);
            let faces = 20 * 4usize.pow(n);
            assert_eq!(m.indices.len(), faces * 3, "triangles with {n} subdivisions");
            // Euler for a sphere: V - E + F = 2 with E = 3F/2  =>  V = F/2 + 2.
            assert_eq!(m.positions.len(), faces / 2 + 2, "vertices with {n} subdivisions");
        }
    }

    #[test]
    fn every_vertex_is_on_the_unit_sphere() {
        for p in &icosphere(3).positions {
            let len = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-5, "{p:?} mide {len}");
        }
    }

    #[test]
    fn indices_are_valid_and_no_triangle_is_degenerate() {
        let m = icosphere(3);
        for tri in m.indices.chunks_exact(3) {
            assert!(tri.iter().all(|&i| (i as usize) < m.positions.len()));
            assert!(tri[0] != tri[1] && tri[1] != tri[2] && tri[0] != tri[2], "{tri:?}");
        }
    }

    #[test]
    fn the_surface_is_closed_every_edge_is_shared_by_exactly_two_triangles() {
        let m = icosphere(2);
        let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
        for tri in m.indices.chunks_exact(3) {
            for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
                *edges.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        assert!(edges.values().all(|&n| n == 2), "hay aristas abiertas o repetidas");
    }

    #[test]
    fn all_triangles_wind_the_same_way_outward() {
        // With a consistent orientation, each triangle's cross product points
        // outward (toward its center, since it is a sphere centered at the origin).
        let m = icosphere(2);
        for tri in m.indices.chunks_exact(3) {
            let p = |i: u32| m.positions[i as usize];
            let (a, b, c) = (p(tri[0]), p(tri[1]), p(tri[2]));
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            let center = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0, (a[2] + b[2] + c[2]) / 3.0];
            let dot = n[0] * center[0] + n[1] * center[1] + n[2] * center[2];
            assert!(dot > 0.0, "triangle {tri:?} points inward");
        }
    }
}
