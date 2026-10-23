// Lanczos-3: 6×6 відліків з вагами sinc(x)·sinc(x/3). Щоб не було «дзвону» (світлих і темних
// ореолів) біля різких країв, результат обмежується мінімумом і максимумом найближчих 2×2 пікселів.

cbuffer Const : register(b0) {
    float2 SrcSize;   // розмір кадру гри
    float2 DstSize;   // розмір прямокутника призначення
    float2 DstOrigin; // його лівий верхній кут у задньому буфері
    float2 Pad;
};

Texture2D<float4> Source : register(t0);

struct VsOut {
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
};

static const float PI = 3.14159265;

float Weight(float x) {
    x = abs(x);
    if (x < 1e-5) return 1.0;
    if (x >= 3.0) return 0.0;
    float px = PI * x;
    return 3.0 * sin(px) * sin(px / 3.0) / (px * px);
}

float4 ps_main(VsOut i) : SV_Target {
    // Центр пікселя виходу → координата у вході (центри текселів на цілих).
    float2 src = (i.pos.xy - DstOrigin) * SrcSize / DstSize - 0.5;
    float2 base = floor(src);
    float2 f = src - base;
    int2 maxCoord = int2(SrcSize) - 1;

    float wx[6];
    float wy[6];
    float sx = 0.0;
    float sy = 0.0;
    [unroll] for (int t = 0; t < 6; ++t) {
        wx[t] = Weight(f.x - (t - 2));
        wy[t] = Weight(f.y - (t - 2));
        sx += wx[t];
        sy += wy[t];
    }

    float3 acc = 0.0;
    float3 lo = 1.0;
    float3 hi = 0.0;
    [unroll] for (int y = 0; y < 6; ++y) {
        [unroll] for (int x = 0; x < 6; ++x) {
            int2 c = clamp(int2(base) + int2(x - 2, y - 2), 0, maxCoord);
            float3 s = Source.Load(int3(c, 0)).rgb;
            acc += s * (wx[x] * wy[y]);
            if (x >= 2 && x <= 3 && y >= 2 && y <= 3) {
                lo = min(lo, s);
                hi = max(hi, s);
            }
        }
    }
    return float4(clamp(acc / (sx * sy), lo, hi), 1.0);
}
