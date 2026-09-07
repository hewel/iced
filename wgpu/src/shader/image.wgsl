struct Globals {
    transform: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var u_sampler: sampler;
@group(1) @binding(0) var u_texture: texture_2d_array<f32>;

struct VertexInput {
    @builtin(vertex_index) vertex_index: u32,
    @location(0) bounds: vec4<f32>,
    @location(1) clip_bounds: vec4<f32>,
    @location(2) border_radius: vec4<f32>,
    @location(3) tile: vec4<f32>,
    @location(4) atlas: vec4<f32>,
    @location(5) rotation: f32,
    @location(6) opacity: f32,
    @location(7) smoothing: f32,
    @location(8) layer: u32,
    @location(9) edges: u32,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) bounds: vec4<f32>,
    @location(1) @interpolate(flat) clip_bounds: vec4<f32>,
    @location(2) @interpolate(flat) border_radius: vec4<f32>,
    @location(3) @interpolate(flat) tile: vec4<f32>,
    @location(4) @interpolate(flat) atlas: vec4<f32>,
    @location(5) @interpolate(flat) rotation: vec2<f32>,
    @location(6) @interpolate(flat) opacity: f32,
    @location(7) @interpolate(flat) smoothing: f32,
    @location(8) @interpolate(flat) layer: u32,
    @location(9) @interpolate(flat) edges: u32,
}

fn image_rotate(p: vec2<f32>, rotation: vec2<f32>) -> vec2<f32> {
    return vec2(p.x * rotation.x - p.y * rotation.y, p.x * rotation.y + p.y * rotation.x);
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let rotation = vec2(cos(input.rotation), sin(input.rotation));
    let center = input.bounds.xy + input.bounds.zw * 0.5;
    // Only content exterior edges acquire a guard. Internal seams partition
    // one image; they have neither their own shape nor their own AA ramp.
    let guard_min = vec2(select(0.0, 0.5, (input.edges & 1u) != 0u), select(0.0, 0.5, (input.edges & 2u) != 0u));
    let guard_max = vec2(select(0.0, 0.5, (input.edges & 4u) != 0u), select(0.0, 0.5, (input.edges & 8u) != 0u));
    let tile_min = input.tile.xy - guard_min;
    let tile_max = input.tile.xy + input.tile.zw + guard_max;
    let corners = array<vec2<f32>, 4>(tile_min, vec2(tile_max.x, tile_min.y), vec2(tile_min.x, tile_max.y), tile_max);
    var low = image_rotate(corners[0] - center, rotation) + center;
    var high = low;
    for (var i = 1u; i < 4u; i++) {
        let p = image_rotate(corners[i] - center, rotation) + center;
        low = min(low, p);
        high = max(high, p);
    }
    low = max(low, input.clip_bounds.xy - vec2(0.5));
    high = min(high, input.clip_bounds.xy + input.clip_bounds.zw + vec2(0.5));
    let p = low + vertex_position(input.vertex_index) * max(high - low, vec2(0.0));
    out.position = globals.transform * vec4(p, 0.0, 1.0);
    out.bounds = input.bounds;
    out.clip_bounds = input.clip_bounds;
    out.border_radius = input.border_radius;
    out.tile = input.tile;
    out.atlas = input.atlas;
    out.rotation = rotation;
    out.opacity = input.opacity;
    out.smoothing = input.smoothing;
    out.layer = input.layer;
    out.edges = input.edges;
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let fragment = input.position.xy;
    let center = input.bounds.xy + input.bounds.zw * 0.5;
    let p = image_rotate(fragment - center, vec2(input.rotation.x, -input.rotation.y)) + center;
    let tile_max = input.tile.xy + input.tile.zw;
    // Half-open ownership prevents overlapping rotated tile AABBs from
    // compositing a source pixel twice. Exterior guards belong to edge tiles.
    if ((input.edges & 1u) == 0u && p.x < input.tile.x)
        || ((input.edges & 2u) == 0u && p.y < input.tile.y)
        || ((input.edges & 4u) == 0u && p.x >= tile_max.x)
        || ((input.edges & 8u) == 0u && p.y >= tile_max.y) {
        discard;
    }
    let d_frame = shape_distance(fragment, input.clip_bounds, input.border_radius, input.smoothing);
    let q = abs(p - center) - input.bounds.zw * 0.5;
    let d_content = length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0);
    let coverage = select(shape_coverage(max(d_frame, d_content)), 1.0, (input.edges & 32u) != 0u);
    var uv = input.atlas.xy + (p - input.tile.xy) / input.tile.zw * input.atlas.zw;
    let half_texel = vec2(0.5) / vec2<f32>(textureDimensions(u_texture));
    // Clamp only at the selected crop boundary; internal seams sample the
    // adjacent-source gutters rather than repeating the fragment's edge.
    if (input.edges & 1u) != 0u { uv.x = max(uv.x, input.atlas.x + half_texel.x); }
    if (input.edges & 2u) != 0u { uv.y = max(uv.y, input.atlas.y + half_texel.y); }
    if (input.edges & 4u) != 0u { uv.x = min(uv.x, input.atlas.x + input.atlas.z - half_texel.x); }
    if (input.edges & 8u) != 0u { uv.y = min(uv.y, input.atlas.y + input.atlas.w - half_texel.y); }
    if (input.edges & 32u) != 0u {
        let dimensions = vec2<i32>(textureDimensions(u_texture));
        if (input.edges & 64u) != 0u {
            return premultiply(textureLoad(u_texture, clamp(vec2<i32>(uv * vec2<f32>(dimensions)), vec2(0), dimensions - vec2(1)), i32(input.layer), 0));
        }
        let texel = uv * vec2<f32>(dimensions) - vec2(0.5);
        let low = vec2<i32>(floor(texel));
        let fraction = fract(texel);
        let a = premultiply(textureLoad(u_texture, clamp(low, vec2(0), dimensions - vec2(1)), i32(input.layer), 0));
        let b = premultiply(textureLoad(u_texture, clamp(low + vec2(1, 0), vec2(0), dimensions - vec2(1)), i32(input.layer), 0));
        let c = premultiply(textureLoad(u_texture, clamp(low + vec2(0, 1), vec2(0), dimensions - vec2(1)), i32(input.layer), 0));
        let d = premultiply(textureLoad(u_texture, clamp(low + vec2(1, 1), vec2(0), dimensions - vec2(1)), i32(input.layer), 0));
        return mix(mix(a, b, fraction.x), mix(c, d, fraction.x), fraction.y);
    }
    let sample = textureSampleLevel(u_texture, u_sampler, uv, i32(input.layer), 0.0);
    if (input.edges & 16u) != 0u { return sample * (coverage * input.opacity); }
    return premultiply(sample * vec4(1.0, 1.0, 1.0, coverage * input.opacity));
}
