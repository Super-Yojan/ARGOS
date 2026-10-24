struct Uniforms {
    mvp: mat4x4<f32>,
    cutoff: f32,
    opaque: f32,
    survey: f32,
    padding: f32,
    highlight: vec4<f32>,
}
@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var base_color: texture_2d<f32>;
@group(0) @binding(2) var base_sampler: sampler;
struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}
@vertex fn vs_main(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> Varying {
    var output: Varying;
    output.position = uniforms.mvp * vec4<f32>(position, 1.0);
    output.uv = uv;
    output.color = color;
    return output;
}
@fragment fn fs_main(input: Varying) -> @location(0) vec4<f32> {
    var color = textureSample(base_color, base_sampler, input.uv) * input.color;
    if color.a < uniforms.cutoff { discard; }
    if uniforms.opaque > 0.5 { color.a = 1.0; }
    if uniforms.survey > 0.5 {
        let luminance = dot(color.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        let toned = vec3<f32>(0.16, 0.21, 0.24) + vec3<f32>(0.46, 0.48, 0.50) * pow(max(luminance, 0.0), 0.7);
        color = vec4<f32>(toned, color.a);
    }
    color = vec4<f32>(mix(color.rgb, uniforms.highlight.rgb, uniforms.highlight.a * 0.85), color.a);
    return color;
}
