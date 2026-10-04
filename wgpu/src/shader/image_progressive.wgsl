// The broad-kernel fallback reads the fragmented atlas directly. It trades
// extra samples for bounded storage when a convolution halo cannot fit a tile.
@group(0) @binding(0) var image: texture_2d_array<f32>;

struct Parameters {
    reference: vec4<f32>,
    transition: vec4<f32>,
    output: vec4<f32>,
    fragments: vec4<f32>,
}
struct Fragment {
    tile: vec4<f32>,
    atlas: vec4<f32>,
    flags: vec4<f32>,
}
@group(1) @binding(0) var<uniform> parameters: Parameters;
@group(1) @binding(1) var fragments: texture_2d<f32>;

fn descriptor_texel(index: u32) -> vec4<f32> {
    let width = textureDimensions(fragments).x;
    return textureLoad(fragments, vec2<i32>(i32(index % width), i32(index / width)), 0);
}

@vertex fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corners = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(corners[index], 0.0, 1.0);
}

fn premultiply(sample: vec4<f32>) -> vec4<f32> {
    return vec4(sample.rgb * sample.a, sample.a);
}

fn load_premultiplied(pixel: vec2<i32>, layer: i32) -> vec4<f32> {
    let dimensions = vec2<i32>(textureDimensions(image));
    return premultiply(textureLoad(image, clamp(pixel, vec2(0), dimensions - vec2(1)), layer, 0));
}

fn source(point: vec2<f32>) -> vec4<f32> {
    // Clamp to the cropped image's domain before selecting an atlas fragment.
    // Cropped exterior edges clamp to source texel centers below; internal
    // fragment edges keep their adjacent-source gutters for interpolation.
    let p = clamp(point, vec2(0.0), parameters.reference.xy);
    let dimensions = vec2<f32>(textureDimensions(image));
    let half_texel = vec2(0.5) / dimensions;
    for (var index = 0u; index < u32(parameters.fragments.x); index++) {
        let fragment = Fragment(descriptor_texel(index * 3u), descriptor_texel(index * 3u + 1u), descriptor_texel(index * 3u + 2u));
        let edges = u32(fragment.flags.y);
        let high = fragment.tile.xy + fragment.tile.zw;
        if ((edges & 1u) == 0u && p.x < fragment.tile.x)
            || ((edges & 2u) == 0u && p.y < fragment.tile.y)
            || ((edges & 4u) == 0u && p.x >= high.x)
            || ((edges & 8u) == 0u && p.y >= high.y) {
            continue;
        }
        var uv = fragment.atlas.xy + (p - fragment.tile.xy) / fragment.tile.zw * fragment.atlas.zw;
        if (edges & 1u) != 0u { uv.x = max(uv.x, fragment.atlas.x + half_texel.x); }
        if (edges & 2u) != 0u { uv.y = max(uv.y, fragment.atlas.y + half_texel.y); }
        if (edges & 4u) != 0u { uv.x = min(uv.x, fragment.atlas.x + fragment.atlas.z - half_texel.x); }
        if (edges & 8u) != 0u { uv.y = min(uv.y, fragment.atlas.y + fragment.atlas.w - half_texel.y); }
        if parameters.transition.w > 0.5 {
            return load_premultiplied(vec2<i32>(uv * dimensions), i32(fragment.flags.x));
        }
        let texel = uv * dimensions - vec2(0.5);
        let low = vec2<i32>(floor(texel));
        let fraction = fract(texel);
        let layer = i32(fragment.flags.x);
        return mix(
            mix(load_premultiplied(low, layer), load_premultiplied(low + vec2(1, 0), layer), fraction.x),
            mix(load_premultiplied(low + vec2(0, 1), layer), load_premultiplied(low + vec2(1, 1), layer), fraction.x),
            fraction.y,
        );
    }
    return vec4(0.0);
}

@fragment fn fs_main(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let point = pixel.xy + parameters.output.xy;
    let position = select(point.y / parameters.reference.y, point.x / parameters.reference.x, parameters.transition.z > 0.5);
    let start = parameters.transition.x;
    let end = parameters.transition.y;
    var fraction = select(0.0, 1.0, position >= start);
    if end > start { fraction = clamp((position - start) / (end - start), 0.0, 1.0); }
    let sigma = mix(parameters.reference.z, parameters.reference.w, fraction);
    if sigma < 0.01 { return source(point); }
    let step = max(1.0, 3.0 * sigma / 24.0);
    var total = 0.0;
    var result = vec4(0.0);
    for (var y = -24; y <= 24; y++) {
        let dy = f32(y) * step;
        let wy = exp(-0.5 * (dy / sigma) * (dy / sigma));
        for (var x = -24; x <= 24; x++) {
            let dx = f32(x) * step;
            let weight = wy * exp(-0.5 * (dx / sigma) * (dx / sigma));
            result += source(point + vec2(dx, dy)) * weight;
            total += weight;
        }
    }
    return result / total;
}
