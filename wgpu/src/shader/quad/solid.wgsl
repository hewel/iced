struct SolidVertexInput {
    @builtin(vertex_index) vertex_index: u32,
    @location(0) color: vec4<f32>,
    @location(1) pos: vec2<f32>,
    @location(2) scale: vec2<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) border_radius: vec4<f32>,
    @location(5) border_width: f32,
    @location(6) shadow_color: vec4<f32>,
    @location(7) shadow_offset: vec2<f32>,
    @location(8) shadow_blur_radius: f32,
    @location(9) snap: u32,
    @location(10) smoothing: f32,
}

struct SolidVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) color: vec4<f32>,
    @location(1) @interpolate(flat) border_color: vec4<f32>,
    @location(2) @interpolate(flat) bounds: vec4<f32>,
    @location(3) @interpolate(flat) border_radius: vec4<f32>,
    @location(4) @interpolate(flat) border_width: f32,
    @location(5) @interpolate(flat) shadow_color: vec4<f32>,
    @location(6) @interpolate(flat) shadow_and_smoothing: vec4<f32>,
}

@vertex
fn solid_vs_main(input: SolidVertexInput) -> SolidVertexOutput {
    var out: SolidVertexOutput;
    let bounds = quad_bounds(vec4(input.pos, input.scale), input.snap != 0u);
    let cap = max(min(bounds.z, bounds.w) * 0.5, 0.0);
    let offset = quad_shadow_offset(input.shadow_offset);
    let blur = quad_shadow_blur(input.shadow_blur_radius);
    out.position = quad_vertex(bounds, offset, blur, input.vertex_index);
    out.color = premultiply(input.color);
    out.border_color = premultiply(input.border_color);
    out.bounds = bounds;
    out.border_radius = min(input.border_radius, vec4(cap / globals.scale)) * globals.scale;
    out.border_width = min(input.border_width, cap / globals.scale) * globals.scale;
    if input.border_width >= cap / globals.scale {
        out.border_width = cap;
    }
    out.shadow_color = premultiply(input.shadow_color);
    out.shadow_and_smoothing = vec4(offset, blur, input.smoothing);
    return out;
}

@fragment
fn solid_fs_main(input: SolidVertexOutput) -> @location(0) vec4<f32> {
    return quad_color(input.position.xy, input.bounds, input.border_radius,
        input.shadow_and_smoothing.w, input.border_width, input.color, input.border_color,
        input.shadow_color, input.shadow_and_smoothing.xy, input.shadow_and_smoothing.z);
}
