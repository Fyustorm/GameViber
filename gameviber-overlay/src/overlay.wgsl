// Draws epaint meshes: positions in points, premultiplied gamma-space colors.

struct Params {
    screen_size: vec2<f32>,
    // 1 when the swapchain stores sRGB: colors are then converted to linear.
    srgb: u32,
    opacity: f32,
}

var<immediate> params: Params;

@group(0) @binding(0) var font_texture: texture_2d<f32>;
@group(0) @binding(1) var font_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> VertexOutput {
    var out: VertexOutput;
    // WGSL clip space is y-up; naga flips it for Vulkan.
    let ndc = 2.0 * pos / params.screen_size - 1.0;
    out.position = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
    out.uv = uv;
    out.color = color * params.opacity;
    return out;
}

fn linear_from_gamma(c: vec3<f32>) -> vec3<f32> {
    let cutoff = c < vec3<f32>(0.04045);
    let lower = c / 12.92;
    let higher = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(higher, lower, cutoff);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = in.color * textureSample(font_texture, font_sampler, in.uv);
    if params.srgb != 0u {
        return vec4<f32>(linear_from_gamma(c.rgb), c.a);
    }
    return c;
}
