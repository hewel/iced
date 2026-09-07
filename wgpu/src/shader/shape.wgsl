// Physical pixel geometry. Radii order: top-left, top-right, bottom-right, bottom-left.
// Uploaders reject non-finite bounds and normalize parameters before scaling.
fn shape_coverage(distance: f32) -> f32 {
    return clamp(0.5 - distance, 0.0, 1.0);
}

fn shape_snap(bounds: vec4<f32>, enabled: bool) -> vec4<f32> {
    if !enabled { return bounds; }
    // Rust round uses ties away from zero; WGSL round uses ties to even.
    let edges = vec4(bounds.xy, bounds.xy + bounds.zw) + vec4(0.001);
    let snapped = sign(edges) * floor(abs(edges) + vec4(0.5));
    return vec4(snapped.xy, snapped.zw - snapped.xy);
}

fn shape_segment_distance(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    return length(p - clamp(p, min(a, b), max(a, b)));
}

fn shape_arc_distance(p: vec2<f32>, r: f32, n: f32) -> f32 {
    let endpoints = min(length(p - vec2(r, 0.0)), length(p - vec2(0.0, r)));
    if r == 0.0 || any(p < vec2(0.0)) { return endpoints; }
    if n == 2.0 { return abs(length(p) - r); }
    let a = max(p.x, p.y) / r;
    let b = min(p.x, p.y) / r;
    let diagonal = r * pow(2.0, -1.0 / n);
    var best = min(endpoints, length(p - vec2(diagonal)));
    var lo = 0.0;
    var hi = 1.0;
    for (var i = 0u; i < 24u; i += 1u) {
        if r * (hi - lo) <= 1.0 / 512.0 { break; }
        let t = (lo + hi) * 0.5;
        if t == lo || t == hi { break; }
        let power = pow(t, n - 1.0);
        let x = pow(1.0 + pow(t, n), -1.0 / n);
        if x * (t - power) + a * power < b { lo = t; } else { hi = t; }
    }
    let t = (lo + hi) * 0.5;
    let x = r * pow(1.0 + pow(t, n), -1.0 / n);
    best = min(best, length(vec2(max(p.x, p.y), min(p.x, p.y)) - vec2(x, t * x)));
    return best;
}

fn shape_distance(point: vec2<f32>, bounds: vec4<f32>, radii: vec4<f32>, smoothing: f32) -> f32 {
    let p = point - bounds.xy;
    let size = bounds.zw;
    let r = clamp(radii, vec4(0.0), vec4(min(size.x, size.y) * 0.5));
    let s = clamp(smoothing, 0.0, 1.0);
    let n = 2.0 + 2.0 * s;
    if all(r == vec4(0.0)) || (s == 0.0 && all(r == vec4(r.x))) {
        let q = abs(p - size * 0.5) - size * 0.5 + vec2(r.x);
        return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r.x;
    }
    var distance = min(
        min(shape_segment_distance(p, vec2(r.x, 0.0), vec2(size.x - r.y, 0.0)),
            shape_segment_distance(p, vec2(size.x, r.y), vec2(size.x, size.y - r.z))),
        min(shape_segment_distance(p, vec2(r.w, size.y), vec2(size.x - r.z, size.y)),
            shape_segment_distance(p, vec2(0.0, r.x), vec2(0.0, size.y - r.w))));
    let corners = array<vec2<f32>, 4>(
        vec2(r.x) - p,
        vec2(p.x - (size.x - r.y), r.y - p.y),
        p - (size - vec2(r.z)),
        vec2(r.w - p.x, p.y - (size.y - r.w)));
    var inside = all(p >= vec2(0.0)) && all(p <= size);
    for (var i = 0u; i < 4u; i += 1u) {
        let local = corners[i];
        let radius = r[i];
        if radius > 0.0 && all(local > vec2(0.0)) {
            inside = inside && pow(local.x / radius, n) + pow(local.y / radius, n) <= 1.0;
        }
        let lower = length(local - clamp(local, vec2(0.0), vec2(radius)));
        if lower < distance {
            distance = min(distance, shape_arc_distance(local, radius, n));
        }
    }
    return select(distance, -distance, inside);
}
