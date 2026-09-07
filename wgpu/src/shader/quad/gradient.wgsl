struct GradientVertexInput {
    @builtin(vertex_index) vertex_index: u32,
    @location(0) @interpolate(flat) colors_1: vec4<u32>,
    @location(1) @interpolate(flat) colors_2: vec4<u32>,
    @location(2) @interpolate(flat) colors_3: vec4<u32>,
    @location(3) @interpolate(flat) colors_4: vec4<u32>,
    @location(4) @interpolate(flat) offsets: vec4<u32>,
    @location(5) direction: vec4<f32>,
    @location(6) position_and_scale: vec4<f32>,
    @location(7) border_color: vec4<f32>,
    @location(8) border_radius: vec4<f32>,
    @location(9) border_width: f32,
    @location(10) shadow_color: vec4<f32>,
    @location(11) shadow_offset: vec2<f32>,
    @location(12) shadow_blur_radius: f32,
    @location(13) snap: u32,
    @location(14) smoothing: f32,
}

struct GradientVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) colors_1: vec4<u32>,
    @location(1) @interpolate(flat) colors_2: vec4<u32>,
    @location(2) @interpolate(flat) colors_3: vec4<u32>,
    @location(3) @interpolate(flat) colors_4: vec4<u32>,
    @location(4) @interpolate(flat) offsets: vec4<u32>,
    @location(5) @interpolate(flat) direction: vec4<f32>,
    @location(6) @interpolate(flat) position_and_scale: vec4<f32>,
    @location(7) @interpolate(flat) border_color: vec4<f32>,
    @location(8) @interpolate(flat) border_radius: vec4<f32>,
    @location(9) @interpolate(flat) border_width: f32,
    @location(10) @interpolate(flat) shadow_color: vec4<f32>,
    @location(11) @interpolate(flat) shadow_and_smoothing: vec4<f32>,
}

@vertex
fn gradient_vs_main(input: GradientVertexInput) -> GradientVertexOutput {
    var out: GradientVertexOutput;

    let bounds = quad_bounds(input.position_and_scale, input.snap != 0u);
    let cap = max(min(bounds.z, bounds.w) * 0.5, 0.0);
    let offset = quad_shadow_offset(input.shadow_offset);
    let blur = quad_shadow_blur(input.shadow_blur_radius);
    out.position = quad_vertex(bounds, offset, blur, input.vertex_index);
    out.colors_1 = input.colors_1;
    out.colors_2 = input.colors_2;
    out.colors_3 = input.colors_3;
    out.colors_4 = input.colors_4;
    out.offsets = input.offsets;
    out.direction = input.direction * globals.scale;
    out.position_and_scale = bounds;
    out.border_color = premultiply(input.border_color);
    out.border_radius = min(input.border_radius, vec4(cap / globals.scale)) * globals.scale;
    out.border_width = min(input.border_width, cap / globals.scale) * globals.scale;
    if input.border_width >= cap / globals.scale {
        out.border_width = cap;
    }
    out.shadow_color = premultiply(input.shadow_color);
    out.shadow_and_smoothing = vec4(offset, blur, input.smoothing);

    return out;
}

fn random(coords: vec2<f32>) -> f32 {
    return fract(sin(dot(coords, vec2(12.9898,78.233))) * 43758.5453);
}

/// Returns the current interpolated color with a max 8-stop gradient
fn gradient(
    raw_position: vec2<f32>,
    direction: vec4<f32>,
    colors: array<vec4<f32>, 8>,
    offsets: array<f32, 8>,
    last_index: i32
) -> vec4<f32> {
    let start = direction.xy;
    let end = direction.zw;

    let v1 = end - start;
    let v2 = raw_position - start;
    let unit = normalize(v1);
    let coord_offset = dot(unit, v2) / length(v1);

    //need to store these as a var to use dynamic indexing in a loop
    //this is already added to wgsl spec but not in wgpu yet
    var colors_arr = colors;
    var offsets_arr = offsets;

    var color: vec4<f32>;

    let noise_granularity: f32 = 0.3/255.0;

    for (var i: i32 = 0; i < last_index; i++) {
        let curr_offset = offsets_arr[i];
        let next_offset = offsets_arr[i+1];

        if (coord_offset <= offsets_arr[0]) {
            color = colors_arr[0];
        }

        if (curr_offset <= coord_offset && coord_offset <= next_offset) {
            let from_ = colors_arr[i];
            let to_ = colors_arr[i+1];
            let factor = smoothstep(curr_offset, next_offset, coord_offset);

            color = interpolate_color(from_, to_, factor);
        }

        if (coord_offset >= offsets_arr[last_index]) {
            color = colors_arr[last_index];
        }
    }

    return color + mix(-noise_granularity, noise_granularity, random(raw_position));
}

@fragment
fn gradient_fs_main(input: GradientVertexOutput) -> @location(0) vec4<f32> {
    let colors = array<vec4<f32>, 8>(
        unpack_color(input.colors_1.xy),
        unpack_color(input.colors_1.zw),
        unpack_color(input.colors_2.xy),
        unpack_color(input.colors_2.zw),
        unpack_color(input.colors_3.xy),
        unpack_color(input.colors_3.zw),
        unpack_color(input.colors_4.xy),
        unpack_color(input.colors_4.zw),
    );

    let offsets_1: vec4<f32> = unpack_u32(input.offsets.xy);
    let offsets_2: vec4<f32> = unpack_u32(input.offsets.zw);

    var offsets = array<f32, 8>(
        offsets_1.x,
        offsets_1.y,
        offsets_1.z,
        offsets_1.w,
        offsets_2.x,
        offsets_2.y,
        offsets_2.z,
        offsets_2.w,
    );

    //TODO could just pass this in to the shader but is probably more performant to just check it here
    var last_index = 7;
    for (var i: i32 = 0; i <= 7; i++) {
        if (offsets[i] > 1.0) {
            last_index = i - 1;
            break;
        }
    }

    let fill = gradient(input.position.xy, input.direction, colors, offsets, last_index);
    return quad_color(input.position.xy, input.position_and_scale, input.border_radius,
        input.shadow_and_smoothing.w, input.border_width, fill, input.border_color,
        input.shadow_color, input.shadow_and_smoothing.xy, input.shadow_and_smoothing.z);
}
