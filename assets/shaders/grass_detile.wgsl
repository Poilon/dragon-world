// Replace the StandardMaterial fragment with a de-tiled variant for the
// terrain. The base material still samples the grass albedo at its tiled
// scale (UV × tile_count via `uv_transform`) for fine detail; this shader
// adds a second sample at the *raw* mesh UV (0..1 across the whole
// terrain) and uses it as a per-area color tint. Since the macro sample
// doesn't repeat, every patch of ground gets a unique tint — the "same
// tile 50× in a row" look goes away.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
    pbr_bindings,
    mesh_view_bindings::view,
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // Macro sample: albedo stretched once across the whole mesh. Doesn't
    // repeat, so every area of the terrain reads a different slice of
    // the texture.
    let macro_sample = textureSampleBias(
        pbr_bindings::base_color_texture,
        pbr_bindings::base_color_sampler,
        in.uv,
        view.mip_bias,
    );

    // Convert the macro sample into a tint that shifts hue without
    // dramatically changing brightness: divide out its luminance, mix
    // toward 1.0.
    let lum = dot(macro_sample.rgb, vec3<f32>(0.299, 0.587, 0.114));
    let tint = mix(vec3<f32>(1.0), macro_sample.rgb / max(lum, 0.05), 0.55);

    var base_color = pbr_input.material.base_color;
    base_color = vec4<f32>(base_color.rgb * tint, base_color.a);
    base_color = alpha_discard(pbr_input.material, base_color);
    pbr_input.material.base_color = base_color;

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
