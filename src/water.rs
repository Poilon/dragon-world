//! Lake with animated water. A flat plane with a custom fragment shader
//! (see `assets/shaders/water.wgsl`) that perturbs the surface normal
//! from a scrolling noise field and pushes alpha up at grazing angles
//! (Fresnel). Swimming logic is handled in `player::move_player` by
//! reading the exported `Lake` resource.

use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

use crate::world::{LAKE_CENTER, LAKE_RADIUS, LAKE_SURFACE_Y};

#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct WaterExtension {}

impl MaterialExtension for WaterExtension {
    fn fragment_shader() -> ShaderRef {
        "shaders/water.wgsl".into()
    }
}

pub type WaterMaterial = ExtendedMaterial<StandardMaterial, WaterExtension>;

/// Authoritative source of truth for lake geometry — used by swim logic
/// in player.rs.
#[derive(Resource, Clone, Copy)]
pub struct Lake {
    pub xz: Vec2,
    pub radius: f32,
    pub surface_y: f32,
}

pub struct WaterPlugin;

impl Plugin for WaterPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<WaterMaterial>::default())
            .insert_resource(Lake {
                xz: LAKE_CENTER,
                radius: LAKE_RADIUS,
                surface_y: LAKE_SURFACE_Y,
            })
            .add_systems(Startup, spawn_water);
    }
}

fn spawn_water(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut water_materials: ResMut<Assets<WaterMaterial>>,
) {
    // Slightly wider than the lake radius so the plane extends under the
    // visible shore; the terrain culls the out-of-basin portions via
    // depth test (water plane at y = LAKE_SURFACE_Y, shore terrain above
    // it).
    let plane_half = LAKE_RADIUS + 2.5;

    let material = water_materials.add(WaterMaterial {
        base: StandardMaterial {
            base_color: Color::srgba(0.18, 0.42, 0.55, 0.35),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.05,
            metallic: 0.0,
            reflectance: 0.8,
            ior: 1.33,
            double_sided: true,
            cull_mode: None,
            ..default()
        },
        extension: WaterExtension {},
    });

    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(plane_half * 2.0, 0.02, plane_half * 2.0))),
        MeshMaterial3d(material),
        Transform::from_xyz(LAKE_CENTER.x, LAKE_SURFACE_Y, LAKE_CENTER.y),
        bevy::light::NotShadowCaster,
        Name::new("WaterSurface"),
    ));
}
