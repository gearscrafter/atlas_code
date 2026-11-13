//! 3D math, meshes and spatial interaction for Atlas Code.
//!
//! It does not depend on OpenXR, Vulkan or Android: everything is tested on the PC with `cargo test`.

pub mod anchor;
pub mod camera;
pub mod cube;
pub mod desk;
pub mod drawer;
pub mod handmesh;
pub mod keyboard;
pub mod hands;
pub mod mesh;
pub mod physics;
pub mod wall;

pub use anchor::Anchor;
pub use drawer::{fan_poses, title_pose, window, CardPose, Drawer, Fan, CARD_ASPECT, PAGE, TRAVEL};
pub use cube::{Cube, Face, FacePose, OrbPose, PalmSwipe};
pub use desk::{Desk, DeskCalibrator};
pub use camera::{eye_camera, projection, view_matrix, EyeCamera, Fov};
pub use glam::{Mat3, Mat4, Quat, Vec3, Vec4};
pub use hands::{classify_swipe, grasp_point, Grasp, Grip, Swipe, nearest, pinch_point, DragFrame, DragSystem, Draggable, HandPoint, Pinch, PinchEvent, GRAB_MARGIN, PINCH_END, PINCH_START};
pub use handmesh::{HandMeshData, HandVertex, JOINTS};
pub use keyboard::{KeyAction, KeyRect, Keyboard, Layer, PadEvent, KEYS_SHIFT, GRIP_NEAR, GRIP_RADIUS, PAD_H, PAD_W, PAD_X, PAD_Y, PLATE_PAD, SCROLL_W};
pub use physics::{Body, Hit, VelocityTracker, World, GRAVITY, THROW_MIN_SPEED};
pub use wall::{Wall, WallCalibrator};
pub use mesh::{icosphere, Mesh};
