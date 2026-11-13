//! Builds a sample scene and saves it as JSON (the Atlas format) and as binary glTF, to
//! open it in Blender, Godot or any 3D viewer: `cargo run -p atlas-scene --example export_demo -- folder`

use atlas_scene::{Anchor, Content, Material, NodeSpec, Scene, Shape, Transform};

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let mut scene = Scene::new();

    // A workbench: a plane on the desk with a cube, a sphere and a cylinder on top.
    let mut desk = NodeSpec::new("desk", Content::Empty);
    desk.anchor = Anchor::Desk;
    let desk = scene.add(desk).unwrap();

    let mut cube = NodeSpec::new("tests", Content::Shape(Shape::Cube { size: [0.08, 0.08, 0.08] }));
    cube.parent = Some(desk);
    cube.transform = Transform::at(-0.15, 0.04, -0.1);
    cube.material = Material { color: [0.3, 0.9, 0.6, 1.0], emissive: [0.0, 0.2, 0.1] };
    scene.add(cube).unwrap();

    let mut orb = NodeSpec::new("focus", Content::Shape(Shape::Sphere { radius: 0.05 }));
    orb.parent = Some(desk);
    orb.transform = Transform::at(0.0, 0.09, -0.1);
    orb.material = Material { color: [1.0, 0.6, 0.25, 0.6], emissive: [0.3, 0.12, 0.0] };
    scene.add(orb).unwrap();

    let mut pillar = NodeSpec::new("build", Content::Shape(Shape::Cylinder { radius: 0.03, height: 0.16 }));
    pillar.parent = Some(desk);
    pillar.transform = Transform::at(0.15, 0.08, -0.1);
    pillar.material = Material { color: [0.45, 0.85, 1.0, 1.0], emissive: [0.0; 3] };
    scene.add(pillar).unwrap();

    let mut label = NodeSpec::new("status", Content::Panel { size: [0.4, 0.12], text: "tests: 12 ok".into() });
    label.anchor = Anchor::Wall(0);
    label.transform = Transform::at(0.0, 0.3, -0.6);
    label.material = Material { color: [0.05, 0.1, 0.15, 0.8], emissive: [0.0; 3] };
    scene.add(label).unwrap();

    std::fs::write(format!("{dir}/demo.scene.json"), scene.to_json()).unwrap();
    std::fs::write(format!("{dir}/demo.glb"), scene.to_glb().unwrap()).unwrap();
    println!("wrote {dir}/demo.scene.json and {dir}/demo.glb ({} nodes)", scene.len());
}
