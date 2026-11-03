//! Статистика без залежностей: квантилі нормального й Стьюдентового розподілів, лінійна
//! регресія «рівень + варіант + час», детермінований генератор для bootstrap.

/// Квантиль стандартного нормального розподілу (алгоритм Acklam, похибка < 1.2e-9).
pub fn normal_quantile(p: f64) -> f64 {
    const A: [f64; 6] = [
        -3.969_683_028_665_376e1,
        2.209_460_984_245_205e2,
        -2.759_285_104_469_687e2,
        1.383_577_518_672_69e2,
        -3.066_479_806_614_716e1,
        2.506_628_277_459_239,
    ];
    const B: [f64; 5] = [
        -5.447_609_879_822_406e1,
        1.615_858_368_580_409e2,
        -1.556_989_798_598_866e2,
        6.680_131_188_771_972e1,
        -1.328_068_155_288_572e1,
    ];
    const C: [f64; 6] = [
        -7.784_894_002_430_293e-3,
        -3.223_964_580_411_365e-1,
        -2.400_758_277_161_838,
        -2.549_732_539_343_734,
        4.374_664_141_464_968,
        2.938_163_982_698_783,
    ];
    const D: [f64; 4] = [
        7.784_695_709_041_462e-3,
        3.224_671_290_700_398e-1,
        2.445_134_137_142_996,
        3.754_408_661_907_416,
    ];
    let p = p.clamp(1e-300, 1.0 - 1e-16);
    let tail = |q: f64| {
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };
    if p < 0.024_25 {
        tail((-2.0 * p.ln()).sqrt())
    } else if p > 1.0 - 0.024_25 {
        -tail((-2.0 * (1.0 - p).ln()).sqrt())
    } else {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    }
}

/// Квантиль розподілу Стьюдента з `df` ступенями свободи.
///
/// `df` 1 і 2 — точні формули; інакше розклад Корніша — Фішера (Hill, 1970), похибка для
/// `df ≥ 3` і двобічних рівнів до 99,9 % — менше 0,5 %. Дробове `df < 3` округлюється вниз
/// (ширший інтервал — консервативно).
pub fn t_quantile(p: f64, df: f64) -> f64 {
    if !df.is_finite() || df > 1e7 {
        return normal_quantile(p);
    }
    if df < 3.0 {
        let p = p.clamp(1e-12, 1.0 - 1e-12);
        return if df < 2.0 {
            (std::f64::consts::PI * (p - 0.5)).tan()
        } else {
            (2.0 * p - 1.0) / (2.0 * p * (1.0 - p)).sqrt()
        };
    }
    let z = normal_quantile(p);
    let z2 = z * z;
    let g1 = (z2 + 1.0) * z / 4.0;
    let g2 = ((5.0 * z2 + 16.0) * z2 + 3.0) * z / 96.0;
    let g3 = (((3.0 * z2 + 19.0) * z2 + 17.0) * z2 - 15.0) * z / 384.0;
    let g4 = ((((79.0 * z2 + 776.0) * z2 + 1482.0) * z2 - 1920.0) * z2 - 945.0) * z / 92_160.0;
    z + g1 / df + g2 / df.powi(2) + g3 / df.powi(3) + g4 / df.powi(4)
}

/// Оцінка моделі `y = рівень + ефект·[варіант B] + дрейф·(номер прогону − середній)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// Рівень варіанта A в середній момент тесту.
    pub level_a: f64,
    /// Ефект B − A.
    pub effect: f64,
    /// Стандартна похибка ефекту.
    pub effect_se: f64,
    /// Дрейф за один прогін.
    pub drift: f64,
    /// Ступені свободи залишку.
    pub df: f64,
    /// Стандартне відхилення залишку (розкид одного прогону).
    pub residual_sd: f64,
}

/// Найменші квадрати для трьох параметрів. `points` — (є B, номер прогону, значення).
/// `None`, якщо точок менше чотирьох або варіант і час не розрізняються (вироджена матриця).
pub fn fit(points: &[(bool, f64, f64)]) -> Option<Fit> {
    let n = points.len();
    if n < 4 {
        return None;
    }
    let mean_t = points.iter().map(|p| p.1).sum::<f64>() / n as f64;
    let rows: Vec<([f64; 3], f64)> = points
        .iter()
        .map(|&(b, t, y)| ([1.0, if b { 1.0 } else { 0.0 }, t - mean_t], y))
        .collect();
    let mut xtx = [[0.0f64; 3]; 3];
    let mut xty = [0.0f64; 3];
    for (x, y) in &rows {
        for i in 0..3 {
            xty[i] += x[i] * y;
            for j in 0..3 {
                xtx[i][j] += x[i] * x[j];
            }
        }
    }
    let inv = invert3(&xtx)?;
    let mut beta = [0.0f64; 3];
    for (i, b) in beta.iter_mut().enumerate() {
        *b = (0..3).map(|j| inv[i][j] * xty[j]).sum();
    }
    let ssr: f64 = rows
        .iter()
        .map(|(x, y)| {
            let pred: f64 = (0..3).map(|i| x[i] * beta[i]).sum();
            (y - pred).powi(2)
        })
        .sum();
    let df = (n - 3) as f64;
    let var = ssr / df;
    Some(Fit {
        level_a: beta[0],
        effect: beta[1],
        effect_se: (var * inv[1][1]).max(0.0).sqrt(),
        drift: beta[2],
        df,
        residual_sd: var.sqrt(),
    })
}

fn invert3(m: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let c =
        |r0: usize, r1: usize, c0: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    let cof = [
        [c(1, 2, 1, 2), -c(1, 2, 0, 2), c(1, 2, 0, 1)],
        [-c(0, 2, 1, 2), c(0, 2, 0, 2), -c(0, 2, 0, 1)],
        [c(0, 1, 1, 2), -c(0, 1, 0, 2), c(0, 1, 0, 1)],
    ];
    let det = m[0][0] * cof[0][0] + m[0][1] * cof[0][1] + m[0][2] * cof[0][2];
    let scale = m.iter().flatten().fold(0.0f64, |a, v| a.max(v.abs()));
    if !det.is_finite() || det.abs() <= 1e-9 * scale.powi(3).max(1e-300) {
        return None;
    }
    // Обернена = транспонована матриця алгебраїчних доповнень / визначник.
    let mut out = [[0.0f64; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = cof[j][i] / det;
        }
    }
    Some(out)
}

/// Медіана (порожньо — 0).
pub fn median(values: &[f64]) -> f64 {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let m = v.len() / 2;
    if v.len() % 2 == 1 {
        v[m]
    } else {
        (v[m - 1] + v[m]) / 2.0
    }
}

/// Детермінований генератор SplitMix64: однаковий результат для однакових даних.
pub struct SplitMix(u64);

impl SplitMix {
    /// Генератор із зерном.
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// Наступне число.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Ціле в `[0, n)` (`n > 0`).
    pub fn below(&mut self, n: usize) -> usize {
        ((u128::from(self.next_u64()) * n as u128) >> 64) as usize
    }
}
