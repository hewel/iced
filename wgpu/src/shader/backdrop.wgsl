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
    sample_region: vec4<f32>,
    optics: vec4<f32>,
    light: vec4<f32>,
    tint: vec4<f32>,
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
    let distance = shape_distance(p.xy, parameters.bounds, parameters.radii, parameters.smoothing.x);
    let coverage = shape_coverage(distance);
    if coverage <= 0.0 { return source; }
    var normal = vec2(0.0);
    var edge = 0.0;
    if parameters.light.z > 0.5 && parameters.optics.y > 0.0 {
        let q = clamp(1.0 + distance / max(parameters.optics.y, 0.001), 0.0, 1.0);
        edge = q * q * (3.0 - 2.0 * q);
        if edge > 0.0 {
            // Average the surface slope across the bevel. Keep its magnitude
            // so opposing edge normals fade together instead of forming an
            // interior fold when the optical depth exceeds a corner radius.
            let span = max(parameters.optics.y * 0.5, 0.5);
            let dx = shape_distance(p.xy + vec2(span, 0.0), parameters.bounds, parameters.radii, parameters.smoothing.x)
                - shape_distance(p.xy - vec2(span, 0.0), parameters.bounds, parameters.radii, parameters.smoothing.x);
            let dy = shape_distance(p.xy + vec2(0.0, span), parameters.bounds, parameters.radii, parameters.smoothing.x)
                - shape_distance(p.xy - vec2(0.0, span), parameters.bounds, parameters.radii, parameters.smoothing.x);
            let gradient = vec2(dx, dy);
            normal = gradient / max(2.0 * span, length(gradient));
        }
    }
    let refracted = p.xy - normal * parameters.optics.x * edge;
    let uv = (refracted - parameters.sample_region.xy) / parameters.sample_region.zw
        * parameters.blurred.xy / parameters.blurred.zw;
    let low = vec2(0.5) / parameters.blurred.zw;
    let high = (parameters.blurred.xy - vec2(0.5)) / parameters.blurred.zw;
    var filtered = textureSampleLevel(blurred, image_sampler, clamp(uv, low, high), 0, 0.0);
    if parameters.light.z > 0.5 {
        filtered = parameters.tint + filtered * (1.0 - parameters.tint.a);
        let facing = dot(normal, parameters.light.xy);
        let highlight = parameters.optics.z * max(facing, 0.0) * edge;
        let shadow = parameters.optics.w * max(-facing, 0.0) * edge;
        filtered = vec4(mix(filtered.rgb, vec3(filtered.a), highlight) * (1.0 - shadow), filtered.a);
    }
    // Replace premultiplied pixels. Source-over would composite a translucent
    // lower scene twice, instead of filtering its existing contribution.
    return mix(source, filtered, coverage);
}
