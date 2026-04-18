use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

/// Heavier-than-Earth gravity — BOTW-style weight. Dynamic bodies feel
/// chunky and fall decisively.
const GRAVITY: Vec3 = Vec3::new(0.0, -28.0, 0.0);

pub struct PhysicsPlugin;

impl Plugin for PhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, set_gravity);
    }
}

fn set_gravity(mut configs: Query<&mut RapierConfiguration>) {
    for mut cfg in &mut configs {
        cfg.gravity = GRAVITY;
    }
}
