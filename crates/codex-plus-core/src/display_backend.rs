use serde::Serialize;

/// Linux 图形会话与管理器实际使用的显示后端诊断。
/// 设计要求：诊断必须能看到请求的会话类型、生效的后端，
/// 以及是否处于 XWayland / 软件合成回退状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayBackendDiagnostics {
    /// 登录会话请求的类型：wayland、x11 或 unknown。
    pub requested_session: String,
    /// GTK 实际使用的后端：wayland 或 x11。
    pub effective_backend: String,
    /// Wayland 会话下通过 XWayland 运行（当前阶段的默认兼容模式）。
    pub xwayland_fallback: bool,
    /// WebKitGTK 软件合成回退（规避部分驱动的黑屏问题）。
    pub software_compositing_fallback: bool,
}

pub fn display_backend_diagnostics_from(
    wayland_display: Option<&str>,
    x11_display: Option<&str>,
    gdk_backend: Option<&str>,
    compositing_disabled: Option<&str>,
) -> DisplayBackendDiagnostics {
    let has_wayland = wayland_display.is_some_and(|value| !value.trim().is_empty());
    let has_x11 = x11_display.is_some_and(|value| !value.trim().is_empty());
    let requested_session = if has_wayland {
        "wayland"
    } else if has_x11 {
        "x11"
    } else {
        "unknown"
    };
    let effective_backend = match gdk_backend.map(str::trim) {
        Some(value) if !value.is_empty() => {
            // GDK_BACKEND 可以是候选列表（如 "x11,wayland"），第一项生效。
            value.split(',').next().unwrap_or(value).to_string()
        }
        _ => {
            if has_wayland {
                "wayland".to_string()
            } else {
                "x11".to_string()
            }
        }
    };
    DisplayBackendDiagnostics {
        requested_session: requested_session.to_string(),
        xwayland_fallback: requested_session == "wayland" && effective_backend == "x11",
        software_compositing_fallback: compositing_disabled
            .is_some_and(|value| value.trim() == "1"),
        effective_backend,
    }
}

/// 从当前进程环境读取显示后端诊断。
pub fn display_backend_diagnostics() -> DisplayBackendDiagnostics {
    let wayland = std::env::var("WAYLAND_DISPLAY").ok();
    let x11 = std::env::var("DISPLAY").ok();
    let gdk = std::env::var("GDK_BACKEND").ok();
    let compositing = std::env::var("WEBKIT_DISABLE_COMPOSITING_MODE").ok();
    display_backend_diagnostics_from(
        wayland.as_deref(),
        x11.as_deref(),
        gdk.as_deref(),
        compositing.as_deref(),
    )
}
