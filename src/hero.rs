//! A fully procedural stylized low-poly hero, assembled from Bevy primitives.
//! Acts as a placeholder for a real rigged glTF model — same spirit as Link's
//! proportions: short, chunky, big head, strong silhouette.

use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::math::Affine2;
use bevy::prelude::*;

use crate::player::Player;
use crate::sword::HasSword;

// Root of the hero, attached as a child of the player entity. Its local Y is
// offset so the feet sit at the bottom of the player's physics capsule.
pub const HERO_FOOT_OFFSET: f32 = -0.9;

#[derive(Component)]
pub struct HeroRoot;

#[derive(Component, Clone, Copy)]
pub enum HeroLimb {
    ArmLeft,
    ArmRight,
    LegLeft,
    LegRight,
    Torso,
    Head,
}

/// Sword visible in the right hand once picked up. Toggled via the
/// `HasSword` resource.
#[derive(Component)]
pub struct SwordInHand;

/// Holds the rest-pose local translation for a limb so animation can layer
/// offsets on top without drifting.
#[derive(Component, Clone, Copy)]
pub struct RestTransform(pub Transform);

pub struct HeroPlugin;

impl Plugin for HeroPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (animate_hero, toggle_sword_visibility, animate_pose),
        );
    }
}

/// Continuous body-pose adjustments on the HeroRoot: jump crouch squash
/// and the swim tilt toward prone when the player is in water. Both
/// share the same Transform so we handle them in one system.
fn animate_pose(
    time: Res<Time>,
    mut swim_tilt: Local<f32>,
    player_q: Query<&Player, Without<HeroRoot>>,
    mut hero_root_q: Query<&mut Transform, With<HeroRoot>>,
) {
    let Ok(player) = player_q.single() else {
        return;
    };
    let Ok(mut tf) = hero_root_q.single_mut() else {
        return;
    };

    // Smooth exponential lerp of the swim tilt toward its target. -1.4
    // rad ≈ 80° forward lean — reads unambiguously as "horizontal in
    // the water" without the hero literally clipping through its own
    // kinematic capsule.
    let target = if player.in_water { -1.4 } else { 0.0 };
    let alpha = 1.0 - (-7.0 * time.delta_secs()).exp();
    *swim_tilt += (target - *swim_tilt) * alpha;

    // Jump crouch squash.
    let c = player.crouch_amount.clamp(0.0, 1.0);
    let eased = c * c * (3.0 - 2.0 * c);

    // When swimming we also lift the HeroRoot up a bit so the body
    // sits at the water surface instead of rotating the feet at
    // capsule-bottom height.
    let swim_lift = (-*swim_tilt / 1.4).clamp(0.0, 1.0) * 0.55;

    tf.translation.y = HERO_FOOT_OFFSET + swim_lift - eased * 0.22;
    tf.scale = Vec3::new(
        1.0 + eased * 0.05,
        1.0 - eased * 0.18,
        1.0 + eased * 0.05,
    );
    tf.rotation = Quat::from_rotation_x(*swim_tilt);
}

fn load_hero_texture(asset_server: &AssetServer, path: &'static str) -> Handle<Image> {
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

/// Spawn a hero as a child of `parent`. The caller is responsible for the
/// parent's own Transform (position in the world).
pub fn spawn_hero(
    parent: &mut ChildSpawnerCommands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_server: &AssetServer,
) {
    // Shared fabric+leather PBR, tinted per body part. One texture
    // covers the tunic, trousers, belt, and boots — the base_color
    // modulation turns it green / brown / dark for each role.
    let fabric_diff = load_hero_texture(asset_server, "textures/fabric_leather_02_diff.jpg");
    let fabric_nor = load_hero_texture(asset_server, "textures/fabric_leather_02_nor.jpg");
    let fabric_arm = load_hero_texture(asset_server, "textures/fabric_leather_02_arm.jpg");
    let fabric_uv = Affine2::from_scale(Vec2::splat(3.5));

    let make_fabric_mat = |m: &mut Assets<StandardMaterial>, tint: Color, rough_bump: f32| -> Handle<StandardMaterial> {
        m.add(StandardMaterial {
            base_color: tint,
            base_color_texture: Some(fabric_diff.clone()),
            normal_map_texture: Some(fabric_nor.clone()),
            occlusion_texture: Some(fabric_arm.clone()),
            metallic_roughness_texture: Some(fabric_arm.clone()),
            perceptual_roughness: rough_bump,
            metallic: 0.0,
            uv_transform: fabric_uv,
            ..default()
        })
    };

    // Palette — Link-ish
    let skin = materials.add(StandardMaterial {
        base_color: Color::srgb(0.95, 0.78, 0.63),
        perceptual_roughness: 0.7,
        ..default()
    });
    let tunic = make_fabric_mat(materials, Color::srgb(0.38, 0.72, 0.38), 1.0);
    let trouser = make_fabric_mat(materials, Color::srgb(0.92, 0.78, 0.55), 1.0);
    let boot = make_fabric_mat(materials, Color::srgb(0.45, 0.28, 0.14), 1.0);
    let belt = make_fabric_mat(materials, Color::srgb(0.55, 0.32, 0.15), 1.0);
    let hair = materials.add(StandardMaterial {
        base_color: Color::srgb(0.95, 0.80, 0.30),
        perceptual_roughness: 0.75,
        ..default()
    });
    let steel = materials.add(StandardMaterial {
        // Strong metallic + low roughness so it catches SSR reflections
        // (and the sun's specular highlight) cleanly.
        base_color: Color::srgb(0.85, 0.88, 0.92),
        perceptual_roughness: 0.15,
        metallic: 0.95,
        reflectance: 0.75,
        ..default()
    });
    let leather = materials.add(StandardMaterial {
        base_color: Color::srgb(0.45, 0.25, 0.15),
        perceptual_roughness: 0.8,
        ..default()
    });

    // Meshes
    let head_mesh = meshes.add(Sphere::new(0.22).mesh().ico(3).unwrap());
    let torso_mesh = meshes.add(Cuboid::new(0.55, 0.70, 0.35));
    let belt_mesh = meshes.add(Cuboid::new(0.58, 0.08, 0.38));
    let arm_mesh = meshes.add(Capsule3d::new(0.09, 0.45));
    let hand_mesh = meshes.add(Sphere::new(0.10).mesh().ico(2).unwrap());
    let leg_mesh = meshes.add(Capsule3d::new(0.12, 0.45));
    let boot_mesh = meshes.add(Cuboid::new(0.22, 0.12, 0.30));
    let hair_mesh = meshes.add(Sphere::new(0.24).mesh().ico(3).unwrap());
    // In-hand sword meshes (blade / hilt / grip). Spawned hidden and
    // flipped to Visible by `toggle_sword_visibility` once HasSword = true.
    let sword_blade_hand = meshes.add(Cuboid::new(0.08, 0.70, 0.02));
    let sword_hilt_hand = meshes.add(Cuboid::new(0.20, 0.04, 0.04));
    let sword_grip_hand = meshes.add(Cuboid::new(0.04, 0.15, 0.04));
    let steel_hand = steel;
    let leather_hand = leather;

    // Coordinate system: +Y up, -Z forward (Bevy convention). The hero faces
    // -Z, so the sword goes on its back (+Z side).

    let torso_y = 1.00;
    let torso_rest = Transform::from_xyz(0.0, torso_y, 0.0);

    // ----- Torso (+ belt) -----
    parent
        .spawn((
            Mesh3d(torso_mesh),
            MeshMaterial3d(tunic.clone()),
            torso_rest,
            HeroLimb::Torso,
            RestTransform(torso_rest),
            Name::new("Torso"),
        ))
        .with_children(|torso| {
            torso.spawn((
                Mesh3d(belt_mesh),
                MeshMaterial3d(belt),
                Transform::from_xyz(0.0, -0.37, 0.0),
                Name::new("Belt"),
            ));
        });

    // ----- Neck (small skin cylinder under the head) -----
    let neck_mesh = meshes.add(Cylinder::new(0.11, 0.16).mesh().resolution(16));
    parent.spawn((
        Mesh3d(neck_mesh),
        MeshMaterial3d(skin.clone()),
        Transform::from_xyz(0.0, torso_y + 0.42, 0.0),
        Name::new("Neck"),
    ));

    // ----- Head (+ layered hair + ears) -----
    let head_rest = Transform::from_xyz(0.0, torso_y + 0.55, 0.0);
    let ear_mesh = meshes.add(Cuboid::new(0.04, 0.10, 0.04));
    // Secondary hair meshes — a slightly smaller cap for the back of the
    // skull and a flatter sphere for bangs in front. Multiple shapes
    // break the one-round-blob silhouette.
    let hair_bangs_mesh = meshes.add(Sphere::new(0.18).mesh().ico(3).unwrap());
    let hair_nape_mesh = meshes.add(Sphere::new(0.20).mesh().ico(3).unwrap());
    parent
        .spawn((
            Mesh3d(head_mesh),
            MeshMaterial3d(skin.clone()),
            head_rest,
            HeroLimb::Head,
            RestTransform(head_rest),
            Name::new("Head"),
        ))
        .with_children(|head| {
            // Main crown — squashed sphere slightly above head center.
            head.spawn((
                Mesh3d(hair_mesh),
                MeshMaterial3d(hair.clone()),
                Transform::from_xyz(0.0, 0.07, 0.02).with_scale(Vec3::new(1.05, 0.80, 1.08)),
                Name::new("HairCrown"),
            ));
            // Bangs — squished sphere tilted forward over the brow.
            head.spawn((
                Mesh3d(hair_bangs_mesh),
                MeshMaterial3d(hair.clone()),
                Transform::from_xyz(0.0, 0.08, -0.16)
                    .with_scale(Vec3::new(1.2, 0.55, 0.70))
                    .with_rotation(Quat::from_rotation_x(-0.3)),
                Name::new("HairBangs"),
            ));
            // Nape — slightly-drooped back tuft.
            head.spawn((
                Mesh3d(hair_nape_mesh),
                MeshMaterial3d(hair),
                Transform::from_xyz(0.0, -0.02, 0.18).with_scale(Vec3::new(1.0, 0.85, 0.9)),
                Name::new("HairNape"),
            ));
            for (sign, name) in [(-1.0_f32, "EarL"), (1.0, "EarR")] {
                head.spawn((
                    Mesh3d(ear_mesh.clone()),
                    MeshMaterial3d(skin.clone()),
                    Transform::from_xyz(sign * 0.18, 0.05, 0.0)
                        .with_rotation(Quat::from_rotation_z(-sign * 0.6)),
                    Name::new(name),
                ));
            }
        });

    // ----- Arms (shoulder pivot + arm + hand) -----
    for (side_sign, limb, name) in [
        (-1.0_f32, HeroLimb::ArmLeft, "ArmL"),
        (1.0, HeroLimb::ArmRight, "ArmR"),
    ] {
        let rest = Transform::from_xyz(side_sign * 0.32, torso_y + 0.25, 0.0);
        let is_right = matches!(limb, HeroLimb::ArmRight);
        parent
            .spawn((
                rest,
                Visibility::default(),
                limb,
                RestTransform(rest),
                Name::new(name),
            ))
            .with_children(|shoulder| {
                shoulder.spawn((
                    Mesh3d(arm_mesh.clone()),
                    MeshMaterial3d(skin.clone()),
                    Transform::from_xyz(0.0, -0.27, 0.0),
                ));
                shoulder.spawn((
                    Mesh3d(hand_mesh.clone()),
                    MeshMaterial3d(skin.clone()),
                    Transform::from_xyz(0.0, -0.55, 0.0),
                ));

                // Right hand: sword gripped at the hand, blade extending
                // downward along the arm. When the shoulder swings, the
                // sword follows the rotation naturally.
                if is_right {
                    // Blade: centered 0.35m below the hand so grip is at the hand.
                    shoulder.spawn((
                        Mesh3d(sword_blade_hand.clone()),
                        MeshMaterial3d(steel_hand.clone()),
                        Transform::from_xyz(0.0, -0.90, 0.0),
                        Visibility::Hidden,
                        SwordInHand,
                        Name::new("SwordInHand_Blade"),
                    ));
                    shoulder.spawn((
                        Mesh3d(sword_hilt_hand.clone()),
                        MeshMaterial3d(steel_hand.clone()),
                        Transform::from_xyz(0.0, -0.55, 0.0),
                        Visibility::Hidden,
                        SwordInHand,
                        Name::new("SwordInHand_Hilt"),
                    ));
                    shoulder.spawn((
                        Mesh3d(sword_grip_hand.clone()),
                        MeshMaterial3d(leather_hand.clone()),
                        Transform::from_xyz(0.0, -0.48, 0.0),
                        Visibility::Hidden,
                        SwordInHand,
                        Name::new("SwordInHand_Grip"),
                    ));
                }
            });
    }

    // ----- Legs (hip pivot + leg + boot) -----
    for (side_sign, limb, name) in [
        (-1.0_f32, HeroLimb::LegLeft, "LegL"),
        (1.0, HeroLimb::LegRight, "LegR"),
    ] {
        let rest = Transform::from_xyz(side_sign * 0.14, torso_y - 0.40, 0.0);
        parent
            .spawn((
                rest,
                Visibility::default(),
                limb,
                RestTransform(rest),
                Name::new(name),
            ))
            .with_children(|hip| {
                hip.spawn((
                    Mesh3d(leg_mesh.clone()),
                    MeshMaterial3d(trouser.clone()),
                    Transform::from_xyz(0.0, -0.27, 0.0),
                ));
                hip.spawn((
                    Mesh3d(boot_mesh.clone()),
                    MeshMaterial3d(boot.clone()),
                    Transform::from_xyz(0.0, -0.58, 0.05),
                ));
            });
    }

    // Back-mounted sword removed: the hero starts unarmed. The sword
    // spawns in the world as a SwordPickup (see `sword.rs`) — once the
    // player clicks it, `HasSword` flips and the in-hand meshes (spawned
    // with the right arm above) become visible.
}

/// State driving the hero's procedural animation. Kept as a component on the
/// hero root entity so multiple heroes can coexist later (NPCs, etc.).
#[derive(Component, Default)]
pub struct HeroAnimation {
    pub speed: f32,
    pub phase: f32,
    /// 0.0 = no attack, 1.0 = full swing-down. Driven by the combat system.
    pub attack_phase: f32,
}

fn toggle_sword_visibility(
    has_sword: Res<HasSword>,
    mut in_hand: Query<&mut Visibility, With<SwordInHand>>,
) {
    let target = if has_sword.0 {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    for mut v in &mut in_hand {
        *v = target;
    }
}

fn animate_hero(
    time: Res<Time>,
    mut anim_q: Query<&mut HeroAnimation>,
    mut limbs: Query<(&HeroLimb, &RestTransform, &mut Transform)>,
) {
    let Ok(mut anim) = anim_q.single_mut() else {
        return;
    };
    // Phase advances faster when the hero moves faster. Tuned so that
    // walking (speed = 5.5) produces ~2 steps/s (6.3 rad/s) — a natural
    // human cadence — and running (9.5) ~3 steps/s. Much snappier than
    // the earlier sauntering pace.
    let cadence = 2.0 + anim.speed * 0.78;
    anim.phase += cadence * time.delta_secs();
    let phase = anim.phase;

    // `intent` is 0 when idle, 1 when walking, > 1 when running (clamped).
    let intent = (anim.speed / 5.5).clamp(0.0, 1.8);

    for (limb, rest, mut tf) in &mut limbs {
        let rest = rest.0;
        match limb {
            HeroLimb::ArmLeft => {
                let swing = -phase.sin() * 0.9 * intent;
                tf.translation = rest.translation;
                tf.rotation = rest.rotation * Quat::from_rotation_x(swing);
            }
            HeroLimb::ArmRight => {
                tf.translation = rest.translation;
                if anim.attack_phase > 0.0 {
                    // Arm-angle convention (rotation around local X):
                    //   θ =  0       → arm hanging down (rest)
                    //   θ = -2.0     → arm up-back (overhead windup)
                    //   θ = -π       → arm straight up
                    //   θ = -5.08    → arm forward-down (strike end)
                    //                  (same orientation as +1.2 but
                    //                   reached by continuing past the
                    //                   top instead of sweeping through
                    //                   the bottom).
                    //
                    // By interpolating the strike from -2.0 to -5.08 the
                    // sword travels OVER the head and down through the
                    // front — a real overhead slash, top-to-bottom.
                    // Going -2.0 → +1.2 (the old path) instead swung
                    // the arm behind the body then forward, which read
                    // as "rising from the bottom" on screen.
                    //
                    //   0.00..0.10 windup   : rest   → overhead  (0 → -2.0)
                    //   0.10..0.60 strike   : overhead → down-fwd (-2.0 → -5.08)
                    //   0.60..1.00 recovery : down-fwd → rest    (+1.2 → 0)
                    let p = anim.attack_phase;
                    let swing_angle = if p < 0.10 {
                        -2.0 * (p / 0.10)
                    } else if p < 0.60 {
                        const STRIKE_SPAN: f32 = -5.08 - (-2.0); // -3.08
                        -2.0 + STRIKE_SPAN * ((p - 0.10) / 0.50)
                    } else {
                        1.2 * (1.0 - (p - 0.60) / 0.40)
                    };
                    tf.rotation = rest.rotation * Quat::from_rotation_x(swing_angle);
                } else {
                    let swing = phase.sin() * 0.9 * intent;
                    tf.rotation = rest.rotation * Quat::from_rotation_x(swing);
                }
            }
            HeroLimb::LegLeft => {
                let swing = phase.sin() * 0.8 * intent;
                tf.translation = rest.translation;
                tf.rotation = rest.rotation * Quat::from_rotation_x(swing);
            }
            HeroLimb::LegRight => {
                let swing = -phase.sin() * 0.8 * intent;
                tf.translation = rest.translation;
                tf.rotation = rest.rotation * Quat::from_rotation_x(swing);
            }
            HeroLimb::Torso => {
                // Slight vertical bob (double frequency = one per step).
                let bob = (phase * 2.0).sin() * 0.04 * intent;
                // Idle breathing when standing.
                let breath = (time.elapsed_secs() * 1.8).sin() * 0.015 * (1.0 - intent.min(1.0));
                tf.translation = rest.translation + Vec3::Y * (bob + breath);
                tf.rotation = rest.rotation;
            }
            HeroLimb::Head => {
                // Counter-bob keeps the head steady above the bobbing torso.
                let bob = (phase * 2.0).sin() * 0.02 * intent;
                tf.translation = rest.translation + Vec3::Y * bob;
                tf.rotation = rest.rotation;
            }
        }
    }
}
