//! Small village of procedural houses built on stone socles, each with a
//! door the player can open with E. Some houses have a second floor reached
//! by an internal staircase.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_rapier3d::prelude::*;
use noise::{NoiseFn, Perlin};

use crate::player::Player;
use crate::sword::spawn_sword_pickup;
use crate::world::terrain_height_at;

// --- Shared constants --------------------------------------------------------

const WALL_THICKNESS: f32 = 0.12;

/// Stone socle/plinth that every house sits on. Extends beyond the walls,
/// and is buried well below terrain so Perlin bumps never peek through.
const SOCLE_EXT: f32 = 1.4;
const SOCLE_HEIGHT: f32 = 0.55;
const SOCLE_DEPTH_BELOW: f32 = 1.2;

/// Door dimensions are shared across all houses (the village is lo-fi).
const DOOR_W: f32 = 1.1;
const DOOR_H: f32 = 2.2;
const DOOR_THICKNESS: f32 = 0.10;
const DOOR_OPEN_ANGLE: f32 = std::f32::consts::FRAC_PI_2 * 0.95;
const DOOR_OPEN_RATE: f32 = 7.0;

/// Stair steps leading up to each socle in front of its door.
const SOCLE_STEP_DEPTH: f32 = 0.5;
const SOCLE_STEP_WIDTH: f32 = 2.3;
const SOCLE_STEP_COUNT: usize = 2;

/// Internal staircase (only used on 2-story houses).
const STAIR_WIDTH: f32 = 1.0;
const STAIR_STEP_DEPTH: f32 = 0.4;
const STAIR_STEP_RISE: f32 = 0.3;

// --- HouseConfig -------------------------------------------------------------

#[derive(Clone, Copy)]
struct HouseConfig {
    /// Horizontal world position (X, Z) of the house's center.
    xz: Vec2,
    /// Rotation around Y (radians). 0 = door faces world -X.
    rotation: f32,
    /// Footprint half-extents in local frame: (half_x, half_z).
    half_size: Vec2,
    /// Total wall height. For 2-story houses this is roughly 2 × single-story.
    wall_height: f32,
    /// 1 or 2.
    stories: u8,
    has_fireplace: bool,
}

/// Cached material handles shared across every building in the village —
/// avoids re-adding the same StandardMaterial dozens of times.
struct VillageMaterials {
    wall: Handle<StandardMaterial>,
    roof: Handle<StandardMaterial>,
    wood: Handle<StandardMaterial>,
    wood_floor: Handle<StandardMaterial>,
    handle: Handle<StandardMaterial>,
    stone: Handle<StandardMaterial>,
    ember: Handle<StandardMaterial>,
    flame: Handle<StandardMaterial>,
    /// Transparent pane for windows. Colored stained glass is built
    /// on-the-fly inside `spawn_church`.
    glass_clear: Handle<StandardMaterial>,
}

/// A house-local frame: world origin (socle top) and rotation (around Y).
/// Every wall / floor / door in a house is laid out in the local frame then
/// rotated+translated into world coords through this.
#[derive(Clone, Copy)]
struct Place {
    origin: Vec3,
    rotation: Quat,
}

impl Place {
    fn world(&self, local: Vec3) -> Vec3 {
        self.origin + self.rotation * local
    }
}

// --- Components --------------------------------------------------------------

#[derive(Component)]
pub struct Interactable {
    pub range: f32,
}

#[derive(Component)]
pub struct Door {
    pub open: bool,
    pub progress: f32,
    /// Rotation the door's Transform takes when fully closed — i.e. the
    /// host house's rotation around Y. Animation multiplies a local swing
    /// rotation on top of this.
    pub base_rotation: Quat,
}

#[derive(Component)]
struct FireFlame;

#[derive(Component)]
struct FireLight;

#[derive(Component)]
struct DoorLeaf;

// --- Plugin ------------------------------------------------------------------

pub struct HousePlugin;

impl Plugin for HousePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_village)
            .add_systems(Update, (interact_action, animate_door, animate_fire));
    }
}

// --- Village layout ----------------------------------------------------------

fn spawn_village(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    asset_server: Res<AssetServer>,
) {
    let mats = build_materials(&mut materials, &mut images, &asset_server);

    // Photoscanned PBR cobblestone from Poly Haven (CC0). Albedo + normal
    // + ARM (R=AO, G=roughness, B=metallic) used together give genuine
    // depth on the path surface. All three are loaded with the Repeat
    // sampler configured by `load_repeat_texture`.
    let path_diff = load_repeat_texture(&asset_server, "textures/cobblestone_diff.jpg");
    let path_nor = load_repeat_texture(&asset_server, "textures/cobblestone_nor.jpg");
    let path_arm = load_repeat_texture(&asset_server, "textures/cobblestone_arm.jpg");

    const PATH_TILE_SIZE: f32 = 2.5; // meters of ground covered by one texture tile

    // Path extends further south than the last house so it reaches the
    // foot of the church's front stairs.
    let path_z_start = -25.5;
    let path_z_end = 4.0;
    let path_center_z = (path_z_start + path_z_end) * 0.5;
    let path_length = path_z_end - path_z_start;
    let path_width = 4.0;
    let path_ground = terrain_height_at(0.0, path_center_z);
    let path_top = path_ground + 0.08;
    let path_bottom = path_ground - 0.25;
    let path_half_y = (path_top - path_bottom) * 0.5;
    let path_center_y = (path_top + path_bottom) * 0.5;
    let path_mat = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(path_diff),
        normal_map_texture: Some(path_nor.clone()),
        // The ARM map's R is AO, G is roughness, B is metallic — the R
        // channel drops naturally into `occlusion_texture`, and G/B are
        // exactly what `metallic_roughness_texture` samples.
        occlusion_texture: Some(path_arm.clone()),
        metallic_roughness_texture: Some(path_arm),
        // `perceptual_roughness` and `metallic` are multiplied with the
        // texture values. Keeping them at 1 / 0 means the texture drives
        // them entirely.
        perceptual_roughness: 1.0,
        metallic: 0.0,
        uv_transform: Affine2::from_scale(Vec2::new(
            path_width / PATH_TILE_SIZE,
            path_length / PATH_TILE_SIZE,
        )),
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(path_width, path_half_y * 2.0, path_length))),
        MeshMaterial3d(path_mat),
        Transform::from_xyz(0.0, path_center_y, path_center_z),
        RigidBody::Fixed,
        Collider::cuboid(path_width * 0.5, path_half_y, path_length * 0.5),
        Name::new("Path"),
    ));

    // Houses: xz, rotation, half_size, wall_height, stories, has_fireplace.
    // West-side houses (negative X) face +X (rotation = PI).
    // East-side houses (positive X) face -X (rotation = 0).
    const PI: f32 = std::f32::consts::PI;
    let configs = [
        HouseConfig {
            xz: Vec2::new(-9.0, -3.0),
            rotation: PI,
            half_size: Vec2::new(2.5, 2.5),
            wall_height: 2.8,
            stories: 1,
            has_fireplace: true,
        },
        HouseConfig {
            xz: Vec2::new(10.0, -7.0),
            rotation: 0.0,
            half_size: Vec2::new(3.0, 3.0),
            wall_height: 3.0,
            stories: 1,
            has_fireplace: true,
        },
        HouseConfig {
            xz: Vec2::new(-11.5, -15.0),
            rotation: PI,
            half_size: Vec2::new(3.5, 3.5),
            wall_height: 5.8,
            stories: 2,
            has_fireplace: true,
        },
        HouseConfig {
            xz: Vec2::new(10.5, -17.0),
            rotation: 0.0,
            half_size: Vec2::new(3.2, 3.2),
            wall_height: 3.1,
            stories: 1,
            has_fireplace: true,
        },
        HouseConfig {
            xz: Vec2::new(-8.5, -25.0),
            rotation: PI,
            half_size: Vec2::new(2.4, 2.4),
            wall_height: 2.7,
            stories: 1,
            has_fireplace: false,
        },
        HouseConfig {
            xz: Vec2::new(10.0, -26.5),
            rotation: 0.0,
            half_size: Vec2::new(3.3, 2.5),
            wall_height: 2.9,
            stories: 1,
            has_fireplace: false,
        },
    ];

    for config in configs {
        spawn_house(&mut commands, &mut meshes, &mats, &config);
    }

    // Church at the south end of the path (just inside the flat clearing
    // so its socle never pokes into the mountain transition).
    spawn_church(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mats,
        Vec2::new(0.0, -34.0),
        std::f32::consts::FRAC_PI_2,
    );

    // Sword pickup: lives on the mezzanine of the 2-story house (configs
    // index 2). Local (+X side, above the mezzanine slab).
    let sword_config = &configs[2];
    let ground = terrain_height_at(sword_config.xz.x, sword_config.xz.y);
    let floor_top = ground + SOCLE_HEIGHT;
    let mezz_top = floor_top + sword_config.wall_height * 0.5;
    let rotation = Quat::from_rotation_y(sword_config.rotation);
    // Local offset: 1.6m toward the back half (+X local) on the mezzanine,
    // sword grip ~0.2m above floor so the blade stands proud.
    let local_offset = Vec3::new(1.6, 0.1, 0.0);
    let world_pos = Vec3::new(sword_config.xz.x, mezz_top, sword_config.xz.y)
        + rotation * local_offset;
    let sword_tf = Transform::from_translation(world_pos).with_rotation(rotation);
    spawn_sword_pickup(&mut commands, &mut meshes, &mut materials, sword_tf);
}

fn build_materials(
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    asset_server: &AssetServer,
) -> VillageMaterials {
    let brick_diff = load_repeat_texture(asset_server, "textures/red_brick_03_diff.jpg");
    let brick_nor = load_repeat_texture(asset_server, "textures/red_brick_03_nor.jpg");
    let brick_arm = load_repeat_texture(asset_server, "textures/red_brick_03_arm.jpg");

    let stone_diff = load_repeat_texture(asset_server, "textures/medieval_blocks_02_diff.jpg");
    let stone_nor = load_repeat_texture(asset_server, "textures/medieval_blocks_02_nor.jpg");
    let stone_arm = load_repeat_texture(asset_server, "textures/medieval_blocks_02_arm.jpg");

    let wood_diff = load_repeat_texture(asset_server, "textures/wood_floor_deck_diff.jpg");
    let wood_nor = load_repeat_texture(asset_server, "textures/wood_floor_deck_nor.jpg");
    let wood_arm = load_repeat_texture(asset_server, "textures/wood_floor_deck_arm.jpg");

    // Procedural warm-tone fire texture (tileable 4D Perlin) used as the
    // base_color_texture on the flame meshes. Gives each flame visible
    // internal patches of yellow/orange/red on top of the emissive glow.
    let fire_texture = images.add(build_fire_texture());

    VillageMaterials {
        wall: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.85, 0.78),
            base_color_texture: Some(brick_diff),
            normal_map_texture: Some(brick_nor),
            occlusion_texture: Some(brick_arm.clone()),
            metallic_roughness_texture: Some(brick_arm),
            perceptual_roughness: 1.0,
            metallic: 0.0,
            // ~2.2 tiles per face for typical walls — bricks around
            // 15-20cm each, readable at player range without looking
            // pebble-sized.
            uv_transform: Affine2::from_scale(Vec2::splat(2.2)),
            ..default()
        }),
        roof: materials.add(StandardMaterial {
            base_color: Color::srgb(0.32, 0.22, 0.18),
            perceptual_roughness: 0.80,
            ..default()
        }),
        wood: materials.add(StandardMaterial {
            base_color: Color::srgb(0.46, 0.28, 0.15),
            perceptual_roughness: 0.72,
            ..default()
        }),
        wood_floor: materials.add(StandardMaterial {
            base_color: Color::srgb(0.90, 0.82, 0.68),
            base_color_texture: Some(wood_diff),
            normal_map_texture: Some(wood_nor),
            occlusion_texture: Some(wood_arm.clone()),
            metallic_roughness_texture: Some(wood_arm),
            perceptual_roughness: 1.0,
            metallic: 0.0,
            uv_transform: Affine2::from_scale(Vec2::splat(3.0)),
            ..default()
        }),
        handle: materials.add(StandardMaterial {
            base_color: Color::srgb(0.70, 0.55, 0.25),
            perceptual_roughness: 0.35,
            metallic: 0.8,
            ..default()
        }),
        stone: materials.add(StandardMaterial {
            base_color: Color::srgb(0.92, 0.90, 0.86),
            base_color_texture: Some(stone_diff),
            normal_map_texture: Some(stone_nor),
            occlusion_texture: Some(stone_arm.clone()),
            metallic_roughness_texture: Some(stone_arm),
            perceptual_roughness: 1.0,
            metallic: 0.0,
            // Socle is large (~11m) so we pack more tiles; fireplace
            // blocks are small but the extra tiles just mean finer stone,
            // still fine.
            uv_transform: Affine2::from_scale(Vec2::splat(4.5)),
            ..default()
        }),
        ember: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.55, 0.20),
            base_color_texture: Some(fire_texture.clone()),
            emissive: LinearRgba::new(25.0, 9.0, 1.5, 1.0),
            emissive_texture: Some(fire_texture.clone()),
            perceptual_roughness: 0.6,
            ..default()
        }),
        flame: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.85, 0.45),
            base_color_texture: Some(fire_texture.clone()),
            emissive: LinearRgba::new(40.0, 25.0, 6.0, 1.0),
            emissive_texture: Some(fire_texture),
            perceptual_roughness: 0.5,
            ..default()
        }),
        glass_clear: materials.add(StandardMaterial {
            base_color: Color::srgba(0.72, 0.82, 0.98, 0.28),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.10,
            reflectance: 0.4,
            double_sided: true,
            cull_mode: None,
            ..default()
        }),
    }
}

/// Procedural tileable fire texture: warm gradient driven by two octaves
/// of 4D Perlin noise (sampled on a torus so the image wraps with no
/// seam). Used as both `base_color_texture` and `emissive_texture` on the
/// flame blobs so the spatial noise modulates both the diffuse and the
/// bloom-pushing emissive pass.
fn build_fire_texture() -> Image {
    const SIZE: u32 = 128;
    let mut data = vec![0u8; (SIZE * SIZE * 4) as usize];
    let big = Perlin::new(31);
    let fine = Perlin::new(71);
    let tau = std::f64::consts::TAU;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let u = x as f64 / SIZE as f64;
            let v = y as f64 / SIZE as f64;
            let (su, cu) = (u * tau).sin_cos();
            let (sv, cv) = (v * tau).sin_cos();

            let n_big = big.get([cu * 2.2, su * 2.2, cv * 2.2, sv * 2.2]) as f32;
            let n_fine = fine.get([cu * 7.0, su * 7.0, cv * 7.0, sv * 7.0]) as f32;
            // Intensity ∈ roughly [0, 1] with hot cores where the two
            // noises align.
            let intensity = ((n_big * 0.6 + n_fine * 0.5) * 0.7 + 0.5).clamp(0.0, 1.0);

            // Three-stop warm gradient: deep red → orange → yellow-white.
            let (r, g, b) = if intensity < 0.35 {
                let t = intensity / 0.35;
                (0.30 + t * 0.70, 0.02 + t * 0.28, 0.00 + t * 0.05)
            } else if intensity < 0.72 {
                let t = (intensity - 0.35) / 0.37;
                (1.00, 0.30 + t * 0.50, 0.05 + t * 0.35)
            } else {
                let t = (intensity - 0.72) / 0.28;
                (1.00, 0.80 + t * 0.18, 0.40 + t * 0.45)
            };

            let idx = ((y * SIZE + x) * 4) as usize;
            data[idx] = (r.clamp(0.0, 1.0) * 255.0) as u8;
            data[idx + 1] = (g.clamp(0.0, 1.0) * 255.0) as u8;
            data[idx + 2] = (b.clamp(0.0, 1.0) * 255.0) as u8;
            data[idx + 3] = 255;
        }
    }

    let mut image = Image::new(
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    image
}

/// Load a texture with a Repeat sampler so tiled materials work.
fn load_repeat_texture(asset_server: &AssetServer, path: &'static str) -> Handle<Image> {
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
}

// --- Single house ------------------------------------------------------------

fn spawn_house(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &VillageMaterials,
    config: &HouseConfig,
) {
    let ground = terrain_height_at(config.xz.x, config.xz.y);
    let floor_top = ground + SOCLE_HEIGHT;
    let place = Place {
        origin: Vec3::new(config.xz.x, floor_top, config.xz.y),
        rotation: Quat::from_rotation_y(config.rotation),
    };
    let half_x = config.half_size.x;
    let half_z = config.half_size.y;
    let wall_h = config.wall_height;

    // ---- Socle (buried stone plinth) ----
    let socle_bottom = ground - SOCLE_DEPTH_BELOW;
    let socle_local_center_y = (socle_bottom - floor_top) * 0.5;
    let socle_half_y = (floor_top - socle_bottom) * 0.5;
    spawn_local_box(
        commands,
        meshes,
        &mats.stone,
        &place,
        Vec3::new(0.0, socle_local_center_y, 0.0),
        Vec3::new(half_x + SOCLE_EXT, socle_half_y, half_z + SOCLE_EXT),
        "Socle",
    );

    // ---- Stairs leading up to the socle, in front of the door ----
    let socle_front_x = -half_x - SOCLE_EXT;
    let step_rise = SOCLE_HEIGHT / (SOCLE_STEP_COUNT as f32 + 1.0);
    for i in 0..SOCLE_STEP_COUNT {
        let top_local = (-SOCLE_HEIGHT) + step_rise * (i as f32 + 1.0);
        let outward = (SOCLE_STEP_COUNT - i) as f32;
        let center_x = socle_front_x - (outward - 0.5) * SOCLE_STEP_DEPTH;
        let bottom_local = socle_local_center_y - socle_half_y;
        let half_y = (top_local - bottom_local) * 0.5;
        let center_y = (top_local + bottom_local) * 0.5;
        spawn_local_box(
            commands,
            meshes,
            &mats.stone,
            &place,
            Vec3::new(center_x, center_y, 0.0),
            Vec3::new(SOCLE_STEP_DEPTH * 0.5, half_y, SOCLE_STEP_WIDTH * 0.5),
            "SocleStep",
        );
    }

    // ---- Walls ----
    let wall_half_h = wall_h * 0.5;
    let wall_y = wall_half_h; // floor_top is local y=0

    // Back wall (+X side, opposite the door)
    spawn_local_box(
        commands,
        meshes,
        &mats.wall,
        &place,
        Vec3::new(half_x, wall_y, 0.0),
        Vec3::new(WALL_THICKNESS * 0.5, wall_half_h, half_z),
        "WallBack",
    );
    // Side walls (±Z) — each with a window cut out at waist height; the
    // 2-story house gets a second window at mezzanine level.
    let mut windows: Vec<(f32, f32, f32, Handle<StandardMaterial>)> =
        vec![(1.4, 1.2, 1.0, mats.glass_clear.clone())];
    if config.stories >= 2 {
        windows.push((4.2, 1.2, 1.0, mats.glass_clear.clone()));
    }
    for sign in [1.0_f32, -1.0] {
        spawn_walled_side(
            commands,
            meshes,
            &mats.wall,
            &place,
            sign * half_z,
            Vec2::new(half_x, wall_h),
            WALL_THICKNESS,
            &windows,
        );
    }

    // Front wall (-X side): split around the door opening.
    let front_x = -half_x;
    let flank_width = (half_z * 2.0 - DOOR_W) * 0.5;
    let flank_center_offset = DOOR_W * 0.5 + flank_width * 0.5;
    for sign in [1.0_f32, -1.0] {
        spawn_local_box(
            commands,
            meshes,
            &mats.wall,
            &place,
            Vec3::new(front_x, wall_y, sign * flank_center_offset),
            Vec3::new(WALL_THICKNESS * 0.5, wall_half_h, flank_width * 0.5),
            "WallFrontFlank",
        );
    }
    // Lintel above the door, plus any second-floor wall segment that
    // would have been above the door on the front.
    let lintel_h = wall_h - DOOR_H;
    if lintel_h > 0.01 {
        spawn_local_box(
            commands,
            meshes,
            &mats.wall,
            &place,
            Vec3::new(front_x, DOOR_H + lintel_h * 0.5, 0.0),
            Vec3::new(WALL_THICKNESS * 0.5, lintel_h * 0.5, DOOR_W * 0.5),
            "WallFrontLintel",
        );
    }

    // ---- Roof ----
    let roof_overhang = 0.25;
    spawn_local_box(
        commands,
        meshes,
        &mats.roof,
        &place,
        Vec3::new(0.0, wall_h + 0.05, 0.0),
        Vec3::new(half_x + roof_overhang, 0.05, half_z + roof_overhang),
        "Roof",
    );

    // ---- Interior parquet floor (laid on top of the stone socle) ----
    // Sits flush with the wall interiors (inset by half the wall thickness
    // so the wood doesn't clip through them).
    let interior_half_x = half_x - WALL_THICKNESS * 0.5;
    let interior_half_z = half_z - WALL_THICKNESS * 0.5;
    let parquet_thickness = 0.03;
    spawn_local_box(
        commands,
        meshes,
        &mats.wood_floor,
        &place,
        Vec3::new(0.0, parquet_thickness * 0.5, 0.0),
        Vec3::new(interior_half_x, parquet_thickness * 0.5, interior_half_z),
        "InteriorParquet",
    );

    // ---- Door ----
    spawn_door(commands, meshes, mats, &place, config);

    // ---- Fireplace ----
    if config.has_fireplace {
        spawn_fireplace(commands, meshes, mats, &place, half_x, wall_h);
    }

    // ---- 2-story additions: mezzanine floor + staircase ----
    if config.stories >= 2 {
        spawn_mezzanine_and_stairs(commands, meshes, mats, &place, half_x, half_z, wall_h);
    }
}


fn spawn_door(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &VillageMaterials,
    place: &Place,
    config: &HouseConfig,
) {
    let front_x = -config.half_size.x;
    // Hinge sits on the +Z edge of the door opening (local). A negative
    // rotation of the hinge entity swings the leaf outward (-X, away from
    // the house) — see DOOR_OPEN_ANGLE sign + the leaf offset below.
    let hinge_local = Vec3::new(front_x, 0.0, DOOR_W * 0.5);
    let hinge_world = place.world(hinge_local);

    let door_mesh = meshes.add(Cuboid::new(DOOR_THICKNESS, DOOR_H, DOOR_W));
    let handle_mesh = meshes.add(Sphere::new(0.045).mesh().ico(2).unwrap());

    commands
        .spawn((
            Transform {
                translation: hinge_world,
                rotation: place.rotation,
                ..default()
            },
            Visibility::default(),
            RigidBody::KinematicPositionBased,
            Door {
                open: false,
                progress: 0.0,
                base_rotation: place.rotation,
            },
            Interactable { range: 2.8 },
            Name::new("Door"),
        ))
        .with_children(|hinge| {
            hinge
                .spawn((
                    Mesh3d(door_mesh),
                    MeshMaterial3d(mats.wood.clone()),
                    Transform::from_xyz(0.0, DOOR_H * 0.5, -DOOR_W * 0.5),
                    Collider::cuboid(DOOR_THICKNESS * 0.5, DOOR_H * 0.5, DOOR_W * 0.5),
                    DoorLeaf,
                    Name::new("DoorLeaf"),
                ))
                .with_children(|leaf| {
                    leaf.spawn((
                        Mesh3d(handle_mesh),
                        MeshMaterial3d(mats.handle.clone()),
                        Transform::from_xyz(-(DOOR_THICKNESS * 0.5 + 0.02), -0.1, -DOOR_W * 0.35),
                        Name::new("DoorHandle"),
                    ));
                });
        });
}

fn spawn_fireplace(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &VillageMaterials,
    place: &Place,
    half_x: f32,
    wall_h: f32,
) {
    let hearth_depth = 0.7;
    let hearth_width = 1.4;
    let hearth_height = 0.4;
    // Hearth centered in X against the back wall (+X side).
    let hearth_local_x = half_x - hearth_depth * 0.5 - WALL_THICKNESS * 0.5;
    let hearth_local_z = 0.0;

    // Hearth base (stone block, solid collider — player can't walk into fire).
    spawn_local_box(
        commands,
        meshes,
        &mats.stone,
        place,
        Vec3::new(hearth_local_x, hearth_height * 0.5, hearth_local_z),
        Vec3::new(hearth_depth * 0.5, hearth_height * 0.5, hearth_width * 0.5),
        "Hearth",
    );

    // Mantle / hood above the hearth.
    spawn_local_deco(
        commands,
        meshes,
        &mats.stone,
        place,
        Vec3::new(hearth_local_x, 1.6, hearth_local_z),
        Vec3::new(hearth_depth * 0.5, 0.08, hearth_width * 0.5 + 0.1),
        "Mantle",
    );

    // Side cheeks flanking the fire.
    for sign in [1.0_f32, -1.0] {
        spawn_local_deco(
            commands,
            meshes,
            &mats.stone,
            place,
            Vec3::new(
                hearth_local_x,
                0.8,
                hearth_local_z + sign * (hearth_width * 0.5 + 0.07),
            ),
            Vec3::new(hearth_depth * 0.5, 0.8, 0.07),
            "HearthCheek",
        );
    }

    // Chimney protruding through the roof.
    spawn_local_deco(
        commands,
        meshes,
        &mats.stone,
        place,
        Vec3::new(hearth_local_x, wall_h + 0.9, hearth_local_z),
        Vec3::new(0.4, 1.0, 0.55),
        "Chimney",
    );

    // Fire — emissive blobs + flickering point light.
    let fire_local = Vec3::new(hearth_local_x, hearth_height + 0.2, hearth_local_z);
    let flame_shapes: [(Vec3, f32, bool); 5] = [
        (Vec3::new(0.0, 0.05, 0.0), 0.22, true),
        (Vec3::new(-0.15, -0.02, 0.10), 0.15, false),
        (Vec3::new(0.12, -0.04, -0.12), 0.13, false),
        (Vec3::new(0.04, 0.22, 0.02), 0.11, true),
        (Vec3::new(-0.06, 0.15, -0.05), 0.09, true),
    ];
    for (offset, radius, hot) in flame_shapes {
        let local = fire_local + offset;
        commands.spawn((
            Mesh3d(meshes.add(Sphere::new(radius).mesh().ico(3).unwrap())),
            MeshMaterial3d(if hot { mats.flame.clone() } else { mats.ember.clone() }),
            Transform {
                translation: place.world(local),
                rotation: place.rotation,
                ..default()
            },
            FireFlame,
            Name::new("Flame"),
        ));
    }

    commands.spawn((
        PointLight {
            color: Color::srgb(1.0, 0.55, 0.22),
            intensity: 130_000.0,
            range: 18.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_translation(place.world(fire_local + Vec3::Y * 0.15)),
        FireLight,
        Name::new("FireLight"),
    ));
}

fn spawn_mezzanine_and_stairs(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &VillageMaterials,
    place: &Place,
    half_x: f32,
    half_z: f32,
    wall_h: f32,
) {
    // Second floor lands at half the wall height (so both floors fit with
    // a reasonable ceiling).
    let floor_y = wall_h * 0.5;
    let step_count = (floor_y / STAIR_STEP_RISE).ceil() as u32;
    let rise = floor_y / step_count as f32;

    // Staircase runs along +X, against the +Z wall, starting near the
    // front-right corner (so it doesn't block the entrance).
    let stair_center_z = half_z - STAIR_WIDTH * 0.5 - WALL_THICKNESS * 0.5;
    let stair_x_start = -half_x + 0.7; // 0.7m gap from the front wall
    let stairs_end_x = stair_x_start + step_count as f32 * STAIR_STEP_DEPTH;

    for i in 0..step_count {
        // Each step is a solid wooden block from the house floor (y=0) up
        // to its top — the staircase becomes a sloped parquet wedge.
        let step_top = (i + 1) as f32 * rise;
        let center_x = stair_x_start + (i as f32 + 0.5) * STAIR_STEP_DEPTH;
        spawn_local_box(
            commands,
            meshes,
            &mats.wood_floor,
            place,
            Vec3::new(center_x, step_top * 0.5, stair_center_z),
            Vec3::new(
                STAIR_STEP_DEPTH * 0.5,
                step_top * 0.5,
                STAIR_WIDTH * 0.5,
            ),
            "Stair",
        );
    }

    // Mezzanine slab: covers the back portion of the house, starting at
    // the end of the staircase run so the stairs stay open.
    let mezz_x_start = stairs_end_x - 0.05;
    let mezz_x_end = half_x - WALL_THICKNESS * 0.5;
    let mezz_z_start = -half_z + WALL_THICKNESS * 0.5;
    let mezz_z_end = half_z - WALL_THICKNESS * 0.5;
    let mezz_center_x = (mezz_x_start + mezz_x_end) * 0.5;
    let mezz_center_z = (mezz_z_start + mezz_z_end) * 0.5;
    let mezz_half_x = (mezz_x_end - mezz_x_start) * 0.5;
    let mezz_half_z = (mezz_z_end - mezz_z_start) * 0.5;
    let slab_half_y = 0.05;
    // Top of the slab is at floor_y — flush with the last stair step.
    spawn_local_box(
        commands,
        meshes,
        &mats.wood_floor,
        place,
        Vec3::new(mezz_center_x, floor_y - slab_half_y, mezz_center_z),
        Vec3::new(mezz_half_x, slab_half_y, mezz_half_z),
        "Mezzanine",
    );

    // Simple railing along the mezzanine edge (where it meets the open
    // space above the ground floor) — a thin horizontal bar.
    let rail_y = floor_y + 0.55;
    spawn_local_deco(
        commands,
        meshes,
        &mats.wood,
        place,
        Vec3::new(mezz_x_start, rail_y, mezz_center_z),
        Vec3::new(0.04, 0.04, mezz_half_z),
        "MezzanineRail",
    );
}

/// Build the village church — a tall rectangular nave with stained-glass
/// side windows, a central spire above the entrance, and one colored
/// spot-light per window that paints the floor with the tint of that
/// pane.
fn spawn_church(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    mats: &VillageMaterials,
    xz: Vec2,
    rotation: f32,
) {
    let half_x: f32 = 6.0; // nave length (post-rotation becomes world Z)
    let half_z: f32 = 3.5; // nave half-width
    let wall_h: f32 = 7.5; // taller than houses

    let ground = terrain_height_at(xz.x, xz.y);
    let floor_top = ground + SOCLE_HEIGHT;
    let place = Place {
        origin: Vec3::new(xz.x, floor_top, xz.y),
        rotation: Quat::from_rotation_y(rotation),
    };

    // Socle (extended, matches the nave footprint + SOCLE_EXT).
    let socle_bottom = ground - SOCLE_DEPTH_BELOW;
    let socle_local_center_y = (socle_bottom - floor_top) * 0.5;
    let socle_half_y = (floor_top - socle_bottom) * 0.5;
    spawn_local_box(
        commands,
        meshes,
        &mats.stone,
        &place,
        Vec3::new(0.0, socle_local_center_y, 0.0),
        Vec3::new(half_x + SOCLE_EXT, socle_half_y, half_z + SOCLE_EXT),
        "ChurchSocle",
    );

    // Stairs leading up to the front door.
    let socle_front_x = -half_x - SOCLE_EXT;
    let step_rise = SOCLE_HEIGHT / (SOCLE_STEP_COUNT as f32 + 1.0);
    for i in 0..SOCLE_STEP_COUNT {
        let top_local = (-SOCLE_HEIGHT) + step_rise * (i as f32 + 1.0);
        let outward = (SOCLE_STEP_COUNT - i) as f32;
        let center_x = socle_front_x - (outward - 0.5) * SOCLE_STEP_DEPTH;
        let bottom_local = socle_local_center_y - socle_half_y;
        let half_y = (top_local - bottom_local) * 0.5;
        let center_y = (top_local + bottom_local) * 0.5;
        spawn_local_box(
            commands,
            meshes,
            &mats.stone,
            &place,
            Vec3::new(center_x, center_y, 0.0),
            Vec3::new(SOCLE_STEP_DEPTH * 0.5, half_y, SOCLE_STEP_WIDTH * 0.5),
            "ChurchStep",
        );
    }

    // Interior parquet floor.
    let parquet_thickness = 0.03;
    spawn_local_box(
        commands,
        meshes,
        &mats.wood_floor,
        &place,
        Vec3::new(0.0, parquet_thickness * 0.5, 0.0),
        Vec3::new(
            half_x - WALL_THICKNESS * 0.5,
            parquet_thickness * 0.5,
            half_z - WALL_THICKNESS * 0.5,
        ),
        "ChurchParquet",
    );

    // Back wall (+X), solid.
    spawn_local_box(
        commands,
        meshes,
        &mats.wall,
        &place,
        Vec3::new(half_x, wall_h * 0.5, 0.0),
        Vec3::new(WALL_THICKNESS * 0.5, wall_h * 0.5, half_z),
        "ChurchBackWall",
    );

    // Front wall: flanks + lintel around the door (same as houses).
    let front_x = -half_x;
    let flank_width = (half_z * 2.0 - DOOR_W) * 0.5;
    let flank_center_offset = DOOR_W * 0.5 + flank_width * 0.5;
    for sign in [1.0_f32, -1.0] {
        spawn_local_box(
            commands,
            meshes,
            &mats.wall,
            &place,
            Vec3::new(front_x, wall_h * 0.5, sign * flank_center_offset),
            Vec3::new(WALL_THICKNESS * 0.5, wall_h * 0.5, flank_width * 0.5),
            "ChurchFrontFlank",
        );
    }
    let lintel_h = wall_h - DOOR_H;
    spawn_local_box(
        commands,
        meshes,
        &mats.wall,
        &place,
        Vec3::new(front_x, DOOR_H + lintel_h * 0.5, 0.0),
        Vec3::new(WALL_THICKNESS * 0.5, lintel_h * 0.5, DOOR_W * 0.5),
        "ChurchFrontLintel",
    );

    // Side walls (±Z). Five stained-glass windows per side, colors
    // cycling so both walls look different from each other and from
    // themselves end-to-end.
    let stained_colors = [
        Color::srgb(0.90, 0.12, 0.18), // red
        Color::srgb(0.15, 0.30, 0.95), // blue
        Color::srgb(0.20, 0.78, 0.25), // green
        Color::srgb(0.95, 0.82, 0.15), // gold
        Color::srgb(0.65, 0.20, 0.85), // purple
    ];
    let stained_mats: Vec<Handle<StandardMaterial>> = stained_colors
        .iter()
        .map(|c| {
            let c: &Color = c;
            let lr: LinearRgba = (*c).into();
            materials.add(StandardMaterial {
                base_color: c.with_alpha(0.65),
                alpha_mode: AlphaMode::Blend,
                // Self-glow so the pane reads as "lit up" even in shadow.
                emissive: LinearRgba::new(lr.red * 1.6, lr.green * 1.6, lr.blue * 1.6, 1.0),
                perceptual_roughness: 0.12,
                reflectance: 0.4,
                double_sided: true,
                cull_mode: None,
                ..default()
            })
        })
        .collect();

    let window_cy = 4.0;
    let window_size = Vec2::new(1.3, 3.2);
    // Five windows along the nave, evenly spaced.
    let window_xs: Vec<f32> = (0..5)
        .map(|i| -half_x + 1.5 + (i as f32) * ((half_x * 2.0 - 3.0) / 4.0))
        .collect();

    for sign in [1.0_f32, -1.0] {
        spawn_church_side_wall(
            commands,
            meshes,
            &mats.wall,
            &place,
            sign * half_z,
            half_x,
            wall_h,
            WALL_THICKNESS,
            &window_xs,
            window_size,
            window_cy,
            &stained_mats,
        );
        // Colored spot-light inside just behind each pane, aimed down
        // toward the floor — fakes "sunlight filtered by stained glass"
        // hitting the nave.
        for (i, wx) in window_xs.iter().enumerate() {
            let color = stained_colors[i % stained_colors.len()];
            // Spot sits just inside the window (back off the wall) and
            // points down-and-inward toward the middle of the church.
            let local_pos =
                Vec3::new(*wx, window_cy, sign * (half_z - WALL_THICKNESS * 0.5 - 0.05));
            let inward = Vec3::new(0.0, -1.0, -sign * 0.4).normalize();
            let world_pos = place.world(local_pos);
            let world_aim = place.world(local_pos + inward);
            commands.spawn((
                SpotLight {
                    color,
                    intensity: 45_000.0,
                    range: 14.0,
                    outer_angle: 0.55,
                    inner_angle: 0.40,
                    shadows_enabled: false,
                    ..default()
                },
                Transform::from_translation(world_pos).looking_at(world_aim, Vec3::Y),
                Name::new("StainedGlassLight"),
            ));
        }
    }

    // Roof (slab just above wall height).
    spawn_local_box(
        commands,
        meshes,
        &mats.roof,
        &place,
        Vec3::new(0.0, wall_h + 0.1, 0.0),
        Vec3::new(half_x + 0.3, 0.1, half_z + 0.3),
        "ChurchRoof",
    );

    // Bell tower / spire above the front (entrance) third.
    let tower_x = -half_x + 1.5;
    let tower_base_h = 3.0;
    spawn_local_box(
        commands,
        meshes,
        &mats.wall,
        &place,
        Vec3::new(tower_x, wall_h + tower_base_h * 0.5, 0.0),
        Vec3::new(1.3, tower_base_h * 0.5, 1.3),
        "ChurchTowerBase",
    );
    // Spire (tall pyramid-ish block).
    let spire_h = 3.5;
    spawn_local_deco(
        commands,
        meshes,
        &mats.roof,
        &place,
        Vec3::new(tower_x, wall_h + tower_base_h + spire_h * 0.5, 0.0),
        Vec3::new(0.4, spire_h * 0.5, 0.4),
        "ChurchSpire",
    );

    // Door — reuse the house spawn function by building a temporary
    // HouseConfig that shares the same footprint half.
    let door_config = HouseConfig {
        xz,
        rotation,
        half_size: Vec2::new(half_x, half_z),
        wall_height: wall_h,
        stories: 1,
        has_fireplace: false,
    };
    spawn_door(commands, meshes, mats, &place, &door_config);
}

/// Long wall running along X with windows distributed along the length.
/// Separate from `spawn_walled_side` which places windows along Y.
fn spawn_church_side_wall(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    wall_mat: &Handle<StandardMaterial>,
    place: &Place,
    wall_z_center: f32,
    wall_half_x: f32,
    wall_h: f32,
    wall_thickness: f32,
    window_centers_x: &[f32],
    window_size: Vec2,
    window_cy: f32,
    glass_mats: &[Handle<StandardMaterial>],
) {
    let ww = window_size.x;
    let wh = window_size.y;
    let wb = window_cy - wh * 0.5;
    let wt = window_cy + wh * 0.5;
    let half_thick = wall_thickness * 0.5;

    // Bottom strip (full width, y = 0..wb).
    spawn_local_box(
        commands,
        meshes,
        wall_mat,
        place,
        Vec3::new(0.0, wb * 0.5, wall_z_center),
        Vec3::new(wall_half_x, wb * 0.5, half_thick),
        "ChurchSideBottom",
    );
    // Top strip (full width, y = wt..wall_h).
    spawn_local_box(
        commands,
        meshes,
        wall_mat,
        place,
        Vec3::new(0.0, (wt + wall_h) * 0.5, wall_z_center),
        Vec3::new(wall_half_x, (wall_h - wt) * 0.5, half_thick),
        "ChurchSideTop",
    );

    // Vertical columns between/around windows.
    let mut sorted: Vec<f32> = window_centers_x.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut x_cursor = -wall_half_x;
    for cx in &sorted {
        let window_left = cx - ww * 0.5;
        if window_left > x_cursor + 0.001 {
            let col_half_x = (window_left - x_cursor) * 0.5;
            let col_center_x = (x_cursor + window_left) * 0.5;
            spawn_local_box(
                commands,
                meshes,
                wall_mat,
                place,
                Vec3::new(col_center_x, (wt + wb) * 0.5, wall_z_center),
                Vec3::new(col_half_x, (wt - wb) * 0.5, half_thick),
                "ChurchSideCol",
            );
        }
        x_cursor = cx + ww * 0.5;
    }
    if wall_half_x > x_cursor + 0.001 {
        let col_half_x = (wall_half_x - x_cursor) * 0.5;
        let col_center_x = (x_cursor + wall_half_x) * 0.5;
        spawn_local_box(
            commands,
            meshes,
            wall_mat,
            place,
            Vec3::new(col_center_x, (wt + wb) * 0.5, wall_z_center),
            Vec3::new(col_half_x, (wt - wb) * 0.5, half_thick),
            "ChurchSideCol",
        );
    }

    // Glass panes, colored per window. Visual stays thin (4cm) but the
    // COLLIDER fills the full wall thickness — a 4cm-thin slab was
    // letting the camera raycast squeak past at grazing angles.
    // NotShadowCaster keeps the sun streaming through.
    for (i, cx) in window_centers_x.iter().enumerate() {
        let glass_mat = &glass_mats[i % glass_mats.len()];
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(ww, wh, 0.04))),
            MeshMaterial3d(glass_mat.clone()),
            Transform {
                translation: place.world(Vec3::new(*cx, window_cy, wall_z_center)),
                rotation: place.rotation,
                ..default()
            },
            RigidBody::Fixed,
            Collider::cuboid(ww * 0.5, wh * 0.5, wall_thickness * 0.55),
            bevy::light::NotShadowCaster,
            Name::new("StainedGlassPane"),
        ));
    }
}

/// Spawn a side wall (oriented along X, thickness along Z) with one or
/// more rectangular window openings cut out. For each window we also
/// place a thin glass pane so the opening reads as "covered by glass".
///
/// `windows` entries are `(center_y, width, height, glass_material)` in
/// local wall coords. Windows are assumed not to overlap and to lie
/// within the wall bounds.
fn spawn_walled_side(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    wall_mat: &Handle<StandardMaterial>,
    place: &Place,
    wall_z_center: f32,
    wall_size: Vec2, // (half_x, total_height)
    wall_thickness: f32,
    windows: &[(f32, f32, f32, Handle<StandardMaterial>)],
) {
    let (half_x, wall_h) = (wall_size.x, wall_size.y);
    let half_thick = wall_thickness * 0.5;

    // Sort windows bottom → top so we can walk upward.
    let mut sorted = windows.to_vec();
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    let mut y_cursor = 0.0_f32;
    for (cy, ww, wh, glass_mat) in &sorted {
        let cy = *cy;
        let ww = *ww;
        let wh = *wh;
        let wb = cy - wh * 0.5;
        let wt = cy + wh * 0.5;

        // Wall strip below / between windows (full width).
        if wb > y_cursor + 0.001 {
            let strip_h = wb - y_cursor;
            spawn_local_box(
                commands,
                meshes,
                wall_mat,
                place,
                Vec3::new(0.0, (wb + y_cursor) * 0.5, wall_z_center),
                Vec3::new(half_x, strip_h * 0.5, half_thick),
                "WallStrip",
            );
        }

        // Left/right columns alongside the window.
        let col_half_x = (half_x - ww * 0.5) * 0.5;
        let col_center_x = (half_x + ww * 0.5) * 0.5;
        let col_half_y = wh * 0.5;
        for col_sign in [-1.0_f32, 1.0] {
            spawn_local_box(
                commands,
                meshes,
                wall_mat,
                place,
                Vec3::new(col_sign * col_center_x, cy, wall_z_center),
                Vec3::new(col_half_x, col_half_y, half_thick),
                "WallCol",
            );
        }

        // Glass pane: transparent visual + NotShadowCaster so the sun
        // still lights the interior, plus a Fixed collider spanning the
        // full wall thickness so the orbit camera can't slip past the
        // thin visual.
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(ww, wh, 0.03))),
            MeshMaterial3d(glass_mat.clone()),
            Transform {
                translation: place.world(Vec3::new(0.0, cy, wall_z_center)),
                rotation: place.rotation,
                ..default()
            },
            RigidBody::Fixed,
            Collider::cuboid(ww * 0.5, wh * 0.5, wall_thickness * 0.55),
            bevy::light::NotShadowCaster,
            Name::new("WindowGlass"),
        ));

        y_cursor = wt;
    }

    // Final strip above the last (or only) window up to the ceiling.
    if wall_h > y_cursor + 0.001 {
        let strip_h = wall_h - y_cursor;
        spawn_local_box(
            commands,
            meshes,
            wall_mat,
            place,
            Vec3::new(0.0, (wall_h + y_cursor) * 0.5, wall_z_center),
            Vec3::new(half_x, strip_h * 0.5, half_thick),
            "WallStrip",
        );
    }
}

/// Spawn a fixed-collider box laid out in house-local coordinates. The
/// collider and mesh both inherit the house rotation from `place`, so one
/// spec is enough to produce a properly-oriented wall regardless of how
/// the house itself is turned.
fn spawn_local_box(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: &Handle<StandardMaterial>,
    place: &Place,
    local_pos: Vec3,
    half: Vec3,
    name: &'static str,
) {
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(half.x * 2.0, half.y * 2.0, half.z * 2.0))),
        MeshMaterial3d(material.clone()),
        Transform {
            translation: place.world(local_pos),
            rotation: place.rotation,
            ..default()
        },
        RigidBody::Fixed,
        Collider::cuboid(half.x, half.y, half.z),
        Name::new(name),
    ));
}

/// Like `spawn_local_box` but no rigid body / collider — for decorations
/// the player doesn't need to interact with (mantle, chimney, cheeks).
fn spawn_local_deco(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: &Handle<StandardMaterial>,
    place: &Place,
    local_pos: Vec3,
    half: Vec3,
    name: &'static str,
) {
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(half.x * 2.0, half.y * 2.0, half.z * 2.0))),
        MeshMaterial3d(material.clone()),
        Transform {
            translation: place.world(local_pos),
            rotation: place.rotation,
            ..default()
        },
        Name::new(name),
    ));
}

// --- Systems ----------------------------------------------------------------

fn interact_action(
    keys: Res<ButtonInput<KeyCode>>,
    player_q: Query<&Transform, (With<Player>, Without<Interactable>)>,
    interactables: Query<(Entity, &GlobalTransform, &Interactable)>,
    mut doors: Query<&mut Door>,
) {
    if !keys.just_pressed(KeyCode::KeyE) {
        return;
    }
    let Ok(player_tf) = player_q.single() else {
        return;
    };
    let origin = player_tf.translation + Vec3::Y * 0.5;
    let forward = (player_tf.rotation * Vec3::NEG_Z).normalize();

    let mut best: Option<(Entity, f32)> = None;
    for (entity, tf, interact) in interactables.iter() {
        let diff = tf.translation() - origin;
        let dist = diff.length();
        if dist > interact.range {
            continue;
        }
        let dir = if dist > 0.001 { diff / dist } else { forward };
        if dir.dot(forward) < 0.3 {
            continue;
        }
        if best.is_none_or(|(_, d)| dist < d) {
            best = Some((entity, dist));
        }
    }

    let Some((entity, _)) = best else {
        return;
    };
    if let Ok(mut door) = doors.get_mut(entity) {
        door.open = !door.open;
    }
}

fn animate_door(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(&mut Door, &mut Transform, &Children)>,
    leaves: Query<Entity, With<DoorLeaf>>,
    disabled: Query<Entity, With<ColliderDisabled>>,
) {
    let alpha = 1.0 - (-DOOR_OPEN_RATE * time.delta_secs()).exp();
    for (mut door, mut tf, children) in &mut q {
        let target = if door.open { 1.0 } else { 0.0 };
        door.progress += (target - door.progress) * alpha;
        // Base rotation (the house's world rotation) multiplied by the
        // local swing rotation — keeps the closed-state aligned with the
        // wall it sits in.
        tf.rotation =
            door.base_rotation * Quat::from_rotation_y(door.progress * DOOR_OPEN_ANGLE);

        let should_block = door.progress < 0.05;
        for child in children.iter() {
            if !leaves.contains(child) {
                continue;
            }
            let currently_disabled = disabled.contains(child);
            if should_block && currently_disabled {
                commands.entity(child).remove::<ColliderDisabled>();
            } else if !should_block && !currently_disabled {
                commands.entity(child).insert(ColliderDisabled);
            }
        }
    }
}

fn animate_fire(
    time: Res<Time>,
    mut flames: Query<&mut Transform, (With<FireFlame>, Without<FireLight>)>,
    mut lights: Query<&mut PointLight, With<FireLight>>,
) {
    let t = time.elapsed_secs();
    for mut tf in &mut flames {
        let seed = tf.translation.x * 3.7 + tf.translation.z * 2.3;
        let flick = (t * 9.0 + seed).sin() * 0.08 + (t * 15.2 + seed * 1.7).sin() * 0.05;
        tf.scale = Vec3::splat(1.0 + flick);
    }
    let global = (t * 5.5).sin() * 0.12 + (t * 11.0 + 0.7).sin() * 0.08;
    for mut light in &mut lights {
        light.intensity = 130_000.0 * (1.0 + global);
    }
}
