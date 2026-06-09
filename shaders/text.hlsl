// Screen-space text: pixel coordinates in, alpha-blended glyph coverage out.

cbuffer Screen : register(b0)
{
    float2 viewport;
    float2 _pad;
};

Texture2D atlas : register(t0);
SamplerState samp : register(s0);

struct VsIn
{
    float2 pos   : POSITION;
    float2 uv    : TEXCOORD;
    float4 color : COLOR;
};

struct VsOut
{
    float4 sv_pos : SV_Position;
    float2 uv     : TEXCOORD;
    float4 color  : COLOR;
};

VsOut vs_main(VsIn i)
{
    VsOut o;
    o.sv_pos = float4(
        i.pos.x / viewport.x * 2.0 - 1.0,
        1.0 - i.pos.y / viewport.y * 2.0,
        0.0, 1.0);
    o.uv = i.uv;
    o.color = i.color;
    return o;
}

float4 ps_main(VsOut i) : SV_Target
{
    float coverage = atlas.Sample(samp, i.uv).r;
    return float4(i.color.rgb, i.color.a * coverage);
}
