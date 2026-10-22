//! Перевірка бандл-бінарника PresentMon під час збірки: SHA-256 файлу має збігатися з
//! `presentmon.sha256`. Не збігся — збірка падає, підмінений файл у exe не потрапить.

use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

const BINARY: &str = "bin/PresentMon-2.6.0-x64.exe";
const HASH_FILE: &str = "presentmon.sha256";

fn main() {
    println!("cargo::rerun-if-changed={BINARY}");
    println!("cargo::rerun-if-changed={HASH_FILE}");

    let expected = match fs::read_to_string(HASH_FILE) {
        Ok(s) => s
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase(),
        Err(e) => {
            println!("cargo::error=не вдалося прочитати {HASH_FILE}: {e}");
            return;
        }
    };
    let bytes = match fs::read(Path::new(BINARY)) {
        Ok(b) => b,
        Err(e) => {
            println!("cargo::error=не знайдено бандл PresentMon {BINARY}: {e}");
            return;
        }
    };
    let actual: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != expected {
        println!(
            "cargo::error=SHA-256 {BINARY} не збігається: очікувано {expected}, отримано {actual}"
        );
    }
}
