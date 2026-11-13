#version 450
#extension GL_EXT_multiview : require

// The hand: a mesh already skinned on the CPU, with positions and normals in reference space.

layout(location = 0) in vec3 in_pos;
layout(location = 1) in vec3 in_normal;

layout(set = 0, binding = 0) uniform Frame {
    mat4 view_proj[2];
    vec4 cam_pos[2];
    vec4 time;
} frame;

layout(location = 0) out vec3 v_normal;
layout(location = 1) out vec3 v_to_cam;

void main() {
    v_normal = in_normal;
    v_to_cam = frame.cam_pos[gl_ViewIndex].xyz - in_pos;
    gl_Position = frame.view_proj[gl_ViewIndex] * vec4(in_pos, 1.0);
}
