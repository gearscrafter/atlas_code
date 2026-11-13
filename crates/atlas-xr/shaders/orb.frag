#version 450

layout(location = 0) in vec3 v_normal;
layout(location = 1) in vec3 v_to_cam;
layout(location = 2) in vec3 v_obj;

layout(set = 0, binding = 0) uniform Frame {
    mat4 view_proj[2];
    vec4 cam_pos[2];
    vec4 time;
} frame;

layout(push_constant) uniform Orb {
    vec4 center_radius;
    vec4 color_core;
    vec4 color_rim;
    vec4 params;
} orb;

layout(location = 0) out vec4 out_color;

void main() {
    vec3 n = normalize(v_normal);
    // Back faces are seen "from inside": that gives the glass depth.
    if (!gl_FrontFacing) n = -n;
    vec3 v = normalize(v_to_cam);
    float ndv = clamp(dot(n, v), 0.0, 1.0);

    float fresnel = pow(1.0 - ndv, 2.4);   // borde luminoso
    float core = pow(ndv, 6.0);            // warm glow, only in the center

    // Specular highlight from a light at the top left: gives the glass volume.
    vec3 light = normalize(vec3(-0.4, 0.8, 0.5));
    float spec = pow(max(dot(reflect(-v, n), light), 0.0), 48.0);

    // Iridescence: the rim shifts toward violet with position and time.
    float band = 0.5 + 0.5 * sin(6.0 * (v_obj.y + 0.4 * v_obj.x) + frame.time.x * 0.6);
    vec3 rim = mix(orb.color_rim.rgb, vec3(0.75, 0.45, 1.0), band * fresnel * 0.6);

    vec3 color = (rim * fresnel * 1.6 + orb.color_core.rgb * core * 0.9 + vec3(1.0) * spec * 0.9) * orb.params.x;
    float alpha = clamp(0.05 + 0.70 * fresnel + 0.40 * core + 0.6 * spec, 0.0, 1.0);
    if (!gl_FrontFacing) {
        color *= 0.30;
        alpha *= 0.30;
    }
    // Premultiplied alpha, as the projection layer expects.
    out_color = vec4(color * alpha, alpha);
}
