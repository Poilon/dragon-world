use bevy::anti_alias::smaa::{Smaa, SmaaPreset};
use bevy::camera::Exposure;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::ecs::message::MessageReader;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::pbr::{
    Atmosphere, AtmosphereSettings, ScatteringMedium, ScreenSpaceAmbientOcclusion,
    ScreenSpaceAmbientOcclusionQualityLevel,
};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::window::CursorMoved;
use bevy_rapier3d::prelude::{QueryFilter, ReadRapierContext};

use crate::player::Player;

// Radians per pixel of *logical* cursor movement. A full screen sweep
// (~1280 px) then rotates the camera by ~4 rad — comfortable feel.
const DEFAULT_MOUSE_SENS: f32 = 0.003;
const ZOOM_SENS: f32 = 0.8;

fn mouse_sens() -> f32 {
    std::env::var("DW_MOUSE_SENS")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(DEFAULT_MOUSE_SENS)
}
// 3rd-person orbit. Pitch range is symmetric around horizontal so the
// camera can dip well below the player (to frame from under their feet
// looking up) as well as circle up for a top-down view.
const MIN_PITCH: f32 = -1.4;
const MAX_PITCH: f32 = 1.4;
const MIN_DISTANCE: f32 = 2.5;
const MAX_DISTANCE: f32 = 20.0;
const DEFAULT_DISTANCE: f32 = 6.0;
const CAMERA_HEIGHT_OFFSET: f32 = 1.2;
// Higher = snappier, lower = smoother. 18 gives a ~55ms time-to-target feel.
const ROTATION_SMOOTHING: f32 = 18.0;
const POSITION_SMOOTHING: f32 = 22.0;
/// How fast the camera catches up to "behind the player" when moving and
/// not manually dragging. Lower = softer, higher = snappier.
const AUTO_FOLLOW_RATE: f32 = 3.5;
/// Minimum horizontal speed (m/s) before auto-follow kicks in — avoids
/// drift while the player is barely moving / decelerating.
const AUTO_FOLLOW_MIN_SPEED: f32 = 0.5;
/// Seconds the player has to keep moving (above MIN_SPEED, not strafing)
/// before auto-follow starts drifting the yaw. Short bursts of movement
/// don't snap the camera around; only sustained walks do.
const AUTO_FOLLOW_DELAY: f32 = 0.45;
/// How close the camera is allowed to approach solid geometry along its
/// focus-to-target ray before being pulled in. Large-ish buffer because
/// walls + glass are thin and smoothing lag can otherwise let the
/// camera briefly poke through.
const CAMERA_COLLISION_BUFFER: f32 = 0.45;
/// Absolute minimum distance from the focus — prevents a degenerate
/// camera-at-head position when `toi` collapses (e.g. player wedged
/// against a wall).
const CAMERA_COLLISION_MIN_FLOOR: f32 = 0.10;

#[derive(Component)]
pub struct OrbitCamera {
    // Target yaw/pitch are what the mouse directly drives.
    pub yaw_target: f32,
    pub pitch_target: f32,
    // Current yaw/pitch lerp toward the targets each frame for smooth feel.
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    /// Seconds the player has been continuously moving forward (not
    /// strafing, not dragging). Auto-follow only engages after this
    /// crosses `AUTO_FOLLOW_DELAY`, so short jinks don't whip the camera.
    pub follow_hold: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            yaw_target: 0.0,
            pitch_target: 0.35,
            yaw: 0.0,
            pitch: 0.35,
            distance: DEFAULT_DISTANCE,
            follow_hold: 0.0,
        }
    }
}

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_camera)
            // Run in PostUpdate so we read the player Transform AFTER Rapier
            // has synced kinematic bodies, avoiding one-frame lag jitter.
            .add_systems(PostUpdate, update_camera);
    }
}

fn spawn_camera(mut commands: Commands, mut media: ResMut<Assets<ScatteringMedium>>) {
    // Atmosphere needs a scattering medium handle. Default = earthlike
    // (Rayleigh + Mie + ozone), which gives the classic blue sky at noon and
    // golden/red scattering at low sun angles.
    let medium = media.add(ScatteringMedium::default());

    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 5.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
        // Physically-based exposure matched to the 110k-lux sun — without
        // this, a realistic outdoor scene clips to white.
        Exposure::SUNLIGHT,
        // HDR is auto-required by both Bloom and Atmosphere, but being
        // explicit reads better.
        Tonemapping::TonyMcMapface,
        // NATURAL preset: subtle, physically-plausible halo around emissives
        // and bright highlights. Pairs well with TonyMcMapface.
        Bloom::NATURAL,
        // Procedural sky — Rayleigh + Mie scattering. Replaces the flat
        // ClearColor and also feeds ambient light into the scene.
        Atmosphere::earthlike(medium),
        AtmosphereSettings::default(),
        // Grounding contact shadows in crevices, under the hero's chin, etc.
        ScreenSpaceAmbientOcclusion {
            quality_level: ScreenSpaceAmbientOcclusionQualityLevel::High,
            ..default()
        },
        // SMAA: cheap, no motion vectors required, works well with SSAO.
        Smaa {
            preset: SmaaPreset::High,
        },
        OrbitCamera::default(),
        Name::new("OrbitCamera"),
    ));
}

fn update_camera(
    time: Res<Time>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut cursor_moved: MessageReader<CursorMoved>,
    mouse_scroll: Res<AccumulatedMouseScroll>,
    rapier: ReadRapierContext,
    player_q: Query<(&Transform, &Player), Without<OrbitCamera>>,
    mut camera_q: Query<(&mut OrbitCamera, &mut Transform), Without<Player>>,
) {
    let Ok((player_tf, player)) = player_q.single() else {
        return;
    };
    let Ok((mut orbit, mut cam_tf)) = camera_q.single_mut() else {
        return;
    };

    // Sum the per-frame cursor delta from CursorMoved events. Using logical
    // pixels (via cursor position deltas) keeps feel consistent across mouse
    // DPIs, unlike raw DeviceEvent::MouseMotion which is device-units per event
    // and explodes on high-DPI/gaming mice.
    let mut cursor_delta = Vec2::ZERO;
    for ev in cursor_moved.read() {
        if let Some(d) = ev.delta {
            cursor_delta += d;
        }
    }

    let dt = time.delta_secs();

    // Right-drag = manual orbit. While dragging we suspend auto-follow so the
    // player can freely look around the hero.
    let dragging = mouse_buttons.pressed(MouseButton::Right);
    let auto_follow_eligible =
        !dragging && !player.is_strafing && player.horizontal_speed > AUTO_FOLLOW_MIN_SPEED;
    if auto_follow_eligible {
        orbit.follow_hold += dt;
    } else {
        orbit.follow_hold = 0.0;
    }
    if dragging {
        let sens = mouse_sens();
        orbit.yaw_target -= cursor_delta.x * sens;
        orbit.pitch_target =
            (orbit.pitch_target + cursor_delta.y * sens).clamp(MIN_PITCH, MAX_PITCH);
    } else if orbit.follow_hold > AUTO_FOLLOW_DELAY {
        // Auto-orient: drift the target yaw toward the player's facing
        // direction so the camera eases behind them as they move.
        // Suppressed while strafing so Q/D move purely sideways in view,
        // and gated by the hold timer so brief direction changes don't
        // drag the camera with them.
        let alpha = 1.0 - (-AUTO_FOLLOW_RATE * dt).exp();
        orbit.yaw_target = lerp_angle(orbit.yaw_target, player.yaw, alpha);
    }

    // Scroll wheel zooms. Snappy (no smoothing).
    if mouse_scroll.delta.y.abs() > f32::EPSILON {
        orbit.distance =
            (orbit.distance - mouse_scroll.delta.y * ZOOM_SENS).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    // Exponential smoothing, framerate-independent.
    let rot_alpha = 1.0 - (-ROTATION_SMOOTHING * dt).exp();
    orbit.yaw = lerp_angle(orbit.yaw, orbit.yaw_target, rot_alpha);
    orbit.pitch += (orbit.pitch_target - orbit.pitch) * rot_alpha;

    let focus = player_tf.translation + Vec3::Y * CAMERA_HEIGHT_OFFSET;
    let offset = Vec3::new(
        orbit.yaw.sin() * orbit.pitch.cos(),
        orbit.pitch.sin(),
        orbit.yaw.cos() * orbit.pitch.cos(),
    ) * orbit.distance;
    let mut target_pos = focus + offset;

    // Raycast from the player's head to the desired camera spot. If any
    // Fixed collider (walls, glass, terrain, roof, socle, hearth) is in
    // the way, pull the camera in just before the hit — this stops the
    // view from clipping through walls when you enter a house, going
    // below terrain, etc.
    let mut collision_clamped = false;
    let ray_dir_full = target_pos - focus;
    let ray_len = ray_dir_full.length();
    if ray_len > 1e-4 {
        let ray_dir = ray_dir_full / ray_len;
        if let Ok(ctx) = rapier.single() {
            if let Some((_, toi)) = ctx.cast_ray(
                focus,
                ray_dir,
                ray_len,
                true,
                QueryFilter::only_fixed(),
            ) {
                // Cap by `toi` so the camera never overshoots the wall when
                // the player is pressed against it (small toi, previously
                // the MIN_DISTANCE floor would push the camera *past* the
                // wall along the ray).
                let clamped = ((toi - CAMERA_COLLISION_BUFFER).max(CAMERA_COLLISION_MIN_FLOOR))
                    .min(toi.max(CAMERA_COLLISION_MIN_FLOOR));
                target_pos = focus + ray_dir * clamped.min(ray_len);
                collision_clamped = true;
            }
        }
    }

    // When we had to clip in, snap instead of lerp. The smoothed lerp
    // would let the camera drift through the wall for a handful of
    // frames before catching up with the clamp. Snapping is imperceptible
    // because the target is already *close* to the previous position
    // (same orbit, just shortened).
    if collision_clamped {
        cam_tf.translation = target_pos;
    } else {
        let pos_alpha = 1.0 - (-POSITION_SMOOTHING * dt).exp();
        cam_tf.translation = cam_tf.translation.lerp(target_pos, pos_alpha);
    }
    cam_tf.look_at(focus, Vec3::Y);
}

/// Lerp between two angles taking the shortest path (handles wrap-around).
fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let two_pi = std::f32::consts::TAU;
    let diff = ((b - a) % two_pi + two_pi + std::f32::consts::PI) % two_pi - std::f32::consts::PI;
    a + diff * t
}
