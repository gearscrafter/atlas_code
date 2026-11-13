//! Throwing an orb: gravity, bouncing on the desk and hitting the walls, plus
//! estimating the hand's speed on release. Data only: tested on the PC.

use std::collections::VecDeque;

use glam::Vec3;

use crate::wall::Wall;

/// Gravity (m/s²). Lower than Earth's: the orb draws a long, visible curve, it does not drop like a weight.
pub const GRAVITY: f32 = 4.5;
/// Fraction of the speed kept on bouncing.
const RESTITUTION: f32 = 0.45;
/// Horizontal friction on each bounce on the desk.
const BOUNCE_FRICTION: f32 = 0.8;
/// Air drag (per second).
const AIR_DRAG: f32 = 0.25;
/// Below this speed (m/s) a bounce turns into rest.
const REST_SPEED: f32 = 0.35;
/// Minimum hand speed (m/s) on release for it to count as a throw.
pub const THROW_MIN_SPEED: f32 = 0.8;
/// Maximum step of the simulation (s): it keeps a fast orb from passing through a wall.
const MAX_STEP: f32 = 0.004;
/// Maximum speed (m/s) of a throw.
pub const MAX_THROW_SPEED: f32 = 8.0;

/// The environment the orb collides with.
#[derive(Debug, Clone)]
pub struct World {
    /// Height of the desk, if mapped.
    pub desk_height: Option<f32>,
    pub walls: Vec<Wall>,
    /// Gravedad (m/s²).
    pub gravity: f32,
}

impl Default for World {
    fn default() -> Self {
        Self { desk_height: None, walls: Vec::new(), gravity: GRAVITY }
    }
}

/// A collision.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hit {
    Desk { point: Vec3, speed: f32 },
    Wall { point: Vec3, speed: f32 },
}

/// An orb in flight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Body {
    pub pos: Vec3,
    pub vel: Vec3,
    pub radius: f32,
    /// Still on the desk.
    pub resting: bool,
}

impl Body {
    pub fn thrown(pos: Vec3, vel: Vec3, radius: f32) -> Self {
        Self { pos, vel: vel.clamp_length_max(MAX_THROW_SPEED), radius, resting: false }
    }

    /// Advances `dt` seconds and returns the collisions that happened.
    pub fn step(&mut self, dt: f32, world: &World) -> Vec<Hit> {
        let mut hits = Vec::new();
        let mut left = dt.max(0.0);
        while left > 1e-6 && !self.resting {
            let h = left.min(MAX_STEP);
            left -= h;
            self.vel.y -= world.gravity * h;
            self.vel *= 1.0 - AIR_DRAG * h;
            self.pos += self.vel * h;
            self.collide(world, &mut hits);
        }
        hits
    }

    fn collide(&mut self, world: &World, hits: &mut Vec<Hit>) {
        for wall in &world.walls {
            let d = wall.distance(self.pos) - self.radius;
            let toward = self.vel.dot(wall.normal);
            if d < 0.0 && toward < 0.0 {
                self.pos -= wall.normal * d;
                self.vel -= wall.normal * (1.0 + RESTITUTION) * toward;
                hits.push(Hit::Wall { point: self.pos - wall.normal * self.radius, speed: -toward });
            }
        }
        if let Some(height) = world.desk_height {
            if self.pos.y - self.radius < height && self.vel.y < 0.0 {
                let speed = -self.vel.y;
                self.pos.y = height + self.radius;
                self.vel.y = -self.vel.y * RESTITUTION;
                self.vel.x *= BOUNCE_FRICTION;
                self.vel.z *= BOUNCE_FRICTION;
                if self.vel.y < REST_SPEED {
                    self.vel = Vec3::ZERO;
                    self.resting = true;
                }
                hits.push(Hit::Desk { point: Vec3::new(self.pos.x, height, self.pos.z), speed });
            }
        }
    }
}

/// Estimates the hand's speed from its latest positions.
#[derive(Debug, Clone, Default)]
pub struct VelocityTracker {
    samples: VecDeque<(Vec3, f32)>,
}

/// Window (s) over which the speed is measured: the last thing the hand did before releasing.
const WINDOW: f32 = 0.10;

impl VelocityTracker {
    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// Adds this frame's position and the time (s) since the previous one.
    pub fn push(&mut self, pos: Vec3, dt: f32) {
        self.samples.push_back((pos, dt.max(1e-4)));
        let mut total: f32 = self.samples.iter().skip(1).map(|(_, d)| *d).sum();
        while total > WINDOW && self.samples.len() > 2 {
            if let Some((_, d)) = self.samples.get(1) {
                total -= *d;
            }
            self.samples.pop_front();
        }
    }

    /// Average speed over the window; zero if there is too little data.
    pub fn velocity(&self) -> Vec3 {
        let (Some((first, _)), Some((last, _))) = (self.samples.front(), self.samples.back()) else {
            return Vec3::ZERO;
        };
        let time: f32 = self.samples.iter().skip(1).map(|(_, d)| *d).sum();
        if time < 0.02 {
            Vec3::ZERO
        } else {
            (*last - *first) / time
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall_at_z(z: f32) -> Wall {
        // Wall at z = `z`; the user is on the positive side of z.
        Wall { point: Vec3::new(0.0, 0.0, z), normal: Vec3::Z }
    }

    #[test]
    fn gravity_pulls_a_free_orb_down() {
        let mut b = Body::thrown(Vec3::new(0.0, 1.5, -0.5), Vec3::ZERO, 0.05);
        for _ in 0..30 {
            b.step(1.0 / 72.0, &World::default());
        }
        assert!(b.pos.y < 1.5 - 0.3, "it must have fallen: {}", b.pos.y);
        assert!(b.vel.y < -1.0);
    }

    #[test]
    fn it_bounces_on_the_desk_and_comes_to_rest() {
        let world = World { desk_height: Some(0.7), walls: vec![], ..World::default() };
        let mut b = Body::thrown(Vec3::new(0.0, 1.4, -0.5), Vec3::new(0.3, 0.0, 0.0), 0.05);
        let mut bounces = 0;
        for _ in 0..720 {
            bounces += b.step(1.0 / 72.0, &world).iter().filter(|h| matches!(h, Hit::Desk { .. })).count();
        }
        assert!(bounces >= 2, "it must bounce more than once: {bounces}");
        assert!(b.resting && b.vel == Vec3::ZERO);
        assert!((b.pos.y - (0.7 + 0.05)).abs() < 1e-4, "it rests on the desk: {}", b.pos.y);
        assert!(b.pos.x > 0.0, "it moved a little in the direction it was going");
    }

    #[test]
    fn it_hits_a_wall_and_reports_where_and_how_hard() {
        let world = World { desk_height: None, walls: vec![wall_at_z(-2.0)], ..World::default() };
        let mut b = Body::thrown(Vec3::new(0.0, 1.2, -0.5), Vec3::new(0.0, 0.5, -4.0), 0.05);
        let mut hit = None;
        for _ in 0..200 {
            if let Some(h) = b.step(1.0 / 72.0, &world).into_iter().find(|h| matches!(h, Hit::Wall { .. })) {
                hit = Some(h);
                break;
            }
        }
        let Some(Hit::Wall { point, speed }) = hit else { panic!("it did not hit the wall") };
        assert!((point.z + 2.0).abs() < 0.02, "it hits on the wall: {point}");
        assert!(speed > 3.0, "{speed}");
        assert!(b.vel.z > 0.0, "it bounces back toward the user");
    }

    #[test]
    fn a_fast_orb_does_not_tunnel_through_a_wall() {
        let world = World { desk_height: None, walls: vec![wall_at_z(-1.0)], ..World::default() };
        let mut b = Body::thrown(Vec3::new(0.0, 1.0, -0.9), Vec3::new(0.0, 0.0, -8.0), 0.02);
        b.step(0.5, &world);
        assert!(b.pos.z > -1.0, "it must not pass through it: {}", b.pos.z);
    }

    #[test]
    fn moving_away_from_a_wall_does_not_collide() {
        let world = World { desk_height: None, walls: vec![wall_at_z(-1.0)], ..World::default() };
        let mut b = Body::thrown(Vec3::new(0.0, 1.0, -1.04), Vec3::new(0.0, 0.0, 1.0), 0.05);
        assert!(b.step(0.01, &world).is_empty());
    }

    #[test]
    fn throw_speed_is_capped() {
        let b = Body::thrown(Vec3::ZERO, Vec3::new(0.0, 0.0, -100.0), 0.05);
        assert!(b.vel.length() <= MAX_THROW_SPEED + 1e-4);
    }

    #[test]
    fn a_resting_orb_stays_put() {
        let mut b = Body { pos: Vec3::new(0.0, 0.75, 0.0), vel: Vec3::ZERO, radius: 0.05, resting: true };
        b.step(1.0, &World { desk_height: Some(0.7), walls: vec![], ..World::default() });
        assert_eq!(b.pos, Vec3::new(0.0, 0.75, 0.0));
    }

    // ----- hand speed -----

    #[test]
    fn velocity_follows_a_steady_hand_motion() {
        let mut t = VelocityTracker::default();
        let dt = 1.0 / 72.0;
        for i in 0..20 {
            t.push(Vec3::new(0.0, 0.0, -2.0 * dt * i as f32), dt); // 2 m/s hacia delante
        }
        let v = t.velocity();
        assert!((v.z + 2.0).abs() < 0.05 && v.x.abs() < 1e-4, "{v}");
    }

    #[test]
    fn only_the_last_moments_count() {
        let mut t = VelocityTracker::default();
        let dt = 1.0 / 72.0;
        // One second moving fast and then still: on release, the speed is ~0.
        for i in 0..72 {
            t.push(Vec3::new(i as f32 * 0.05, 0.0, 0.0), dt);
        }
        for _ in 0..20 {
            t.push(Vec3::new(3.6, 0.0, 0.0), dt);
        }
        assert!(t.velocity().length() < 0.05, "{}", t.velocity());
    }

    #[test]
    fn too_little_data_gives_zero() {
        let mut t = VelocityTracker::default();
        assert_eq!(t.velocity(), Vec3::ZERO);
        t.push(Vec3::ZERO, 0.014);
        assert_eq!(t.velocity(), Vec3::ZERO);
        t.clear();
        assert_eq!(t.velocity(), Vec3::ZERO);
    }
}
