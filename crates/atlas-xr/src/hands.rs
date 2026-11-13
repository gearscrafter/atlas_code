//! Headset hand tracking (XR_EXT_hand_tracking): reads the thumb and index
//! tips, and detects the pinch with them (see `atlas_spatial::hands`).

use atlas_spatial::{grasp_point, pinch_point, Grasp, Grip, Quat, HandPoint, Pinch, PinchEvent, Vec3};
use openxr as xr;

/// What is known about a hand in a frame.
#[derive(Debug, Clone, Copy)]
pub struct HandFrame {
    pub tracked: bool,
    pub index_tip: Vec3,
    pub pinch_point: Vec3,
    pub pinch: PinchEvent,
    pub pinching: bool,
    /// Palm and index, middle, ring and little fingertips (to detect the flat hand).
    pub palm: Vec3,
    pub finger_tips: [Vec3; 4],
    /// Closed fist (grabbing in the air) and its event. While there is a fist, the pinch is
    /// ignored so as not to mistake a fist for a pinch.
    pub gripping: bool,
    pub grip: PinchEvent,
    /// Hand closed around something (grabbing an orb) and its event, and the point where it closes.
    pub grasping: bool,
    pub grasp: PinchEvent,
    pub grasp_point: Vec3,
    /// The hand's 26 joints and which ones are reliable (bit i = joint i).
    pub joints: [Vec3; 26],
    pub joint_rot: [Quat; 26],
    pub joint_mask: u32,
}

impl HandFrame {
    pub const LOST_FRAME: HandFrame = HandFrame::LOST;

    const LOST: HandFrame = HandFrame {
        tracked: false,
        index_tip: Vec3::ZERO,
        pinch_point: Vec3::ZERO,
        pinch: PinchEvent::None,
        pinching: false,
        palm: Vec3::ZERO,
        finger_tips: [Vec3::ZERO; 4],
        gripping: false,
        grip: PinchEvent::None,
        grasping: false,
        grasp: PinchEvent::None,
        grasp_point: Vec3::ZERO,
        joints: [Vec3::ZERO; 26],
        joint_rot: [Quat::IDENTITY; 26],
        joint_mask: 0,
    };

    pub fn as_point(&self) -> HandPoint {
        HandPoint {
            tracked: self.tracked,
            point: self.pinch_point,
            pinch: self.pinch,
            pinching: self.pinching,
            grasp_point: self.grasp_point,
            grasp: self.grasp,
            grasping: self.grasping,
        }
    }
}

/// Roughly where the shoulder of hand `hand` is (0 left, 1 right), with the head at `head` looking toward
/// `forward` (horizontal). The ray that goes through the pinch to reach what is far comes from there, as in the rest
/// of Quest: pointing with the arm is steadier than with the wrist.
pub fn shoulder(head: Vec3, forward: Vec3, hand: usize) -> Vec3 {
    let right = forward.cross(Vec3::Y).normalize_or_zero();
    head + Vec3::new(0.0, -0.18, 0.0) + right * if hand == 1 { 0.17 } else { -0.17 }
}

pub struct HandInput {
    trackers: [xr::HandTracker; 2],
    pinch: [Pinch; 2],
    grip: [Grip; 2],
    grasp: [Grasp; 2],
}

impl HandInput {
    /// `None` if the runtime does not offer hand tracking.
    pub fn new(session: &xr::Session<xr::Vulkan>, available: bool) -> xr::Result<Option<Self>> {
        if !available {
            return Ok(None);
        }
        Ok(Some(Self {
            trackers: [session.create_hand_tracker(xr::Hand::LEFT)?, session.create_hand_tracker(xr::Hand::RIGHT)?],
            pinch: [Pinch::default(); 2],
            grip: [Grip::default(); 2],
            grasp: [Grasp::default(); 2],
        }))
    }

    /// The official mesh of each hand (left and right), if the runtime offers it.
    pub fn meshes(&self, instance: &xr::Instance) -> Result<[atlas_spatial::HandMeshData; 2], String> {
        Ok([
            crate::handmesh::fetch(instance, &self.trackers[0])?,
            crate::handmesh::fetch(instance, &self.trackers[1])?,
        ])
    }

    /// Positions of the hands in `space` for the instant `time`. The left one is 0.
    pub fn update(&mut self, space: &xr::Space, time: xr::Time) -> [HandFrame; 2] {
        let mut frames = [HandFrame::LOST; 2];
        for hand in 0..2 {
            let joints = space.locate_hand_joints(&self.trackers[hand], time).ok().flatten();
            let (cloud, cloud_rot, cloud_mask) = joints.map_or(([Vec3::ZERO; 26], [Quat::IDENTITY; 26], 0u32), |j| {
                let mut pts = [Vec3::ZERO; 26];
                let mut rots = [Quat::IDENTITY; 26];
                let mut mask = 0u32;
                let valid = xr::SpaceLocationFlags::POSITION_VALID | xr::SpaceLocationFlags::ORIENTATION_VALID;
                for (i, l) in j.iter().enumerate().take(26) {
                    if l.location_flags.contains(valid) {
                        pts[i] = Vec3::new(l.pose.position.x, l.pose.position.y, l.pose.position.z);
                        let q = l.pose.orientation;
                        rots[i] = Quat::from_xyzw(q.x, q.y, q.z, q.w);
                        mask |= 1 << i;
                    }
                }
                (pts, rots, mask)
            });
            let extra = joints.and_then(|j| {
                let valid = xr::SpaceLocationFlags::POSITION_VALID;
                let at = |joint: xr::HandJoint| {
                    let l = &j[joint.into_raw() as usize];
                    l.location_flags.contains(valid).then(|| Vec3::new(l.pose.position.x, l.pose.position.y, l.pose.position.z))
                };
                Some((
                    at(xr::HandJoint::PALM)?,
                    [
                        at(xr::HandJoint::INDEX_TIP)?,
                        at(xr::HandJoint::MIDDLE_TIP)?,
                        at(xr::HandJoint::RING_TIP)?,
                        at(xr::HandJoint::LITTLE_TIP)?,
                    ],
                ))
            });
            let tips = joints.and_then(|j| {
                let thumb = j[xr::HandJoint::THUMB_TIP.into_raw() as usize];
                let index = j[xr::HandJoint::INDEX_TIP.into_raw() as usize];
                let valid = xr::SpaceLocationFlags::POSITION_VALID;
                (thumb.location_flags.contains(valid) && index.location_flags.contains(valid)).then(|| {
                    let p = |l: &xr::HandJointLocation| Vec3::new(l.pose.position.x, l.pose.position.y, l.pose.position.z);
                    (p(&thumb), p(&index))
                })
            });
            frames[hand] = match tips {
                Some((thumb, index)) => {
                    let event = self.pinch[hand].update(thumb, index);
                    let (palm, finger_tips) = extra.unwrap_or((Vec3::ZERO, [Vec3::ZERO; 4]));
                    let grip_event = if extra.is_some() { self.grip[hand].update(palm, &finger_tips) } else { PinchEvent::None };
                    let gripping = self.grip[hand].is_active();
                    let grasp_event = if extra.is_some() { self.grasp[hand].update(palm, &finger_tips) } else { PinchEvent::None };
                    HandFrame {
                        tracked: true,
                        index_tip: index,
                        pinch_point: pinch_point(thumb, index),
                        // With a closed fist there is no pinch.
                        pinch: if gripping { PinchEvent::None } else { event },
                        pinching: self.pinch[hand].is_active() && !gripping,
                        palm,
                        finger_tips,
                        gripping,
                        grip: grip_event,
                        grasping: self.grasp[hand].is_active(),
                        grasp: grasp_event,
                        grasp_point: grasp_point(palm, &finger_tips),
                        joints: cloud,
                        joint_rot: cloud_rot,
                        joint_mask: cloud_mask,
                    }
                }
                None => {
                    self.grip[hand].lost();
                    HandFrame { pinch: self.pinch[hand].lost(), grasp: self.grasp[hand].lost(), ..HandFrame::LOST }
                }
            };
        }
        frames
    }
}
