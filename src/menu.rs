//! Escape-key pause menu. Shows two sliders (music / SFX) via egui and a
//! Resume / Quit pair of buttons. When `MenuState::open` is true, input
//! systems elsewhere in the codebase gate themselves off.

use bevy::prelude::*;
use bevy_inspector_egui::bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};

use crate::audio::AudioSettings;

#[derive(Resource, Default)]
pub struct MenuState {
    pub open: bool,
}

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MenuState>()
            .add_systems(Update, toggle_menu)
            // The egui UI has to live in this dedicated schedule (not
            // Update) for input routing — sliders and buttons to
            // receive clicks — to work in bevy_egui's multi-pass mode.
            .add_systems(EguiPrimaryContextPass, draw_menu);
    }
}

fn toggle_menu(keys: Res<ButtonInput<KeyCode>>, mut state: ResMut<MenuState>) {
    if keys.just_pressed(KeyCode::Escape) {
        state.open = !state.open;
    }
}

fn draw_menu(
    mut contexts: EguiContexts,
    mut settings: ResMut<AudioSettings>,
    mut state: ResMut<MenuState>,
) {
    if !state.open {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    egui::Window::new("Pause")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .default_width(320.0)
        .show(ctx, |ui| {
            ui.add_space(4.0);
            ui.heading("Options audio");
            ui.add_space(10.0);

            ui.label("Musique");
            ui.add(
                egui::Slider::new(&mut settings.music_volume, 0.0..=1.0)
                    .show_value(true)
                    .custom_formatter(|v, _| format!("{:3.0}%", v * 100.0)),
            );
            ui.add_space(8.0);

            ui.label("Effets sonores (pas, etc.)");
            ui.add(
                egui::Slider::new(&mut settings.sfx_volume, 0.0..=1.0)
                    .show_value(true)
                    .custom_formatter(|v, _| format!("{:3.0}%", v * 100.0)),
            );

            ui.add_space(18.0);
            ui.separator();
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                if ui.button("Reprendre").clicked() {
                    state.open = false;
                }
                if ui.button("Quitter le jeu").clicked() {
                    std::process::exit(0);
                }
            });

            ui.add_space(6.0);
            ui.label(egui::RichText::new("Échap pour fermer").weak());
        });
}
