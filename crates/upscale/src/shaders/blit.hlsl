// Просте масштабування: повноекранний трикутник у межах viewport (= прямокутник призначення),
// вибірка кадру гри семплером (точковим або білінійним — задає CPU).

Texture2D<float4> Source : register(t0);
SamplerState Sampler : register(s0);

struct VsOut {
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
};

// Три вершини без вершинного буфера: трикутник, що накриває весь viewport.
VsOut vs_main(uint id : SV_VertexID) {
    VsOut o;
    o.uv = float2((id << 1) & 2, id & 2);
    o.pos = float4(o.uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return o;
}

float4 ps_main(VsOut i) : SV_Target {
    // Альфа кадру гри не потрібна — вікно непрозоре.
    return float4(Source.Sample(Sampler, i.uv).rgb, 1.0);
}
