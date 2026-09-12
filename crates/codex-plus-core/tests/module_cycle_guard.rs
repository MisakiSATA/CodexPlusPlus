use codex_plus_core::assets::injection_script_with_settings;
use codex_plus_core::settings::BackendSettings;

/// Codex 26.908 的 authed-route ↔ app-primary 模块循环会在应用自己的并发 import 中随机以
/// 错误顺序求值（"n is not a function"），注入脚本必须在文档最早期抢先从 authed-route 一侧
/// import 一次来固定顺序。
#[test]
fn injection_script_installs_module_cycle_guard_before_other_patches() {
    let script = injection_script_with_settings(57321, &BackendSettings::default());

    let guard = script
        .find("function installCodexModuleCycleGuard()")
        .expect("module cycle guard should be defined");
    let call = script
        .find("\n    installCodexModuleCycleGuard();")
        .expect("module cycle guard should be invoked");
    let fast_startup = script
        .find("function installCodexPlusFastStartup()")
        .expect("fast startup patch should still exist");

    assert!(guard < call, "guard must be defined before it is invoked");
    assert!(
        call < fast_startup,
        "guard must run before any other renderer patch touches the page"
    );
    assert!(script.contains("\"authed-route-\""));
    assert!(script.contains("\"app-initial-\""));
    assert!(script.contains("await import(routeUrl);"));
    assert!(script.contains("sendCodexPlusDiagnostic(\"module_cycle_guard\""));
}
