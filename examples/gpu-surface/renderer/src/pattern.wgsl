struct Uniforms { frame: vec4<f32>, input: vec4<f32>, flags: vec4<f32> }
@group(0) @binding(0) var<uniform> uniforms: Uniforms;

@vertex fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let positions = array<vec2<f32>, 3>(vec2(-1., -1.), vec2(3., -1.), vec2(-1., 3.));
    return vec4(positions[i], 0., 1.);
}

@fragment fn fragment(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let data = uniforms.frame;
    let uv = p.xy / data.yz;
    var color = vec3(0.035, 0.08, 0.13);
    // Repeated sharp E-like glyphs and one-pixel grid reveal scale and resize drift.
    let cell = vec2(p.x % 20., p.y % 28.);
    let glyph = cell.x > 3. && cell.x < 15. && cell.y > 4. && cell.y < 23.
        && (cell.x < 5. || cell.y < 6. || abs(cell.y - 13.) < 1. || cell.y > 21.);
    if glyph { color = vec3(0.58, 0.7, 0.8); }
    if p.x % 64. < 1. || p.y % 64. < 1. { color = vec3(0.14, 0.26, 0.36); }
    let center = vec2(0.5 + sin(data.x * 1.7) * 0.35, 0.5 + cos(data.x * 1.1) * 0.22);
    if distance((uv - center) * vec2(data.y / data.z, 1.), vec2(0.)) < 0.12 * uniforms.input.z {
        color = mix(vec3(1., 0.42, 0.13), vec3(0.2, 0.9, 0.5), uniforms.flags.z);
    }
    if uniforms.flags.x > 0. {
        let d = abs(p.xy - uniforms.input.xy);
        let radius = 16. + uniforms.flags.y * 12.;
        if (d.x < 2. && d.y < radius) || (d.y < 2. && d.x < radius) {
            color = mix(vec3(0.6), vec3(0.1, 1., 0.8), uniforms.input.w);
        }
    }
    return vec4(color, 1.);
}
