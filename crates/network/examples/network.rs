//! Ручна перевірка мережі:
//!
//! ```text
//! cargo run --release -p optimax-network --example network -- [вузол] [секунди]
//! ```
//!
//! Показує адаптери, пінгує роутер і вузол (за замовчуванням — DNS із налаштувань мережі,
//! якщо це не сам роутер) і друкує висновки. Система нічого не змінює.

use optimax_core::Catalog;
use optimax_network::{
    analyze, analyze_link, network_info, resolve, run_test, system_echo, CancelToken, Measurement,
    TestOptions,
};
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let catalog = Catalog::ukrainian()?;
    let info = network_info()?;
    for a in &info.adapters {
        println!(
            "{} {:?} «{}» ({}) — {} Мбіт/с, шлюз {}, DNS {:?}",
            if a.carries_internet { "→" } else { " " },
            a.kind,
            a.name,
            a.description,
            a.link_mbps.map_or("?".into(), |m| m.to_string()),
            a.gateway.as_deref().unwrap_or("—"),
            a.dns,
        );
        if let Some(w) = &a.wifi {
            println!("    Wi-Fi: {w:?}");
        }
    }
    let link = info.internet();
    for f in analyze_link(link) {
        println!("  • {}", catalog.render(&f.title));
    }

    let seconds: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(15);
    let remote_text = args.first().cloned().or_else(|| info.default_remote());
    let gateway = link
        .and_then(|l| l.gateway.as_deref())
        .and_then(|g| resolve(g).ok());
    let remote = match remote_text.as_deref() {
        Some(r) => Some(resolve(r)?),
        None => None,
    };
    let mut targets = Vec::new();
    targets.extend(gateway);
    targets.extend(remote);
    anyhow::ensure!(!targets.is_empty(), "немає ні шлюзу, ні вузла для заміру");
    println!("\nЗамір {seconds} с: {targets:?}");

    let echo = system_echo();
    let stats = run_test(
        echo.as_ref(),
        &targets,
        TestOptions::new(Duration::from_secs(seconds)),
        &CancelToken::new(),
        &mut |s, t| {
            let line: Vec<String> = s
                .iter()
                .map(|p| {
                    format!(
                        "{}: сер {:.2} мс, джитер {:.2} мс, втрати {:.1}% ({}/{})",
                        p.target,
                        p.avg_ms.unwrap_or(f64::NAN),
                        p.jitter_ms.unwrap_or(f64::NAN),
                        p.loss_pct,
                        p.received,
                        p.sent
                    )
                })
                .collect();
            println!("  {t:>4.1} с | {}", line.join(" | "));
        },
    );
    let mut it = stats.into_iter();
    let m = Measurement {
        gateway: gateway.and_then(|_| it.next()),
        remote: remote.and_then(|_| it.next()),
    };
    println!();
    for f in analyze(&m, link) {
        println!("[{:?}] {}", f.impact, catalog.render(&f.title));
        println!("    {}", catalog.render(&f.detail));
        if let Some(a) = &f.advice {
            println!("    → {}", catalog.render(a));
        }
    }
    Ok(())
}
