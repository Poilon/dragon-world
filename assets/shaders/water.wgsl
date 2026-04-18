// Animated water surface. Takes the StandardMaterial lighting pipeline
// and substitutes:
//   - a time-scrolling multi-octave value-noise height field for the
//     normal (waves), via finite-difference derivatives,
//   - a Fresnel-based alpha boost so the water is transparent when
//     looking straight down into it and opaque/reflective at grazing.
//
// Reflections come from the usual StandardMaterial path — with bloom,
// sun, and atmosphere the wet surface catches the sky colors.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::{view, globals},
}

fn hash12(p: vec2<f32>) -> f32 {
    let h = sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.547;
    return fract(h);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let a = hash12(i);
    let b = hash12(i + vec2<f32>(1.0, 0.0));
    let c = hash12(i + vec2<f32>(0.0, 1.0));
    let d = hash12(i + vec2<f32>(1.0, 1.0));
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

/// Three octaves of noise at different scales, each scrolled in a
/// different direction. The mix gives waves that interfere and keep
/// changing shape instead of just drifting.
fn wave_height(p: vec2<f32>, t: f32) -> f32 {
    let p1 = p * 0.85 + vec2<f32>( t * 0.30,  t * 0.22);
    let p2 = p * 2.30 + vec2<f32>(-t * 0.42,  t * 0.36);
    let p3 = p * 5.10 + vec2<f32>( t * 0.82, -t * 0.55);
    return value_noise(p1) * 0.60
         + value_noise(p2) * 0.30
         + value_noise(p3) * 0.10;
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    let t = globals.time;
    let wp = in.world_position.xz;

    // Central difference for the horizontal gradient of the wave height.
    let eps: f32 = 0.12;
    let h0 = wave_height(wp, t);
    let dx = (wave_height(wp + vec2<f32>(eps, 0.0), t) - h0) / eps;
    let dz = (wave_height(wp + vec2<f32>(0.0, eps), t) - h0) / eps;

    // Tangent-space normal from the slope, then tilt it so water lit by
    // the sun shows plausible specular movement.
    let wave_strength: f32 = 0.65;
    let n = normalize(vec3<f32>(-dx * wave_strength, 1.0, -dz * wave_strength));

    pbr_input.N = n;
    pbr_input.world_normal = n;

    // Fresnel: cos of the angle between view and normal. 0 = looking
    // straight down, 1 = grazing. Push alpha toward 1 at grazing so the
    // surface reads as opaque / reflective at shallow angles.
    let v = normalize(view.world_position - in.world_position.xyz);
    let cos_theta = clamp(dot(n, v), 0.0, 1.0);
    let fresnel = pow(1.0 - cos_theta, 5.0);
    let base_alpha = pbr_input.material.base_color.a;
    let final_alpha = clamp(base_alpha + fresnel * 0.7, 0.0, 1.0);

    pbr_input.material.base_color = vec4<f32>(
        pbr_input.material.base_color.rgb,
        final_alpha,
    );
    pbr_input.material.base_color =
        alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
