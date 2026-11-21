// The visible session's picture, drawn into its destination rectangle on the
// native layer (spec rdp-native-renderer, task 4.1).

struct Params {
    // Destination in normalized device coordinates: left, top, right, bottom.
    dst: vec4<f32>,
    // Source in texture coordinates: left, top, right, bottom.
    src: vec4<f32>,
    // x: 1 for smooth (linear) sampling, 0 for nearest.
    flags: vec4<u32>,
};

@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var nearest: sampler;
@group(0) @binding(2) var linear: sampler;
@group(0) @binding(3) var<uniform> params: Params;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    // Triangle strip: top-left, top-right, bottom-left, bottom-right.
    let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u));
    var out: VertexOutput;
    out.position = vec4<f32>(mix(params.dst.xy, params.dst.zw, corner), 0.0, 1.0);
    out.uv = mix(params.src.xy, params.src.zw, corner);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let sharp = textureSample(picture, nearest, in.uv);
    let soft = textureSample(picture, linear, in.uv);
    let color = select(sharp, soft, params.flags.x == 1u);
    // Remote pixels are opaque; the layer below the page must be too.
    return vec4<f32>(color.rgb, 1.0);
}
