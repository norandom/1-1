// YUV 4:2:0 to RGBA on the GPU, into a session's picture texture (spec
// rdp-native-renderer, task 8.1). The planes are read with exact texel loads,
// like the CPU conversion; chroma is shared by 2x2 luma pixels.

struct Params {
    // rgb = floor(matrix * (scale * (yuv - offset)) + rounding);
    // offset x: luma, y: chroma, z: rounding (0 rounds down like the
    // fixed-point MS-RDPEGFX transform, 0.5 rounds to nearest).
    offset: vec4<f32>,
    scale: vec4<f32>,
    // Rows of the 3x3 matrix, applied to (Y, U, V).
    r: vec4<f32>,
    g: vec4<f32>,
    b: vec4<f32>,
    // x: 1 when U and V are interleaved in one RG texture (NV12).
    flags: vec4<u32>,
};

@group(0) @binding(0) var luma: texture_2d<f32>;
@group(0) @binding(1) var chroma_u: texture_2d<f32>;
@group(0) @binding(2) var chroma_v: texture_2d<f32>;
@group(0) @binding(3) var<uniform> params: Params;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    // One triangle covering the target.
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    // Back to the integer sample values (a texel reads as value / 255).
    let y = round(textureLoad(luma, pixel, 0).r * 255.0);
    let c = pixel / 2;
    let interleaved = textureLoad(chroma_u, c, 0);
    let planar_u = interleaved.r;
    let planar_v = textureLoad(chroma_v, c, 0).r;
    let u = round(select(planar_u, interleaved.r, params.flags.x == 1u) * 255.0);
    let v = round(select(planar_v, interleaved.g, params.flags.x == 1u) * 255.0);
    let yuv = vec3<f32>(
        (y - params.offset.x) * params.scale.x,
        (u - params.offset.y) * params.scale.y,
        (v - params.offset.y) * params.scale.y,
    );
    let rgb = floor(
        vec3<f32>(dot(params.r.xyz, yuv), dot(params.g.xyz, yuv), dot(params.b.xyz, yuv)) + params.offset.z,
    );
    return vec4<f32>(clamp(rgb / 255.0, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
