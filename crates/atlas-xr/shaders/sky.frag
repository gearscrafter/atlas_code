#version 450
#extension GL_EXT_multiview : require

// Environments generated in the shader: 1 = a canyon by day over a sea of clouds,
// 2 = sunset, 3 = starry night. Each pixel's direction comes from undoing the
// eye's projection, so the sky does not move when you turn your head.

layout(location = 0) in vec2 v_ndc;

layout(set = 0, binding = 0) uniform Frame {
    mat4 view_proj[2];
    vec4 cam_pos[2];
    vec4 time;
    mat4 inv_view_proj[2];
} frame;

layout(push_constant) uniform Sky {
    vec4 p0; // x = environment
    vec4 p1;
    vec4 p2;
    vec4 p3;
} sky;

layout(location = 0) out vec4 out_color;

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

float hash3(vec3 p) {
    return fract(sin(dot(p, vec3(127.1, 311.7, 74.7))) * 43758.5453);
}

float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1, 0)), f.x), mix(hash(i + vec2(0, 1)), hash(i + vec2(1, 1)), f.x), f.y);
}

float fbm(vec2 p) {
    float v = 0.0;
    float a = 0.5;
    for (int i = 0; i < 3; i++) {
        v += a * noise(p);
        p *= 2.03;
        a *= 0.5;
    }
    return v;
}

vec3 canyon(vec3 d, float t) {
    float up = clamp(d.y, 0.0, 1.0);
    vec3 col = mix(vec3(0.74, 0.86, 0.96), vec3(0.18, 0.43, 0.86), pow(up, 0.55));
    vec2 uv = d.xz / (abs(d.y) + 0.22) * 1.3 + vec2(t * 0.012, 0.0);
    float c = smoothstep(0.48, 0.80, fbm(uv));
    if (d.y >= 0.0) {
        col = mix(col, vec3(1.0), c * 0.85 * smoothstep(0.0, 0.18, d.y));
    } else {
        // Sea of clouds below the horizon.
        vec3 mist = mix(vec3(0.90, 0.94, 0.98), vec3(0.50, 0.60, 0.72), clamp(-d.y * 2.2, 0.0, 1.0));
        col = mix(mist, vec3(1.0), c * 0.5);
    }
    float sun = pow(max(dot(d, normalize(vec3(0.35, 0.45, -0.8))), 0.0), 220.0);
    return col + vec3(1.0, 0.95, 0.8) * sun;
}

vec3 sunset(vec3 d, float t) {
    float up = clamp(d.y, 0.0, 1.0);
    vec3 col = mix(vec3(1.0, 0.52, 0.24), vec3(0.16, 0.10, 0.36), pow(up, 0.45));
    vec3 sun_dir = normalize(vec3(0.3, 0.06, -1.0));
    float s = max(dot(d, sun_dir), 0.0);
    col += vec3(1.0, 0.65, 0.3) * pow(s, 14.0) * 0.8 + vec3(1.0, 0.95, 0.8) * pow(s, 900.0) * 3.0;
    if (d.y < 0.0) {
        col = mix(col, vec3(0.10, 0.06, 0.16), clamp(-d.y * 2.5, 0.0, 0.85));
    }
    float c = smoothstep(0.55, 0.85, fbm(d.xz / (abs(d.y) + 0.3) * 1.5 + t * 0.01));
    return mix(col, vec3(1.0, 0.45, 0.3) * 0.7, c * 0.35 * step(0.0, d.y));
}

vec3 night(vec3 d, float t) {
    float up = clamp(d.y, 0.0, 1.0);
    vec3 col = mix(vec3(0.05, 0.08, 0.20), vec3(0.005, 0.01, 0.045), pow(up, 0.5));
    vec3 cell = floor(d * 80.0);
    float h = hash3(cell);
    float twinkle = 0.6 + 0.4 * sin(t * 2.0 + h * 40.0);
    col += vec3(0.8, 0.9, 1.0) * step(0.9965, h) * twinkle * step(-0.05, d.y);
    vec3 moon_dir = normalize(vec3(-0.4, 0.5, -0.75));
    float m = dot(d, moon_dir);
    col += vec3(0.85, 0.9, 1.0) * smoothstep(0.9985, 0.9992, m) + vec3(0.3, 0.4, 0.6) * pow(max(m, 0.0), 60.0) * 0.4;
    if (d.y < 0.0) {
        col = mix(col, vec3(0.01, 0.015, 0.04), clamp(-d.y * 3.0, 0.0, 0.9));
    }
    return col;
}

void main() {
    vec4 h = frame.inv_view_proj[gl_ViewIndex] * vec4(v_ndc, 0.9, 1.0);
    vec3 dir = normalize(h.xyz / h.w - frame.cam_pos[gl_ViewIndex].xyz);
    float t = frame.time.x;
    int mode = int(sky.p0.x + 0.5);
    vec3 col = mode == 1 ? canyon(dir, t) : mode == 2 ? sunset(dir, t) : night(dir, t);
    // Opaque: it covers the passthrough.
    out_color = vec4(col, 1.0);
}
