//! A 12-storey dystopian tower. Footprint 24m × 24m, storey height 7m
//! (total ~84m). Each floor is furnished with a bed, sofa, TV and coffee
//! table; each outer wall carries three floor-to-near-ceiling windows.
//! A U-shaped switchback staircase in the east wing links every floor
//! — the flight climbing from level N uses the west half of the
//! stairwell if N is even, the east half if N is odd, so successive
//! flights never share the same XZ column.

use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::math::Affine2;
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use crate::world::terrain_height_at;

// --- Layout ------------------------------------------------------------------

const TOWER_CENTER: Vec2 = Vec2::new(22.0, 12.0);
/// Half-width of the square footprint → 24m × 24m exterior.
const TOWER_HALF: f32 = 12.0;
/// Ceiling-to-floor clearance on every storey. Lofty brutalist scale.
const FLOOR_HEIGHT: f32 = 7.0;
const FLOOR_THICKNESS: f32 = 0.3;
const WALL_THICKNESS: f32 = 0.6;
const NUM_FLOORS: usize = 12;

// Stairwell: 3m × 9m rectangle in the east wing.
const STAIRWELL_X_MIN: f32 = TOWER_HALF - WALL_THICKNESS - 3.0;
const STAIRWELL_X_MAX: f32 = TOWER_HALF - WALL_THICKNESS;
const STAIRWELL_HALF_Z: f32 = 4.5;
const FLIGHT_SPLIT_X: f32 = (STAIRWELL_X_MIN + STAIRWELL_X_MAX) * 0.5;
const STAIR_STEP_COUNT: usize = 28;

const DOOR_HALF_WIDTH: f32 = 1.2;
const DOOR_HEIGHT: f32 = 3.4;

// Windows: three large panels centred along each wall, except ground-
// floor west wall (door).
const WINDOW_HALF_WIDTH: f32 = 2.4;
const WINDOW_SILL_Y: f32 = 1.1; // sill top above current floor
const WINDOW_HEAD_Y: f32 = 5.6; // head (top of glass) above current floor
const WINDOW_POSITIONS: [f32; 3] = [-8.0, 0.0, 8.0];

// --- Plugin ------------------------------------------------------------------

pub struct TrumpTowerPlugin;

impl Plugin for TrumpTowerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_tower);
    }
}

/// Which half of the stairwell is used by the flight climbing from `level`.
fn flight_x_range(level: usize) -> (f32, f32) {
    if level % 2 == 0 {
        (STAIRWELL_X_MIN, FLIGHT_SPLIT_X) // west half
    } else {
        (FLIGHT_SPLIT_X, STAIRWELL_X_MAX) // east half
    }
}

// --- Materials bundle --------------------------------------------------------

struct TowerMats {
    wall: Handle<StandardMaterial>,
    floor: Handle<StandardMaterial>,
    stair: Handle<StandardMaterial>,
    roof: Handle<StandardMaterial>,
    glass: Handle<StandardMaterial>,
    wood: Handle<StandardMaterial>,
    fabric_dark: Handle<StandardMaterial>,
    fabric_red: Handle<StandardMaterial>,
    tv_screen: Handle<StandardMaterial>,
}

fn spawn_tower(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
) {
    let stone_diff = load_repeat(&asset_server, "textures/medieval_blocks_02_diff.jpg");
    let stone_nor = load_repeat(&asset_server, "textures/medieval_blocks_02_nor.jpg");
    let stone_arm = load_repeat(&asset_server, "textures/medieval_blocks_02_arm.jpg");
    let wood_diff = load_repeat(&asset_server, "textures/wood_floor_deck_diff.jpg");
    let wood_nor = load_repeat(&asset_server, "textures/wood_floor_deck_nor.jpg");
    let wood_arm = load_repeat(&asset_server, "textures/wood_floor_deck_arm.jpg");
    let fabric_diff = load_repeat(&asset_server, "textures/fabric_leather_02_diff.jpg");
    let fabric_nor = load_repeat(&asset_server, "textures/fabric_leather_02_nor.jpg");
    let fabric_arm = load_repeat(&asset_server, "textures/fabric_leather_02_arm.jpg");

    let stone_mat = |tile: f32, tint: Color| -> StandardMaterial {
        StandardMaterial {
            base_color: tint,
            base_color_texture: Some(stone_diff.clone()),
            normal_map_texture: Some(stone_nor.clone()),
            occlusion_texture: Some(stone_arm.clone()),
            metallic_roughness_texture: Some(stone_arm.clone()),
            perceptual_roughness: 1.0,
            metallic: 0.0,
            uv_transform: Affine2::from_scale(Vec2::splat(tile)),
            ..default()
        }
    };
    let wood = |tile: f32, tint: Color| -> StandardMaterial {
        StandardMaterial {
            base_color: tint,
            base_color_texture: Some(wood_diff.clone()),
            normal_map_texture: Some(wood_nor.clone()),
            occlusion_texture: Some(wood_arm.clone()),
            metallic_roughness_texture: Some(wood_arm.clone()),
            perceptual_roughness: 0.85,
            metallic: 0.0,
            uv_transform: Affine2::from_scale(Vec2::splat(tile)),
            ..default()
        }
    };
    let fabric = |tile: f32, tint: Color| -> StandardMaterial {
        StandardMaterial {
            base_color: tint,
            base_color_texture: Some(fabric_diff.clone()),
            normal_map_texture: Some(fabric_nor.clone()),
            occlusion_texture: Some(fabric_arm.clone()),
            metallic_roughness_texture: Some(fabric_arm.clone()),
            perceptual_roughness: 0.95,
            metallic: 0.0,
            uv_transform: Affine2::from_scale(Vec2::splat(tile)),
            ..default()
        }
    };

    let mats = TowerMats {
        wall: materials.add(stone_mat(4.0, Color::srgb(0.22, 0.21, 0.22))),
        floor: materials.add(wood(2.5, Color::srgb(0.45, 0.35, 0.28))),
        stair: materials.add(stone_mat(1.2, Color::srgb(0.12, 0.12, 0.13))),
        roof: materials.add(stone_mat(5.0, Color::srgb(0.08, 0.08, 0.09))),
        glass: materials.add(StandardMaterial {
            base_color: Color::srgba(0.55, 0.72, 0.92, 0.22),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.1,
            reflectance: 0.5,
            ..default()
        }),
        wood: materials.add(wood(1.2, Color::srgb(0.52, 0.36, 0.24))),
        fabric_dark: materials.add(fabric(1.5, Color::srgb(0.18, 0.18, 0.22))),
        fabric_red: materials.add(fabric(1.5, Color::srgb(0.55, 0.15, 0.18))),
        tv_screen: materials.add(StandardMaterial {
            base_color: Color::srgb(0.03, 0.04, 0.08),
            emissive: LinearRgba::rgb(0.4, 0.6, 2.2),
            perceptual_roughness: 0.3,
            metallic: 0.4,
            ..default()
        }),
    };

    let ground_y = terrain_height_at(TOWER_CENTER.x, TOWER_CENTER.y);

    // Plinth (buries any terrain slope under the footprint).
    let plinth_half_y = 0.8;
    let plinth_center_y = ground_y - plinth_half_y + 0.3;
    spawn_box(
        &mut commands,
        &mut meshes,
        &mats.wall,
        Vec3::new(TOWER_CENTER.x, plinth_center_y, TOWER_CENTER.y),
        Vec3::new(TOWER_HALF + 0.5, plinth_half_y, TOWER_HALF + 0.5),
        "TrumpTower_Plinth",
    );

    // Walls + furniture + slabs per storey --------------------------------
    for level in 0..NUM_FLOORS {
        let floor_y = ground_y + level as f32 * FLOOR_HEIGHT;
        spawn_storey_walls(&mut commands, &mut meshes, &mats, floor_y, level);
        spawn_furniture(&mut commands, &mut meshes, &mats, floor_y);
    }

    // Floor slabs (ceiling of N-1 = floor of N).
    for slab_level in 1..=NUM_FLOORS {
        let slab_top_y = ground_y + slab_level as f32 * FLOOR_HEIGHT;
        let slab_center_y = slab_top_y - FLOOR_THICKNESS * 0.5;
        let slab_half_y = FLOOR_THICKNESS * 0.5;
        let is_roof = slab_level == NUM_FLOORS;
        let mat = if is_roof { &mats.roof } else { &mats.floor };

        if is_roof {
            spawn_box(
                &mut commands,
                &mut meshes,
                mat,
                Vec3::new(TOWER_CENTER.x, slab_center_y, TOWER_CENTER.y),
                Vec3::new(TOWER_HALF, slab_half_y, TOWER_HALF),
                "TrumpTower_Roof",
            );
            continue;
        }

        let (hole_x_min, hole_x_max) = flight_x_range(slab_level - 1);
        spawn_slab_with_hole(
            &mut commands,
            &mut meshes,
            mat,
            slab_center_y,
            slab_half_y,
            hole_x_min,
            hole_x_max,
            -STAIRWELL_HALF_Z,
            STAIRWELL_HALF_Z,
        );
    }

    // Staircases -----------------------------------------------------------
    for level in 0..(NUM_FLOORS - 1) {
        let base_y = ground_y + level as f32 * FLOOR_HEIGHT;
        let (x_min, x_max) = flight_x_range(level);
        let flight_center_x = (x_min + x_max) * 0.5;
        let flight_half_x = (x_max - x_min) * 0.5;
        let run_length = STAIRWELL_HALF_Z * 2.0;
        let step_run = run_length / STAIR_STEP_COUNT as f32;
        let step_rise = FLOOR_HEIGHT / STAIR_STEP_COUNT as f32;

        for step in 0..STAIR_STEP_COUNT {
            let top_y = base_y + step_rise * (step as f32 + 1.0);
            let center_y = (base_y + top_y) * 0.5;
            let half_y = (top_y - base_y) * 0.5;
            let z_center_local = -STAIRWELL_HALF_Z + (step as f32 + 0.5) * step_run;
            spawn_box(
                &mut commands,
                &mut meshes,
                &mats.stair,
                Vec3::new(
                    TOWER_CENTER.x + flight_center_x,
                    center_y,
                    TOWER_CENTER.y + z_center_local,
                ),
                Vec3::new(flight_half_x, half_y, step_run * 0.5),
                "TrumpTower_Step",
            );
        }
    }
}

// --- Storey walls ------------------------------------------------------------

fn spawn_storey_walls(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &TowerMats,
    floor_y: f32,
    level: usize,
) {
    let ceiling_y = floor_y + FLOOR_HEIGHT;

    // +X wall (east)
    spawn_windowed_wall(
        commands,
        meshes,
        mats,
        WallAxis::ZAlong,
        TOWER_CENTER.x + TOWER_HALF,
        TOWER_CENTER.y,
        TOWER_HALF,
        floor_y,
        ceiling_y,
        "TrumpTower_WallEast",
    );
    // -Z wall (south)
    spawn_windowed_wall(
        commands,
        meshes,
        mats,
        WallAxis::XAlong,
        TOWER_CENTER.y - TOWER_HALF,
        TOWER_CENTER.x,
        TOWER_HALF,
        floor_y,
        ceiling_y,
        "TrumpTower_WallSouth",
    );
    // +Z wall (north)
    spawn_windowed_wall(
        commands,
        meshes,
        mats,
        WallAxis::XAlong,
        TOWER_CENTER.y + TOWER_HALF,
        TOWER_CENTER.x,
        TOWER_HALF,
        floor_y,
        ceiling_y,
        "TrumpTower_WallNorth",
    );

    // -X wall (west): ground floor gets a door, others get windows.
    if level == 0 {
        spawn_doored_wall_z(
            commands,
            meshes,
            &mats.wall,
            TOWER_CENTER.x - TOWER_HALF,
            TOWER_CENTER.y,
            TOWER_HALF,
            floor_y,
            ceiling_y,
        );
    } else {
        spawn_windowed_wall(
            commands,
            meshes,
            mats,
            WallAxis::ZAlong,
            TOWER_CENTER.x - TOWER_HALF,
            TOWER_CENTER.y,
            TOWER_HALF,
            floor_y,
            ceiling_y,
            "TrumpTower_WallWest",
        );
    }
}

#[derive(Copy, Clone)]
enum WallAxis {
    /// Wall extends along world X (N or S face).
    XAlong,
    /// Wall extends along world Z (E or W face).
    ZAlong,
}

/// Build one exterior wall as a stone frame pierced by three floor-to-
/// near-ceiling windows. `fixed_coord` is the world coord perpendicular
/// to the wall (X for a ZAlong wall, Z for an XAlong wall). `center_along`
/// is the world coord of the wall's centre on the along axis.
fn spawn_windowed_wall(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &TowerMats,
    axis: WallAxis,
    fixed_coord: f32,
    center_along: f32,
    wall_half_len: f32,
    floor_y: f32,
    ceiling_y: f32,
    name: &'static str,
) {
    let sill_top_y = floor_y + WINDOW_SILL_Y;
    let head_y = floor_y + WINDOW_HEAD_Y;
    let wall_center_y = (floor_y + ceiling_y) * 0.5;
    let wall_half_h = (ceiling_y - floor_y) * 0.5;
    let sill_half_y = (sill_top_y - floor_y) * 0.5;
    let sill_center_y = (floor_y + sill_top_y) * 0.5;
    let lintel_half_y = (ceiling_y - head_y) * 0.5;
    let lintel_center_y = (head_y + ceiling_y) * 0.5;
    let mid_half_y = (head_y - sill_top_y) * 0.5;
    let mid_center_y = (sill_top_y + head_y) * 0.5;

    let spawn_piece =
        |cmd: &mut Commands, mh: &mut Assets<Mesh>, cx: f32, cy: f32, cz: f32, hx: f32, hy: f32, hz: f32, nm: &'static str| {
            spawn_box(cmd, mh, &mats.wall, Vec3::new(cx, cy, cz), Vec3::new(hx, hy, hz), nm);
        };
    let spawn_glass =
        |cmd: &mut Commands, mh: &mut Assets<Mesh>, cx: f32, cy: f32, cz: f32, hx: f32, hy: f32, hz: f32| {
            // Glass pane is thinner than the stone wall and collides so
            // the camera can't poke through from outside.
            spawn_box(cmd, mh, &mats.glass, Vec3::new(cx, cy, cz), Vec3::new(hx, hy, hz), "TrumpTower_Glass");
        };

    match axis {
        WallAxis::XAlong => {
            let z = fixed_coord;
            let half_th = WALL_THICKNESS * 0.5;
            // Sill (bottom) full-width strip
            spawn_piece(commands, meshes, center_along, sill_center_y, z, wall_half_len, sill_half_y, half_th, name);
            // Lintel (top) full-width strip
            spawn_piece(commands, meshes, center_along, lintel_center_y, z, wall_half_len, lintel_half_y, half_th, name);
            // Jamb strips between / outside the windows
            let mut prev_edge = -wall_half_len;
            for &pos in WINDOW_POSITIONS.iter() {
                let win_left = pos - WINDOW_HALF_WIDTH;
                let win_right = pos + WINDOW_HALF_WIDTH;
                if win_left > prev_edge + 0.01 {
                    let jamb_half = (win_left - prev_edge) * 0.5;
                    let jamb_center = prev_edge + jamb_half;
                    spawn_piece(commands, meshes, center_along + jamb_center, mid_center_y, z, jamb_half, mid_half_y, half_th, name);
                }
                prev_edge = win_right;
                // Glass pane inside the opening
                spawn_glass(
                    commands,
                    meshes,
                    center_along + pos,
                    mid_center_y,
                    z,
                    WINDOW_HALF_WIDTH,
                    mid_half_y,
                    WALL_THICKNESS * 0.25,
                );
            }
            if wall_half_len > prev_edge + 0.01 {
                let jamb_half = (wall_half_len - prev_edge) * 0.5;
                let jamb_center = prev_edge + jamb_half;
                spawn_piece(commands, meshes, center_along + jamb_center, mid_center_y, z, jamb_half, mid_half_y, half_th, name);
            }
            let _ = wall_center_y;
            let _ = wall_half_h;
        }
        WallAxis::ZAlong => {
            let x = fixed_coord;
            let half_th = WALL_THICKNESS * 0.5;
            spawn_piece(commands, meshes, x, sill_center_y, center_along, half_th, sill_half_y, wall_half_len, name);
            spawn_piece(commands, meshes, x, lintel_center_y, center_along, half_th, lintel_half_y, wall_half_len, name);
            let mut prev_edge = -wall_half_len;
            for &pos in WINDOW_POSITIONS.iter() {
                let win_left = pos - WINDOW_HALF_WIDTH;
                let win_right = pos + WINDOW_HALF_WIDTH;
                if win_left > prev_edge + 0.01 {
                    let jamb_half = (win_left - prev_edge) * 0.5;
                    let jamb_center = prev_edge + jamb_half;
                    spawn_piece(commands, meshes, x, mid_center_y, center_along + jamb_center, half_th, mid_half_y, jamb_half, name);
                }
                prev_edge = win_right;
                spawn_glass(
                    commands,
                    meshes,
                    x,
                    mid_center_y,
                    center_along + pos,
                    WALL_THICKNESS * 0.25,
                    mid_half_y,
                    WINDOW_HALF_WIDTH,
                );
            }
            if wall_half_len > prev_edge + 0.01 {
                let jamb_half = (wall_half_len - prev_edge) * 0.5;
                let jamb_center = prev_edge + jamb_half;
                spawn_piece(commands, meshes, x, mid_center_y, center_along + jamb_center, half_th, mid_half_y, jamb_half, name);
            }
            let _ = wall_center_y;
            let _ = wall_half_h;
        }
    }
}

/// Ground-floor west wall: split around a door opening centred on Z.
fn spawn_doored_wall_z(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mat: &Handle<StandardMaterial>,
    x: f32,
    center_z: f32,
    wall_half_len: f32,
    floor_y: f32,
    ceiling_y: f32,
) {
    let wall_center_y = (floor_y + ceiling_y) * 0.5;
    let wall_half_h = (ceiling_y - floor_y) * 0.5;
    let half_th = WALL_THICKNESS * 0.5;
    let door_top_y = floor_y + DOOR_HEIGHT;
    let panel_half_z = (wall_half_len - DOOR_HALF_WIDTH) * 0.5;

    spawn_box(
        commands,
        meshes,
        mat,
        Vec3::new(x, wall_center_y, center_z - DOOR_HALF_WIDTH - panel_half_z),
        Vec3::new(half_th, wall_half_h, panel_half_z),
        "TrumpTower_WallWest_Left",
    );
    spawn_box(
        commands,
        meshes,
        mat,
        Vec3::new(x, wall_center_y, center_z + DOOR_HALF_WIDTH + panel_half_z),
        Vec3::new(half_th, wall_half_h, panel_half_z),
        "TrumpTower_WallWest_Right",
    );
    let lintel_half_y = (ceiling_y - door_top_y) * 0.5;
    let lintel_center_y = (door_top_y + ceiling_y) * 0.5;
    spawn_box(
        commands,
        meshes,
        mat,
        Vec3::new(x, lintel_center_y, center_z),
        Vec3::new(half_th, lintel_half_y, DOOR_HALF_WIDTH),
        "TrumpTower_WallWest_Lintel",
    );
}

// --- Furniture ---------------------------------------------------------------

fn spawn_furniture(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &TowerMats,
    floor_y: f32,
) {
    // Place furniture in the western two-thirds of the storey — the east
    // wing is the stairwell (X ≥ 8.5) so we stay clear of it.

    // Bed (double). Frame + mattress + pillow. South-west area.
    let bed_cx = TOWER_CENTER.x - 7.0;
    let bed_cz = TOWER_CENTER.y - 7.5;
    // Frame
    spawn_box(
        commands,
        meshes,
        &mats.wood,
        Vec3::new(bed_cx, floor_y + 0.2, bed_cz),
        Vec3::new(1.1, 0.2, 2.1),
        "TrumpTower_BedFrame",
    );
    // Mattress
    spawn_box(
        commands,
        meshes,
        &mats.fabric_red,
        Vec3::new(bed_cx, floor_y + 0.55, bed_cz),
        Vec3::new(1.0, 0.15, 2.0),
        "TrumpTower_Mattress",
    );
    // Headboard (north side)
    spawn_box(
        commands,
        meshes,
        &mats.wood,
        Vec3::new(bed_cx, floor_y + 1.1, bed_cz - 2.05),
        Vec3::new(1.15, 0.85, 0.1),
        "TrumpTower_Headboard",
    );
    // Pillow
    spawn_box(
        commands,
        meshes,
        &mats.fabric_dark,
        Vec3::new(bed_cx, floor_y + 0.78, bed_cz - 1.6),
        Vec3::new(0.8, 0.1, 0.35),
        "TrumpTower_Pillow",
    );

    // Coffee table. West-centre, near the sofa.
    spawn_box(
        commands,
        meshes,
        &mats.wood,
        Vec3::new(TOWER_CENTER.x - 5.5, floor_y + 0.35, TOWER_CENTER.y + 1.0),
        Vec3::new(1.0, 0.1, 0.6),
        "TrumpTower_CoffeeTable",
    );
    // Four table legs (aesthetic, optional but cheap).
    for (dx, dz) in [(-0.9, -0.55), (0.9, -0.55), (-0.9, 0.55), (0.9, 0.55)] {
        spawn_box(
            commands,
            meshes,
            &mats.wood,
            Vec3::new(
                TOWER_CENTER.x - 5.5 + dx,
                floor_y + 0.17,
                TOWER_CENTER.y + 1.0 + dz,
            ),
            Vec3::new(0.05, 0.17, 0.05),
            "TrumpTower_TableLeg",
        );
    }

    // Sofa (facing the TV on the north wall). West-centre Z ≈ -1.
    let sofa_cx = TOWER_CENTER.x - 5.5;
    let sofa_cz = TOWER_CENTER.y - 2.3;
    // Base (seat)
    spawn_box(
        commands,
        meshes,
        &mats.fabric_dark,
        Vec3::new(sofa_cx, floor_y + 0.45, sofa_cz),
        Vec3::new(1.8, 0.35, 0.7),
        "TrumpTower_SofaBase",
    );
    // Backrest (against south)
    spawn_box(
        commands,
        meshes,
        &mats.fabric_dark,
        Vec3::new(sofa_cx, floor_y + 1.0, sofa_cz - 0.6),
        Vec3::new(1.8, 0.55, 0.15),
        "TrumpTower_SofaBack",
    );
    // Arm rests
    for side in [-1.0_f32, 1.0] {
        spawn_box(
            commands,
            meshes,
            &mats.fabric_dark,
            Vec3::new(sofa_cx + side * 1.7, floor_y + 0.75, sofa_cz),
            Vec3::new(0.15, 0.35, 0.7),
            "TrumpTower_SofaArm",
        );
    }
    // Seat cushions
    for dx in [-1.0_f32, 0.0, 1.0] {
        spawn_box(
            commands,
            meshes,
            &mats.fabric_red,
            Vec3::new(sofa_cx + dx, floor_y + 0.88, sofa_cz + 0.1),
            Vec3::new(0.47, 0.1, 0.55),
            "TrumpTower_SofaCushion",
        );
    }

    // TV: mounted on the north wall (+Z side), facing the sofa.
    let tv_cx = TOWER_CENTER.x - 5.5;
    let tv_cz = TOWER_CENTER.y + 4.5; // stands a metre in front of the wall
    let tv_cy = floor_y + 2.0;
    // Wooden stand
    spawn_box(
        commands,
        meshes,
        &mats.wood,
        Vec3::new(tv_cx, floor_y + 0.35, tv_cz),
        Vec3::new(1.2, 0.35, 0.45),
        "TrumpTower_TVStand",
    );
    // TV body
    spawn_box(
        commands,
        meshes,
        &mats.wood,
        Vec3::new(tv_cx, tv_cy, tv_cz + 0.2),
        Vec3::new(1.4, 0.85, 0.12),
        "TrumpTower_TVBody",
    );
    // Emissive screen slightly in front so bloom catches it
    spawn_box(
        commands,
        meshes,
        &mats.tv_screen,
        Vec3::new(tv_cx, tv_cy, tv_cz + 0.08),
        Vec3::new(1.3, 0.78, 0.02),
        "TrumpTower_TVScreen",
    );
}

// --- Slab with rectangular hole ---------------------------------------------

fn spawn_slab_with_hole(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mat: &Handle<StandardMaterial>,
    slab_center_y: f32,
    slab_half_y: f32,
    hole_x_min: f32,
    hole_x_max: f32,
    hole_z_min: f32,
    hole_z_max: f32,
) {
    let t_min_x = -TOWER_HALF;
    let t_max_x = TOWER_HALF;
    let t_min_z = -TOWER_HALF;
    let t_max_z = TOWER_HALF;

    if hole_x_min > t_min_x + 0.01 {
        let half_x = (hole_x_min - t_min_x) * 0.5;
        let cx = t_min_x + half_x;
        spawn_box(
            commands,
            meshes,
            mat,
            Vec3::new(TOWER_CENTER.x + cx, slab_center_y, TOWER_CENTER.y),
            Vec3::new(half_x, slab_half_y, TOWER_HALF),
            "TrumpTower_Slab_West",
        );
    }
    if hole_x_max < t_max_x - 0.01 {
        let half_x = (t_max_x - hole_x_max) * 0.5;
        let cx = t_max_x - half_x;
        spawn_box(
            commands,
            meshes,
            mat,
            Vec3::new(TOWER_CENTER.x + cx, slab_center_y, TOWER_CENTER.y),
            Vec3::new(half_x, slab_half_y, TOWER_HALF),
            "TrumpTower_Slab_East",
        );
    }
    if hole_z_max < t_max_z - 0.01 {
        let half_z = (t_max_z - hole_z_max) * 0.5;
        let cz = t_max_z - half_z;
        let half_x = (hole_x_max - hole_x_min) * 0.5;
        let cx = (hole_x_min + hole_x_max) * 0.5;
        spawn_box(
            commands,
            meshes,
            mat,
            Vec3::new(TOWER_CENTER.x + cx, slab_center_y, TOWER_CENTER.y + cz),
            Vec3::new(half_x, slab_half_y, half_z),
            "TrumpTower_Slab_NCap",
        );
    }
    if hole_z_min > t_min_z + 0.01 {
        let half_z = (hole_z_min - t_min_z) * 0.5;
        let cz = t_min_z + half_z;
        let half_x = (hole_x_max - hole_x_min) * 0.5;
        let cx = (hole_x_min + hole_x_max) * 0.5;
        spawn_box(
            commands,
            meshes,
            mat,
            Vec3::new(TOWER_CENTER.x + cx, slab_center_y, TOWER_CENTER.y + cz),
            Vec3::new(half_x, slab_half_y, half_z),
            "TrumpTower_Slab_SCap",
        );
    }
}

// --- Helpers -----------------------------------------------------------------

fn spawn_box(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mat: &Handle<StandardMaterial>,
    center: Vec3,
    half_extents: Vec3,
    name: &'static str,
) {
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(
            half_extents.x * 2.0,
            half_extents.y * 2.0,
            half_extents.z * 2.0,
        ))),
        MeshMaterial3d(mat.clone()),
        Transform::from_translation(center),
        RigidBody::Fixed,
        Collider::cuboid(half_extents.x, half_extents.y, half_extents.z),
        Name::new(name),
    ));
}

fn load_repeat(asset_server: &AssetServer, path: &'static str) -> Handle<Image> {
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
