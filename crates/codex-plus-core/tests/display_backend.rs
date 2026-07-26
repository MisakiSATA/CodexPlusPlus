use codex_plus_core::display_backend::display_backend_diagnostics_from;

#[test]
fn wayland_session_with_forced_x11_backend_reports_xwayland_fallback() {
    let diagnostics =
        display_backend_diagnostics_from(Some("wayland-0"), Some(":0"), Some("x11"), Some("1"));
    assert_eq!(diagnostics.requested_session, "wayland");
    assert_eq!(diagnostics.effective_backend, "x11");
    assert!(diagnostics.xwayland_fallback);
    assert!(diagnostics.software_compositing_fallback);
}

#[test]
fn wayland_session_without_override_uses_native_backend() {
    let diagnostics = display_backend_diagnostics_from(Some("wayland-0"), Some(":0"), None, None);
    assert_eq!(diagnostics.requested_session, "wayland");
    assert_eq!(diagnostics.effective_backend, "wayland");
    assert!(!diagnostics.xwayland_fallback);
    assert!(!diagnostics.software_compositing_fallback);
}

#[test]
fn plain_x11_session_never_reports_fallback() {
    let diagnostics = display_backend_diagnostics_from(None, Some(":0"), None, Some("0"));
    assert_eq!(diagnostics.requested_session, "x11");
    assert_eq!(diagnostics.effective_backend, "x11");
    assert!(!diagnostics.xwayland_fallback);
    assert!(!diagnostics.software_compositing_fallback);
}

#[test]
fn gdk_backend_candidate_list_uses_first_entry() {
    let diagnostics =
        display_backend_diagnostics_from(Some("wayland-0"), None, Some("x11,wayland"), None);
    assert_eq!(diagnostics.effective_backend, "x11");
    assert!(diagnostics.xwayland_fallback);
}

#[test]
fn headless_environment_reports_unknown_session() {
    let diagnostics = display_backend_diagnostics_from(None, None, None, None);
    assert_eq!(diagnostics.requested_session, "unknown");
    assert!(!diagnostics.xwayland_fallback);
}
