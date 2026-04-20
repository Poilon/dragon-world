mod audio;
mod camera;
mod combat;
mod hero;
mod house;
mod menu;
mod physics;
mod player;
mod quest;
mod sword;
mod trees;
mod trump_tower;
mod water;
mod world;

use bevy::asset::AssetPlugin;
use bevy::dev_tools::fps_overlay::{FpsOverlayConfig, FpsOverlayPlugin};
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::light::GlobalAmbientLight;
use bevy::prelude::*;
use bevy::window::WindowResolution;
use bevy_inspector_egui::bevy_egui::EguiPlugin;
use bevy_inspector_egui::quick::WorldInspectorPlugin;
use bevy_rapier3d::prelude::*;

fn main() {
    // Allow launching both via `cargo run` (CWD = project root) and by
    // running the built .exe directly (CWD = exe folder or System32).
    // Both `std::fs::read("musics/...")` and `AssetServer::load(...)`
    // need the project root — the first via CWD, the second via
    // `AssetPlugin::file_path` (bevy resolves file_path from the exe's
    // parent dir, not CWD, so we pass an absolute path).
    let project_root = locate_project_root();
    if let Some(ref root) = project_root {
        let _ = std::env::set_current_dir(root);
    }
    let asset_path = project_root
        .as_ref()
        .map(|r| r.join("assets").to_string_lossy().into_owned())
        .unwrap_or_else(|| "assets".to_string());

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: asset_path,
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Dragon World — Sandbox 1".into(),
                        resolution: WindowResolution::new(1280, 720),
                        ..default()
                    }),
                    ..default()
                }),
        )
        .add_plugins(FrameTimeDiagnosticsPlugin::default())
        .add_plugins(FpsOverlayPlugin {
            config: FpsOverlayConfig {
                text_color: Color::srgb(0.95, 0.95, 0.4),
                ..default()
            },
        })
        .add_plugins(RapierPhysicsPlugin::<NoUserData>::default())
        .add_plugins(RapierDebugRenderPlugin::default().disabled())
        .add_plugins(EguiPlugin::default())
        .add_plugins(WorldInspectorPlugin::new().run_if(inspector_visible))
        // Global fill light. Brightness is in cd/m² — the default 80 is
        // invisible under Exposure::SUNLIGHT (EV100 = 15). 7500 lifts
        // interior/shadowed areas aggressively so the inside of houses
        // (lit only by sun through windows + this) reads cleanly.
        .insert_resource(GlobalAmbientLight {
            color: Color::srgb(0.82, 0.88, 0.98),
            brightness: 3500.0,
            ..default()
        })
        .init_resource::<DebugFlags>()
        .add_plugins(world::WorldPlugin)
        .add_plugins(physics::PhysicsPlugin)
        .add_plugins(water::WaterPlugin)
        .add_plugins(sword::SwordPlugin)
        .add_plugins(house::HousePlugin)
        .add_plugins(trump_tower::TrumpTowerPlugin)
        .add_plugins(trees::TreesPlugin)
        .add_plugins(player::PlayerPlugin)
        .add_plugins(hero::HeroPlugin)
        .add_plugins(combat::CombatPlugin)
        .add_plugins(camera::CameraPlugin)
        .add_plugins(quest::QuestPlugin)
        .add_plugins(audio::AudioPlugin)
        .add_plugins(menu::MenuPlugin)
        .add_systems(Update, (toggle_debug_render, toggle_inspector))
        .run();
}

#[derive(Resource, Default)]
struct DebugFlags {
    inspector_visible: bool,
}

fn inspector_visible(flags: Res<DebugFlags>) -> bool {
    flags.inspector_visible
}

fn toggle_debug_render(keys: Res<ButtonInput<KeyCode>>, mut ctx: ResMut<DebugRenderContext>) {
    if keys.just_pressed(KeyCode::F1) {
        ctx.enabled = !ctx.enabled;
    }
}

fn toggle_inspector(keys: Res<ButtonInput<KeyCode>>, mut flags: ResMut<DebugFlags>) {
    if keys.just_pressed(KeyCode::F2) {
        flags.inspector_visible = !flags.inspector_visible;
    }
}

fn locate_project_root() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    // First try the CWD: this is the common case when launching via
    // `cargo run` from the project directory.
    if std::path::Path::new("assets").is_dir() {
        return std::env::current_dir().ok();
    }
    // Otherwise walk upward from the exe (typical layout:
    // `<root>/target/<profile>/<name>.exe`, so the root is 2 levels up).
    let exe = std::env::current_exe().ok()?;
    let mut dir: PathBuf = exe.parent()?.to_path_buf();
    for _ in 0..5 {
        if dir.join("assets").is_dir() {
            return Some(dir);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}
