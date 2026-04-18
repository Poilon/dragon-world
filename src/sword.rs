//! Picking up the hero's sword. Hero starts empty-handed; the sword sits
//! on the mezzanine of the 2-story house. When the player hovers the
//! cursor over it, the blade highlights; when they click while close
//! enough, the sword moves into the hero's hand and combat becomes
//! available.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_rapier3d::prelude::{QueryFilter, ReadRapierContext};

use crate::camera::OrbitCamera;
use crate::combat::trigger_attack;
use crate::menu::MenuState;
use crate::player::Player;

/// Whether the hero currently holds the sword. Set by the pickup system.
/// Read by `hero::toggle_sword_visibility` and `combat::trigger_attack`.
#[derive(Resource, Default)]
pub struct HasSword(pub bool);

/// Attached to the hover-and-click-able sword standing on the mezzanine.
#[derive(Component, Default)]
pub struct SwordPickup {
    pub highlighted: bool,
}

/// Meshes (blade/hilt/grip) of the pickup. The pickup system mutates
/// these meshes' material emissive to light them up on hover.
#[derive(Component)]
pub struct SwordPickupPart;

/// Max distance the player can be from the sword for the pickup click to
/// register.
const PICKUP_RANGE: f32 = 2.8;

pub struct SwordPlugin;

impl Plugin for SwordPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HasSword>().add_systems(
            Update,
            (
                hover_sword,
                // Pickup must run AFTER combat so that on the frame of the
                // pickup click, combat sees the *old* `HasSword = false`
                // and skips attacking.
                click_pickup.after(trigger_attack),
                animate_pickup,
                update_highlight,
            ),
        );
    }
}

/// Spawn the sword pickup at a given world transform.
pub fn spawn_sword_pickup(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    world_transform: Transform,
) {
    // Steel with a faint idle glow so the sword reads as an interactive
    // object from across the mezzanine. Highlighted state boosts the
    // emissive to a stronger blue.
    let blade_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.82, 0.85, 0.92),
        emissive: LinearRgba::new(0.15, 0.35, 0.9, 1.0),
        perceptual_roughness: 0.22,
        metallic: 0.85,
        ..default()
    });
    let leather_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.35, 0.22, 0.14),
        emissive: LinearRgba::new(0.08, 0.15, 0.32, 1.0),
        perceptual_roughness: 0.75,
        ..default()
    });

    let blade_mesh = meshes.add(Cuboid::new(0.09, 0.95, 0.02));
    let hilt_mesh = meshes.add(Cuboid::new(0.26, 0.05, 0.05));
    let grip_mesh = meshes.add(Cuboid::new(0.05, 0.20, 0.05));

    // Root — floats at `world_transform`. The child meshes compose the
    // sword centered around that root (grip at origin, blade up, hilt just
    // below the blade). All three parts share a collider via the root so
    // ray-casts against the SwordPickup only return this entity.
    commands
        .spawn((
            world_transform,
            Visibility::default(),
            SwordPickup::default(),
            // AABB-ish collider that envelops the whole sword so the
            // cursor raycast catches any part of it.
            bevy_rapier3d::prelude::RigidBody::Fixed,
            bevy_rapier3d::prelude::Collider::cuboid(0.18, 0.7, 0.08),
            Name::new("SwordPickup"),
        ))
        .with_children(|root| {
            root.spawn((
                Mesh3d(blade_mesh),
                MeshMaterial3d(blade_mat.clone()),
                Transform::from_xyz(0.0, 0.55, 0.0),
                SwordPickupPart,
                Name::new("PickupBlade"),
            ));
            root.spawn((
                Mesh3d(hilt_mesh),
                MeshMaterial3d(blade_mat),
                Transform::from_xyz(0.0, 0.03, 0.0),
                SwordPickupPart,
                Name::new("PickupHilt"),
            ));
            root.spawn((
                Mesh3d(grip_mesh),
                MeshMaterial3d(leather_mat),
                Transform::from_xyz(0.0, -0.1, 0.0),
                SwordPickupPart,
                Name::new("PickupGrip"),
            ));
        });
}

fn hover_sword(
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<OrbitCamera>>,
    rapier: ReadRapierContext,
    mut pickups: Query<&mut SwordPickup>,
) {
    // Clear previous highlights before re-evaluating.
    for mut p in &mut pickups {
        p.highlighted = false;
    }

    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok((camera, cam_tf)) = camera_q.single() else {
        return;
    };
    let Ok(ray) = camera.viewport_to_world(cam_tf, cursor) else {
        return;
    };
    let Ok(ctx) = rapier.single() else {
        return;
    };

    // Ray-cast into Fixed colliders (walls, socle, sword). First hit is
    // what the player "sees" at their cursor — if it's a sword, highlight
    // it; otherwise the wall/socle blocks the pickup and we stay idle.
    if let Some((entity, _toi)) = ctx.cast_ray(
        ray.origin,
        ray.direction.into(),
        60.0,
        true,
        QueryFilter::only_fixed(),
    ) {
        if let Ok(mut p) = pickups.get_mut(entity) {
            p.highlighted = true;
        }
    }
}

fn click_pickup(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    mut has_sword: ResMut<HasSword>,
    menu: Res<MenuState>,
    pickups: Query<(Entity, &GlobalTransform, &SwordPickup)>,
    player_q: Query<&Transform, With<Player>>,
) {
    if has_sword.0 || menu.open {
        return;
    }
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let Ok(player_tf) = player_q.single() else {
        return;
    };

    for (entity, tf, pickup) in &pickups {
        if !pickup.highlighted {
            continue;
        }
        let dist = (tf.translation() - player_tf.translation).length();
        if dist > PICKUP_RANGE {
            continue;
        }
        has_sword.0 = true;
        commands.entity(entity).despawn();
        break;
    }
}

/// Slow idle motion so the pickup reads as "interactive" — a subtle
/// bob and spin.
fn animate_pickup(time: Res<Time>, mut q: Query<&mut Transform, With<SwordPickup>>) {
    let t = time.elapsed_secs();
    for mut tf in &mut q {
        let bob = (t * 1.6).sin() * 0.08;
        // Base Y is set at spawn; bobbing overwrites by reading the first
        // element. We instead store the base Y implicitly by only
        // modifying the visual via rotation, not translation. Rotation
        // around Y spins the sword visually without moving its collider
        // much (the collider stays axis-aligned because only the root
        // rotates and the collider is near-square on X/Z).
        tf.rotation = Quat::from_rotation_y(t * 0.6)
            * Quat::from_rotation_z(bob * 0.1);
    }
}

/// Per-frame emissive pulse for the sword parts; brighter and bluer when
/// the cursor is hovering the pickup.
fn update_highlight(
    time: Res<Time>,
    pickups: Query<(&SwordPickup, &Children)>,
    parts: Query<&MeshMaterial3d<StandardMaterial>, With<SwordPickupPart>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let t = time.elapsed_secs();
    let pulse = (t * 3.0).sin() * 0.5 + 0.5;
    for (pickup, children) in &pickups {
        let emissive = if pickup.highlighted {
            // Brighter, pulsing.
            LinearRgba::new(
                0.8 + pulse * 1.2,
                1.6 + pulse * 2.4,
                3.5 + pulse * 4.5,
                1.0,
            )
        } else {
            LinearRgba::new(0.15, 0.35, 0.9, 1.0)
        };
        for child in children.iter() {
            if let Ok(mat_handle) = parts.get(child) {
                if let Some(mat) = materials.get_mut(&mat_handle.0) {
                    mat.emissive = emissive;
                }
            }
        }
    }
}
