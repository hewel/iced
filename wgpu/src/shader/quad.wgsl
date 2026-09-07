struct Globals {
    transform: mat4x4<f32>,
    scale: f32,
}

@group(0) @binding(0) var<uniform> globals: Globals;

fn quad_bounds(bounds: vec4<f32>, snap: bool) -> vec4<f32> {
    // A non-finite physical box cannot be submitted to the rasterizer.
    let limit = 3.402823e38 / max(globals.scale, 1.0);
    if any(abs(bounds) > vec4(limit)) {
        return vec4(0.0);
    }
    return shape_snap(bounds * globals.scale, snap);
}

fn quad_shadow_offset(offset: vec2<f32>) -> vec2<f32> {
    let limit = 3.402823e38 / max(globals.scale, 1.0);
    return clamp(offset, vec2(-limit), vec2(limit)) * globals.scale;
}

fn quad_shadow_blur(blur: f32) -> f32 {
    return min(blur, 3.402823e38 / max(globals.scale, 1.0)) * globals.scale;
}

fn quad_vertex(bounds: vec4<f32>, offset: vec2<f32>, blur: f32, index: u32) -> vec4<f32> {
    var point = bounds.xy;
    if all(bounds.zw > vec2(0.0)) {
        let lo = min(bounds.xy, bounds.xy + offset - blur) - 0.5;
        let hi = max(bounds.xy + bounds.zw, bounds.xy + bounds.zw + offset + blur) + 0.5;
        point = mix(lo, hi, vertex_position(index));
    }
    return globals.transform * vec4(point, 0.0, 1.0);
}

fn quad_color(point: vec2<f32>, bounds: vec4<f32>, radii: vec4<f32>, smoothing: f32,
    width: f32, fill: vec4<f32>, border: vec4<f32>, shadow: vec4<f32>,
    offset: vec2<f32>, blur: f32) -> vec4<f32> {
    let distance = shape_distance(point, bounds, radii, smoothing);
    let outer = shape_coverage(distance);
    var inner = outer;
    if width > 0.0 {
        inner = shape_coverage(distance + width);
        if width >= min(bounds.z, bounds.w) * 0.5 {
            inner = 0.0;
        }
    }
    var color = fill * inner + border * max(outer - inner, 0.0);
    if shadow.a > 0.0 {
        let ds = shape_distance(point - offset, bounds, radii, smoothing);
        var coverage = shape_coverage(ds);
        if blur > 0.0 {
            coverage = 1.0 - smoothstep(-blur, blur, ds);
        }
        color += shadow * coverage * (1.0 - outer);
    }
    return color;
}
