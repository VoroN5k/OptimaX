// AMD FSR 1 RCAS (адаптивна різкість) як піксельний шейдер: читає результат EASU і пише
// одразу в задній буфер у межах прямокутника призначення (viewport).

cbuffer Const : register(b0) {
    uint4 Con;      // FsrRcasCon
    uint2 Origin;   // лівий верхній кут прямокутника призначення
    uint2 Size;     // розмір результату EASU
};

Texture2D<float4> Source : register(t0);

#define A_GPU 1
#define A_HLSL 1
#include "ffx_a.h"

#define FSR_RCAS_F 1
AF4 FsrRcasLoadF(ASU2 p) {
    // Край кадру: повторюємо крайній піксель замість чорного.
    return Source.Load(int3(clamp(p, ASU2(0, 0), ASU2(Size) - 1), 0));
}
void FsrRcasInputF(inout AF1 r, inout AF1 g, inout AF1 b) {}
#include "ffx_fsr1.h"

struct VsOut {
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
};

float4 ps_main(VsOut i) : SV_Target {
    AU2 ip = AU2(i.pos.xy) - Origin;
    AF3 c;
    FsrRcasF(c.r, c.g, c.b, ip, Con);
    return float4(c, 1.0);
}
