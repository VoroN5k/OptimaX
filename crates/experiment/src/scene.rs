//! «Відбиток» сцени: чи схожий прогін на решту прогонів свого варіанта.
//!
//! Порівнюються лише прогони одного варіанта (зміна сама може змінити і рівень, і форму).
//! Рівень — середній FPS проти медіани решти; форма — квантилі frametime, поділені на медіану
//! прогону (не залежать від загальної швидкості): інша сцена зазвичай дає іншу частку довгих
//! кадрів. Позначка — лише підказка: виключає прогін користувач.

use crate::stats::median;
use crate::{Run, RunSummary, Suspect};

/// Відхилення середнього FPS від медіани решти прогонів варіанта, з якого прогін підозрілий.
const LEVEL_MAX: f64 = 0.05;
/// Межа відстані форм (сума |різниць| нормованих квантилів).
const SHAPE_MAX: f64 = 0.3;
/// Квантилі форми (плюс середнє найгірших 1 % — рідкі довгі кадри квантиль не ловить).
const QUANTILES: [f64; 4] = [0.10, 0.25, 0.75, 0.90];
/// Найменше прогонів варіанта (включно з перевірюваним) для порівняння.
const MIN_RUNS: usize = 3;

/// Форма розподілу frametime прогону: квантилі й середнє найгірших 1 %, поділені на медіану.
pub fn shape(frametimes_ms: &[f64]) -> Option<[f64; 5]> {
    let mut v: Vec<f64> = frametimes_ms
        .iter()
        .copied()
        .filter(|x| x.is_finite() && *x > 0.0)
        .collect();
    if v.len() < 100 {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    let med = at(0.5);
    if med <= 0.0 {
        return None;
    }
    let k = ((v.len() as f64 * 0.01).ceil() as usize).clamp(1, v.len());
    let worst = v[v.len() - k..].iter().sum::<f64>() / k as f64;
    let [a, b, c, d] = QUANTILES.map(|q| at(q) / med);
    Some([a, b, c, d, worst / med])
}

fn distance(a: &[f64; 5], b: &[f64; 5]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum()
}

/// Позначає прогони, що відрізняються від решти свого варіанта (виключені не враховуються).
pub fn mark_suspects(runs: &[Run], sums: &mut [RunSummary]) {
    let shapes: Vec<Option<[f64; 5]>> = runs.iter().map(|r| shape(&r.frametimes_ms)).collect();
    let n = sums.len().min(runs.len());
    for i in 0..n {
        if sums[i].excluded {
            continue;
        }
        let peers: Vec<usize> = (0..n)
            .filter(|&j| j != i && !sums[j].excluded && sums[j].arm == sums[i].arm)
            .collect();
        if peers.len() + 1 < MIN_RUNS {
            continue;
        }
        let level = median(&peers.iter().map(|&j| sums[j].avg_fps).collect::<Vec<_>>());
        if level > 0.0 && (sums[i].avg_fps / level - 1.0).abs() > LEVEL_MAX {
            sums[i].suspect = Some(Suspect::Level);
            continue;
        }
        let Some(own) = shapes[i] else {
            continue;
        };
        let peer_shapes: Vec<[f64; 5]> = peers.iter().filter_map(|&j| shapes[j]).collect();
        if peer_shapes.len() + 1 < MIN_RUNS {
            continue;
        }
        let typical: [f64; 5] =
            std::array::from_fn(|k| median(&peer_shapes.iter().map(|s| s[k]).collect::<Vec<_>>()));
        if distance(&own, &typical) > SHAPE_MAX {
            sums[i].suspect = Some(Suspect::Shape);
        }
    }
}
