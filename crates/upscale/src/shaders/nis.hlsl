// NVIDIA Image Scaling (NVScaler): збільшення з напрямленою різкістю, SDR, Shader Model 5.
// Параметри — типові для NVScaler з NIS_Config.h: блок 32×24, 256 потоків, повна точність.

#define NIS_SCALER 1
#define NIS_HDR_MODE 0
#define NIS_BLOCK_WIDTH 32
#define NIS_BLOCK_HEIGHT 24
#define NIS_THREAD_GROUP_SIZE 256
#define NIS_USE_HALF_PRECISION 0
#define NIS_VIEWPORT_SUPPORT 0
#define NIS_CLAMP_OUTPUT 1

#include "NIS_Main.hlsl"
