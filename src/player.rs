use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use crate::camera::OrbitCamera;
use crate::combat::Attack;
use crate::hero::{spawn_hero, HeroAnimation, HeroRoot, HERO_FOOT_OFFSET};
use crate::menu::MenuState;
use crate::water::Lake;
use crate::world::terrain_height_at;

const CAPSULE_HALF_HEIGHT: f32 = 0.5;
const CAPSULE_RADIUS: f32 = 0.4;
const WALK_SPEED: f32 = 5.5;
const RUN_SPEED: f32 = 9.5;
// Jump apex ≈ JUMP_SPEED² / (2 * GRAVITY). 14² / 70 ≈ 2.8 m — tall enough
// to hop onto stacked cubes without feeling floaty.
const JUMP_SPEED: f32 = 15.8;
const GRAVITY: f32 = 35.0;
const SWIM_SPEED: f32 = 3.2;
/// How long the hero spends crouching before actually leaving the ground
/// (anticipation animation). 180ms feels like a deliberate windup without
/// making the player wait too long.
const CROUCH_DURATION: f32 = 0.18;
const FACING_TURN_RATE: f32 = 14.0;
// Horizontal acceleration (m/s²). Higher = snappier response.
const GROUND_ACCEL: f32 = 55.0;
const GROUND_DECEL: f32 = 40.0;
// In the air you have less control — slide momentum feels better.
const AIR_ACCEL: f32 = 18.0;
const AIR_DECEL: f32 = 8.0;

#[derive(Component)]
pub struct Player {
    pub vertical_velocity: f32,
    pub grounded: bool,
    /// Horizontal velocity in the XZ plane (m/s). Y is always 0 here;
    /// vertical motion lives in `vertical_velocity`.
    pub horizontal_velocity: Vec3,
    /// Current horizontal speed (m/s) — drives the hero walk/run cadence.
    pub horizontal_speed: f32,
    /// Current yaw (Y rotation, radians). Smoothed toward the input direction.
    pub yaw: f32,
    pub yaw_target: f32,
    /// True while a strafe key (A/Q/D or arrow L/R) is held.
    pub is_strafing: bool,
    /// Counts down during the jump windup.
    pub jump_crouch_timer: f32,
    /// 0 (standing) .. 1 (fully crouched).
    pub crouch_amount: f32,
    /// True when the player's body is currently in the lake (inside the
    /// XZ disc, below the water surface). Read by `hero.rs` to swap to
    /// a prone swim pose.
    pub in_water: bool,
    /// Previous frame's `grounded` — used to detect the landing edge
    /// and apply fall damage.
    pub was_grounded: bool,
    /// Current hit points.
    pub hp: f32,
    /// Max hit points (for HUD percent).
    pub max_hp: f32,
    /// True once HP reaches zero — locks out movement and shows the
    /// YOU ARE DEAD overlay.
    pub dead: bool,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            vertical_velocity: 0.0,
            grounded: false,
            horizontal_velocity: Vec3::ZERO,
            horizontal_speed: 0.0,
            yaw: 0.0,
            yaw_target: 0.0,
            is_strafing: false,
            jump_crouch_timer: 0.0,
            crouch_amount: 0.0,
            in_water: false,
            was_grounded: false,
            hp: 100.0,
            max_hp: 100.0,
            dead: false,
        }
    }
}

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_player)
            .add_systems(Update, (move_player, sync_hero_animation));
    }
}

fn spawn_player(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
) {
    let spawn_x = 0.0;
    let spawn_z = 0.0;
    let spawn_y = terrain_height_at(spawn_x, spawn_z) + 2.0;

    commands
        .spawn((
            Transform::from_xyz(spawn_x, spawn_y, spawn_z),
            Visibility::default(),
            RigidBody::KinematicPositionBased,
            Collider::capsule_y(CAPSULE_HALF_HEIGHT, CAPSULE_RADIUS),
            KinematicCharacterController {
                offset: CharacterLength::Absolute(0.05),
                max_slope_climb_angle: 50_f32.to_radians(),
                min_slope_slide_angle: 55_f32.to_radians(),
                autostep: Some(CharacterAutostep {
                    max_height: CharacterLength::Absolute(0.4),
                    min_width: CharacterLength::Absolute(0.2),
                    include_dynamic_bodies: false,
                }),
                snap_to_ground: Some(CharacterLength::Absolute(0.5)),
                ..default()
            },
            Player::default(),
            Attack::default(),
            Name::new("Player"),
        ))
        .with_children(|parent| {
            // Hero root: offset downward so the feet align with the bottom of
            // the capsule. All hero parts are children of this root.
            parent
                .spawn((
                    Transform::from_xyz(0.0, HERO_FOOT_OFFSET, 0.0),
                    Visibility::default(),
                    HeroRoot,
                    HeroAnimation::default(),
                    Name::new("HeroRoot"),
                ))
                .with_children(|hero| {
                    spawn_hero(hero, &mut meshes, &mut materials, &asset_server);
                });
        });
}

fn move_player(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    lake: Res<Lake>,
    menu: Res<MenuState>,
    camera_q: Query<&Transform, (With<OrbitCamera>, Without<Player>)>,
    mut player_q: Query<
        (
            &mut Player,
            &mut Transform,
            &mut KinematicCharacterController,
            Option<&KinematicCharacterControllerOutput>,
        ),
        With<Player>,
    >,
) {
    let Ok(cam_tf) = camera_q.single() else {
        return;
    };
    let Ok((mut player, mut player_tf, mut controller, output)) = player_q.single_mut() else {
        return;
    };

    let dt = time.delta_secs();

    // --- Landing detection (for fall damage) ---
    // Capture the impact velocity BEFORE Rapier zeroes it on grounding.
    let prev_grounded = player.grounded;
    let pre_landing_vy = player.vertical_velocity;

    if let Some(out) = output {
        player.grounded = out.grounded;
        if out.grounded && player.vertical_velocity < 0.0 {
            player.vertical_velocity = 0.0;
        }
    }

    let just_landed = !prev_grounded && player.grounded;
    // Safe-drop threshold is set above the landing speed of a normal
    // jump (JUMP_SPEED = 15.8 m/s) so just hopping around never hurts.
    // Falls from about 6m and up start dealing damage; ~20m fall is
    // lethal.
    const FALL_DAMAGE_MIN_VY: f32 = 20.0;
    if just_landed && pre_landing_vy < -FALL_DAMAGE_MIN_VY {
        let impact = -pre_landing_vy;
        let dmg = ((impact - FALL_DAMAGE_MIN_VY) * 5.0).min(150.0);
        player.hp = (player.hp - dmg).max(0.0);
        if player.hp <= 0.0 {
            player.dead = true;
        }
    }

    if player.dead {
        // Freeze input when dead so the corpse stays put.
        player.horizontal_velocity = Vec3::ZERO;
        player.horizontal_speed = 0.0;
        player.was_grounded = player.grounded;
        return;
    }

    let mut forward = *cam_tf.forward();
    forward.y = 0.0;
    let forward = forward.normalize_or_zero();
    let mut right = *cam_tf.right();
    right.y = 0.0;
    let right = right.normalize_or_zero();

    // If the pause menu is open, ignore keyboard directional input and
    // suppress the strafing flag — the player freezes on the spot so the
    // user can adjust volume sliders without the hero wandering.
    let mut input = Vec3::ZERO;
    let (strafe_left, strafe_right) = if menu.open {
        (false, false)
    } else {
        if keys.any_pressed([KeyCode::KeyW, KeyCode::KeyZ, KeyCode::ArrowUp]) {
            input += forward;
        }
        if keys.any_pressed([KeyCode::KeyS, KeyCode::ArrowDown]) {
            input -= forward;
        }
        let sl =
            keys.any_pressed([KeyCode::KeyA, KeyCode::KeyQ, KeyCode::ArrowLeft]);
        let sr = keys.any_pressed([KeyCode::KeyD, KeyCode::ArrowRight]);
        if sl {
            input -= right;
        }
        if sr {
            input += right;
        }
        (sl, sr)
    };
    player.is_strafing = strafe_left || strafe_right;
    let input_magnitude = input.length();
    let input_dir = if input_magnitude > 0.001 {
        input / input_magnitude
    } else {
        Vec3::ZERO
    };

    // --- In-water detection: inside the lake's XZ disc AND below the
    // water surface (with a small margin so the transition feels
    // natural as the player wades in). ---
    let to_lake = player_tf.translation.xz() - lake.xz;
    let in_lake_xz = to_lake.length_squared() < lake.radius * lake.radius;
    let in_water = in_lake_xz && player_tf.translation.y < lake.surface_y + 0.15;
    player.in_water = in_water;

    let running = !menu.open
        && (keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight));
    let max_speed = if in_water {
        SWIM_SPEED
    } else if running {
        RUN_SPEED
    } else {
        WALK_SPEED
    };

    if in_water {
        // Water physics: no gravity, buoyancy restores player toward the
        // surface, no jumping. Player can dive a bit by swimming down
        // via terrain contact but mostly floats near the surface.
        let depth = lake.surface_y - player_tf.translation.y;
        if depth > 0.05 {
            // Below surface — push up.
            player.vertical_velocity += (4.0 + depth * 6.0) * dt;
        } else {
            // At/above surface — light downward pull toward surface
            // unless player is cleanly above (wading out).
            player.vertical_velocity -= 3.0 * dt;
        }
        player.vertical_velocity = player.vertical_velocity.clamp(-2.5, 2.5);
        // Exiting water resets any in-progress jump windup.
        player.jump_crouch_timer = 0.0;
    } else {
        // Jump windup. Space committed the player to a jump but they
        // linger on the ground for CROUCH_DURATION before actually
        // launching — during that time the hero visibly crouches.
        if player.jump_crouch_timer > 0.0 {
            player.jump_crouch_timer -= dt;
            if player.jump_crouch_timer <= 0.0 {
                player.vertical_velocity = JUMP_SPEED;
                player.grounded = false;
                player.jump_crouch_timer = 0.0;
            }
        } else if !menu.open && keys.just_pressed(KeyCode::Space) && player.grounded {
            player.jump_crouch_timer = CROUCH_DURATION;
        }
        player.vertical_velocity -= GRAVITY * dt;
        player.vertical_velocity = player.vertical_velocity.max(-40.0);
    }

    // Compute a 0..1 crouch value from the windup timer (0 = standing,
    // 1 = fully crouched). hero.rs reads this to squash the mesh.
    player.crouch_amount = if player.jump_crouch_timer > 0.0 {
        (1.0 - player.jump_crouch_timer / CROUCH_DURATION).clamp(0.0, 1.0)
    } else {
        // Decay back to 0 quickly (~0.15s) so the hero eases out of
        // crouch instead of snapping up the moment of launch.
        (player.crouch_amount - dt * 6.0).max(0.0)
    };

    // --- Horizontal acceleration ---
    // Target velocity is input direction * max_speed. Push current velocity
    // toward the target with per-frame acceleration rather than snapping.
    let target_velocity = input_dir * max_speed * input_magnitude.clamp(0.0, 1.0);
    let (accel, decel) = if player.grounded {
        (GROUND_ACCEL, GROUND_DECEL)
    } else {
        (AIR_ACCEL, AIR_DECEL)
    };
    let rate = if target_velocity.length_squared() > 0.0 {
        accel
    } else {
        decel
    };
    let diff = target_velocity - player.horizontal_velocity;
    let max_step = rate * dt;
    let step = if diff.length() > max_step {
        diff.normalize() * max_step
    } else {
        diff
    };
    player.horizontal_velocity += step;
    // Zero out tiny residuals so the idle animation fully settles.
    if player.horizontal_velocity.length_squared() < 0.001 {
        player.horizontal_velocity = Vec3::ZERO;
    }
    player.horizontal_speed = player.horizontal_velocity.length();

    // Smoothly rotate the player to face the movement direction — but not
    // while strafing: sideways input should not steer the character, only
    // translate them.
    if !player.is_strafing && input_dir.length_squared() > 0.0 {
        player.yaw_target = input_dir.x.atan2(input_dir.z) + std::f32::consts::PI;
    }
    let turn_alpha = 1.0 - (-FACING_TURN_RATE * dt).exp();
    player.yaw = lerp_angle(player.yaw, player.yaw_target, turn_alpha);
    player_tf.rotation = Quat::from_rotation_y(player.yaw);

    let horizontal = player.horizontal_velocity * dt;
    let vertical = Vec3::Y * player.vertical_velocity * dt;
    controller.translation = Some(horizontal + vertical);

    player.was_grounded = player.grounded;
}

fn sync_hero_animation(
    player_q: Query<&Player>,
    mut hero_q: Query<&mut HeroAnimation, With<HeroRoot>>,
) {
    let Ok(player) = player_q.single() else {
        return;
    };
    for mut anim in &mut hero_q {
        anim.speed = player.horizontal_speed;
    }
}

fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let two_pi = std::f32::consts::TAU;
    let diff =
        ((b - a) % two_pi + two_pi + std::f32::consts::PI) % two_pi - std::f32::consts::PI;
    a + diff * t
}
