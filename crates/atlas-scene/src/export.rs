//! Export of a scene to binary glTF 2.0 (`.glb`): a single file with the node hierarchy, the
//! transforms, the meshes and the materials. glTF uses meters, Y up and -Z forward, like
//! Atlas, so the coordinates go through as they are.

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::{Content, MeshData, Node, NodeId, Scene, Shape};

#[derive(Debug, Clone, PartialEq)]
pub enum ExportError {
    /// A number (position, color...) is NaN or infinite.
    NotFinite(NodeId),
    /// A measure is not positive (a radius of 0, a negative side...).
    BadShape(NodeId),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::NotFinite(id) => write!(f, "node {id} has a value that is not a finite number"),
            ExportError::BadShape(id) => write!(f, "node {id} has a shape with a size that is not positive"),
        }
    }
}

impl std::error::Error for ExportError {}

/// The vertices of a content that is drawn, or `None` if it is not drawn (an empty node).
fn mesh_of(node: &Node) -> Option<(String, MeshData)> {
    match &node.content {
        Content::Empty => None,
        Content::Shape(shape) => Some((format!("{shape:?}"), shape.mesh())),
        // glTF has no text: a sheet of the size it would take is exported, and the text goes in the extras.
        Content::Text { text, size } => {
            let width = size * text.chars().count().max(1) as f32 * 0.6;
            let shape = Shape::Plane { size: [width, *size] };
            Some((format!("{shape:?}"), shape.mesh()))
        }
        Content::Panel { size, .. } => {
            let shape = Shape::Plane { size: *size };
            Some((format!("{shape:?}"), shape.mesh()))
        }
    }
}

fn validate(node: &Node) -> Result<(), ExportError> {
    let t = &node.transform;
    let numbers = t.position.iter().chain(&t.rotation).chain(&t.scale).chain(&node.material.color).chain(&node.material.emissive);
    if numbers.into_iter().any(|v| !v.is_finite()) {
        return Err(ExportError::NotFinite(node.id));
    }
    let sizes: Vec<f32> = match &node.content {
        Content::Empty => vec![],
        Content::Shape(Shape::Cube { size }) => size.to_vec(),
        Content::Shape(Shape::Sphere { radius }) => vec![*radius],
        Content::Shape(Shape::Plane { size }) => size.to_vec(),
        Content::Shape(Shape::Cylinder { radius, height }) => vec![*radius, *height],
        Content::Text { size, .. } => vec![*size],
        Content::Panel { size, .. } => size.to_vec(),
    };
    if sizes.iter().any(|v| !v.is_finite()) {
        return Err(ExportError::NotFinite(node.id));
    }
    if sizes.iter().any(|v| *v <= 0.0) {
        return Err(ExportError::BadShape(node.id));
    }
    Ok(())
}

/// Adds data to the binary buffer and returns the index of its `bufferView`.
fn add_view(bin: &mut Vec<u8>, views: &mut Vec<Value>, data: &[u8], target: u32) -> usize {
    // Everything stored is f32 or u32 (4 bytes), so the buffer always stays aligned.
    debug_assert_eq!(bin.len() % 4, 0);
    views.push(json!({ "buffer": 0, "byteOffset": bin.len(), "byteLength": data.len(), "target": target }));
    bin.extend_from_slice(data);
    views.len() - 1
}

fn floats(values: &[[f32; 3]]) -> Vec<u8> {
    values.iter().flatten().flat_map(|v| v.to_le_bytes()).collect()
}

pub fn to_glb(scene: &Scene) -> Result<Vec<u8>, ExportError> {
    for node in scene.nodes() {
        validate(node)?;
    }

    let mut bin: Vec<u8> = Vec::new();
    let mut views: Vec<Value> = Vec::new();
    let mut accessors: Vec<Value> = Vec::new();
    let mut materials: Vec<Value> = Vec::new();
    let mut meshes: Vec<Value> = Vec::new();
    // What is equal is reused: a shape's geometry, a material and the mesh (shape + material).
    let mut geometry: HashMap<String, (usize, usize, usize)> = HashMap::new();
    let mut material_of: HashMap<String, usize> = HashMap::new();
    let mut mesh_of_pair: HashMap<(String, usize), usize> = HashMap::new();

    let index_of: HashMap<NodeId, usize> = scene.nodes().iter().enumerate().map(|(i, n)| (n.id, i)).collect();
    let mut nodes: Vec<Value> = Vec::new();

    for node in scene.nodes() {
        let mut entry = json!({ "name": node.name });
        let t = &node.transform;
        if t.position != [0.0; 3] {
            entry["translation"] = json!(t.position);
        }
        if t.rotation != [0.0, 0.0, 0.0, 1.0] {
            entry["rotation"] = json!(t.rotation);
        }
        if t.scale != [1.0; 3] {
            entry["scale"] = json!(t.scale);
        }
        let children: Vec<usize> = scene.children(node.id).iter().map(|c| index_of[c]).collect();
        if !children.is_empty() {
            entry["children"] = json!(children);
        }

        if let Some((key, mesh)) = mesh_of(node) {
            let (pos, nor, idx) = *geometry.entry(key.clone()).or_insert_with(|| {
                let (min, max) = mesh.bounds();
                let view = add_view(&mut bin, &mut views, &floats(&mesh.positions), 34962);
                accessors.push(json!({ "bufferView": view, "componentType": 5126, "count": mesh.positions.len(), "type": "VEC3", "min": min, "max": max }));
                let pos = accessors.len() - 1;
                let view = add_view(&mut bin, &mut views, &floats(&mesh.normals), 34962);
                accessors.push(json!({ "bufferView": view, "componentType": 5126, "count": mesh.normals.len(), "type": "VEC3" }));
                let nor = accessors.len() - 1;
                let bytes: Vec<u8> = mesh.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
                let view = add_view(&mut bin, &mut views, &bytes, 34963);
                accessors.push(json!({ "bufferView": view, "componentType": 5125, "count": mesh.indices.len(), "type": "SCALAR" }));
                (pos, nor, accessors.len() - 1)
            });

            let m = &node.material;
            let material_key = format!("{:?}{:?}", m.color, m.emissive);
            let material = *material_of.entry(material_key.clone()).or_insert_with(|| {
                materials.push(json!({
                    "name": format!("material {}", materials.len()),
                    "pbrMetallicRoughness": { "baseColorFactor": m.color, "metallicFactor": 0.0, "roughnessFactor": 0.6 },
                    "emissiveFactor": m.emissive,
                    "alphaMode": if m.color[3] < 1.0 { "BLEND" } else { "OPAQUE" },
                    "doubleSided": true,
                }));
                materials.len() - 1
            });

            let mesh_index = *mesh_of_pair.entry((key, material)).or_insert_with(|| {
                meshes.push(json!({
                    "name": format!("mesh {}", meshes.len()),
                    "primitives": [{ "attributes": { "POSITION": pos, "NORMAL": nor }, "indices": idx, "material": material, "mode": 4 }],
                }));
                meshes.len() - 1
            });
            entry["mesh"] = json!(mesh_index);
        }

        // What glTF cannot represent, in the node's extras.
        let mut atlas = json!({ "anchor": node.anchor.name(), "visible": node.visible });
        match &node.content {
            Content::Empty => atlas["kind"] = json!("empty"),
            Content::Shape(_) => atlas["kind"] = json!("shape"),
            Content::Text { text, size } => {
                atlas["kind"] = json!("text");
                atlas["text"] = json!(text);
                atlas["text_size"] = json!(size);
            }
            Content::Panel { text, .. } => {
                atlas["kind"] = json!("panel");
                atlas["text"] = json!(text);
            }
        }
        entry["extras"] = json!({ "atlas": atlas });
        nodes.push(entry);
    }

    let roots: Vec<usize> = scene.roots().iter().map(|r| index_of[r]).collect();
    let mut root = json!({ "asset": { "version": "2.0", "generator": "Atlas Code" } });
    // A scene with no nodes is not written: glTF requires a scene to have at least one.
    if !roots.is_empty() {
        root["scene"] = json!(0);
        root["scenes"] = json!([{ "nodes": roots }]);
    }
    // glTF does not allow empty lists: only the ones with something are included.
    for (name, list) in [("nodes", nodes), ("meshes", meshes), ("materials", materials), ("accessors", accessors), ("bufferViews", views)] {
        if !list.is_empty() {
            root[name] = Value::Array(list);
        }
    }
    if !bin.is_empty() {
        root["buffers"] = json!([{ "byteLength": bin.len() }]);
    }

    // GLB container: header, JSON chunk (padded with spaces) and binary chunk (padded with zeros).
    let mut json_bytes = serde_json::to_vec(&root).expect("glTF JSON can always be written");
    while json_bytes.len() % 4 != 0 {
        json_bytes.push(b' ');
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let total = 12 + 8 + json_bytes.len() + if bin.is_empty() { 0 } else { 8 + bin.len() };
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(&0x4654_6C67u32.to_le_bytes()); // "glTF"
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x4E4F_534Au32.to_le_bytes()); // "JSON"
    glb.extend_from_slice(&json_bytes);
    if !bin.is_empty() {
        glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        glb.extend_from_slice(&0x004E_4942u32.to_le_bytes()); // "BIN\0"
        glb.extend_from_slice(&bin);
    }
    Ok(glb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Anchor, Material, NodeSpec, Transform};

    fn cube(name: &str) -> NodeSpec {
        NodeSpec::new(name, Content::Shape(Shape::Cube { size: [0.1; 3] }))
    }

    fn parse(glb: &[u8]) -> gltf::Gltf {
        gltf::Gltf::from_slice(glb).expect("valid glTF")
    }

    fn extras(node: &gltf::Node) -> Value {
        serde_json::from_str(node.extras().as_ref().expect("tiene extras").get()).unwrap()
    }

    #[test]
    fn an_empty_scene_is_a_valid_glb() {
        let glb = Scene::new().to_glb().unwrap();
        let g = parse(&glb);
        assert_eq!(g.document.nodes().count(), 0);
        assert_eq!(g.document.scenes().count(), 0, "no nodes, no scene");
    }

    #[test]
    fn the_glb_container_is_well_formed() {
        let mut s = Scene::new();
        s.add(cube("a")).unwrap();
        let glb = s.to_glb().unwrap();
        assert_eq!(&glb[0..4], b"glTF");
        assert_eq!(u32::from_le_bytes(glb[4..8].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(glb[8..12].try_into().unwrap()) as usize, glb.len(), "the header length");
        assert_eq!(&glb[16..20], b"JSON");
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        assert_eq!(json_len % 4, 0);
        assert_eq!(&glb[20 + json_len + 4..20 + json_len + 8], b"BIN\0");
        assert_eq!(glb.len() % 4, 0);
    }

    #[test]
    fn a_cube_exports_its_geometry_and_its_transform() {
        let mut s = Scene::new();
        let mut spec = NodeSpec::new("box", Content::Shape(Shape::Cube { size: [0.2, 0.4, 0.6] }));
        spec.transform = Transform { position: [1.0, 2.0, -3.0], rotation: [0.0, 0.7071068, 0.0, 0.7071068], scale: [1.0, 1.0, 2.0] };
        s.add(spec).unwrap();
        let glb = s.to_glb().unwrap();
        let g = parse(&glb);
        let node = g.document.nodes().next().unwrap();
        assert_eq!(node.name(), Some("box"));
        let (t, r, sc) = node.transform().decomposed();
        assert_eq!(t, [1.0, 2.0, -3.0]);
        assert!((r[1] - 0.7071068).abs() < 1e-6 && (r[3] - 0.7071068).abs() < 1e-6);
        assert_eq!(sc, [1.0, 1.0, 2.0]);

        let prim = node.mesh().unwrap().primitives().next().unwrap();
        let blob = g.blob.as_deref();
        let reader = prim.reader(|b| match b.source() {
            gltf::buffer::Source::Bin => blob,
            _ => None,
        });
        let positions: Vec<[f32; 3]> = reader.read_positions().unwrap().collect();
        let indices: Vec<u32> = reader.read_indices().unwrap().into_u32().collect();
        assert_eq!((positions.len(), indices.len()), (24, 36));
        let bounds = prim.bounding_box();
        assert_eq!((bounds.min, bounds.max), ([-0.1, -0.2, -0.3], [0.1, 0.2, 0.3]));
        assert_eq!(reader.read_normals().unwrap().count(), 24);
    }

    #[test]
    fn the_hierarchy_is_exported() {
        let mut s = Scene::new();
        let root = s.add(cube("root")).unwrap();
        let mut child = cube("child");
        child.parent = Some(root);
        child.transform = Transform::at(0.0, 0.5, 0.0);
        s.add(child).unwrap();
        s.add(cube("other")).unwrap();
        let g = parse(&s.to_glb().unwrap());
        let scene = g.document.default_scene().unwrap();
        let roots: Vec<_> = scene.nodes().map(|n| n.name().unwrap().to_string()).collect();
        assert_eq!(roots, vec!["root", "other"]);
        let root_node = g.document.nodes().find(|n| n.name() == Some("root")).unwrap();
        let kids: Vec<_> = root_node.children().map(|n| n.name().unwrap().to_string()).collect();
        assert_eq!(kids, vec!["child"]);
    }

    #[test]
    fn equal_shapes_share_geometry_and_equal_looks_share_a_mesh() {
        let mut s = Scene::new();
        s.add(cube("a")).unwrap();
        s.add(cube("b")).unwrap();
        let mut blue = cube("c");
        blue.material = Material { color: [0.0, 0.0, 1.0, 1.0], emissive: [0.0; 3] };
        s.add(blue).unwrap();
        let g = parse(&s.to_glb().unwrap());
        assert_eq!(g.document.meshes().count(), 2, "one mesh per distinct look");
        assert_eq!(g.document.materials().count(), 2);
        assert_eq!(g.document.accessors().count(), 3, "positions, normals and indices: only once");
        assert_eq!(g.document.nodes().filter(|n| n.mesh().is_some()).count(), 3);
    }

    #[test]
    fn materials_keep_color_emission_and_transparency() {
        let mut s = Scene::new();
        let mut glass = NodeSpec::new("glass", Content::Shape(Shape::Sphere { radius: 0.05 }));
        glass.material = Material { color: [0.2, 0.8, 1.0, 0.4], emissive: [0.1, 0.2, 0.3] };
        s.add(glass).unwrap();
        s.add(cube("solid")).unwrap();
        let g = parse(&s.to_glb().unwrap());
        let materials: Vec<_> = g.document.materials().collect();
        let glass = &materials[0];
        assert_eq!(glass.pbr_metallic_roughness().base_color_factor(), [0.2, 0.8, 1.0, 0.4]);
        assert_eq!(glass.emissive_factor(), [0.1, 0.2, 0.3]);
        assert_eq!(glass.alpha_mode(), gltf::material::AlphaMode::Blend);
        assert_eq!(materials[1].alpha_mode(), gltf::material::AlphaMode::Opaque);
    }

    #[test]
    fn what_gltf_cannot_say_goes_in_the_extras() {
        let mut s = Scene::new();
        let mut label = NodeSpec::new("label", Content::Text { text: "tests: 12 ok".into(), size: 0.03 });
        label.anchor = Anchor::Wall(2);
        s.add(label).unwrap();
        let mut orb = NodeSpec::new("orb", Content::Shape(Shape::Sphere { radius: 0.05 }));
        orb.anchor = Anchor::Desk;
        s.add(orb).unwrap();
        s.add(NodeSpec::new("origin", Content::Empty)).unwrap();
        let g = parse(&s.to_glb().unwrap());
        let nodes: Vec<_> = g.document.nodes().collect();
        let label = &extras(&nodes[0])["atlas"];
        assert_eq!(label["kind"], "text");
        assert_eq!(label["text"], "tests: 12 ok");
        assert_eq!(label["anchor"], "wall:2");
        assert!(nodes[0].mesh().is_some(), "text comes out as a sheet");
        assert_eq!(extras(&nodes[1])["atlas"]["anchor"], "desk");
        assert!(nodes[2].mesh().is_none(), "an empty node is not drawn");
        assert_eq!(extras(&nodes[2])["atlas"]["kind"], "empty");
    }

    #[test]
    fn every_shape_kind_and_panel_export_and_parse() {
        let mut s = Scene::new();
        for (name, shape) in [
            ("cube", Shape::Cube { size: [0.1, 0.2, 0.3] }),
            ("sphere", Shape::Sphere { radius: 0.05 }),
            ("plane", Shape::Plane { size: [0.4, 0.3] }),
            ("cylinder", Shape::Cylinder { radius: 0.04, height: 0.2 }),
        ] {
            s.add(NodeSpec::new(name, Content::Shape(shape))).unwrap();
        }
        s.add(NodeSpec::new("panel", Content::Panel { size: [0.5, 0.3], text: "hola".into() })).unwrap();
        let g = parse(&s.to_glb().unwrap());
        assert_eq!(g.document.nodes().count(), 5);
        assert_eq!(g.document.meshes().count(), 5);
    }

    #[test]
    fn values_that_would_make_a_broken_file_are_refused() {
        let mut s = Scene::new();
        let mut nan = cube("nan");
        nan.transform.position[1] = f32::NAN;
        let id = s.add(nan).unwrap();
        assert_eq!(s.to_glb(), Err(ExportError::NotFinite(id)));

        let mut s = Scene::new();
        let id = s.add(NodeSpec::new("flat", Content::Shape(Shape::Sphere { radius: 0.0 }))).unwrap();
        assert_eq!(s.to_glb(), Err(ExportError::BadShape(id)));

        let mut s = Scene::new();
        let id = s.add(NodeSpec::new("inf", Content::Shape(Shape::Cube { size: [1.0, f32::INFINITY, 1.0] }))).unwrap();
        assert_eq!(s.to_glb(), Err(ExportError::NotFinite(id)));
    }

    #[test]
    fn a_saved_and_reloaded_scene_exports_the_same_file() {
        let mut s = Scene::new();
        let mut a = cube("a");
        a.transform = Transform::at(0.1, 0.2, 0.3);
        let a = s.add(a).unwrap();
        let mut b = NodeSpec::new("b", Content::Text { text: "x".into(), size: 0.05 });
        b.parent = Some(a);
        s.add(b).unwrap();
        let reloaded = Scene::from_json(&s.to_json()).unwrap();
        assert_eq!(reloaded.to_glb().unwrap(), s.to_glb().unwrap());
    }
}
