// Instanced cube scene. All animation runs here in the vertex shader from a
// single time constant — the instance buffer is immutable, zero per-frame
// uploads. Edit and save: the playground hot-reloads this file.

cbuffer Frame : register(b0)
{
    float4x4 view_proj;
    float3   cam_pos;
    float    time;
    float3   light_dir;
    float    _pad;
};

struct VsIn
{
    float3 pos    : POSITION;
    float3 normal : NORMAL;
    // per instance (slot 1)
    float3 ipos   : I_POS;
    float3 iaxis  : I_AXIS;
    float  ispeed : I_SPEED;
    float  iscale : I_SCALE;
    float4 icolor : I_COLOR;
};

struct VsOut
{
    float4 sv_pos : SV_Position;
    float3 normal : NORMAL;
    float3 world  : WORLDPOS;
    float4 color  : COLOR;
};

// Rodrigues rotation of v around a normalized axis.
float3 rotate_axis(float3 v, float3 axis, float s, float c)
{
    return v * c + cross(axis, v) * s + axis * dot(axis, v) * (1.0 - c);
}

VsOut vs_main(VsIn i)
{
    float s, c;
    sincos(i.ispeed * time, s, c);

    float3 wp = rotate_axis(i.pos * i.iscale, i.iaxis, s, c) + i.ipos;

    VsOut o;
    o.sv_pos = mul(view_proj, float4(wp, 1.0));
    o.normal = rotate_axis(i.normal, i.iaxis, s, c);
    o.world  = wp;
    o.color  = i.icolor;
    return o;
}

float4 ps_main(VsOut i) : SV_Target
{
    float3 n = normalize(i.normal);
    float3 l = normalize(-light_dir);

    float ndl = saturate(dot(n, l));
    float3 v = normalize(cam_pos - i.world);
    float3 h = normalize(l + v);
    float spec = pow(saturate(dot(n, h)), 48.0) * 0.5;

    float3 ambient = float3(0.10, 0.11, 0.15);
    float3 col = i.color.rgb * (ambient + ndl) + spec * ndl;

    // subtle distance fade into the background
    float fog = saturate(length(cam_pos - i.world) / 1400.0);
    col = lerp(col, float3(0.013, 0.015, 0.022), fog * fog);

    return float4(col, 1.0);
}
