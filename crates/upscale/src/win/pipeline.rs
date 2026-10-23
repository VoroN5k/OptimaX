//! Конвеєр масштабування для обраного фільтра. Шейдери компілюються лише для нього —
//! FSR і NIS великі, компіляція всіх одразу затягнула б старт.
//!
//! - точковий / білінійний: один піксельний прохід;
//! - Lanczos: один піксельний прохід;
//! - FSR 1: EASU (обчислювальний) → проміжна текстура → RCAS (піксельний) у задній буфер;
//! - NIS: NVScaler (обчислювальний, до ×2) → проміжна → копія 1:1 або білінійне доведення.

use super::d3d::{
    compute_shader, pixel_shader, sampler, table_texture, vertex_shader, Constants, Dispatch, Draw,
    Gpu, Intermediate, SourceTexture,
};
use crate::error::UpscaleError;
use crate::geometry::{Rect, Size};
use crate::settings::{Filter, UpscaleSettings};
use crate::shaders;
use windows::core::s;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11ComputeShader, ID3D11PixelShader, ID3D11RenderTargetView, ID3D11SamplerState,
    ID3D11ShaderResourceView, ID3D11VertexShader,
};

enum Stage {
    Simple,
    Lanczos {
        ps: ID3D11PixelShader,
        constants: Constants,
    },
    Fsr {
        easu: ID3D11ComputeShader,
        rcas: ID3D11PixelShader,
        easu_constants: Constants,
        rcas_constants: Constants,
        scaled: Option<Intermediate>,
    },
    Nis {
        cs: ID3D11ComputeShader,
        constants: Constants,
        coef_scale: ID3D11ShaderResourceView,
        coef_usm: ID3D11ShaderResourceView,
        /// `None` — NIS не застосовний до цих розмірів, малюємо білінійно.
        scaled: Option<Intermediate>,
    },
}

/// Що фактично застосовано (для статусу й вимірювань).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Applied {
    pub filter: Filter,
    /// Фільтр зробив лише частину масштабу (NIS > ×2), решту — білінійний прохід.
    pub partial: bool,
}

pub(super) struct Pipeline {
    filter: Filter,
    sharpness: f32,
    vs: ID3D11VertexShader,
    blit: ID3D11PixelShader,
    point: ID3D11SamplerState,
    linear: ID3D11SamplerState,
    stage: Stage,
    prepared: Option<(Size, Rect)>,
    applied: Applied,
}

fn source_error(e: String) -> UpscaleError {
    UpscaleError::Shader(e)
}

impl Pipeline {
    pub fn new(gpu: &Gpu, settings: &UpscaleSettings) -> Result<Self, UpscaleError> {
        let vs = vertex_shader(gpu, shaders::BLIT, s!("vs_main"))?;
        let blit = pixel_shader(gpu, shaders::BLIT, s!("ps_main"))?;
        let stage = match settings.filter {
            Filter::Nearest | Filter::Bilinear => Stage::Simple,
            Filter::Lanczos => Stage::Lanczos {
                ps: pixel_shader(gpu, shaders::LANCZOS, s!("ps_main"))?,
                constants: Constants::new(gpu, 8)?,
            },
            Filter::Fsr => Stage::Fsr {
                easu: compute_shader(gpu, &shaders::fsr_easu_source().map_err(source_error)?)?,
                rcas: pixel_shader(
                    gpu,
                    &shaders::fsr_rcas_source().map_err(source_error)?,
                    s!("ps_main"),
                )?,
                easu_constants: Constants::new(gpu, 20)?,
                rcas_constants: Constants::new(gpu, 8)?,
                scaled: None,
            },
            Filter::Nis => {
                let table = |name: &str| {
                    shaders::nis_coefficients(name)
                        .ok_or_else(|| UpscaleError::Shader(format!("немає таблиці {name}")))
                        .and_then(|v| table_texture(gpu, shaders::NIS_COEF_SIZE, &v))
                };
                Stage::Nis {
                    cs: compute_shader(gpu, &shaders::nis_source().map_err(source_error)?)?,
                    constants: Constants::new(gpu, 32)?,
                    coef_scale: table("coef_scale")?,
                    coef_usm: table("coef_usm")?,
                    scaled: None,
                }
            }
        };
        Ok(Self {
            filter: settings.filter,
            sharpness: settings.sharpness,
            vs,
            blit,
            point: sampler(gpu, false)?,
            linear: sampler(gpu, true)?,
            stage,
            prepared: None,
            applied: Applied {
                filter: settings.filter,
                partial: false,
            },
        })
    }

    /// Що фактично застосовано за останньої підготовки.
    pub fn applied(&self) -> Applied {
        self.applied
    }

    /// Готує константи й проміжні текстури під розміри (лише коли вони змінились).
    pub fn prepare(&mut self, gpu: &Gpu, source: Size, dest: Rect) -> Result<(), UpscaleError> {
        if self.prepared == Some((source, dest)) {
            return Ok(());
        }
        let out = dest.size();
        let origin = (
            u32::try_from(dest.x).unwrap_or(0),
            u32::try_from(dest.y).unwrap_or(0),
        );
        self.applied = Applied {
            filter: self.filter,
            partial: false,
        };
        match &mut self.stage {
            Stage::Simple => {}
            Stage::Lanczos { constants, .. } => {
                let f = f32::to_bits;
                constants.update(
                    gpu,
                    &[
                        f(source.width as f32),
                        f(source.height as f32),
                        f(out.width as f32),
                        f(out.height as f32),
                        f(origin.0 as f32),
                        f(origin.1 as f32),
                    ],
                );
            }
            Stage::Fsr {
                easu_constants,
                rcas_constants,
                scaled,
                ..
            } => {
                easu_constants.update(gpu, &shaders::fsr_easu_constants(source, out));
                rcas_constants.update(
                    gpu,
                    &shaders::fsr_rcas_constants(
                        shaders::fsr_sharpness_stops(self.sharpness),
                        origin,
                        out,
                    ),
                );
                *scaled = Some(Intermediate::new(gpu, out)?);
            }
            Stage::Nis {
                constants, scaled, ..
            } => {
                let config = shaders::nis_output_size(source, out).and_then(|size| {
                    shaders::nis_constants(self.sharpness, source, size).map(|c| (size, c))
                });
                *scaled = match config {
                    Some((size, c)) => {
                        constants.update(gpu, &c);
                        self.applied.partial = size != out;
                        Some(Intermediate::new(gpu, size)?)
                    }
                    None => {
                        tracing::warn!(?source, ?out, "NIS не застосовний, білінійний фільтр");
                        self.applied = Applied {
                            filter: Filter::Bilinear,
                            partial: false,
                        };
                        None
                    }
                };
            }
        }
        self.prepared = Some((source, dest));
        Ok(())
    }

    /// Простий прохід: копія з точковою або білінійною вибіркою.
    fn simple<'a>(&'a self, view: &'a ID3D11ShaderResourceView, linear: bool) -> Draw<'a> {
        Draw {
            vs: &self.vs,
            ps: &self.blit,
            source: view,
            sampler: Some(if linear { &self.linear } else { &self.point }),
            constants: None,
        }
    }

    /// Малює кадр у задній буфер (після [`Pipeline::prepare`] з тими самими розмірами).
    pub fn render(
        &self,
        gpu: &Gpu,
        source: &SourceTexture,
        target: &ID3D11RenderTargetView,
        dest: Rect,
    ) {
        match &self.stage {
            Stage::Simple => {
                self.simple(&source.view, self.filter != Filter::Nearest)
                    .run(gpu, target, dest);
            }
            Stage::Lanczos { ps, constants } => Draw {
                vs: &self.vs,
                ps,
                source: &source.view,
                sampler: None,
                constants: Some(constants),
            }
            .run(gpu, target, dest),
            Stage::Fsr {
                easu,
                rcas,
                easu_constants,
                rcas_constants,
                scaled: Some(scaled),
            } => {
                Dispatch {
                    cs: easu,
                    sources: &[Some(source.view.clone())],
                    sampler: Some(&self.linear),
                    constants: easu_constants,
                    output: scaled,
                    groups: shaders::dispatch_groups(
                        scaled.size,
                        (shaders::FSR_TILE, shaders::FSR_TILE),
                    ),
                }
                .run(gpu);
                Draw {
                    vs: &self.vs,
                    ps: rcas,
                    source: &scaled.view,
                    sampler: None,
                    constants: Some(rcas_constants),
                }
                .run(gpu, target, dest);
            }
            Stage::Nis {
                cs,
                constants,
                coef_scale,
                coef_usm,
                scaled: Some(scaled),
            } => {
                Dispatch {
                    cs,
                    sources: &[
                        Some(source.view.clone()),
                        Some(coef_scale.clone()),
                        Some(coef_usm.clone()),
                    ],
                    sampler: Some(&self.linear),
                    constants,
                    output: scaled,
                    groups: shaders::dispatch_groups(scaled.size, shaders::NIS_BLOCK),
                }
                .run(gpu);
                self.simple(&scaled.view, scaled.size != dest.size())
                    .run(gpu, target, dest);
            }
            // Ще не підготовлено або фільтр не застосовний — білінійно.
            Stage::Fsr { scaled: None, .. } | Stage::Nis { scaled: None, .. } => {
                self.simple(&source.view, true).run(gpu, target, dest);
            }
        }
    }
}
