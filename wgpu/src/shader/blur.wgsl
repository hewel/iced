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
    let low = vec2(0.5) / dimensions;
    let high = (parameters.source.xy - vec2(0.5)) / dimensions;
    let uv = p.xy / params.xy * parameters.source.xy / dimensions;
    if params.z < 0.01 { return textureSampleLevel(image, image_sampler, clamp(uv, low, high), 0, 0.0); }
    let direction = select(vec2(0.0, 1.0), vec2(1.0, 0.0), params.w > 0.5) / dimensions;
    let step = max(1.0, params.z * 3.0 / 24.0);
    var result = vec4(0.0);
    var total = 0.0;
    for (var i = -24; i <= 24; i++) {
        let distance = f32(i) * step;
        let weight = exp(-0.5 * distance * distance / (params.z * params.z));
        result += textureSampleLevel(image, image_sampler, clamp(uv + direction * distance, low, high), 0, 0.0) * weight;
        total += weight;
    }
    return result / total;
}
