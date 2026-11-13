#version 450
#extension GL_EXT_multiview : require

// Flat horizontal surface (the desk): a square with side 2*half that is
// generated right here, with no vertex buffer.

layout(set = 0, binding = 0) uniform Frame {
    mat4 view_proj[2];
    vec4 cam_pos[2];
    vec4 time;
} frame;

layout(push_constant) uniform Surf {
    vec4 center_half; // xyz = center in the world, w = half the side (m)
    vec4 color;       // rgb
    vec4 params;      // x = charge progress (0..1), y = seconds since it was mapped (<0 = charging)
    vec4 unused;      // xyz = surface normal, w = rotation (rad) around the vertical
} surf;

layout(location = 0) out vec2 v_uv;

const vec2 CORNERS[6] = vec2[6](
    vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
    vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0)
);

void main() {
    vec2 uv = CORNERS[gl_VertexIndex];
    v_uv = uv;
    // The surface axes: on the floor or the desk, X and Z; on a wall, the horizontal and
    // the vertical.
    vec3 n = normalize(surf.unused.xyz);
    float c = cos(surf.unused.w);
    float sn = sin(surf.unused.w);
    vec3 u = vec3(c, 0.0, -sn);
    vec3 v = vec3(sn, 0.0, c);
    if (abs(n.y) < 0.99) {
        u = normalize(cross(vec3(0.0, 1.0, 0.0), n));
        v = vec3(0.0, 1.0, 0.0);
    }
    vec3 world = surf.center_half.xyz + (u * uv.x + v * uv.y) * surf.center_half.w;
    gl_Position = frame.view_proj[gl_ViewIndex] * vec4(world, 1.0);
}
