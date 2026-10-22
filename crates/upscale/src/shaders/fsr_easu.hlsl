// AMD FSR 1 EASU (збільшення з урахуванням країв), 32-бітний варіант для Shader Model 5.
// Обгортка за прикладом з ffx_fsr1.h: 64 потоки, кожен рахує 4 пікселі плитки 16×16.

cbuffer Const : register(b0) {
    uint4 Con0;
    uint4 Con1;
    uint4 Con2;
    uint4 Con3;
    uint4 OutSize; // xy — розмір виходу
};

Texture2D<float4> Source : register(t0);
SamplerState LinearClamp : register(s0);
RWTexture2D<float4> Output : register(u0);

#define A_GPU 1
#define A_HLSL 1
#include "ffx_a.h"

#define FSR_EASU_F 1
AF4 FsrEasuRF(AF2 p) { return Source.GatherRed(LinearClamp, p); }
AF4 FsrEasuGF(AF2 p) { return Source.GatherGreen(LinearClamp, p); }
AF4 FsrEasuBF(AF2 p) { return Source.GatherBlue(LinearClamp, p); }
#include "ffx_fsr1.h"

void Filter(AU2 pos) {
    if (pos.x >= OutSize.x || pos.y >= OutSize.y) return;
    AF3 c;
    FsrEasuF(c, pos, Con0, Con1, Con2, Con3);
    Output[pos] = AF4(c, 1.0);
}

[numthreads(64, 1, 1)]
void main(uint3 local : SV_GroupThreadID, uint3 group : SV_GroupID) {
    AU2 gxy = ARmp8x8(local.x) + AU2(group.x << 4u, group.y << 4u);
    Filter(gxy);
    gxy.x += 8u;
    Filter(gxy);
    gxy.y += 8u;
    Filter(gxy);
    gxy.x -= 8u;
    Filter(gxy);
}
