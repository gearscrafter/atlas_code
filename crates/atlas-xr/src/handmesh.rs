//! The hand as a mesh: it is requested from Meta's own runtime (XR_FB_hand_tracking_mesh). Skinning
//! with the joints is in `atlas_spatial::HandMeshData::skin`.

use atlas_spatial::{HandMeshData, Mat4, Quat, Vec3, JOINTS};
use openxr as xr;
use openxr::sys;

/// Asks the runtime for the mesh of `tracker`'s hand.
pub fn fetch(instance: &xr::Instance, tracker: &xr::HandTracker) -> Result<HandMeshData, String> {
    let ext = instance.exts().fb_hand_tracking_mesh.ok_or("the runtime does not offer the hand mesh")?;
    let zero3 = sys::Vector3f { x: 0.0, y: 0.0, z: 0.0 };

    let mut mesh = sys::HandTrackingMeshFB {
        ty: sys::HandTrackingMeshFB::TYPE,
        next: std::ptr::null_mut(),
        joint_capacity_input: 0,
        joint_count_output: 0,
        joint_bind_poses: std::ptr::null_mut(),
        joint_radii: std::ptr::null_mut(),
        joint_parents: std::ptr::null_mut(),
        vertex_capacity_input: 0,
        vertex_count_output: 0,
        vertex_positions: std::ptr::null_mut(),
        vertex_normals: std::ptr::null_mut(),
        vertex_u_vs: std::ptr::null_mut(),
        vertex_blend_indices: std::ptr::null_mut(),
        vertex_blend_weights: std::ptr::null_mut(),
        index_capacity_input: 0,
        index_count_output: 0,
        indices: std::ptr::null_mut(),
    };
    // First call: how many joints, vertices and indices there are.
    let r = unsafe { (ext.get_hand_mesh)(tracker.as_raw(), &mut mesh) };
    if r.into_raw() < 0 {
        return Err(format!("xrGetHandMeshFB (sizes): {r:?}"));
    }
    let (jn, vn, inn) = (mesh.joint_count_output as usize, mesh.vertex_count_output as usize, mesh.index_count_output as usize);
    if jn != JOINTS || vn == 0 || inn == 0 || vn > u16::MAX as usize {
        return Err(format!("unexpected hand mesh: {jn} joints, {vn} vertices, {inn} indices"));
    }

    let mut poses = vec![sys::Posef::IDENTITY; jn];
    let mut radii = vec![0.0f32; jn];
    let mut parents = vec![sys::HandJointEXT::PALM; jn];
    let mut positions = vec![zero3; vn];
    let mut normals = vec![zero3; vn];
    let mut uvs = vec![sys::Vector2f { x: 0.0, y: 0.0 }; vn];
    let mut blend_idx = vec![sys::Vector4sFB { x: 0, y: 0, z: 0, w: 0 }; vn];
    let mut blend_w = vec![sys::Vector4f { x: 0.0, y: 0.0, z: 0.0, w: 0.0 }; vn];
    let mut indices = vec![0i16; inn];
    mesh.joint_capacity_input = jn as u32;
    mesh.joint_bind_poses = poses.as_mut_ptr();
    mesh.joint_radii = radii.as_mut_ptr();
    mesh.joint_parents = parents.as_mut_ptr();
    mesh.vertex_capacity_input = vn as u32;
    mesh.vertex_positions = positions.as_mut_ptr();
    mesh.vertex_normals = normals.as_mut_ptr();
    mesh.vertex_u_vs = uvs.as_mut_ptr();
    mesh.vertex_blend_indices = blend_idx.as_mut_ptr();
    mesh.vertex_blend_weights = blend_w.as_mut_ptr();
    mesh.index_capacity_input = inn as u32;
    mesh.indices = indices.as_mut_ptr();
    let r = unsafe { (ext.get_hand_mesh)(tracker.as_raw(), &mut mesh) };
    if r.into_raw() < 0 {
        return Err(format!("xrGetHandMeshFB (datos): {r:?}"));
    }

    let v3 = |v: &sys::Vector3f| Vec3::new(v.x, v.y, v.z);
    let mut bind = [Mat4::IDENTITY; JOINTS];
    for (m, p) in bind.iter_mut().zip(&poses) {
        let q = Quat::from_xyzw(p.orientation.x, p.orientation.y, p.orientation.z, p.orientation.w);
        *m = Mat4::from_rotation_translation(q, v3(&p.position));
    }
    Ok(HandMeshData::from_parts(
        positions.iter().map(v3).collect(),
        normals.iter().map(v3).collect(),
        blend_idx.iter().map(|b| [b.x as u16, b.y as u16, b.z as u16, b.w as u16]).collect(),
        blend_w.iter().map(|w| [w.x, w.y, w.z, w.w]).collect(),
        indices.iter().map(|&i| i as u16).collect(),
        &bind,
    ))
}
