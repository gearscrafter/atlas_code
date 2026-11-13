#version 450

// Desk surface. While calibrating, a wave that charges under the hand; once
// mapped, a wave that sweeps the desk and leaves the platform's trace: a faint
// grid, an outline and a sonar pulse that comes out again every now and then.

layout(location = 0) in vec2 v_uv;

layout(set = 0, binding = 0) uniform Frame {
    mat4 view_proj[2];
    vec4 cam_pos[2];
    vec4 time;
} frame;

layout(push_constant) uniform Surf {
    vec4 center_half;
    vec4 color;
    vec4 params;
    vec4 unused;
} surf;

layout(location = 0) out vec4 out_color;

float band(float x, float width) {
    return exp(-x * x * width * width);
}

// Signed distance to a rounded rectangle.
float round_box(vec2 p, vec2 half_size, float radius) {
    vec2 q = abs(p) - half_size + radius;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}

void main() {
    float t = frame.time.x;
    float half_m = surf.center_half.w;
    vec2 p = v_uv;
    float r = length(p);
    float a;

    if (surf.params.y < 0.0) {
        // Charging: the wave front grows with the progress and leaves ripples behind.
        float prog = surf.params.x;
        float inside = 1.0 - smoothstep(prog - 0.08, prog, r);
        float ripples = 0.5 + 0.5 * sin(r * 34.0 - t * 9.0);
        float front = band(r - prog, 26.0);
        float glow = band(r, 4.5) * (0.35 + 0.65 * prog);
        a = inside * (0.10 + 0.28 * ripples * (1.0 - 0.6 * r / max(prog, 0.05))) + front * 1.1 + glow * 0.6;
    } else {
        // Mapped: a wave sweeps the desk and reveals the trace as it passes.
        float age = surf.params.y;
        float reveal = age * 1.5;
        float seen = 1.0 - smoothstep(reveal - 0.06, reveal, r);

        vec2 world = p * half_m;

        float border_sdf = round_box(world, vec2(half_m * 0.92, half_m * 0.55), 0.06);
        float border = band(border_sdf, 90.0);

        float cycle = fract(t * 0.20);
        float sonar = band(r - cycle * 1.2, 22.0) * (1.0 - cycle);

        float wave_front = band(r - reveal, 22.0) * (1.0 - smoothstep(0.0, 2.0, age));
        a = seen * (border * 0.60 + sonar * 0.35) + wave_front * 1.2;
    }

    // Once mapped, `progress` acts as opacity (1 = visible, 0 = faded).
    if (surf.params.y >= 0.0) a *= surf.params.x;
    a *= 1.0 - smoothstep(0.88, 1.0, r);
    a = clamp(a, 0.0, 1.0);
    // Premultiplied alpha, as the projection layer expects.
    out_color = vec4(surf.color.rgb * a, a);
}
