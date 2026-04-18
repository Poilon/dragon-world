use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use crate::hero::{HeroAnimation, HeroRoot};
use crate::menu::MenuState;
use crate::player::Player;
use crate::quest::{Crate, CrateBroken, GameState};
use crate::sword::HasSword;

const ATTACK_DURATION: f32 = 0.55;
/// Frames where the blade connects. Before: windup. After: recovery.
const HIT_WINDOW_START: f32 = 0.18;
const HIT_WINDOW_END: f32 = 0.30;
/// Radius of the spherical hit zone in front of the player.
const ATTACK_RANGE: f32 = 2.4;
/// Cosine of the half-angle of the hit cone (0.5 = 60°, 0.0 = 90°).
const ATTACK_CONE_COS: f32 = 0.3;
const HIT_IMPULSE_FORWARD: f32 = 16.0;
const HIT_IMPULSE_UP: f32 = 6.0;
const HIT_TORQUE: f32 = 3.0;

#[derive(Component, Default)]
pub struct Attack {
    pub timer: f32,
    pub active: bool,
    pub hit_done: bool,
}

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (trigger_attack, update_attack).chain());
    }
}

pub fn trigger_attack(
    mouse: Res<ButtonInput<MouseButton>>,
    has_sword: Res<HasSword>,
    game_state: Res<GameState>,
    menu: Res<MenuState>,
    mut player_q: Query<&mut Attack, With<Player>>,
) {
    // No sword → left-click is reserved for interactions (picking up
    // sword/coin). Attacking is off without a sword, during the win
    // state, or while the pause menu is open.
    if !has_sword.0 || game_state.won || menu.open {
        return;
    }
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let Ok(mut attack) = player_q.single_mut() else {
        return;
    };
    if !attack.active {
        attack.active = true;
        attack.timer = 0.0;
        attack.hit_done = false;
    }
}

fn update_attack(
    time: Res<Time>,
    mut commands: Commands,
    mut break_events: MessageWriter<CrateBroken>,
    crates: Query<(), With<Crate>>,
    mut player_q: Query<(Entity, &mut Attack, &Transform), With<Player>>,
    mut hero_q: Query<&mut HeroAnimation, With<HeroRoot>>,
    targets: Query<(Entity, &Transform, &RigidBody), (Without<Player>, Without<HeroRoot>)>,
) {
    let Ok((player_entity, mut attack, player_tf)) = player_q.single_mut() else {
        return;
    };
    let Ok(mut anim) = hero_q.single_mut() else {
        return;
    };

    if !attack.active {
        anim.attack_phase = 0.0;
        return;
    }

    attack.timer += time.delta_secs();
    let t = attack.timer;

    // Linear 0 → 1 progress across the whole attack. hero.rs maps this
    // piecewise to windup → slash → recovery arm angles, so recovery eases
    // back to rest instead of re-tracing the slash in reverse.
    anim.attack_phase = (t / ATTACK_DURATION).clamp(0.0, 1.0);

    // Hit window: apply impulses to dynamic bodies inside a forward cone.
    if !attack.hit_done && t >= HIT_WINDOW_START && t <= HIT_WINDOW_END {
        attack.hit_done = true;
        let origin = player_tf.translation + Vec3::Y * 0.5;
        let forward = (player_tf.rotation * Vec3::NEG_Z).normalize();

        for (entity, tf, body) in targets.iter() {
            if entity == player_entity {
                continue;
            }
            if !matches!(body, RigidBody::Dynamic) {
                continue;
            }
            let diff = tf.translation - origin;
            let dist = diff.length();
            if dist < 0.0001 || dist > ATTACK_RANGE {
                continue;
            }
            let dir = diff / dist;
            if dir.dot(forward) < ATTACK_CONE_COS {
                continue;
            }

            // A crate in the cone breaks instead of being knocked — the
            // quest system listens for the message and spawns the coin.
            if crates.contains(entity) {
                commands.entity(entity).despawn();
                break_events.write(CrateBroken { position: tf.translation });
                continue;
            }

            // Launch the target: forward shove + upward kick + a bit of spin.
            let impulse = forward * HIT_IMPULSE_FORWARD + Vec3::Y * HIT_IMPULSE_UP;
            // Torque around a non-forward axis so objects visibly spin.
            let right = forward.cross(Vec3::Y).normalize_or_zero();
            let torque = right * HIT_TORQUE;
            commands.entity(entity).insert(ExternalImpulse {
                impulse,
                torque_impulse: torque,
            });
        }
    }

    if t >= ATTACK_DURATION {
        attack.active = false;
        attack.hit_done = false;
        attack.timer = 0.0;
        anim.attack_phase = 0.0;
    }
}
