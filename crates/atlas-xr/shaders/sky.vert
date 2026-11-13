#version 450
#extension GL_EXT_multiview : require

// Environment: one triangle that covers the whole screen, with no vertex buffer.

layout(location = 0) out vec2 v_ndc;

void main() {
    vec2 uv = vec2(float((gl_VertexIndex << 1) & 2), float(gl_VertexIndex & 2));
    v_ndc = uv * 2.0 - 1.0;
    gl_Position = vec4(v_ndc, 0.0, 1.0);
}
