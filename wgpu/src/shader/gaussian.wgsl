// Dense Gaussian convolution on the source texel grid. The two effective
// weights include the fractional sampling phase before adjacent texels are
// paired; merging two already-bilinear samples would not be equivalent.
fn gaussian(point: vec2<f32>, size: vec2<f32>, dimensions: vec2<f32>, sigma: f32, horizontal: bool) -> vec4<f32> {
    let low = vec2(0.5) / dimensions;
    let high = (size - vec2(0.5)) / dimensions;
    if sigma < 0.01 {
        return textureSampleLevel(image, image_sampler, clamp(point / dimensions, low, high), 0, 0.0);
    }
    let coordinate = select(point.y, point.x, horizontal);
    let origin = floor(coordinate - 0.5) + 0.5;
    let phase = coordinate - origin;
    let radius = i32(ceil(3.0 * sigma));
    // Incremental Gaussian weights avoid an exponential for each texel.
    let inverse = 1.0 / sigma;
    var ratio = exp(-0.5 * inverse * inverse);
    let decay = ratio * ratio;
    var previous = 1.0;
    var result = vec4(0.0);
    var total = 0.0;
    for (var n = 0; n <= radius; n += 2) {
        var first = previous * ratio;
        ratio *= decay;
        var second = first * ratio;
        ratio *= decay;
        if n + 1 > radius { first = 0.0; }
        if n + 2 > radius { second = 0.0; }
        let positive = vec2(mix(first, previous, phase), mix(second, first, phase));
        let negative = vec2(mix(previous, first, phase), mix(first, second, phase));
        let positive_weight = positive.x + positive.y;
        let negative_weight = negative.x + negative.y;
        if positive_weight > 0.0 {
            let position = origin + f32(n + 1) + positive.y / positive_weight;
            let sample_point = select(vec2(point.x, position), vec2(position, point.y), horizontal);
            result += textureSampleLevel(image, image_sampler, clamp(sample_point / dimensions, low, high), 0, 0.0) * positive_weight;
        }
        if negative_weight > 0.0 {
            let position = origin - f32(n) - negative.y / negative_weight;
            let sample_point = select(vec2(point.x, position), vec2(position, point.y), horizontal);
            result += textureSampleLevel(image, image_sampler, clamp(sample_point / dimensions, low, high), 0, 0.0) * negative_weight;
        }
        total += positive_weight + negative_weight;
        previous = second;
    }
    return result / total;
}
