// Broad kernels are separable in the cropped source's reconstruction basis.
// The first axis retains physical output resolution; the other uses native
// texel centers and is streamed through bounded 16-bit fixed-point stripes.
@group(0) @binding(0) var image: texture_2d_array<f32>;

struct Parameters {
    reference: vec4<f32>,
    transition: vec4<f32>,
    output: vec4<f32>,
    fragments: vec4<f32>,
    stripe: vec4<f32>,
    grid: vec4<f32>,
}
struct Fragment {
    tile: vec4<f32>,
    atlas: vec4<f32>,
    flags: vec4<f32>,
}
@group(1) @binding(0) var<uniform> parameters: Parameters;
@group(1) @binding(1) var fragments: texture_2d<f32>;
@group(1) @binding(2) var accumulation: texture_2d_array<f32>;

struct PackedColor {
    @location(0) high: vec4<f32>,
    @location(1) low: vec4<f32>,
}

fn pack_color(value: vec4<f32>) -> PackedColor {
    // Arithmetic byte packing also works on GLSL 330 / WebGL2 GLSL ES 300;
    // their native packHalf/packUnorm built-ins are not universally available.
    let fixed = floor(clamp(value, vec4(0.0), vec4(1.0)) * 65535.0 + vec4(0.5));
    let high = floor(fixed / 256.0);
    return PackedColor(high / 255.0, (fixed - high * 256.0) / 255.0);
}

fn unpack_color(high: vec4<f32>, low: vec4<f32>) -> vec4<f32> {
    return (floor(high * 255.0 + vec4(0.5)) * 256.0 + floor(low * 255.0 + vec4(0.5))) / 65535.0;
}

fn output_point(pixel: vec2<f32>) -> vec2<f32> {
    return parameters.output.xy + pixel / parameters.grid.xy * parameters.output.zw;
}

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

fn radius(point: vec2<f32>) -> f32 {
    let position = select(point.y / parameters.reference.y, point.x / parameters.reference.x, parameters.transition.z > 0.5);
    let start = parameters.transition.x;
    let end = parameters.transition.y;
    var fraction = select(0.0, 1.0, position >= start);
    if end > start { fraction = clamp((position - start) / (end - start), 0.0, 1.0); }
    return mix(parameters.reference.z, parameters.reference.w, fraction);
}

fn density(z: f32) -> f32 {
    return 0.3989422804 * exp(-0.5 * z * z);
}

fn distribution(z: f32) -> f32 {
    // Error below 1.5e-7; the tail form avoids subtracting nearby ones.
    let x = abs(z) * 0.7071067812;
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let tail = 0.5 * t * (0.254829592 + t * (-0.284496736 + t * (1.421413741 + t * (-1.453152027 + t * 1.061405429)))) * exp(-x * x);
    return select(tail, 1.0 - tail, z >= 0.0);
}

fn integrated_distribution(x: f32, sigma: f32) -> f32 {
    let z = x / sigma;
    return x * distribution(z) + sigma * density(z);
}

fn left_weight(center: f32, sigma: f32) -> f32 {
    if parameters.transition.w > 0.5 {
        return distribution((0.5 - center) / sigma);
    }
    if sigma < 1.0 {
        return max(0.0, integrated_distribution(1.0 - center, sigma) - integrated_distribution(-center, sigma));
    }
    // Integrating the ramp's CDF avoids cancellation between O(sigma)
    // primitives when the native radius is very large.
    return (5.0 / 18.0) * distribution((0.1127016654 - center) / sigma)
        + (4.0 / 9.0) * distribution((0.5 - center) / sigma)
        + (5.0 / 18.0) * distribution((0.8872983346 - center) / sigma);
}

fn native_weight(index: i32, center: f32, sigma: f32, count: i32) -> f32 {
    if count == 1 { return 1.0; }
    if sigma <= 0.0 {
        let clamped = clamp(center, 0.0, f32(count - 1));
        if parameters.transition.w > 0.5 {
            return select(0.0, 1.0, index == i32(floor(clamped + 0.5)));
        }
        return max(0.0, 1.0 - abs(f32(index) - clamped));
    }
    if index == 0 { return left_weight(center, sigma); }
    if index == count - 1 { return left_weight(f32(count - 1) - center, sigma); }
    let d = f32(index) - center;
    if sigma < 1.0 {
        if parameters.transition.w > 0.5 {
            return max(0.0, distribution((d + 0.5) / sigma) - distribution((d - 0.5) / sigma));
        }
        return max(0.0, integrated_distribution(d + 1.0, sigma)
            - 2.0 * integrated_distribution(d, sigma)
            + integrated_distribution(d - 1.0, sigma));
    }
    // Positive three-point quadrature is stable for wide kernels. It
    // integrates a native texel's box/triangle reconstruction footprint,
    // instead of sparsely sampling an unfiltered source at a growing stride.
    let nodes = array<f32, 3>(0.1127016654, 0.5, 0.8872983346);
    let weights = array<f32, 3>(5.0 / 18.0, 4.0 / 9.0, 5.0 / 18.0);
    var weight = 0.0;
    for (var i = 0u; i < 3u; i++) {
        let u = nodes[i];
        if parameters.transition.w > 0.5 {
            weight += weights[i] * density((d + u - 0.5) / sigma) / sigma;
        } else {
            weight += weights[i] * (1.0 - u)
                * (density((d - u) / sigma) + density((d + u) / sigma)) / sigma;
        }
    }
    return weight;
}

fn native_limits(center: f32, sigma: f32, count: i32) -> vec2<i32> {
    // Four sigmas omit less than 0.007% of Gaussian mass. Clamp tails are
    // included in the first/last native texel instead of iterated virtually.
    let support = 4.0 * sigma + 1.0;
    return vec2(i32(clamp(floor(center - support), 0.0, f32(count - 1))), i32(clamp(ceil(center + support), 0.0, f32(count - 1))));
}

@fragment fn fs_first(@builtin(position) pixel: vec4<f32>) -> PackedColor {
    let horizontal = parameters.transition.z > 0.5;
    let native_size = parameters.fragments.yz;
    let native_b = parameters.stripe.x + pixel.y;
    let physical_a = select(parameters.output.y, parameters.output.x, horizontal)
        + pixel.x * select(parameters.output.w / parameters.grid.y, parameters.output.z / parameters.grid.x, horizontal);
    var point = vec2(native_b * parameters.reference.x / native_size.x, physical_a);
    if horizontal { point = vec2(physical_a, native_b * parameters.reference.y / native_size.y); }
    let physical_sigma = radius(point);
    if physical_sigma < 0.01 { return pack_color(vec4(0.0)); }
    let count = i32(select(native_size.y, native_size.x, horizontal));
    let ratio = f32(count) / select(parameters.reference.y, parameters.reference.x, horizontal);
    let sigma = physical_sigma * ratio;
    let center = physical_a * ratio - 0.5;
    let limits = native_limits(center, sigma, count);
    var result = vec4(0.0);
    for (var index = limits.x; index <= limits.y; index++) {
        let physical = (f32(index) + 0.5) / ratio;
        var sample = vec2(point.x, physical);
        if horizontal { sample = vec2(physical, point.y); }
        result += source(sample) * native_weight(index, center, sigma, count);
    }
    return pack_color(result);
}

@fragment fn fs_accumulate(@builtin(position) pixel: vec4<f32>) -> PackedColor {
    let horizontal = parameters.transition.z > 0.5;
    let point = output_point(pixel.xy);
    let physical_sigma = radius(point);
    if physical_sigma < 0.01 { return pack_color(vec4(0.0)); }
    let count = i32(select(parameters.fragments.y, parameters.fragments.z, horizontal));
    let ratio = f32(count) / select(parameters.reference.x, parameters.reference.y, horizontal);
    let center = select(point.x, point.y, horizontal) * ratio - 0.5;
    let sigma = physical_sigma * ratio;
    let limits = native_limits(center, sigma, count);
    let begin = max(limits.x, i32(parameters.stripe.x));
    let end = min(limits.y, i32(parameters.stripe.x + parameters.stripe.y) - 1);
    let a = i32(select(pixel.y, pixel.x, horizontal));
    var result = vec4(0.0);
    if parameters.stripe.w > 0.5 {
        let p = vec2<i32>(pixel.xy);
        result = unpack_color(textureLoad(accumulation, p, 0, 0), textureLoad(accumulation, p, 1, 0));
    }
    for (var index = begin; index <= end; index++) {
        let p = vec2(a, index - i32(parameters.stripe.x));
        let texel = unpack_color(textureLoad(image, p, 0, 0), textureLoad(image, p, 1, 0));
        result += texel * native_weight(index, center, sigma, count);
    }
    return pack_color(result);
}

@fragment fn fs_resolve(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let point = output_point(pixel.xy);
    if radius(point) < 0.01 { return source(point); }
    let p = vec2<i32>(pixel.xy);
    return unpack_color(textureLoad(accumulation, p, 0, 0), textureLoad(accumulation, p, 1, 0));
}
