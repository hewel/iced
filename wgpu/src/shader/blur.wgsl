@group(0) @binding(0) var image: texture_2d_array<f32>;
@group(1) @binding(0) var image_sampler: sampler;
struct Parameters {
    destination: vec4<f32>,
    source: vec4<f32>,
}
@group(1) @binding(1) var<uniform> parameters: Parameters;
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(p[i], 0.0, 1.0);
}
@fragment fn fs_main(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let params = parameters.destination;
    let dimensions = parameters.source.zw;
    let point = p.xy / params.xy * parameters.source.xy;
    return gaussian(point, parameters.source.xy, dimensions, params.z, params.w > 0.5);
}
