#version 450
#extension GL_EXT_multiview : require

// Orb: a sphere of radius 1 that is placed and scaled per orb.

layout(location = 0) in vec3 in_pos;

// Frame data, a matrix and a camera position per eye (multiview).
layout(set = 0, binding = 0) uniform Frame {
    mat4 view_proj[2];
    vec4 cam_pos[2];
    vec4 time; // x = segundos
} frame;

// Per-orb data.
layout(push_constant) uniform Orb {
    vec4 center_radius; // xyz = center in the world, w = radius
    vec4 color_core;    // core color (warm)
    vec4 color_rim;     // rim color (cool)
    vec4 params;        // x = intensity, y = pulse amplitude, z = speed, w = phase
} orb;

layout(location = 0) out vec3 v_normal;
layout(location = 1) out vec3 v_to_cam;
layout(location = 2) out vec3 v_obj;

void main() {
    float pulse = 1.0 + orb.params.y * sin(frame.time.x * orb.params.z + orb.params.w);
    vec3 world = orb.center_radius.xyz + in_pos * orb.center_radius.w * pulse;
    v_normal = normalize(in_pos);
    v_to_cam = frame.cam_pos[gl_ViewIndex].xyz - world;
    v_obj = in_pos;
    gl_Position = frame.view_proj[gl_ViewIndex] * vec4(world, 1.0);
}
