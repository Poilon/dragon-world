//! Procedural forest ringing the village. Three tree types (oaks, pines,
//! bushes) scattered via density noise, on an annulus that excludes the
//! village footprint. Each tree sways in the wind.

use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::light::NotShadowCaster;
use bevy::math::Affine2;
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;
use noise::{NoiseFn, Perlin};

use crate::player::Player;
use crate::world::{terrain_height_at, LAKE_CENTER, LAKE_RADIUS};

// --- Placement parameters ----------------------------------------------------

/// Trees can't grow closer to the origin than this. Keeps the village
/// center clear.
const INNER_RADIUS: f32 = 25.0;
/// Outer edge of the forest. Beyond this the mountain slopes are usually
/// too steep, and the trees would show past the terrain edge.
const OUTER_RADIUS: f32 = 88.0;

const GRID_SPACING: f32 = 4.2;
/// Perlin density field: points where the sample is below this threshold
/// get a tree. Tuning this changes how dense the forest is.
const DENSITY_FREQ: f64 = 0.025;
const DENSITY_CUTOFF: f32 = -0.05;
const VARIETY_FREQ: f64 = 0.09;
const JITTER_FREQ: f64 = 0.73;
const JITTER_AMP: f32 = 1.8;

/// Skip a tree if terrain is too steep at its spot — avoids trees
/// floating half-buried in cliff faces.
const MAX_TERRAIN_SLOPE: f32 = 4.0;

/// Keep trees out of a radius around each house center.
const HOUSE_EXCLUSION_RADIUS: f32 = 8.0;
/// House centers — mirrors the layout in house.rs. If you move the
/// village, update this list too.
const HOUSE_CENTERS: &[(f32, f32)] = &[
    (-9.0, -3.0),
    (10.0, -7.0),
    (-11.5, -15.0),
    (10.5, -17.0),
    (-8.5, -25.0),
    (10.0, -26.5),
];

/// Church is bigger than a regular house, so it gets its own larger
/// exclusion disc.
const CHURCH_CENTER: (f32, f32) = (0.0, -34.0);
const CHURCH_EXCLUSION_RADIUS: f32 = 10.0;

/// Keep trees off the village path (a strip along the Z axis) — extended
/// to reach the church door.
const PATH_X_HALF: f32 = 4.0;
const PATH_Z_MIN: f32 = -27.0;
const PATH_Z_MAX: f32 = 6.0;

// --- Components --------------------------------------------------------------

/// Wind sway on the tree's visual root. The trunk tilts a tiny amount
/// around its base every frame; foliage follows via the child hierarchy.
#[derive(Component)]
struct WindSway {
    phase: f32,
    amplitude: f32,
    base_rotation: Quat,
}

// --- Plugin ------------------------------------------------------------------

pub struct TreesPlugin;

impl Plugin for TreesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_forest)
            .add_systems(Update, animate_wind);
    }
}

// --- Forest placement --------------------------------------------------------

fn spawn_forest(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
) {
    // Bark textures (Poly Haven CC0). Repeat sampler so we can tile the
    // capture multiple times around each trunk.
    let load_repeat = |path: &'static str| {
        asset_server.load_with_settings::<Image, ImageLoaderSettings>(
            path,
            |settings: &mut ImageLoaderSettings| {
                settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                    address_mode_u: ImageAddressMode::Repeat,
                    address_mode_v: ImageAddressMode::Repeat,
                    address_mode_w: ImageAddressMode::Repeat,
                    ..ImageSamplerDescriptor::linear()
                });
            },
        )
    };
    let bark_diff = load_repeat("textures/bark_brown_02_diff.jpg");
    let bark_nor = load_repeat("textures/bark_brown_02_nor.jpg");
    let bark_arm = load_repeat("textures/bark_brown_02_arm.jpg");

    // Trunks share one bark texture but get slightly different base_color
    // tints so pines read as more reddish and oaks as cooler brown.
    // `uv_transform` tiles the bark capture 2× around the trunk and 3×
    // vertically — enough to avoid stretching on tall pines.
    let bark_uv = Affine2::from_scale(Vec2::new(2.0, 3.0));
    let trunk_oak = materials.add(StandardMaterial {
        base_color: Color::srgb(0.95, 0.88, 0.80),
        base_color_texture: Some(bark_diff.clone()),
        normal_map_texture: Some(bark_nor.clone()),
        occlusion_texture: Some(bark_arm.clone()),
        metallic_roughness_texture: Some(bark_arm.clone()),
        perceptual_roughness: 1.0,
        metallic: 0.0,
        uv_transform: bark_uv,
        ..default()
    });
    let trunk_pine = materials.add(StandardMaterial {
        base_color: Color::srgb(0.78, 0.65, 0.55),
        base_color_texture: Some(bark_diff),
        normal_map_texture: Some(bark_nor),
        occlusion_texture: Some(bark_arm.clone()),
        metallic_roughness_texture: Some(bark_arm),
        perceptual_roughness: 1.0,
        metallic: 0.0,
        uv_transform: bark_uv,
        ..default()
    });
    // Foliage uses a photoscanned leafy-grass capture. Sphere/cone UV
    // mapping chops it up into organic-looking patches, and per-material
    // base_color tints shift the hue for each species/variant.
    let leaves_diff = load_repeat("textures/leafy_grass_diff.jpg");
    let leaves_nor = load_repeat("textures/leafy_grass_nor.jpg");
    let leaves_arm = load_repeat("textures/leafy_grass_arm.jpg");
    let leaves_uv = Affine2::from_scale(Vec2::splat(2.2));
    let mut foliage_mat = |tint: Color| -> Handle<StandardMaterial> {
        materials.add(StandardMaterial {
            base_color: tint,
            base_color_texture: Some(leaves_diff.clone()),
            normal_map_texture: Some(leaves_nor.clone()),
            occlusion_texture: Some(leaves_arm.clone()),
            metallic_roughness_texture: Some(leaves_arm.clone()),
            perceptual_roughness: 1.0,
            metallic: 0.0,
            uv_transform: leaves_uv,
            ..default()
        })
    };
    // Tints control the per-variant green: the texture provides the
    // per-leaf micro-variation, the tint provides the macro tone.
    let leaves_oak = foliage_mat(Color::srgb(0.62, 0.85, 0.45));
    let leaves_oak_light = foliage_mat(Color::srgb(0.82, 0.95, 0.52));
    let leaves_oak_dark = foliage_mat(Color::srgb(0.38, 0.58, 0.28));
    let leaves_pine = foliage_mat(Color::srgb(0.28, 0.48, 0.30));
    let leaves_pine_frost = foliage_mat(Color::srgb(0.40, 0.62, 0.42));
    let leaves_bush = foliage_mat(Color::srgb(0.68, 0.90, 0.48));

    // ---- Shared meshes (unit-sized, scaled per instance via Transform) ----
    let trunk_mesh = meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(10));
    let foliage_sphere = meshes.add(Sphere::new(1.0).mesh().ico(2).unwrap());
    let cone_mesh = meshes.add(Cone::new(1.0, 1.0).mesh().resolution(14));

    // ---- Deterministic noise so the forest is the same across runs ----
    let density = Perlin::new(7421);
    let variety = Perlin::new(5039);
    let jitter = Perlin::new(2719);

    let grid_span = (OUTER_RADIUS / GRID_SPACING).ceil() as i32;
    for ix in -grid_span..=grid_span {
        for iz in -grid_span..=grid_span {
            let gx = ix as f32 * GRID_SPACING;
            let gz = iz as f32 * GRID_SPACING;

            // Jitter so trees don't line up on an obvious grid.
            let jx = jitter.get([gx as f64 * JITTER_FREQ, gz as f64 * JITTER_FREQ]) as f32
                * JITTER_AMP;
            let jz = jitter
                .get([gx as f64 * JITTER_FREQ + 42.0, gz as f64 * JITTER_FREQ + 17.0])
                as f32
                * JITTER_AMP;
            let x = gx + jx;
            let z = gz + jz;

            let dist = (x * x + z * z).sqrt();
            if dist < INNER_RADIUS || dist > OUTER_RADIUS {
                continue;
            }

            let d = density.get([x as f64 * DENSITY_FREQ, z as f64 * DENSITY_FREQ]) as f32;
            if d < DENSITY_CUTOFF {
                continue;
            }

            if in_village_exclusion(x, z) {
                continue;
            }

            // Slope check — central diff on sampled terrain.
            let h_n = terrain_height_at(x, z + 1.2);
            let h_s = terrain_height_at(x, z - 1.2);
            let h_e = terrain_height_at(x + 1.2, z);
            let h_w = terrain_height_at(x - 1.2, z);
            let slope = ((h_n - h_s).abs() + (h_e - h_w).abs()) * 0.5;
            if slope > MAX_TERRAIN_SLOPE {
                continue;
            }

            let ground = terrain_height_at(x, z);
            let var = variety.get([x as f64 * VARIETY_FREQ, z as f64 * VARIETY_FREQ]) as f32;
            let yaw = jitter.get([x as f64 + 77.0, z as f64 + 33.0]) as f32
                * std::f32::consts::TAU;
            let phase = ((x * 0.17 + z * 0.29) % std::f32::consts::TAU).abs() + var * 2.0;

            // Variety noise splits the forest into biomes: bushes in
            // low-variety spots, oaks in the middle, pines where the
            // noise trends high. Thresholds roughly give 20%/40%/40%.
            if var < -0.25 {
                spawn_bush(
                    &mut commands,
                    &foliage_sphere,
                    &leaves_bush,
                    x,
                    ground,
                    z,
                    var,
                    yaw,
                    phase,
                );
            } else if var < 0.15 {
                spawn_oak(
                    &mut commands,
                    &trunk_mesh,
                    &foliage_sphere,
                    &trunk_oak,
                    &leaves_oak,
                    &leaves_oak_light,
                    &leaves_oak_dark,
                    x,
                    ground,
                    z,
                    var,
                    yaw,
                    phase,
                );
            } else {
                spawn_pine(
                    &mut commands,
                    &trunk_mesh,
                    &cone_mesh,
                    &trunk_pine,
                    &leaves_pine,
                    &leaves_pine_frost,
                    x,
                    ground,
                    z,
                    var,
                    yaw,
                    phase,
                );
            }
        }
    }
}

fn in_village_exclusion(x: f32, z: f32) -> bool {
    if x.abs() < PATH_X_HALF && z > PATH_Z_MIN && z < PATH_Z_MAX {
        return true;
    }
    for (hx, hz) in HOUSE_CENTERS {
        let dx = x - hx;
        let dz = z - hz;
        if dx * dx + dz * dz < HOUSE_EXCLUSION_RADIUS * HOUSE_EXCLUSION_RADIUS {
            return true;
        }
    }
    let (cx, cz) = CHURCH_CENTER;
    let dx = x - cx;
    let dz = z - cz;
    if dx * dx + dz * dz < CHURCH_EXCLUSION_RADIUS * CHURCH_EXCLUSION_RADIUS {
        return true;
    }
    // Lake exclusion — no trees in the water or on the shoreline.
    let lake_dx = x - LAKE_CENTER.x;
    let lake_dz = z - LAKE_CENTER.y;
    let shore_margin: f32 = 2.5;
    if lake_dx * lake_dx + lake_dz * lake_dz
        < (LAKE_RADIUS + shore_margin) * (LAKE_RADIUS + shore_margin)
    {
        return true;
    }
    false
}

// --- Tree variants -----------------------------------------------------------

fn spawn_oak(
    commands: &mut Commands,
    trunk_mesh: &Handle<Mesh>,
    foliage_mesh: &Handle<Mesh>,
    trunk_mat: &Handle<StandardMaterial>,
    leaves_medium: &Handle<StandardMaterial>,
    leaves_light: &Handle<StandardMaterial>,
    leaves_dark: &Handle<StandardMaterial>,
    x: f32,
    ground: f32,
    z: f32,
    variety: f32,
    yaw: f32,
    phase: f32,
) {
    let scale = 1.0 + variety.abs() * 0.6;
    let trunk_h = 4.2 * scale;
    let trunk_r = 0.22 * scale;
    let base_rot = Quat::from_rotation_y(yaw);

    // Static trunk collider — placed as a separate entity so the visual
    // root's wind-sway rotation doesn't drag the physics body around.
    commands.spawn((
        Transform::from_xyz(x, ground + trunk_h * 0.5, z),
        RigidBody::Fixed,
        Collider::cylinder(trunk_h * 0.5, trunk_r),
        Name::new("OakCollider"),
    ));

    commands
        .spawn((
            Transform::from_xyz(x, ground, z).with_rotation(base_rot),
            Visibility::default(),
            WindSway {
                phase,
                amplitude: 0.025,
                base_rotation: base_rot,
            },
            Name::new("Oak"),
        ))
        .with_children(|tree| {
            tree.spawn((
                Mesh3d(trunk_mesh.clone()),
                MeshMaterial3d(trunk_mat.clone()),
                Transform::from_xyz(0.0, trunk_h * 0.5, 0.0)
                    .with_scale(Vec3::new(trunk_r, trunk_h, trunk_r)),
                Name::new("Trunk"),
            ));

            // Canopy: a cluster of 5 overlapping blobs, mixing three
            // shades of green so the silhouette reads as foliage, not
            // one smooth sphere.
            let canopy_y = trunk_h + 0.35;
            let canopy_r = 1.9 * scale;
            let blobs: [(Vec3, f32, &Handle<StandardMaterial>); 5] = [
                (Vec3::new(0.0, 0.0, 0.0), 1.0, leaves_medium),
                (
                    Vec3::new(0.9 * scale, -0.4 * scale, 0.3 * scale),
                    0.75,
                    leaves_light,
                ),
                (
                    Vec3::new(-0.75 * scale, -0.15 * scale, 0.55 * scale),
                    0.7,
                    leaves_medium,
                ),
                (
                    Vec3::new(0.2 * scale, 0.55 * scale, -0.5 * scale),
                    0.62,
                    leaves_light,
                ),
                (
                    Vec3::new(-0.35 * scale, -0.55 * scale, -0.6 * scale),
                    0.58,
                    leaves_dark,
                ),
            ];
            for (offset, size, mat) in blobs {
                tree.spawn((
                    Mesh3d(foliage_mesh.clone()),
                    MeshMaterial3d(mat.clone()),
                    Transform::from_translation(Vec3::new(0.0, canopy_y, 0.0) + offset)
                        .with_scale(Vec3::splat(size * canopy_r)),
                    Name::new("OakLeaves"),
                ));
            }
        });
}

fn spawn_pine(
    commands: &mut Commands,
    trunk_mesh: &Handle<Mesh>,
    cone_mesh: &Handle<Mesh>,
    trunk_mat: &Handle<StandardMaterial>,
    leaves_mat: &Handle<StandardMaterial>,
    leaves_frost: &Handle<StandardMaterial>,
    x: f32,
    ground: f32,
    z: f32,
    variety: f32,
    yaw: f32,
    phase: f32,
) {
    let scale = 1.0 + variety * 0.55;
    let trunk_h = 6.6 * scale;
    let trunk_r = 0.17 * scale;
    let base_rot = Quat::from_rotation_y(yaw);

    commands.spawn((
        Transform::from_xyz(x, ground + trunk_h * 0.5, z),
        RigidBody::Fixed,
        Collider::cylinder(trunk_h * 0.5, trunk_r),
        Name::new("PineCollider"),
    ));

    commands
        .spawn((
            Transform::from_xyz(x, ground, z).with_rotation(base_rot),
            Visibility::default(),
            WindSway {
                phase,
                amplitude: 0.015,
                base_rotation: base_rot,
            },
            Name::new("Pine"),
        ))
        .with_children(|tree| {
            tree.spawn((
                Mesh3d(trunk_mesh.clone()),
                MeshMaterial3d(trunk_mat.clone()),
                Transform::from_xyz(0.0, trunk_h * 0.5, 0.0)
                    .with_scale(Vec3::new(trunk_r, trunk_h, trunk_r)),
                Name::new("Trunk"),
            ));

            // Stacked conical layers for the needle foliage. Alternating
            // materials give the canopy a little depth.
            let layers: [(f32, f32, f32, &Handle<StandardMaterial>); 4] = [
                (2.1 * scale, 2.9 * scale, 1.9 * scale, leaves_mat),
                (1.55 * scale, 2.5 * scale, 3.7 * scale, leaves_frost),
                (1.05 * scale, 2.1 * scale, 5.3 * scale, leaves_mat),
                (0.5 * scale, 1.6 * scale, 6.7 * scale, leaves_frost),
            ];
            for (r, h, y, mat) in layers {
                // Cone::new(radius, height) centers the cone around its
                // lathe axis, so Transform::from_xyz puts its mid-height
                // at `y`.
                tree.spawn((
                    Mesh3d(cone_mesh.clone()),
                    MeshMaterial3d(mat.clone()),
                    Transform::from_xyz(0.0, y, 0.0).with_scale(Vec3::new(r, h, r)),
                    Name::new("PineLeaves"),
                ));
            }
        });
}

fn spawn_bush(
    commands: &mut Commands,
    foliage_mesh: &Handle<Mesh>,
    leaves_mat: &Handle<StandardMaterial>,
    x: f32,
    ground: f32,
    z: f32,
    variety: f32,
    yaw: f32,
    phase: f32,
) {
    let scale = 0.75 + variety.abs() * 0.4;
    let base_rot = Quat::from_rotation_y(yaw);

    commands
        .spawn((
            Transform::from_xyz(x, ground, z).with_rotation(base_rot),
            Visibility::default(),
            WindSway {
                phase,
                amplitude: 0.035,
                base_rotation: base_rot,
            },
            Name::new("Bush"),
        ))
        .with_children(|bush| {
            let blobs: [(Vec3, f32); 3] = [
                (Vec3::new(0.0, 0.55, 0.0), 0.68),
                (Vec3::new(0.38, 0.45, 0.22), 0.52),
                (Vec3::new(-0.32, 0.4, -0.28), 0.48),
            ];
            for (offset, size) in blobs {
                bush.spawn((
                    Mesh3d(foliage_mesh.clone()),
                    MeshMaterial3d(leaves_mat.clone()),
                    Transform::from_translation(offset * scale)
                        .with_scale(Vec3::splat(size * scale)),
                    // Bushes are small — their shadow maps would cost more
                    // than they add visually.
                    NotShadowCaster,
                    Name::new("BushLeaves"),
                ));
            }
        });
}

// --- Wind --------------------------------------------------------------------

fn animate_wind(
    time: Res<Time>,
    player_q: Query<&Transform, (With<Player>, Without<WindSway>)>,
    mut q: Query<(&WindSway, &mut Transform), Without<Player>>,
) {
    // Trees farther than this from the player are culled from the wind
    // animation — the sway is imperceptible at distance and propagating
    // Transform changes through Bevy's hierarchy isn't free.
    const MAX_ANIMATE_DIST_SQ: f32 = 80.0 * 80.0;
    let Ok(player_tf) = player_q.single() else {
        return;
    };
    let player_pos = player_tf.translation;
    let t = time.elapsed_secs();
    for (sway, mut tf) in &mut q {
        if tf.translation.distance_squared(player_pos) > MAX_ANIMATE_DIST_SQ {
            continue;
        }
        // Two incommensurate sines give each tree an irregular sway.
        let tilt_x = (t * 0.7 + sway.phase).sin() * sway.amplitude;
        let tilt_z = (t * 0.9 + sway.phase * 1.3).cos() * sway.amplitude;
        tf.rotation = sway.base_rotation
            * Quat::from_rotation_x(tilt_x)
            * Quat::from_rotation_z(tilt_z);
    }
}
