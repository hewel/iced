struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var video: texture_2d<f32>;
@group(0) @binding(1) var video_sampler: sampler;

@vertex
fn vertex_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let position = positions[index];
    var output: VertexOutput;
    output.position = vec4<f32>(position, 0.0, 1.0);
    // Texture coordinates start at the top left; clip-space Y points up.
    output.uv = vec2<f32>(position.x * 0.5 + 0.5, 0.5 - position.y * 0.5);
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // Both textures are RGB10_A2 UNORM, not sRGB. mpv has already produced
    // full-range BT.709 gamma-2.2 code values with premultiplied alpha at
    // 203 cd/m² reference white / 0.203 cd/m² black. Forward those values:
    // neither a gamma decode/encode nor inverse-sRGB compensation belongs here.
    // Premultiplied blending over opaque black preserves RGB without applying
    // alpha a second time. Nearest sampling also avoids inventing code values.
    return textureSample(video, video_sampler, input.uv);
}
