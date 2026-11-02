//! Перевірка новішого драйвера NVIDIA в мережі (виняток із CLAUDE.md 5.6/5.7, погоджено
//! користувачем 2026-11-02): лише читання, лише за кнопкою або за явно ввімкненим
//! налаштуванням. Запити — лише до двох адрес NVIDIA (їх використовує сторінка завантаження
//! драйверів на nvidia.com; офіційно не задокументовані), передаються лише модель відеокарти
//! й версія ОС. Драйвер не завантажується й не запускається — лише посилання на сторінку.

use super::blocking;
use crate::error::{CmdError, CmdResult};
use optimax_audit::model::GpuVendor;
use optimax_audit::{SystemProbe, WindowsProbe};
use serde::Serialize;
use std::cmp::Ordering;
use std::time::Duration;

/// Список моделей відеокарт NVIDIA (`ParentID` — серія, `Value` — модель).
const LOOKUP_URL: &str = "https://www.nvidia.com/Download/API/lookupValueSearch.aspx?TypeID=3";
/// Пошук драйвера для моделі й ОС.
const DRIVER_URL: &str =
    "https://gfwsl.geforce.com/services_toolkit/services/com/nvidia/services/AjaxDriverService.php";
/// Сторінку драйвера відкриваємо лише з цього сайту.
const PAGE_PREFIX: &str = "https://www.nvidia.com/";
/// Ідентифікатори ОС у пошуку NVIDIA.
const OS_WIN10_64: u32 = 57;
const OS_WIN11: u32 = 135;

/// Модель відеокарти в каталозі NVIDIA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Product {
    /// Серія (`psid`).
    pub series: u32,
    /// Модель (`pfid`).
    pub model: u32,
}

/// Останній драйвер з відповіді NVIDIA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Latest {
    /// Версія, напр. `621.01`.
    pub version: String,
    /// Дата випуску `YYYY-MM-DD`, якщо вдалося розібрати.
    pub released: Option<String>,
    /// Сторінка драйвера на nvidia.com.
    pub page: Option<String>,
}

/// Результат перевірки для інтерфейсу.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DriverCheck {
    /// Відеокарти NVIDIA немає (для AMD/Intel перевірки в мережі немає).
    NotNvidia,
    /// Модель не знайдено в каталозі NVIDIA.
    UnknownModel {
        /// Назва відеокарти з Windows.
        gpu: String,
    },
    /// Відповідь NVIDIA отримано.
    Checked {
        /// Назва відеокарти.
        gpu: String,
        /// Встановлена версія (`None` — Windows не повідомила).
        installed: Option<String>,
        /// Остання версія на сайті NVIDIA.
        latest: String,
        /// Дата випуску останньої версії.
        released: Option<String>,
        /// Сторінка драйвера.
        page: Option<String>,
        /// На сайті новіша версія, ніж встановлена.
        newer: bool,
    },
}

fn decode_xml(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Назва без «NVIDIA», у нижньому регістрі, з одинарними пробілами.
fn normalize(name: &str) -> String {
    let lower = name.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    let words = match words.first() {
        Some(&"nvidia") => &words[1..],
        _ => &words[..],
    };
    words.join(" ")
}

fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = s.find(open)? + open.len();
    let len = s[start..].find(close)?;
    Some(&s[start..start + len])
}

/// Шукає модель у списку NVIDIA (`lookupValueSearch.aspx?TypeID=3`) за назвою з Windows.
pub fn find_product(xml: &str, gpu: &str) -> Option<Product> {
    let wanted = normalize(gpu);
    if wanted.is_empty() {
        return None;
    }
    xml.split("<LookupValue ").skip(1).find_map(|item| {
        let name = decode_xml(between(item, "<Name>", "</Name>")?);
        if normalize(&name) != wanted {
            return None;
        }
        Some(Product {
            series: between(item, "ParentID=\"", "\"")?.trim().parse().ok()?,
            model: between(item, "<Value>", "</Value>")?.trim().parse().ok()?,
        })
    })
}

/// Ідентифікатор ОС для пошуку: Windows 11 — збірки від 22000.
pub fn os_id(build: Option<u32>) -> u32 {
    match build {
        Some(b) if b >= 22000 => OS_WIN11,
        _ => OS_WIN10_64,
    }
}

/// Адреса пошуку драйвера (Game Ready, DCH, WHQL, найновіший).
pub fn driver_url(p: Product, os: u32) -> String {
    format!(
        "{DRIVER_URL}?func=DriverManualLookup&psid={}&pfid={}&osID={os}&languageCode=1033\
         &isWHQL=1&dch=1&sort1=0&numberOfResults=1",
        p.series, p.model
    )
}

/// Версія NVIDIA `566.36` → (566, 36).
fn parse_version(v: &str) -> Option<(u32, u32)> {
    let (major, minor) = v.trim().split_once('.')?;
    let ok = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !ok(major) || !ok(minor) {
        return None;
    }
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// Порівнює версії NVIDIA; `None`, якщо якась не у форматі `NNN.NN`.
pub fn compare_versions(a: &str, b: &str) -> Option<Ordering> {
    Some(parse_version(a)?.cmp(&parse_version(b)?))
}

/// `Wed Oct 21, 2026` → `2026-10-21`.
fn parse_date(s: &str) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut it = s.split_whitespace().skip(1);
    let name = it.next()?;
    let month = MONTHS.iter().position(|m| *m == name)? + 1;
    let day: u32 = it.next()?.trim_end_matches(',').parse().ok()?;
    let year: u32 = it.next()?.parse().ok()?;
    ((1..=31).contains(&day) && (2000..=2200).contains(&year))
        .then(|| format!("{year:04}-{month:02}-{day:02}"))
}

/// Чи безпечно відкривати адресу: лише nvidia.com, без пробілів, лапок і керівних символів.
fn safe_page(url: &str) -> bool {
    url.starts_with(PAGE_PREFIX)
        && url.len() < 300
        && url
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/:.-_?=&%".contains(c))
}

/// Розбирає відповідь пошуку драйвера.
pub fn parse_latest(json: &str) -> Option<Latest> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let info = v.get("IDS")?.get(0)?.get("downloadInfo")?;
    let version = info.get("Version")?.as_str()?.trim().to_string();
    parse_version(&version)?;
    Some(Latest {
        version,
        released: info
            .get("ReleaseDateTime")
            .and_then(|d| d.as_str())
            .and_then(parse_date),
        page: info
            .get("DetailsURL")
            .and_then(|u| u.as_str())
            .filter(|u| safe_page(u))
            .map(str::to_string),
    })
}

fn failed(e: impl std::fmt::Display) -> CmdError {
    CmdError::Failed(format!(
        "не вдалося перевірити драйвер на сайті NVIDIA: {e}"
    ))
}

async fn get(client: &reqwest::Client, url: &str) -> CmdResult<String> {
    client
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(failed)?
        .text()
        .await
        .map_err(failed)
}

/// Відеокарта NVIDIA (з найбільшою відеопам'яттю) і встановлена версія драйвера.
fn local_gpu() -> Option<(String, Option<String>)> {
    let gpu = WindowsProbe::new()
        .gpus()
        .ok()?
        .into_iter()
        .filter(|g| g.vendor == GpuVendor::Nvidia)
        .max_by_key(|g| g.vram_mib.unwrap_or(0))?;
    let installed = gpu
        .driver_version
        .as_deref()
        .and_then(optimax_audit::gpu_info::nvidia_version);
    Some((gpu.name, installed))
}

/// Перевіряє на сайті NVIDIA, чи вийшов новіший драйвер. Мережа — лише за викликом.
#[tauri::command]
pub async fn driver_check() -> CmdResult<DriverCheck> {
    let Some((gpu, installed)) = blocking(|| Ok(local_gpu())).await? else {
        return Ok(DriverCheck::NotNvidia);
    };
    let build = sysinfo::System::kernel_version()
        .and_then(|v| v.split('.').next().and_then(|b| b.trim().parse().ok()));
    tracing::info!("перевірка драйвера на сайті NVIDIA: {gpu}");
    // Той самий TLS, що в оновленні програми (rustls + ring).
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    let client = reqwest::Client::builder()
        .user_agent(concat!("OptimaX/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(failed)?;
    let list = get(&client, LOOKUP_URL).await?;
    let Some(product) = find_product(&list, &gpu) else {
        tracing::info!("модель {gpu} не знайдено в каталозі NVIDIA");
        return Ok(DriverCheck::UnknownModel { gpu });
    };
    let answer = get(&client, &driver_url(product, os_id(build))).await?;
    let latest = parse_latest(&answer).ok_or_else(|| failed("незрозуміла відповідь"))?;
    let newer = installed
        .as_deref()
        .and_then(|i| compare_versions(&latest.version, i))
        == Some(Ordering::Greater);
    tracing::info!(
        "драйвер NVIDIA: встановлено {}, на сайті {}",
        installed.as_deref().unwrap_or("?"),
        latest.version
    );
    Ok(DriverCheck::Checked {
        gpu,
        installed,
        latest: latest.version,
        released: latest.released,
        page: latest.page,
        newer,
    })
}

/// Відкриває сторінку драйвера в браузері (лише адреси nvidia.com з відповіді перевірки).
#[tauri::command]
pub fn driver_open_page(url: String) -> CmdResult<()> {
    if !safe_page(&url) {
        return Err(CmdError::Forbidden(format!("неочікувана адреса: {url}")));
    }
    let explorer = optimax_core::sysdirs::explorer()
        .ok_or_else(|| CmdError::Failed("не вдалося визначити теку Windows".into()))?;
    std::process::Command::new(explorer)
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| CmdError::Failed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?><LookupValueSearch><LookupValues><LookupValue ParentID="127"><Name>NVIDIA GeForce RTX 4070 Ti SUPER</Name><Value>1040</Value></LookupValue><LookupValue ParentID="127"><Name>NVIDIA GeForce RTX 4070 SUPER</Name><Value>1039</Value></LookupValue><LookupValue ParentID="129"><Name>GeForce RTX 4070 Laptop GPU</Name><Value>1006</Value></LookupValue><LookupValue ParentID="9"><Name>Quadro P&amp;Q</Name><Value>77</Value></LookupValue></LookupValues></LookupValueSearch>"#;

    #[test]
    fn finds_model_by_windows_name() {
        let p = |series, model| Some(Product { series, model });
        assert_eq!(
            find_product(LIST, "NVIDIA GeForce RTX 4070 SUPER"),
            p(127, 1039)
        );
        // Ноутбучна назва в каталозі без «NVIDIA»; зайві пробіли й регістр не заважають.
        assert_eq!(
            find_product(LIST, "NVIDIA  GeForce RTX 4070 laptop GPU"),
            p(129, 1006)
        );
        assert_eq!(find_product(LIST, "Quadro P&Q"), p(9, 77));
        // Не «найсхожіша», а точна назва: Ti SUPER ≠ SUPER, невідома — None.
        assert_eq!(find_product(LIST, "NVIDIA GeForce RTX 4070"), None);
        assert_eq!(find_product(LIST, "NVIDIA"), None);
        assert_eq!(
            find_product("<html>", "NVIDIA GeForce RTX 4070 SUPER"),
            None
        );
    }

    #[test]
    fn parses_driver_answer() {
        let json = r#"{ "Success" : "1", "IDS" : [{ "downloadInfo": { "Version" : "621.01", "ReleaseDateTime" : "Wed Oct 21, 2026", "DetailsURL" : "https://www.nvidia.com/en-us/drivers/details/282412/" } }] }"#;
        assert_eq!(
            parse_latest(json),
            Some(Latest {
                version: "621.01".into(),
                released: Some("2026-10-21".into()),
                page: Some("https://www.nvidia.com/en-us/drivers/details/282412/".into()),
            })
        );
        // Чуже посилання не відкриваємо; дивна версія — відповідь не приймаємо.
        let evil = json.replace("https://www.nvidia.com/", "https://evil.example/");
        assert_eq!(parse_latest(&evil).and_then(|l| l.page), None);
        assert_eq!(parse_latest(&json.replace("621.01", "abc")), None);
        assert_eq!(parse_latest(r#"{ "Success" : "0" }"#), None);
        assert_eq!(parse_latest("not json"), None);
    }

    #[test]
    fn versions_os_and_url() {
        assert_eq!(
            compare_versions("621.01", "566.36"),
            Some(Ordering::Greater)
        );
        assert_eq!(compare_versions("566.36", "566.36"), Some(Ordering::Equal));
        assert_eq!(compare_versions("566.4", "566.36"), Some(Ordering::Less));
        assert_eq!(compare_versions("621.01", "32.0.15.6636"), None);
        assert_eq!(os_id(Some(26300)), OS_WIN11);
        assert_eq!(os_id(Some(19045)), OS_WIN10_64);
        assert_eq!(os_id(None), OS_WIN10_64);
        let url = driver_url(
            Product {
                series: 127,
                model: 1039,
            },
            OS_WIN11,
        );
        assert!(url.starts_with(DRIVER_URL));
        assert!(url.contains("psid=127&pfid=1039&osID=135&"));
        assert!(safe_page(
            "https://www.nvidia.com/en-us/drivers/details/282412/"
        ));
        assert!(!safe_page("https://www.nvidia.com/x\" & calc"));
        assert!(!safe_page("https://www.nvidia.com.evil.example/"));
        assert_eq!(
            parse_date("Wed Oct 21, 2026").as_deref(),
            Some("2026-10-21")
        );
        assert_eq!(parse_date("21.10.2026"), None);
    }
}
