#version 450
#extension GL_EXT_multiview : require

// Roll of the editor's scroll: a cylinder seen from the side. It is drawn as a strip that
// widens toward the camera, and the fragment shader shapes it into a tube.

layout(set = 0, binding = 0) uniform Frame {
    mat4 view_proj[2];
    vec4 cam_pos[2];
    vec4 time;
} frame;

layout(push_constant) uniform Roll {
    vec4 center_half;  // xyz = center, w = half the length
    vec4 axis_radius;  // xyz = tube axis, w = radius
    vec4 tint;         // rgb
    vec4 unused;
} roll;

layout(location = 0) out vec2 v_uv;

const vec2 CORNERS[6] = vec2[6](
    vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
    vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0)
);

void main() {
    vec2 c = CORNERS[gl_VertexIndex];
    v_uv = c;
    vec3 axis = normalize(roll.axis_radius.xyz);
    vec3 to_cam = frame.cam_pos[gl_ViewIndex].xyz - roll.center_half.xyz;
    vec3 side = normalize(cross(axis, to_cam));
    vec3 world = roll.center_half.xyz + axis * c.x * roll.center_half.w + side * c.y * roll.axis_radius.w;
    gl_Position = frame.view_proj[gl_ViewIndex] * vec4(world, 1.0);
}
