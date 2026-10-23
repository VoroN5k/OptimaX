//! Малювання збільшеного курсора поверх кадру (з прозорістю).

use super::cursor_shape::{self, CursorState};
use super::d3d::{image_texture, pixel_shader, sampler, vertex_shader, Gpu};
use super::os;
use crate::error::UpscaleError;
use crate::geometry::{Rect, Size};
use crate::shaders;
use windows::core::s;
use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11BlendState, ID3D11PixelShader, ID3D11RenderTargetView, ID3D11SamplerState,
    ID3D11ShaderResourceView, ID3D11VertexShader, D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA,
    D3D11_BLEND_ONE, D3D11_BLEND_OP_ADD, D3D11_COLOR_WRITE_ENABLE_ALL,
    D3D11_RENDER_TARGET_BLEND_DESC, D3D11_VIEWPORT,
};

/// Текстура однієї форми курсора.
struct Shape {
    id: isize,
    view: ID3D11ShaderResourceView,
    size: Size,
    hotspot: (i32, i32),
}

pub(super) struct CursorLayer {
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    linear: ID3D11SamplerState,
    blend: ID3D11BlendState,
    shape: Option<Shape>,
}

impl CursorLayer {
    pub fn new(gpu: &Gpu) -> Result<Self, UpscaleError> {
        let target = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: true.into(),
            SrcBlend: D3D11_BLEND_ONE,
            DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_ONE,
            DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
        };
        let desc = D3D11_BLEND_DESC {
            AlphaToCoverageEnable: false.into(),
            IndependentBlendEnable: false.into(),
            RenderTarget: [target; 8],
        };
        let mut blend = None;
        // SAFETY: опис локальний.
        unsafe { gpu.device.CreateBlendState(&desc, Some(&mut blend)) }
            .map_err(os("CreateBlendState"))?;
        Ok(Self {
            vs: vertex_shader(gpu, shaders::BLIT, s!("vs_main"))?,
            ps: pixel_shader(gpu, shaders::BLIT, s!("ps_cursor"))?,
            linear: sampler(gpu, true)?,
            blend: blend.ok_or(UpscaleError::Os {
                api: "CreateBlendState",
                code: 0,
            })?,
            shape: None,
        })
    }

    /// Розмір і гаряча точка форми `state` (форма готується й кешується за потреби).
    pub fn prepare(&mut self, gpu: &Gpu, state: CursorState) -> Option<(Size, (i32, i32))> {
        if self.shape.as_ref().map(|s| s.id) != Some(state.shape) {
            let image = cursor_shape::image(state.shape)?;
            let view = image_texture(gpu, image.size, &image.pixels).ok()?;
            self.shape = Some(Shape {
                id: state.shape,
                view,
                size: image.size,
                hotspot: image.hotspot,
            });
        }
        self.shape.as_ref().map(|s| (s.size, s.hotspot))
    }

    /// Малює підготовлену форму в прямокутнику `rect` поверх уже намальованого кадру.
    pub fn draw(&self, gpu: &Gpu, target: &ID3D11RenderTargetView, rect: Rect) {
        let Some(shape) = &self.shape else {
            return;
        };
        let viewport = D3D11_VIEWPORT {
            TopLeftX: rect.x as f32,
            TopLeftY: rect.y as f32,
            Width: rect.width as f32,
            Height: rect.height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        let ctx = &gpu.context;
        // SAFETY: усі об'єкти створені на цьому пристрої; масиви локальні.
        unsafe {
            ctx.OMSetRenderTargets(Some(&[Some(target.clone())]), None);
            ctx.OMSetBlendState(&self.blend, None, u32::MAX);
            ctx.RSSetViewports(Some(&[viewport]));
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.IASetInputLayout(None);
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShader(&self.ps, None);
            ctx.PSSetShaderResources(0, Some(&[Some(shape.view.clone())]));
            ctx.PSSetSamplers(0, Some(&[Some(self.linear.clone())]));
            ctx.Draw(3, 0);
            ctx.OMSetBlendState(None, None, u32::MAX);
            ctx.PSSetShaderResources(0, Some(&[None]));
        }
    }
}
