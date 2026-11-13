#version 450

// Holographic tube of the scroll: glowing rim, specular highlight and the turns of the rolled
// paper as bands along the tube.

layout(location = 0) in vec2 v_uv;

layout(set = 0, binding = 0) uniform Frame {
    mat4 view_proj[2];
    vec4 cam_pos[2];
    vec4 time;
} frame;

layout(push_constant) uniform Roll {
    vec4 center_half;
    vec4 axis_radius;
    vec4 tint;
    vec4 unused;
} roll;

layout(location = 0) out vec4 out_color;

void main() {
    float u = clamp(v_uv.y, -1.0, 1.0);
    float nz = sqrt(max(1.0 - u * u, 0.0));        // how much the tube faces the camera
    float theta = asin(u);
    float layers = 0.5 + 0.5 * sin(theta * 22.0);  // turns of the paper
    float diffuse = 0.25 + 0.75 * nz;
    float rim = pow(1.0 - nz, 2.0);
    vec3 n = vec3(u, 0.0, nz);
    float spec = pow(max(dot(n, normalize(vec3(-0.35, 0.0, 0.94))), 0.0), 36.0);

    vec3 base = roll.tint.rgb * (0.30 + 0.55 * diffuse) * (0.85 + 0.15 * layers);
    vec3 color = base + vec3(0.55, 0.95, 1.0) * rim * 0.9 + vec3(1.0) * spec * 0.8;
    float a = clamp(0.62 + 0.30 * rim + 0.4 * spec, 0.0, 1.0);
    a *= 1.0 - smoothstep(0.93, 1.0, abs(u));          // borde suave
    a *= 1.0 - smoothstep(0.97, 1.0, abs(v_uv.x));     // extremos suaves
    out_color = vec4(color * a, a);
}
