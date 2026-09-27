//! Embed `assets/icon.ico` as icon resource #1. The window titlebar/taskbar
//! pick it up via `AppWindow.SetIcon("1")`, and the tray via
//! `Icon::from_resource(1)` — no asset files need shipping next to the exe.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("../../assets/icon.ico")
            .compile()
            .expect("embed icon resource");
    }
}
