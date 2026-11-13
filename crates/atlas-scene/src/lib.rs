//! The Atlas scene model: a hierarchy of nodes with transforms, parametric shapes and
//! materials, saved as readable JSON (made for git) and exported to glTF. It does not depend on XR,
//! so everything is tested on the PC.
//!
//! Units: meters. Axes: X to the right, Y up, -Z forward (those of OpenXR and glTF).

mod export;
pub mod mesh;

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

pub use export::ExportError;
pub use mesh::MeshData;

/// Version of the scene file format.
pub const FORMAT_VERSION: u32 = 1;

/// Stable identifier of a node: it does not change when others are deleted.
pub type NodeId = u32;

/// Position, rotation (quaternion x, y, z, w) and scale.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl Default for Transform {
    fn default() -> Self {
        Self { position: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] }
    }
}

impl Transform {
    pub fn at(x: f32, y: f32, z: f32) -> Self {
        Self { position: [x, y, z], ..Self::default() }
    }

    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(Vec3::from(self.scale), Quat::from_array(self.rotation).normalize(), Vec3::from(self.position))
    }
}

/// A basic shape, defined by its measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Shape {
    Cube { size: [f32; 3] },
    Sphere { radius: f32 },
    Plane { size: [f32; 2] },
    Cylinder { radius: f32, height: f32 },
}

impl Shape {
    pub fn mesh(&self) -> MeshData {
        match self {
            Shape::Cube { size } => mesh::cube(*size),
            Shape::Sphere { radius } => mesh::sphere(*radius),
            Shape::Plane { size } => mesh::plane(*size),
            Shape::Cylinder { radius, height } => mesh::cylinder(*radius, *height),
        }
    }
}

/// What a node is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Content {
    /// It only groups others (a reference point).
    Empty,
    Shape(Shape),
    /// Floating text; `size` is the height (m) of a line.
    Text { text: String, size: f32 },
    /// A rectangular panel with text.
    Panel { size: [f32; 2], text: String },
}

/// What the node is placed relative to (if it has no parent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "anchor", content = "index", rename_all = "snake_case")]
pub enum Anchor {
    /// A fixed position in your room.
    #[default]
    World,
    /// Follows your head.
    Head,
    /// The mapped desk.
    Desk,
    /// A mapped wall (0 is the first).
    Wall(u8),
}

impl Anchor {
    pub fn name(&self) -> String {
        match self {
            Anchor::World => "world".into(),
            Anchor::Head => "head".into(),
            Anchor::Desk => "desk".into(),
            Anchor::Wall(i) => format!("wall:{i}"),
        }
    }
}

/// Look: color with transparency and its own glow (all from 0 to 1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub color: [f32; 4],
    pub emissive: [f32; 3],
}

impl Default for Material {
    fn default() -> Self {
        Self { color: [1.0, 0.6, 0.25, 1.0], emissive: [0.0; 3] }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub name: String,
    pub parent: Option<NodeId>,
    pub transform: Transform,
    pub content: Content,
    pub material: Material,
    pub anchor: Anchor,
    pub visible: bool,
}

/// What it takes to add a node (the scene sets the identifier).
#[derive(Debug, Clone, PartialEq)]
pub struct NodeSpec {
    pub name: String,
    pub parent: Option<NodeId>,
    pub transform: Transform,
    pub content: Content,
    pub material: Material,
    pub anchor: Anchor,
}

impl NodeSpec {
    pub fn new(name: &str, content: Content) -> Self {
        Self {
            name: name.to_string(),
            parent: None,
            transform: Transform::default(),
            content,
            material: Material::default(),
            anchor: Anchor::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SceneError {
    /// The requested parent does not exist.
    NoParent(NodeId),
    /// There is no node with that identifier.
    NoNode(NodeId),
    /// A node cannot hang from itself or from one of its descendants.
    Cycle,
    /// The file could not be read.
    Parse(String),
    /// The file is from a version this Atlas does not understand.
    Version(u32),
}

impl std::fmt::Display for SceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SceneError::NoParent(id) => write!(f, "parent node {id} does not exist"),
            SceneError::NoNode(id) => write!(f, "node {id} does not exist"),
            SceneError::Cycle => write!(f, "a node cannot hang from itself or from one of its descendants"),
            SceneError::Parse(e) => write!(f, "cannot read the scene: {e}"),
            SceneError::Version(v) => write!(f, "scene format version {v} is not supported (this Atlas reads {FORMAT_VERSION})"),
        }
    }
}

impl std::error::Error for SceneError {}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    nodes: Vec<Node>,
    next_id: NodeId,
}

/// What is in a scene file.
#[derive(Serialize, Deserialize)]
struct File {
    atlas_scene: u32,
    next_id: NodeId,
    nodes: Vec<Node>,
}

impl Scene {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// Adds a node and returns its identifier.
    pub fn add(&mut self, spec: NodeSpec) -> Result<NodeId, SceneError> {
        if let Some(parent) = spec.parent {
            if self.get(parent).is_none() {
                return Err(SceneError::NoParent(parent));
            }
        }
        let id = self.next_id;
        self.next_id += 1;
        self.nodes.push(Node {
            id,
            name: spec.name,
            parent: spec.parent,
            transform: spec.transform,
            content: spec.content,
            material: spec.material,
            anchor: spec.anchor,
            visible: true,
        });
        Ok(id)
    }

    /// A node's direct children, in the order they were added.
    pub fn children(&self, id: NodeId) -> Vec<NodeId> {
        self.nodes.iter().filter(|n| n.parent == Some(id)).map(|n| n.id).collect()
    }

    /// The nodes without a parent.
    pub fn roots(&self) -> Vec<NodeId> {
        self.nodes.iter().filter(|n| n.parent.is_none()).map(|n| n.id).collect()
    }

    /// Deletes a node and everything hanging from it. Returns how many nodes were deleted.
    pub fn remove(&mut self, id: NodeId) -> Result<usize, SceneError> {
        if self.get(id).is_none() {
            return Err(SceneError::NoNode(id));
        }
        let mut doomed = vec![id];
        let mut i = 0;
        while i < doomed.len() {
            doomed.extend(self.children(doomed[i]));
            i += 1;
        }
        self.nodes.retain(|n| !doomed.contains(&n.id));
        Ok(doomed.len())
    }

    /// Hangs `id` from `parent` (or releases it if `None`) without moving the node in the scene.
    pub fn reparent(&mut self, id: NodeId, parent: Option<NodeId>) -> Result<(), SceneError> {
        if self.get(id).is_none() {
            return Err(SceneError::NoNode(id));
        }
        if let Some(p) = parent {
            if self.get(p).is_none() {
                return Err(SceneError::NoParent(p));
            }
            // The new parent cannot be the node itself or one of its descendants.
            let mut at = Some(p);
            while let Some(current) = at {
                if current == id {
                    return Err(SceneError::Cycle);
                }
                at = self.get(current).and_then(|n| n.parent);
            }
        }
        let world = self.world_matrix(id);
        let parent_world = parent.map_or(Mat4::IDENTITY, |p| self.world_matrix(p));
        let local = parent_world.inverse() * world;
        let (scale, rotation, position) = local.to_scale_rotation_translation();
        let node = self.get_mut(id).expect("exists: checked above");
        node.parent = parent;
        node.transform = Transform { position: position.to_array(), rotation: rotation.to_array(), scale: scale.to_array() };
        Ok(())
    }

    /// A node's transform relative to its anchor (its parents' composed with its own).
    pub fn world_matrix(&self, id: NodeId) -> Mat4 {
        let mut matrix = Mat4::IDENTITY;
        let mut at = self.get(id);
        // The step limit avoids looping forever if a tampered file brings a cycle.
        for _ in 0..=self.nodes.len() {
            let Some(node) = at else { break };
            matrix = node.transform.matrix() * matrix;
            at = node.parent.and_then(|p| self.get(p));
        }
        matrix
    }

    /// The scene file, as readable JSON.
    pub fn to_json(&self) -> String {
        let file = File { atlas_scene: FORMAT_VERSION, next_id: self.next_id, nodes: self.nodes.clone() };
        serde_json::to_string_pretty(&file).expect("the scene can always be serialized")
    }

    /// Reads a scene file, checking that it is a known version and that the parents exist.
    pub fn from_json(text: &str) -> Result<Self, SceneError> {
        let file: File = serde_json::from_str(text).map_err(|e| SceneError::Parse(e.to_string()))?;
        if file.atlas_scene != FORMAT_VERSION {
            return Err(SceneError::Version(file.atlas_scene));
        }
        let scene = Scene { next_id: file.next_id.max(file.nodes.iter().map(|n| n.id + 1).max().unwrap_or(0)), nodes: file.nodes };
        for node in &scene.nodes {
            if scene.nodes.iter().filter(|n| n.id == node.id).count() > 1 {
                return Err(SceneError::Parse(format!("node id {} is repeated", node.id)));
            }
            if let Some(p) = node.parent {
                if scene.get(p).is_none() {
                    return Err(SceneError::NoParent(p));
                }
            }
        }
        Ok(scene)
    }

    /// The scene as binary glTF (`.glb`), which Godot, Unity, Blender and almost any 3D viewer open.
    ///
    /// The hierarchy, the transforms, the shapes and the materials are exported. What glTF cannot
    /// represent (anchors, text, Lua code) goes in each node's `extras`.
    pub fn to_glb(&self) -> Result<Vec<u8>, ExportError> {
        export::to_glb(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube_spec(name: &str) -> NodeSpec {
        NodeSpec::new(name, Content::Shape(Shape::Cube { size: [0.1; 3] }))
    }

    #[test]
    fn nodes_get_stable_ids_and_a_parent_must_exist() {
        let mut s = Scene::new();
        let a = s.add(cube_spec("a")).unwrap();
        let b = s.add(cube_spec("b")).unwrap();
        assert_eq!((a, b), (0, 1));
        s.remove(a).unwrap();
        let c = s.add(cube_spec("c")).unwrap();
        assert_eq!(c, 2, "a deleted id is not reused");
        let mut orphan = cube_spec("x");
        orphan.parent = Some(99);
        assert_eq!(s.add(orphan), Err(SceneError::NoParent(99)));
        assert_eq!(s.remove(99), Err(SceneError::NoNode(99)));
    }

    #[test]
    fn removing_a_node_removes_everything_hanging_from_it() {
        let mut s = Scene::new();
        let root = s.add(cube_spec("root")).unwrap();
        let mut child = cube_spec("child");
        child.parent = Some(root);
        let child = s.add(child).unwrap();
        let mut grandchild = cube_spec("grandchild");
        grandchild.parent = Some(child);
        s.add(grandchild).unwrap();
        let other = s.add(cube_spec("other")).unwrap();
        assert_eq!(s.children(root), vec![child]);
        assert_eq!(s.remove(root), Ok(3));
        assert_eq!(s.nodes().iter().map(|n| n.id).collect::<Vec<_>>(), vec![other]);
    }

    #[test]
    fn a_child_inherits_the_transform_of_its_parent() {
        let mut s = Scene::new();
        let mut parent = cube_spec("parent");
        parent.transform = Transform::at(1.0, 0.0, 0.0);
        parent.transform.scale = [2.0; 3];
        let parent = s.add(parent).unwrap();
        let mut child = cube_spec("child");
        child.parent = Some(parent);
        child.transform = Transform::at(0.0, 0.5, 0.0);
        let child = s.add(child).unwrap();
        let p = s.world_matrix(child).transform_point3(Vec3::ZERO);
        assert!((p - Vec3::new(1.0, 1.0, 0.0)).length() < 1e-6, "{p:?}");
    }

    #[test]
    fn reparenting_keeps_the_node_where_it_is_and_refuses_cycles() {
        let mut s = Scene::new();
        let mut a = cube_spec("a");
        a.transform = Transform::at(2.0, 0.0, 0.0);
        let a = s.add(a).unwrap();
        let mut b = cube_spec("b");
        b.transform = Transform::at(0.0, 3.0, 0.0);
        let b = s.add(b).unwrap();
        let before = s.world_matrix(b).transform_point3(Vec3::ZERO);
        s.reparent(b, Some(a)).unwrap();
        assert_eq!(s.get(b).unwrap().parent, Some(a));
        let after = s.world_matrix(b).transform_point3(Vec3::ZERO);
        assert!((before - after).length() < 1e-5, "it does not move: {before:?} -> {after:?}");
        assert_eq!(s.reparent(a, Some(b)), Err(SceneError::Cycle));
        assert_eq!(s.reparent(a, Some(a)), Err(SceneError::Cycle));
        s.reparent(b, None).unwrap();
        assert_eq!(s.get(b).unwrap().parent, None);
    }

    #[test]
    fn the_scene_survives_a_save_and_a_load() {
        let mut s = Scene::new();
        let mut a = NodeSpec::new("orb", Content::Shape(Shape::Sphere { radius: 0.05 }));
        a.transform = Transform::at(0.1, 0.2, -0.4);
        a.material = Material { color: [0.2, 0.8, 1.0, 0.5], emissive: [0.1, 0.1, 0.4] };
        a.anchor = Anchor::Desk;
        let a = s.add(a).unwrap();
        let mut label = NodeSpec::new("label", Content::Text { text: "hola ñ".into(), size: 0.03 });
        label.parent = Some(a);
        label.anchor = Anchor::Wall(1);
        s.add(label).unwrap();
        s.add(NodeSpec::new("panel", Content::Panel { size: [0.4, 0.2], text: "tests: 12 ok".into() })).unwrap();

        let text = s.to_json();
        assert!(text.contains("\"atlas_scene\": 1"));
        assert_eq!(Scene::from_json(&text).unwrap(), s);
    }

    #[test]
    fn a_new_node_after_a_load_never_reuses_an_id() {
        let mut s = Scene::new();
        s.add(cube_spec("a")).unwrap();
        let b = s.add(cube_spec("b")).unwrap();
        s.remove(b).unwrap();
        let mut loaded = Scene::from_json(&s.to_json()).unwrap();
        assert_eq!(loaded.add(cube_spec("c")).unwrap(), 2);
    }

    #[test]
    fn bad_files_are_rejected_with_a_reason() {
        assert!(matches!(Scene::from_json("this is not json"), Err(SceneError::Parse(_))));
        assert_eq!(Scene::from_json(r#"{"atlas_scene": 99, "next_id": 0, "nodes": []}"#).unwrap_err(), SceneError::Version(99));
        let orphan = r#"{"atlas_scene":1,"next_id":2,"nodes":[{"id":1,"name":"x","parent":7,
            "transform":{"position":[0,0,0],"rotation":[0,0,0,1],"scale":[1,1,1]},
            "content":{"kind":"empty"},"material":{"color":[1,1,1,1],"emissive":[0,0,0]},
            "anchor":{"anchor":"world"},"visible":true}]}"#;
        assert_eq!(Scene::from_json(orphan).unwrap_err(), SceneError::NoParent(7));
    }

    #[test]
    fn a_manipulated_file_with_a_cycle_does_not_hang_the_transform() {
        let mut s = Scene::new();
        let a = s.add(cube_spec("a")).unwrap();
        let b = s.add(cube_spec("b")).unwrap();
        s.get_mut(a).unwrap().parent = Some(b);
        s.get_mut(b).unwrap().parent = Some(a);
        let _ = s.world_matrix(a); // it does not hang
    }
}
