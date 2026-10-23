//! Шейдери апскейлу та їхні константи. Без системних викликів — покрито тестами.
//!
//! Сторонні вихідники (MIT) лежать у `src/shaders/vendor` без змін, на зафіксованих коммітах:
//! - AMD FidelityFX FSR 1 — `GPUOpen-Effects/FidelityFX-FSR@a21ffb8f6c13233ba336352bdff293894c706575`;
//! - NVIDIA Image Scaling 1.0.3 — `NVIDIAGameWorks/NVIDIAImageScaling@35e13ba316c98eeecf16f37eae70ce88019911f6`.
//!
//! SHA-256 кожного файла перевіряє тест `vendor_sources_match_pinned_hashes`.
//! Константи (`FsrEasuCon`, `FsrRcasCon`, `NVScalerUpdateConfig`) перенесені з C/HLSL у Rust 1:1.

use crate::geometry::Size;

/// `ffx_a.h` (FSR 1).
pub(crate) const FFX_A: &str = include_str!("shaders/vendor/fsr1/ffx_a.h");
/// `ffx_fsr1.h` (FSR 1).
pub(crate) const FFX_FSR1: &str = include_str!("shaders/vendor/fsr1/ffx_fsr1.h");
/// `NIS_Scaler.h`.
pub(crate) const NIS_SCALER: &str = include_str!("shaders/vendor/nis/NIS_Scaler.h");
/// `NIS_Main.hlsl` (точка входу `main`, розкладка буфера констант).
pub(crate) const NIS_MAIN: &str = include_str!("shaders/vendor/nis/NIS_Main.hlsl");
/// `NIS_Config.h` — лише як джерело таблиць коефіцієнтів.
pub(crate) const NIS_CONFIG: &str = include_str!("shaders/vendor/nis/NIS_Config.h");

/// Простий прохід (точковий / білінійний) і вершинний шейдер для всіх піксельних проходів.
pub(crate) const BLIT: &str = include_str!("shaders/blit.hlsl");
/// Lanczos-3 з обмеженням «дзвону».
pub(crate) const LANCZOS: &str = include_str!("shaders/lanczos.hlsl");
/// FSR 1 EASU (обчислювальний шейдер).
pub(crate) const FSR_EASU: &str = include_str!("shaders/fsr_easu.hlsl");
/// FSR 1 RCAS (піксельний шейдер, пише одразу в задній буфер).
pub(crate) const FSR_RCAS: &str = include_str!("shaders/fsr_rcas.hlsl");
/// NIS (обгортка з дефайнами над `NIS_Main.hlsl`).
pub(crate) const NIS: &str = include_str!("shaders/nis.hlsl");

/// Розмір групи потоків EASU по кожній осі в пікселях (64 потоки × 4 пікселі).
pub(crate) const FSR_TILE: u32 = 16;
/// Блок виходу NIS (ширина × висота), як у `nis.hlsl`.
pub(crate) const NIS_BLOCK: (u32, u32) = (32, 24);
/// Кількість фаз і довжина фільтра в таблицях NIS.
const NIS_PHASES: usize = 64;
const NIS_TAPS: usize = 8;

/// Підставляє вміст файлів замість рядків `#include "ім'я"` (рекурсивно).
/// Компілятору не потрібен доступ до диска — усі вихідники вбудовані в exe.
pub(crate) fn expand_includes(source: &str, files: &[(&str, &str)]) -> Result<String, String> {
    expand(source, files, 0)
}

fn expand(source: &str, files: &[(&str, &str)], depth: u32) -> Result<String, String> {
    if depth > 8 {
        return Err("забагато вкладених #include".into());
    }
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("#include") {
            let name = rest.trim().trim_matches('"');
            let (_, body) = files
                .iter()
                .find(|(n, _)| *n == name)
                .ok_or_else(|| format!("невідомий #include «{name}»"))?;
            // Вкладений текст уже закінчується переносом рядка.
            out.push_str(&expand(body, files, depth + 1)?);
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    Ok(out)
}

/// Повний текст шейдера EASU.
pub(crate) fn fsr_easu_source() -> Result<String, String> {
    expand_includes(FSR_EASU, &[("ffx_a.h", FFX_A), ("ffx_fsr1.h", FFX_FSR1)])
}

/// Повний текст шейдера RCAS.
pub(crate) fn fsr_rcas_source() -> Result<String, String> {
    expand_includes(FSR_RCAS, &[("ffx_a.h", FFX_A), ("ffx_fsr1.h", FFX_FSR1)])
}

/// Повний текст шейдера NIS.
pub(crate) fn nis_source() -> Result<String, String> {
    expand_includes(
        NIS,
        &[("NIS_Main.hlsl", NIS_MAIN), ("NIS_Scaler.h", NIS_SCALER)],
    )
}

/// Константи EASU (`FsrEasuCon`): `con0..con3` поспіль, плюс розмір виходу для перевірки меж.
pub(crate) fn fsr_easu_constants(input: Size, output: Size) -> [u32; 20] {
    let (iw, ih) = (input.width as f32, input.height as f32);
    let (ow, oh) = (output.width as f32, output.height as f32);
    let rcp = |v: f32| if v == 0.0 { 0.0 } else { 1.0 / v };
    let b = f32::to_bits;
    [
        // con0: позиція виходу → позиція у вікні входу.
        b(iw * rcp(ow)),
        b(ih * rcp(oh)),
        b(0.5 * iw * rcp(ow) - 0.5),
        b(0.5 * ih * rcp(oh) - 0.5),
        // con1: піксель → нормовані координати; зсуви центрів gather4.
        b(rcp(iw)),
        b(rcp(ih)),
        b(rcp(iw)),
        b(-rcp(ih)),
        // con2
        b(-rcp(iw)),
        b(2.0 * rcp(ih)),
        b(rcp(iw)),
        b(2.0 * rcp(ih)),
        // con3
        b(0.0),
        b(4.0 * rcp(ih)),
        0,
        0,
        // Розмір виходу (не з FSR: обмеження записів за межами текстури).
        output.width,
        output.height,
        0,
        0,
    ]
}

/// Різкість 0…1 (інтерфейс) → «стопи» RCAS: 0 — максимум, 2 — слабко.
pub(crate) fn fsr_sharpness_stops(sharpness: f32) -> f32 {
    2.0 * (1.0 - sharpness.clamp(0.0, 1.0))
}

/// Константи RCAS (`FsrRcasCon`) плюс зсув прямокутника призначення та розмір кадру.
pub(crate) fn fsr_rcas_constants(stops: f32, dest_origin: (u32, u32), size: Size) -> [u32; 8] {
    let sharp = (-stops).exp2();
    let half = u32::from(f32_to_f16(sharp));
    [
        sharp.to_bits(),
        half | (half << 16),
        0,
        0,
        dest_origin.0,
        dest_origin.1,
        size.width,
        size.height,
    ]
}

/// Перетворення f32 → f16 з округленням до найближчого парного (як `f32tof16` у HLSL).
pub(crate) fn f32_to_f16(value: f32) -> u16 {
    let x = value.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let exp = ((x >> 23) & 0xff) as i32;
    let mant = x & 0x007f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if mant != 0 { 0x0200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = mant | 0x0080_0000;
        let shift = (14 - e) as u32;
        let half = m >> shift;
        let rem = m & ((1 << shift) - 1);
        let mid = 1 << (shift - 1);
        let rounded = if rem > mid || (rem == mid && half & 1 == 1) {
            half + 1
        } else {
            half
        };
        return sign | rounded as u16;
    }
    let half = ((e as u32) << 10) | (mant >> 13);
    let rem = mant & 0x1fff;
    let rounded = if rem > 0x1000 || (rem == 0x1000 && half & 1 == 1) {
        half + 1
    } else {
        half
    };
    // Перенос з мантиси в експоненту дає правильний результат, зокрема нескінченність.
    sign | rounded as u16
}

/// Розмір, до якого масштабує NIS: не більше ×2 від входу (обмеження алгоритму), решту
/// дотягує білінійний прохід. `None` — NIS не застосовний (вихід менший за вхід).
pub(crate) fn nis_output_size(input: Size, dest: Size) -> Option<Size> {
    if input.is_empty() || dest.width < input.width || dest.height < input.height {
        return None;
    }
    Some(Size::new(
        dest.width.min(input.width.saturating_mul(2)),
        dest.height.min(input.height.saturating_mul(2)),
    ))
}

/// Буфер констант NIS (`NVScalerUpdateConfig`, SDR), 32 слова = 128 байт.
/// `None` — масштаб поза межами ×1…×2.
pub(crate) fn nis_constants(sharpness: f32, input: Size, output: Size) -> Option<[u32; 32]> {
    if input.is_empty() || output.is_empty() {
        return None;
    }
    let slider = sharpness.clamp(0.0, 1.0) - 0.5;
    let max_scale = if slider >= 0.0 { 1.25 } else { 1.75 };
    let min_scale = if slider >= 0.0 { 1.25 } else { 1.0 };
    let limit_scale = if slider >= 0.0 { 1.25 } else { 1.0 };

    let detect_ratio = 2.0 * 1127.0 / 1024.0;
    let detect_thres = 64.0 / 1024.0;
    let min_contrast = 2.0f32;
    let max_contrast = 10.0f32;
    let sharp_start_y = 0.45f32;
    let sharp_end_y = 0.9f32;
    let strength_min = (0.4 + slider * min_scale * 1.2f32).max(0.0);
    let strength_max = 1.6 + slider * max_scale * 1.8f32;
    let limit_min = (0.14 + slider * limit_scale * 0.32f32).max(0.1);
    let limit_max = 0.5 + slider * limit_scale * 0.6f32;

    let scale_x = input.width as f32 / output.width as f32;
    let scale_y = input.height as f32 / output.height as f32;
    if !(0.5..=1.0).contains(&scale_x) || !(0.5..=1.0).contains(&scale_y) {
        return None;
    }
    let f = f32::to_bits;
    let mut c = [0u32; 32];
    let values = [
        f(detect_ratio),
        f(detect_thres),
        f(min_contrast),
        f(1.0 / (max_contrast - min_contrast)),
        f(1.0),         // kContrastBoost
        f(1.0 / 255.0), // kEps
        f(sharp_start_y),
        f(1.0 / (sharp_end_y - sharp_start_y)),
        f(strength_min),
        f(strength_max - strength_min),
        f(limit_min),
        f(limit_max - limit_min),
        f(scale_x),
        f(scale_y),
        f(1.0 / output.width as f32),
        f(1.0 / output.height as f32),
        f(1.0 / input.width as f32),
        f(1.0 / input.height as f32),
        0, // kInputViewportOriginX
        0,
        input.width,
        input.height,
        0, // kOutputViewportOriginX
        0,
        output.width,
        output.height,
    ];
    c[..values.len()].copy_from_slice(&values);
    Some(c)
}

/// Таблиця коефіцієнтів NIS (`coef_scale` або `coef_usm`) з `NIS_Config.h`:
/// 64 фази × 8 значень, рядок за рядком.
pub(crate) fn nis_coefficients(name: &str) -> Option<Vec<f32>> {
    let marker = format!("constexpr float {name}[kPhaseCount][kFilterSize] = {{");
    let start = NIS_CONFIG.find(&marker)? + marker.len();
    let body = &NIS_CONFIG[start..];
    let end = body.find("};")?;
    let values: Vec<f32> = body[..end]
        .split(|c: char| c == ',' || c == '{' || c == '}' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .map(|t| t.trim_end_matches('f').parse::<f32>())
        .collect::<Result<_, _>>()
        .ok()?;
    (values.len() == NIS_PHASES * NIS_TAPS).then_some(values)
}

/// Розмір текстури коефіцієнтів NIS: float4 × 2 по ширині, 64 фази по висоті.
pub(crate) const NIS_COEF_SIZE: Size = Size::new((NIS_TAPS / 4) as u32, NIS_PHASES as u32);

/// Скільки груп потоків запустити, щоб накрити `size` плитками `tile`.
pub(crate) fn dispatch_groups(size: Size, tile: (u32, u32)) -> (u32, u32) {
    (size.width.div_ceil(tile.0), size.height.div_ceil(tile.1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn vendor_sources_match_pinned_hashes() {
        let pinned = [
            (
                FFX_A,
                "f60e2722fcd13989523b9164d776ab382b3692791767f3bf8bb19967f763f3fb",
            ),
            (
                FFX_FSR1,
                "93c3922362ea7fc99cbcc698ca30c98de4f8c246d1fbb0b09e015ddef38ce3a5",
            ),
            (
                NIS_SCALER,
                "2d3bd5b45d6a0c4169fe36dfe349e7ff5a099dff6fb6876f8a3286489f74d3bf",
            ),
            (
                NIS_MAIN,
                "46402103801007b402aeeb8481d60b1bda37799bf68b543b32daea65f255f301",
            ),
            (
                NIS_CONFIG,
                "f33c5dd82bb63f3c1cc7cefd86690053de0bd3fe0e730463a5e63e90e7778754",
            ),
        ];
        for (source, hash) in pinned {
            let digest = Sha256::digest(source.as_bytes());
            let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(hex, hash);
        }
    }

    #[test]
    fn includes_are_expanded_recursively() -> Result<(), String> {
        let files = [("a.h", "A1\n#include \"b.h\"\nA2"), ("b.h", "B")];
        let out = expand_includes("top\n  #include \"a.h\"\n// #include \"x.h\"", &files)?;
        assert_eq!(out, "top\nA1\nB\nA2\n// #include \"x.h\"\n");
        assert!(expand_includes("#include \"missing.h\"", &files).is_err());
        Ok(())
    }

    #[test]
    fn real_shader_sources_expand() -> Result<(), String> {
        for src in [fsr_easu_source()?, fsr_rcas_source()?, nis_source()?] {
            assert!(!src.lines().any(|l| l.trim_start().starts_with("#include")));
        }
        Ok(())
    }

    #[test]
    fn easu_constants_for_2x() {
        let c = fsr_easu_constants(Size::new(1280, 720), Size::new(2560, 1440));
        let f = f32::from_bits;
        assert_eq!(f(c[0]), 0.5);
        assert_eq!(f(c[1]), 0.5);
        assert_eq!(f(c[2]), -0.25);
        assert_eq!(f(c[3]), -0.25);
        assert_eq!(f(c[4]), 1.0 / 1280.0);
        assert_eq!(f(c[7]), -1.0 / 720.0);
        assert_eq!(f(c[13]), 4.0 / 720.0);
        assert_eq!(&c[16..18], &[2560, 1440]);
    }

    #[test]
    fn rcas_constants() {
        let c = fsr_rcas_constants(0.0, (10, 20), Size::new(100, 50));
        assert_eq!(f32::from_bits(c[0]), 1.0);
        assert_eq!(c[1], 0x3C00_3C00);
        assert_eq!(&c[4..], &[10, 20, 100, 50]);
        let c = fsr_rcas_constants(1.0, (0, 0), Size::new(1, 1));
        assert_eq!(f32::from_bits(c[0]), 0.5);
        assert_eq!(c[1], 0x3800_3800);
    }

    #[test]
    fn sharpness_maps_to_stops() {
        assert_eq!(fsr_sharpness_stops(1.0), 0.0);
        assert_eq!(fsr_sharpness_stops(0.0), 2.0);
        assert_eq!(fsr_sharpness_stops(0.5), 1.0);
        assert_eq!(fsr_sharpness_stops(7.0), 0.0);
    }

    #[test]
    fn f16_conversion() {
        assert_eq!(f32_to_f16(1.0), 0x3C00);
        assert_eq!(f32_to_f16(0.5), 0x3800);
        assert_eq!(f32_to_f16(-2.0), 0xC000);
        assert_eq!(f32_to_f16(0.0), 0);
        assert_eq!(f32_to_f16(65504.0), 0x7BFF);
        assert_eq!(f32_to_f16(1.0e6), 0x7C00);
        assert_eq!(f32_to_f16(f32::NAN) & 0x7C00, 0x7C00);
        // Найменше піднормальне f16 = 2^-24.
        assert_eq!(f32_to_f16(2f32.powi(-24)), 1);
        // 1 + 2^-11 — рівно посередині, округлення до парного (вниз).
        assert_eq!(f32_to_f16(1.0 + 2f32.powi(-11)), 0x3C00);
    }

    #[test]
    fn nis_size_is_capped_at_2x() {
        let i = Size::new(960, 540);
        assert_eq!(
            nis_output_size(i, Size::new(2560, 1440)),
            Some(Size::new(1920, 1080))
        );
        assert_eq!(
            nis_output_size(Size::new(1280, 720), Size::new(2560, 1440)),
            Some(Size::new(2560, 1440))
        );
        assert_eq!(
            nis_output_size(Size::new(2560, 1080), Size::new(1920, 810)),
            None
        );
    }

    #[test]
    fn nis_constants_match_reference_formulae() -> Result<(), &'static str> {
        let c = nis_constants(0.5, Size::new(1280, 720), Size::new(2560, 1440)).ok_or("none")?;
        let f = |i: usize| f32::from_bits(c[i]);
        assert!((f(0) - 2.201_171_9).abs() < 1e-6);
        assert_eq!(f(2), 2.0);
        assert_eq!(f(3), 0.125);
        assert!((f(8) - 0.4).abs() < 1e-6); // kSharpStrengthMin при 50%
        assert!((f(9) - 1.2).abs() < 1e-6); // 1.6 − 0.4
        assert_eq!(f(12), 0.5);
        assert_eq!(&c[18..26], &[0, 0, 1280, 720, 0, 0, 2560, 1440]);
        assert!(nis_constants(0.5, Size::new(640, 360), Size::new(2560, 1440)).is_none());
        assert!(nis_constants(0.5, Size::new(2560, 1440), Size::new(1280, 720)).is_none());
        Ok(())
    }

    #[test]
    fn nis_coefficient_tables_parse() -> Result<(), &'static str> {
        let scale = nis_coefficients("coef_scale").ok_or("coef_scale")?;
        let usm = nis_coefficients("coef_usm").ok_or("coef_usm")?;
        assert_eq!(scale.len(), 512);
        assert_eq!(usm.len(), 512);
        // Фаза 0 — тотожний фільтр.
        assert_eq!(&scale[..8], &[0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(
            &scale[8..14],
            &[0.0029, -0.0127, 1.0000, 0.0132, -0.0034, 0.0]
        );
        assert!(nis_coefficients("coef_nope").is_none());
        Ok(())
    }

    #[test]
    fn dispatch_covers_whole_texture() {
        assert_eq!(dispatch_groups(Size::new(2560, 1440), (16, 16)), (160, 90));
        assert_eq!(dispatch_groups(Size::new(2561, 1441), (32, 24)), (81, 61));
    }
}
