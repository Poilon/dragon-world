use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::light::CascadeShadowConfigBuilder;
use bevy::math::Affine2;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use bevy_rapier3d::prelude::*;
use noise::{NoiseFn, Perlin};

/// ExtendedMaterial that preserves all StandardMaterial behaviour but
/// adds a macro-scale albedo sample to break up tile repetition on the
/// terrain.
#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct GrassDetile {}

impl MaterialExtension for GrassDetile {
    fn fragment_shader() -> ShaderRef {
        "shaders/grass_detile.wgsl".into()
    }
}

pub type GrassMaterial = ExtendedMaterial<StandardMaterial, GrassDetile>;

pub const TERRAIN_SIZE: f32 = 220.0;
pub const TERRAIN_SUBDIV: u32 = 176;

// Radial terrain profile: a flat-ish circular clearing at the origin, with
// progressively taller hills/mountains ringing it. The player spawns and
// the village sits inside the clearing; the mountains form a natural
// boundary that can't be climbed (slopes exceed the character controller
// max_slope_climb_angle).
const CLEARING_RADIUS: f32 = 38.0;
const MOUNTAIN_FULL_AT: f32 = 88.0;

/// Lake in the west part of the clearing. `sample_height` subtracts a
/// smooth bowl centered on `LAKE_CENTER` so the terrain dips below the
/// water surface; the water plane itself is spawned at `LAKE_SURFACE_Y`.
pub const LAKE_CENTER: Vec2 = Vec2::new(-26.0, 0.0);
pub const LAKE_RADIUS: f32 = 10.0;
pub const LAKE_BASIN_DEPTH: f32 = 3.6;
pub const LAKE_SURFACE_Y: f32 = -0.4;
/// Gentle rolling inside the clearing (±0.2m ish). Small enough that
/// autostep handles it and building floors look flat.
const CLEARING_AMP: f32 = 0.18;
/// Peak mountain contribution at `MOUNTAIN_FULL_AT` and beyond.
const MOUNTAIN_AMP: f32 = 26.0;
/// High-frequency noise → small bumps everywhere.
const SMALL_FREQ: f64 = 0.035;
/// Low-frequency noise → large mountain masses.
const LARGE_FREQ: f64 = 0.011;

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<GrassMaterial>::default())
            .add_systems(Startup, (spawn_sun, spawn_terrain));
    }
}

fn spawn_sun(mut commands: Commands) {
    // Realistic direct-sun illuminance (~110k lux at solar noon). With the
    // atmosphere shader providing sky/ambient scattering, the sun needs to be
    // this bright for the overall exposure to balance out.
    commands.spawn((
        DirectionalLight {
            // Pushed past realistic solar noon for a hot summer-noon feel;
            // TonyMcMapface's roll-off keeps things from clipping to pure
            // white.
            illuminance: 180_000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(20.0, 40.0, 10.0)
            .looking_at(Vec3::ZERO, Vec3::Y),
        CascadeShadowConfigBuilder {
            first_cascade_far_bound: 15.0,
            maximum_distance: 80.0,
            ..default()
        }
        .build(),
        Name::new("Sun"),
    ));
}

fn spawn_terrain(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut grass_materials: ResMut<Assets<GrassMaterial>>,
    asset_server: Res<AssetServer>,
) {
    let (mesh, positions, indices) = build_terrain_mesh(TERRAIN_SIZE, TERRAIN_SUBDIV);
    let mesh_handle = meshes.add(mesh);

    // Photoscanned grass+rock PBR set (Poly Haven, CC0). Sampler forced to
    // Repeat so the 1K tile can be tiled across the whole 220m terrain via
    // `uv_transform` on the material.
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
    let grass_diff = load_repeat("textures/aerial_grass_rock_diff.jpg");
    let grass_nor = load_repeat("textures/aerial_grass_rock_nor.jpg");
    let grass_arm = load_repeat("textures/aerial_grass_rock_arm.jpg");

    // Tile at ~4.5m — enough repetitions for close-up detail, few enough
    // per screen to hide the repeat pattern. Pairs with the per-vertex
    // color tint (set in build_terrain_mesh) which multiplies the texture
    // and varies at a much larger scale, breaking up obvious repetition.
    const GRASS_TILE: f32 = 4.5;
    let tile_count = TERRAIN_SIZE / GRASS_TILE;

    let material = grass_materials.add(GrassMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(grass_diff),
            normal_map_texture: Some(grass_nor),
            occlusion_texture: Some(grass_arm.clone()),
            metallic_roughness_texture: Some(grass_arm),
            perceptual_roughness: 1.0,
            metallic: 0.0,
            uv_transform: Affine2::from_scale(Vec2::splat(tile_count)),
            ..default()
        },
        extension: GrassDetile {},
    });

    // Collider built from the exact same vertices/indices as the render mesh
    // — no axis/row-column ambiguity, visual and physics match 1:1.
    let collider = Collider::trimesh(positions, indices)
        .expect("terrain trimesh construction should not fail");

    commands.spawn((
        Mesh3d(mesh_handle),
        MeshMaterial3d(material),
        Transform::default(),
        RigidBody::Fixed,
        collider,
        Name::new("Terrain"),
    ));
}

/// Build a subdivided plane, displace Y using Perlin noise, and return both
/// the Bevy mesh and the raw vertex/index buffers (usable directly by
/// Rapier's trimesh collider).
fn build_terrain_mesh(size: f32, subdiv: u32) -> (Mesh, Vec<Vec3>, Vec<[u32; 3]>) {
    let verts_per_side = subdiv + 1;
    let step = size / subdiv as f32;
    let half = size / 2.0;

    let mut positions: Vec<Vec3> =
        Vec::with_capacity((verts_per_side * verts_per_side) as usize);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(positions.capacity());
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(positions.capacity());

    // Large-scale tint Perlin — `Mesh::ATTRIBUTE_COLOR` is multiplied with
    // the sampled albedo in `StandardMaterial`, so a slow-varying
    // vertex-color field produces large "patches" of lighter/darker/warmer
    // grass that break up the tile-repeat pattern without needing a
    // custom shader.
    let tint = Perlin::new(4242);

    for iz in 0..verts_per_side {
        for ix in 0..verts_per_side {
            let x = -half + ix as f32 * step;
            let z = -half + iz as f32 * step;
            let y = sample_height(x, z);
            positions.push(Vec3::new(x, y, z));
            uvs.push([ix as f32 / subdiv as f32, iz as f32 / subdiv as f32]);

            // Two-octave tint: a slow ~55m macro variation and a faster
            // ~12m mid variation.
            let t_big = tint.get([x as f64 * 0.018, z as f64 * 0.018]) as f32;
            let t_mid = tint.get([x as f64 * 0.08 + 50.0, z as f64 * 0.08 + 50.0]) as f32;
            let v = t_big * 0.55 + t_mid * 0.20;
            // Keep the tint subtle (±~15%) so the texture detail reads.
            // Slight hue shift toward warm yellows on positive variation,
            // cooler desaturated green on negative.
            let r = (1.0 + v * 0.15).clamp(0.80, 1.20);
            let g = (1.0 - v.abs() * 0.07).clamp(0.85, 1.12);
            let b = (1.0 - v * 0.14).clamp(0.80, 1.15);
            colors.push([r, g, b, 1.0]);
        }
    }

    // CCW winding so upward-facing triangles have a +Y normal.
    let mut tri_indices: Vec<[u32; 3]> = Vec::with_capacity((subdiv * subdiv * 2) as usize);
    for iz in 0..subdiv {
        for ix in 0..subdiv {
            let i0 = iz * verts_per_side + ix;
            let i1 = i0 + 1;
            let i2 = i0 + verts_per_side;
            let i3 = i2 + 1;
            tri_indices.push([i0, i2, i1]);
            tri_indices.push([i1, i2, i3]);
        }
    }

    // Flatten for the render mesh.
    let mesh_positions: Vec<[f32; 3]> = positions.iter().map(|p| [p.x, p.y, p.z]).collect();
    let flat_indices: Vec<u32> = tri_indices.iter().flatten().copied().collect();

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, mesh_positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(flat_indices));
    mesh.compute_normals();

    (mesh, positions, tri_indices)
}

/// Sample terrain height at a world-space XZ. Used to place the player/objects
/// and to build the trimesh. Same formula is used at mesh build time and at
/// runtime lookups so the render mesh and spawn positions stay in sync.
pub fn terrain_height_at(x: f32, z: f32) -> f32 {
    sample_height(x, z)
}

fn sample_height(x: f32, z: f32) -> f32 {
    // Creating Perlin instances is cheap (seeded LUT); we do it inline so
    // the function stays a pure `(x, z) -> y` without global state.
    let perlin = Perlin::new(1337);
    let small = perlin.get([x as f64 * SMALL_FREQ, z as f64 * SMALL_FREQ]) as f32;
    let large = perlin.get([
        x as f64 * LARGE_FREQ + 100.0,
        z as f64 * LARGE_FREQ + 100.0,
    ]) as f32;

    let dist = (x * x + z * z).sqrt();
    let t = ((dist - CLEARING_RADIUS) / (MOUNTAIN_FULL_AT - CLEARING_RADIUS)).clamp(0.0, 1.0);
    let mountain_falloff = t * t * (3.0 - 2.0 * t);

    let mut y = small * CLEARING_AMP + large * MOUNTAIN_AMP * mountain_falloff;

    // Lake basin: smooth bowl that digs the terrain below the water
    // surface. Smoothstep falloff prevents a sharp ring at the edge.
    let lake_dx = x - LAKE_CENTER.x;
    let lake_dz = z - LAKE_CENTER.y;
    let lake_dist = (lake_dx * lake_dx + lake_dz * lake_dz).sqrt();
    if lake_dist < LAKE_RADIUS {
        let u = 1.0 - lake_dist / LAKE_RADIUS;
        let basin_mask = u * u * (3.0 - 2.0 * u);
        y -= basin_mask * LAKE_BASIN_DEPTH;
    }

    y
}
