#version 450

// Hand silhouette: almost transparent glass with a glowing outline.

layout(location = 0) in vec3 v_normal;
layout(location = 1) in vec3 v_to_cam;

layout(push_constant) uniform Hand {
    vec4 tint;   // rgb = color, w = opacidad general
    vec4 p1;
    vec4 p2;
    vec4 p3;
} hand;

layout(location = 0) out vec4 out_color;

void main() {
    vec3 n = normalize(v_normal);
    vec3 v = normalize(v_to_cam);
    float ndv = abs(dot(n, v));
    float fresnel = pow(1.0 - ndv, 2.2);
    vec3 light = normalize(vec3(-0.3, 0.8, 0.5));
    float spec = pow(max(dot(reflect(-v, n), light), 0.0), 30.0);

    vec3 color = hand.tint.rgb * (0.22 + 1.3 * fresnel) + vec3(0.8, 0.95, 1.0) * spec * 0.5;
    float a = clamp((0.10 + 0.55 * fresnel + 0.25 * spec) * hand.tint.w, 0.0, 1.0);
    // Premultiplied alpha, as the projection layer expects.
    out_color = vec4(color * a, a);
}
