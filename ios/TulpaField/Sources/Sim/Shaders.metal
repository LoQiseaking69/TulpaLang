#include <metal_stdlib>
using namespace metal;

// Port of the Android GLSL shaders (lit geoms with rim light, grid floor, blob shadows).

struct Vertex { packed_float3 pos; packed_float3 nrm; };

struct Uniforms {
    float4x4 mvp;
    float4x4 model;
    float4x4 nrm;      // upper 3x3 = inverse-transpose of model
    float4 color;
    float4 eye;
    float4 params;     // floor: cell, fade · shadow: radius
    float4 center;     // shadow centre (xy)
};

struct VOut { float4 position [[position]]; float3 n; float3 w; };

vertex VOut tf_vertex(uint vid [[vertex_id]], const device Vertex* verts [[buffer(0)]], constant Uniforms& u [[buffer(1)]]) {
    Vertex v = verts[vid];
    float3 p = float3(v.pos);
    float4 w = u.model * float4(p, 1.0);
    VOut o;
    o.w = w.xyz;
    float3x3 nm = float3x3(u.nrm[0].xyz, u.nrm[1].xyz, u.nrm[2].xyz);
    o.n = normalize(nm * float3(v.nrm));
    o.position = u.mvp * float4(p, 1.0);
    return o;
}

fragment float4 tf_lit(VOut in [[stage_in]], constant Uniforms& u [[buffer(1)]]) {
    float3 n = normalize(in.n);
    float3 v = normalize(u.eye.xyz - in.w);
    if (dot(n, v) < 0.0) n = -n;
    float3 l1 = normalize(float3(0.35, -0.5, 0.8));
    float3 l2 = normalize(float3(-0.6, 0.4, 0.35));
    float d = max(dot(n, l1), 0.0) * 0.75 + max(dot(n, l2), 0.0) * 0.25;
    float s = pow(max(dot(reflect(-l1, n), v), 0.0), 32.0) * 0.35;
    float rim = pow(1.0 - max(dot(n, v), 0.0), 3.0) * 0.25;
    float3 c = u.color.rgb * (0.28 + d) + float3(s) + float3(0.25, 0.82, 1.0) * rim;
    return float4(c, u.color.a);
}

static float grid(float2 p, float cell, float w) {
    float2 q = p / cell;
    float2 g = abs(fract(q - 0.5) - 0.5) / max(fwidth(q), float2(1e-6));
    return 1.0 - min(min(g.x, g.y) / w, 1.0);
}

fragment float4 tf_floor(VOut in [[stage_in]], constant Uniforms& u [[buffer(1)]]) {
    float cell = u.params.x, fadeD = u.params.y;
    float minor = grid(in.w.xy, cell, 1.0);
    float major = grid(in.w.xy, cell * 5.0, 1.3);
    float dist = length(in.w.xy - u.eye.xy);
    float fade = clamp(1.0 - dist / fadeD, 0.0, 1.0);
    float3 c = mix(u.color.rgb, float3(0.20, 0.25, 0.42), minor * 0.55 * fade);
    c = mix(c, float3(0.30, 0.36, 0.70), major * 0.8 * fade);
    float ax = 1.0 - min(abs(in.w.y) / max(fwidth(in.w.y), 1e-6) / 1.5, 1.0);
    float ay = 1.0 - min(abs(in.w.x) / max(fwidth(in.w.x), 1e-6) / 1.5, 1.0);
    c = mix(c, float3(0.86, 0.30, 0.42), ax * fade * 0.8);
    c = mix(c, float3(0.30, 0.86, 0.50), ay * fade * 0.8);
    return float4(c, 1.0);
}

fragment float4 tf_shadow(VOut in [[stage_in]], constant Uniforms& u [[buffer(1)]]) {
    float r = length(in.w.xy - u.center.xy) / u.params.x;
    return float4(0.0, 0.0, 0.0, u.color.a * smoothstep(1.0, 0.2, r));
}
