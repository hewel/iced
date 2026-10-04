@group(0) @binding(0) var original: texture_2d_array<f32>;
@group(0) @binding(1) var blurred: texture_2d_array<f32>;
@group(1) @binding(0) var image_sampler: sampler;
struct Parameters {
    original: vec4<f32>,
    tile: vec4<f32>,
    blurred: vec4<f32>,
    bounds: vec4<f32>,
    radii: vec4<f32>,
    smoothing: vec4<f32>,
    clip: vec4<f32>,
}
@group(1) @binding(1) var<uniform> parameters: Parameters;
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(p[i], 0.0, 1.0);
}
@fragment fn fs_main(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let source = textureSampleLevel(original, image_sampler, p.xy / parameters.original.zw, 0, 0.0);
    if any(p.xy < parameters.clip.xy) || any(p.xy >= parameters.clip.xy + parameters.clip.zw) {
        return source;
    }
    let uv = (p.xy - parameters.tile.xy) / parameters.blurred.zw;
    let filtered = textureSampleLevel(blurred, image_sampler, uv, 0, 0.0);
    let coverage = shape_coverage(shape_distance(p.xy, parameters.bounds, parameters.radii, parameters.smoothing.x));
    // Replace premultiplied pixels. Source-over would composite a translucent
    // lower scene twice, instead of filtering its existing contribution.
    return mix(source, filtered, coverage);
}
