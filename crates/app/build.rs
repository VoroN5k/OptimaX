//! Вбудовує маніфест Windows: `asInvoker` — програма запускається без прав адміністратора.
//!
//! Права потрібні лише без служби OptimaX: тоді програма сама перезапускається з UAC
//! (`app::elevation`). Розробник у debug може не перезапускатися: `OPTIMAX_DEV_AS_INVOKER=1`.

fn main() {
    println!("cargo::rerun-if-changed=app.manifest");
    let manifest = include_str!("app.manifest");
    let windows = tauri_build::WindowsAttributes::new().app_manifest(manifest);
    if let Err(e) =
        tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
    {
        println!("cargo::error=tauri-build: {e}");
    }
}
