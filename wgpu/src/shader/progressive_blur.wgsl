@group(0) @binding(0) var image: texture_2d_array<f32>;
@group(1) @binding(0) var image_sampler: sampler;
struct Parameters {
    destination: vec4<f32>,
    region: vec4<f32>,
    source: vec4<f32>,
    profile: vec4<f32>,
    axis: vec4<f32>,
}
@group(1) @binding(1) var<uniform> parameters: Parameters;
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(p[i], 0.0, 1.0);
}
@fragment fn fs_main(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let point = parameters.region.xy
        + (p.xy - parameters.destination.xy) / parameters.destination.zw * parameters.region.zw;
    let coordinate = select(point.x, point.y, parameters.axis.y > 0.5);
    let position = (coordinate - parameters.axis.z) / parameters.axis.w;
    let start = parameters.profile.z;
    let end = parameters.profile.w;
    var fraction = select(0.0, 1.0, position >= start);
    if end > start { fraction = clamp((position - start) / (end - start), 0.0, 1.0); }
    let sigma = mix(parameters.profile.x, parameters.profile.y, fraction);
    return gaussian(point, parameters.source.xy, parameters.source.zw, sigma, parameters.axis.x > 0.5);
}
