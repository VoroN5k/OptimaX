//! Вбудовує маніфест Windows: програма завжди запускається з правами адміністратора.
//!
//! Лише для debug-збірки розробника: `OPTIMAX_DEV_AS_INVOKER=1` дає `asInvoker`, щоб відкрити
//! вікно без UAC (системні зміни тоді не спрацюють — це очікувано). Release ігнорує змінну.

fn main() {
    println!("cargo::rerun-if-changed=app.manifest");
    println!("cargo::rerun-if-env-changed=OPTIMAX_DEV_AS_INVOKER");
    let debug = std::env::var("PROFILE").is_ok_and(|p| p == "debug");
    let as_invoker = debug && std::env::var("OPTIMAX_DEV_AS_INVOKER").is_ok_and(|v| v == "1");
    let level = if as_invoker {
        "asInvoker"
    } else {
        "requireAdministrator"
    };
    let manifest = include_str!("app.manifest").replace("__LEVEL__", level);
    let windows = tauri_build::WindowsAttributes::new().app_manifest(manifest);
    if let Err(e) =
        tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
    {
        println!("cargo::error=tauri-build: {e}");
    }
}
