#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    configure_linux_webview_environment();
    for arg in std::env::args() {
        if arg.starts_with("codexplusplus://") {
            match codex_plus_core::provider_import::save_pending_provider_import_from_url(&arg) {
                Ok(request) => {
                    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                        "manager.provider_import_url.pending",
                        serde_json::json!({
                            "name": request.name,
                            "baseUrl": request.base_url
                        }),
                    );
                    codex_plus_manager_lib::focus_existing_manager_window();
                }
                Err(error) => {
                    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                        "manager.provider_import_url.failed",
                        serde_json::json!({
                            "error": error.to_string()
                        }),
                    );
                }
            }
        }
    }
    if std::env::args().any(|arg| arg == "--show-update") {
        unsafe {
            std::env::set_var("CODEX_PLUS_SHOW_UPDATE", "1");
        }
    }
    codex_plus_manager_lib::run();
}

#[cfg(target_os = "linux")]
fn configure_linux_webview_environment() {
    if std::env::var_os("GDK_BACKEND").is_none()
        && std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var_os("DISPLAY").is_some()
    {
        // Tauri 的 GTK3 托盘在部分 Wayland 会话会触发协议错误。此处仍处于单线程启动阶段。
        unsafe { std::env::set_var("GDK_BACKEND", "x11") };
    }

    if std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_none() {
        // 部分 Linux 显卡驱动无法为 WebKitGTK 分配 GBM 缓冲区，会导致窗口黑屏。
        unsafe { std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1") };
    }
}

#[cfg(not(target_os = "linux"))]
fn configure_linux_webview_environment() {}
