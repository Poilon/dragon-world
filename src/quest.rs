//! "Break the crate, take the coin" quest.
//!
//! Spawns a wooden crate in the forest. Sword hits (from `combat.rs`)
//! emit a `CrateBroken` message that this module listens for; on break,
//! a coin appears at the crate's position. Clicking the coin when close
//! triggers the Win state — a full-screen HUD overlay with a
//! "MISSION ACCOMPLISHED" label and a Retry button that reverts the
//! scene back to its starting state (player to spawn, sword back on the
//! mezzanine, crate respawned, coin despawned).

use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_rapier3d::prelude::{
    Ccd, Collider, ColliderMassProperties, Damping, Friction, LockedAxes, QueryFilter,
    QueryFilterFlags, ReadRapierContext, Restitution, RigidBody,
};

use crate::camera::OrbitCamera;
use crate::combat::trigger_attack;
use crate::menu::MenuState;
use crate::player::Player;
use crate::sword::{spawn_sword_pickup, HasSword};
use crate::world::terrain_height_at;

/// World position where the crate spawns (Y resolved from terrain).
const CRATE_POS_XZ: Vec2 = Vec2::new(26.0, 0.0);
const CRATE_HALF: f32 = 0.45;

/// Canonical location of the initial sword pickup (mezzanine of the
/// 2-story house, mirrors the placement in `house::spawn_village`).
const SWORD_HOUSE_XZ: Vec2 = Vec2::new(-11.5, -15.0);
const SWORD_HOUSE_ROTATION: f32 = std::f32::consts::PI;
const SWORD_HOUSE_WALL_H: f32 = 5.8;
const SOCLE_HEIGHT: f32 = 0.55;

const COIN_PICKUP_RANGE: f32 = 2.8;

// --- Components / resources / messages ---------------------------------------

#[derive(Component)]
pub struct Crate;

#[derive(Component, Default)]
pub struct Coin {
    pub highlighted: bool,
}

/// Meshes that belong to the coin (for mutating their material on hover).
#[derive(Component)]
struct CoinPart;

/// Marker on the full-screen Win overlay Node.
#[derive(Component)]
struct WinOverlay;

#[derive(Component)]
struct DeathOverlay;

#[derive(Component)]
struct HealthBarFill;

/// Marker on the Retry button so we can find and react to its presses.
#[derive(Component)]
struct RetryButton;

#[derive(Resource, Default)]
pub struct GameState {
    pub won: bool,
    /// Mirrors `Player::dead` — updated from a system so UI can react
    /// via Changed<GameState>.
    pub died: bool,
}

#[derive(Message)]
pub struct CrateBroken {
    pub position: Vec3,
}

// --- Plugin ------------------------------------------------------------------

pub struct QuestPlugin;

impl Plugin for QuestPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameState>()
            .add_message::<CrateBroken>()
            .add_systems(Startup, (spawn_initial_crate, setup_ui))
            .add_systems(
                Update,
                (
                    spawn_coin_on_break,
                    update_coin_highlight,
                    hover_coin,
                    // Coin pickup must run BEFORE combat so the click
                    // that grabs the coin doesn't also trigger a sword
                    // swing on the same frame.
                    click_coin.before(trigger_attack),
                    sync_death_state,
                    update_health_bar,
                    update_overlays,
                    handle_retry_button,
                ),
            );
    }
}

// --- Crate spawn / respawn ---------------------------------------------------

fn spawn_initial_crate(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
) {
    spawn_crate_at_default(&mut commands, &mut meshes, &mut materials, &asset_server);
}

/// Spawn a wooden crate at the canonical quest location.
fn spawn_crate_at_default(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_server: &AssetServer,
) {
    let ground = terrain_height_at(CRATE_POS_XZ.x, CRATE_POS_XZ.y);
    let world_pos = Vec3::new(CRATE_POS_XZ.x, ground + CRATE_HALF, CRATE_POS_XZ.y);
    eprintln!(
        "[quest] spawn_crate_at_default: world_pos = {:?}, ground = {}",
        world_pos, ground
    );

    let wood_diff = load_repeat(asset_server, "textures/wood_floor_deck_diff.jpg");
    let wood_nor = load_repeat(asset_server, "textures/wood_floor_deck_nor.jpg");
    let wood_arm = load_repeat(asset_server, "textures/wood_floor_deck_arm.jpg");
    let crate_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.95, 0.78, 0.55),
        base_color_texture: Some(wood_diff),
        normal_map_texture: Some(wood_nor),
        occlusion_texture: Some(wood_arm.clone()),
        metallic_roughness_texture: Some(wood_arm),
        perceptual_roughness: 1.0,
        metallic: 0.0,
        uv_transform: Affine2::from_scale(Vec2::splat(1.8)),
        ..default()
    });

    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(
            CRATE_HALF * 2.0,
            CRATE_HALF * 2.0,
            CRATE_HALF * 2.0,
        ))),
        MeshMaterial3d(crate_mat),
        Transform::from_translation(world_pos),
        RigidBody::Dynamic,
        Collider::cuboid(CRATE_HALF, CRATE_HALF, CRATE_HALF),
        ColliderMassProperties::Mass(3.0),
        Friction::coefficient(0.6),
        Damping {
            linear_damping: 1.2,
            angular_damping: 1.4,
        },
        Ccd::enabled(),
        Crate,
        Name::new("Crate"),
    ));
}

// --- Coin spawn / animation --------------------------------------------------

fn spawn_coin_on_break(
    mut break_events: MessageReader<CrateBroken>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for ev in break_events.read() {
        spawn_coin_at(&mut commands, &mut meshes, &mut materials, ev.position);
    }
}

fn spawn_coin_at(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    pos: Vec3,
) {
    const COIN_RADIUS: f32 = 0.32;
    const COIN_THICKNESS: f32 = 0.06;
    let coin_mesh = meshes.add(
        Cylinder::new(COIN_RADIUS, COIN_THICKNESS)
            .mesh()
            .resolution(32),
    );
    let gold_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.82, 0.25),
        // Strong idle glow — the coin reads from across the clearing.
        // `update_coin_highlight` boosts it further on hover.
        emissive: LinearRgba::new(1.4, 0.9, 0.18, 1.0),
        perceptual_roughness: 0.22,
        metallic: 0.95,
        reflectance: 0.8,
        ..default()
    });

    // Spawn just above the actual ground (sampled directly) — a long
    // fall into a trimesh terrain lets thin cylinders tunnel through
    // even with CCD on. A ~0.3m drop keeps the per-frame displacement
    // well under the collider's thickness.
    let ground_y = terrain_height_at(pos.x, pos.z);
    let spawn_y = ground_y + 0.35;
    let spawn_pos = Vec3::new(pos.x, spawn_y, pos.z);

    // Dynamic rigid body. Cuboid collider (rather than cylinder) gives
    // more reliable contact resolution against the trimesh terrain, and
    // we pad its thickness a little past the visual so it can't
    // squeeze through triangle edges. `LockedAxes` stops the coin from
    // flipping onto its rim — it stays face-up like a real coin at
    // rest, but can still slide/spin around Y when the player walks
    // into it.
    let collider_half_y: f32 = 0.09;
    commands
        .spawn((
            Transform::from_translation(spawn_pos),
            Visibility::default(),
            RigidBody::Dynamic,
            Collider::cuboid(COIN_RADIUS, collider_half_y, COIN_RADIUS),
            ColliderMassProperties::Mass(0.12),
            Friction::coefficient(0.6),
            Restitution::coefficient(0.1),
            Damping {
                linear_damping: 0.8,
                angular_damping: 3.0,
            },
            LockedAxes::ROTATION_LOCKED_X | LockedAxes::ROTATION_LOCKED_Z,
            Ccd::enabled(),
            Coin::default(),
            Name::new("Coin"),
        ))
        .with_children(|coin| {
            coin.spawn((
                Mesh3d(coin_mesh),
                MeshMaterial3d(gold_mat),
                Transform::default(),
                CoinPart,
                Name::new("CoinDisc"),
            ));
        });
}

/// Mirror of `sword::update_highlight` — brighten the coin's emissive
/// when the cursor ray is on it, with a pulsing warm-gold glow that
/// makes it obvious the pickup is interactive.
fn update_coin_highlight(
    time: Res<Time>,
    coins: Query<(&Coin, &Children)>,
    parts: Query<&MeshMaterial3d<StandardMaterial>, With<CoinPart>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let t = time.elapsed_secs();
    let pulse = (t * 3.5).sin() * 0.5 + 0.5;
    for (coin, children) in &coins {
        let emissive = if coin.highlighted {
            LinearRgba::new(
                3.0 + pulse * 2.2,
                2.0 + pulse * 1.6,
                0.4 + pulse * 0.5,
                1.0,
            )
        } else {
            LinearRgba::new(1.4, 0.9, 0.18, 1.0)
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

// --- Coin hover + click ------------------------------------------------------

fn hover_coin(
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<OrbitCamera>>,
    rapier: ReadRapierContext,
    mut coins: Query<&mut Coin>,
) {
    for mut c in &mut coins {
        c.highlighted = false;
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

    // Exclude the player's kinematic capsule and any sensors — we want
    // the first Fixed or Dynamic collider in the ray's path, i.e. the
    // coin if it's in view, or whatever wall/terrain is hiding it.
    let filter = QueryFilter {
        flags: QueryFilterFlags::EXCLUDE_KINEMATIC | QueryFilterFlags::EXCLUDE_SENSORS,
        ..QueryFilter::default()
    };
    if let Some((entity, _toi)) = ctx.cast_ray(
        ray.origin,
        ray.direction.into(),
        80.0,
        true,
        filter,
    ) {
        if let Ok(mut c) = coins.get_mut(entity) {
            c.highlighted = true;
        }
    }
}

fn click_coin(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    mut game_state: ResMut<GameState>,
    menu: Res<MenuState>,
    coins: Query<(Entity, &GlobalTransform, &Coin)>,
    player_q: Query<&Transform, With<Player>>,
) {
    if game_state.won || menu.open {
        return;
    }
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let Ok(player_tf) = player_q.single() else {
        return;
    };
    for (entity, tf, coin) in &coins {
        if !coin.highlighted {
            continue;
        }
        if (tf.translation() - player_tf.translation).length() > COIN_PICKUP_RANGE {
            continue;
        }
        commands.entity(entity).despawn();
        game_state.won = true;
        break;
    }
}

// --- HUD + overlays UI -------------------------------------------------------

fn setup_ui(mut commands: Commands) {
    // ----- Top-left HP bar --------------------------------------------------
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(20.0),
                left: Val::Px(20.0),
                width: Val::Px(240.0),
                height: Val::Px(26.0),
                padding: UiRect::all(Val::Px(3.0)),
                border: UiRect::all(Val::Px(2.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.85)),
            BorderColor::all(Color::srgba(0.9, 0.9, 0.9, 0.85)),
            Name::new("HealthBarBg"),
        ))
        .with_children(|bg| {
            bg.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.82, 0.25, 0.20)),
                HealthBarFill,
                Name::new("HealthBarFill"),
            ));
        });

    // ----- Win overlay ------------------------------------------------------
    spawn_full_screen_overlay(
        &mut commands,
        "MISSION ACCOMPLISHED",
        Color::srgb(1.0, 0.92, 0.35),
        Color::srgb(0.95, 0.85, 0.35),
        WinOverlay,
        "WinOverlay",
    );

    // ----- Death overlay ----------------------------------------------------
    spawn_full_screen_overlay(
        &mut commands,
        "YOU ARE DEAD",
        Color::srgb(0.95, 0.25, 0.22),
        Color::srgb(0.9, 0.2, 0.15),
        DeathOverlay,
        "DeathOverlay",
    );
}

fn spawn_full_screen_overlay<M: Component>(
    commands: &mut Commands,
    title: &str,
    title_color: Color,
    accent_color: Color,
    marker: M,
    name: &'static str,
) {
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                flex_direction: FlexDirection::Column,
                position_type: PositionType::Absolute,
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.04, 0.08, 0.75)),
            Visibility::Hidden,
            marker,
            Name::new(name),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new(title),
                TextFont {
                    font_size: 72.0,
                    ..default()
                },
                TextColor(title_color),
                TextShadow::default(),
            ));
            parent
                .spawn((
                    Button,
                    Node {
                        width: Val::Px(220.0),
                        height: Val::Px(64.0),
                        margin: UiRect::top(Val::Px(48.0)),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(Val::Px(2.0)),
                        ..default()
                    },
                    BorderColor::all(accent_color),
                    BackgroundColor(Color::srgb(0.12, 0.16, 0.22)),
                    RetryButton,
                ))
                .with_children(|button| {
                    button.spawn((
                        Text::new("Retry"),
                        TextFont {
                            font_size: 36.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.95, 0.90, 0.80)),
                    ));
                });
        });
}

/// Mirror Player::dead into the shared GameState resource so UI
/// systems can react with `Changed<GameState>`.
fn sync_death_state(player_q: Query<&Player>, mut game_state: ResMut<GameState>) {
    let Ok(player) = player_q.single() else {
        return;
    };
    if player.dead != game_state.died {
        game_state.died = player.dead;
    }
}

fn update_health_bar(
    player_q: Query<&Player>,
    mut fill_q: Query<(&mut Node, &mut BackgroundColor), With<HealthBarFill>>,
) {
    let Ok(player) = player_q.single() else {
        return;
    };
    let Ok((mut node, mut bg)) = fill_q.single_mut() else {
        return;
    };
    let pct = (player.hp / player.max_hp).clamp(0.0, 1.0);
    node.width = Val::Percent(pct * 100.0);
    // Shift red → green as HP rises, so the bar colors the player's
    // current state.
    bg.0 = Color::srgb(
        (1.0 - pct) * 0.85 + 0.15,
        pct * 0.70 + 0.20,
        0.20,
    );
}

fn update_overlays(
    game_state: Res<GameState>,
    mut win: Query<&mut Visibility, (With<WinOverlay>, Without<DeathOverlay>)>,
    mut death: Query<&mut Visibility, (With<DeathOverlay>, Without<WinOverlay>)>,
) {
    if !game_state.is_changed() {
        return;
    }
    for mut v in &mut win {
        *v = if game_state.won {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    for mut v in &mut death {
        *v = if game_state.died {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

fn handle_retry_button(
    mut commands: Commands,
    interactions: Query<&Interaction, With<RetryButton>>,
    mut was_pressed: Local<bool>,
    mut game_state: ResMut<GameState>,
    mut has_sword: ResMut<HasSword>,
    mut player_q: Query<(&mut Player, &mut Transform), With<Player>>,
    coins: Query<Entity, With<Coin>>,
    existing_crates: Query<Entity, With<Crate>>,
    existing_pickups: Query<Entity, With<crate::sword::SwordPickup>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
) {
    // Explicit edge detection: fire once on the press transition, not on
    // every frame the interaction is `Pressed` (Bevy UI re-writes the
    // component each tick so `Changed<Interaction>` can fire repeatedly
    // while the button is held).
    let is_pressed = interactions
        .iter()
        .any(|i| matches!(i, Interaction::Pressed));
    let just_pressed = is_pressed && !*was_pressed;
    *was_pressed = is_pressed;
    if !just_pressed {
        return;
    }
    eprintln!("[quest] Retry pressed — resetting game state");

    // Clear state.
    game_state.won = false;
    game_state.died = false;
    has_sword.0 = false;

    // Reset player to spawn.
    if let Ok((mut player, mut tf)) = player_q.single_mut() {
        let spawn_y = terrain_height_at(0.0, 0.0) + 2.0;
        tf.translation = Vec3::new(0.0, spawn_y, 0.0);
        tf.rotation = Quat::IDENTITY;
        player.vertical_velocity = 0.0;
        player.horizontal_velocity = Vec3::ZERO;
        player.horizontal_speed = 0.0;
        player.yaw = 0.0;
        player.yaw_target = 0.0;
        player.grounded = false;
        player.was_grounded = false;
        player.is_strafing = false;
        player.jump_crouch_timer = 0.0;
        player.crouch_amount = 0.0;
        player.in_water = false;
        player.dead = false;
        player.hp = player.max_hp;
    }

    // Despawn any lingering coin(s), stray crate, and the old sword
    // pickup if it's still there (e.g. if player clicked Retry before
    // picking up the sword).
    for e in &coins {
        commands.entity(e).despawn();
    }
    for e in &existing_crates {
        commands.entity(e).despawn();
    }
    for e in &existing_pickups {
        commands.entity(e).despawn();
    }

    // Respawn the world's interactables.
    spawn_crate_at_default(&mut commands, &mut meshes, &mut materials, &asset_server);
    eprintln!("[quest] Retry: crate respawn queued at {:?}", CRATE_POS_XZ);
    spawn_sword_at_initial_location(&mut commands, &mut meshes, &mut materials);
    eprintln!("[quest] Retry: sword pickup respawn queued");
}

/// Spawn the sword pickup at the mezzanine of the 2-story house.
fn spawn_sword_at_initial_location(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let ground = terrain_height_at(SWORD_HOUSE_XZ.x, SWORD_HOUSE_XZ.y);
    let floor_top = ground + SOCLE_HEIGHT;
    let mezz_top = floor_top + SWORD_HOUSE_WALL_H * 0.5;
    let rotation = Quat::from_rotation_y(SWORD_HOUSE_ROTATION);
    let local_offset = Vec3::new(1.6, 0.1, 0.0);
    let world_pos = Vec3::new(SWORD_HOUSE_XZ.x, mezz_top, SWORD_HOUSE_XZ.y)
        + rotation * local_offset;
    let tf = Transform::from_translation(world_pos).with_rotation(rotation);
    spawn_sword_pickup(commands, meshes, materials, tf);
}

// --- Util --------------------------------------------------------------------

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
