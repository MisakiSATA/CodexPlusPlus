use std::time::Duration;

use anyhow::{Context, bail};
use serde_json::json;

const MENU_LOCALIZATION_RETRIES: usize = 20;
const MENU_LOCALIZATION_RETRY_DELAY: Duration = Duration::from_millis(500);

const MENU_LABEL_TRANSLATIONS: &[(&str, &str)] = &[
    ("File", "文件"),
    ("Edit", "编辑"),
    ("View", "视图"),
    ("Window", "窗口"),
    ("Help", "帮助"),
    ("Undo", "撤销"),
    ("Redo", "重做"),
    ("Cut", "剪切"),
    ("Copy", "复制"),
    ("Paste", "粘贴"),
    ("Delete", "删除"),
    ("Select All", "全选"),
    ("Copy conversation path", "复制对话路径"),
    ("Copy deeplink", "复制深度链接"),
    ("Copy session id", "复制会话 ID"),
    ("Copy working directory", "复制工作目录"),
    ("Close Tab", "关闭标签页"),
    ("Close", "关闭"),
    ("Reload Browser Page", "重新加载浏览器页面"),
    ("Force Reload Browser Page", "强制重新加载浏览器页面"),
    ("New Window", "新建窗口"),
    ("Open command menu", "打开命令菜单"),
    ("Search Chats…", "搜索对话..."),
    ("Search Files…", "搜索文件..."),
    ("Rename chat", "重命名对话"),
    ("Toggle File Tree", "切换文件树"),
    ("Start Trace Recording", "开始跟踪录制"),
    ("New Chat", "新建对话"),
    ("Quick Chat", "快速对话"),
    ("Open in New Window", "在新窗口中打开"),
    ("Archive chat", "归档对话"),
    ("Pin/unpin chat", "固定/取消固定对话"),
    ("Dictation", "听写"),
    ("Wake Pet", "唤醒助手"),
    ("Previous Chat", "上一个对话"),
    ("Next Chat", "下一个对话"),
    ("Settings…", "设置..."),
    ("Keyboard Shortcuts", "键盘快捷键"),
    ("Process Manager", "进程管理器"),
    ("Open Folder…", "打开文件夹..."),
    ("Toggle Sidebar", "切换边栏"),
    ("Toggle Bottom Panel", "切换底部面板"),
    ("Toggle Pinned Summary", "切换固定摘要"),
    ("Open Terminal", "打开终端"),
    ("Open Browser Tab", "打开浏览器标签页"),
    ("Toggle Browser Panel", "切换浏览器面板"),
    ("Toggle Side Panel", "切换侧边面板"),
    ("Find", "查找"),
    ("Focus Browser Address Bar", "聚焦浏览器地址栏"),
    ("Back", "后退"),
    ("Forward", "前进"),
    ("Go to Chat 1", "转到对话 1"),
    ("Go to Chat 2", "转到对话 2"),
    ("Go to Chat 3", "转到对话 3"),
    ("Go to Chat 4", "转到对话 4"),
    ("Go to Chat 5", "转到对话 5"),
    ("Go to Chat 6", "转到对话 6"),
    ("Go to Chat 7", "转到对话 7"),
    ("Go to Chat 8", "转到对话 8"),
    ("Go to Chat 9", "转到对话 9"),
    ("Log Out", "退出登录"),
    ("Reload Window", "重新加载窗口"),
    ("Zoom In", "放大"),
    ("Zoom Out", "缩小"),
    ("Actual Size", "实际大小"),
    ("Toggle Full Screen", "切换全屏"),
    ("Codex Documentation", "Codex 文档"),
    ("What's new", "更新内容"),
    ("Automations", "自动化"),
    ("Local Environments", "本地环境"),
    ("Worktrees", "工作树"),
    ("Skills", "技能"),
    ("Model Context Protocol", "模型上下文协议"),
    ("Troubleshooting", "故障排查"),
    ("Send Feedback", "发送反馈"),
    ("Check for Updates…", "检查更新..."),
    ("Updates Unavailable", "更新不可用"),
    ("Toggle Debug Menu", "切换调试菜单"),
    ("Open Deeplink from Clipboard", "从剪贴板打开深度链接"),
    ("Toggle Query Devtools", "切换查询 DevTools"),
    ("Toggle React Scan", "切换 React Scan"),
];

pub async fn install_native_menu_localizer(inspector_port: u16) -> anyhow::Result<()> {
    let mut last_error = None;
    for attempt in 1..=MENU_LOCALIZATION_RETRIES {
        match try_install_native_menu_localizer(inspector_port).await {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "native_menu.localization_retry_failed",
                    json!({
                        "inspector_port": inspector_port,
                        "attempt": attempt,
                        "message": last_error.as_ref().map(ToString::to_string).unwrap_or_default()
                    }),
                );
                tokio::time::sleep(MENU_LOCALIZATION_RETRY_DELAY).await;
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("native menu localization failed")))
}

/// Codex 桌面版从这个版本起，菜单栏改为渲染层绘制并自带中文本地化。
///
/// 在这些版本上从外部改写 `Menu`（改 label 或重新 `setApplicationMenu`）会让渲染层抛出
/// `TypeError: n is not a function`，触发「ChatGPT 出现了问题」错误页。脚本会按
/// `app.getVersion()` 自动跳过 >= 该版本的应用。
pub const NATIVE_MENU_I18N_SINCE: (u32, u32) = (26, 908);

const NATIVE_MENU_LOCALIZER_TEMPLATE: &str = r#"
(() => {
  const translations = new Map(__TRANSLATIONS__);
  const translatedLabels = new Set(translations.values());
  const electron = process.mainModule?.require?.("electron");
  if (!electron?.Menu) return JSON.stringify({ status: "skipped", reason: "electron-menu-unavailable" });
  const appVersion = (() => {
    try { return String(electron.app?.getVersion?.() ?? ""); } catch { return ""; }
  })();
  // Codex __I18N_MAJOR__.__I18N_MINOR__ 起菜单栏改为渲染层绘制并自带中文本地化；此时从外部改写 Menu
  // （无论是改 label 还是重新 setApplicationMenu）都会让渲染层抛 "n is not a function"，
  // 触发「ChatGPT 出现了问题」错误页。因此新版本一律不干预。
  const NATIVE_MENU_I18N_SINCE = [__I18N_MAJOR__, __I18N_MINOR__];
  const versionParts = appVersion.split(".").map((part) => Number.parseInt(part, 10));
  const versionKnown = Number.isFinite(versionParts[0]) && Number.isFinite(versionParts[1]);
  if (!versionKnown) {
    return JSON.stringify({ status: "skipped", reason: "app-version-unknown", appVersion });
  }
  const [major, minor] = versionParts;
  if (major > NATIVE_MENU_I18N_SINCE[0] || (major === NATIVE_MENU_I18N_SINCE[0] && minor >= NATIVE_MENU_I18N_SINCE[1])) {
    return JSON.stringify({ status: "skipped", reason: "native-menu-i18n", appVersion });
  }
  const Menu = electron.Menu;
  let changed = 0;
  const topLabels = (menu) => (Array.isArray(menu?.items) ? menu.items.map((item) => item?.label) : []);
  // 顶层任一 label 已是译文，说明应用自己完成了本地化，不再需要（也不应该）我们介入。
  const alreadyLocalized = (menu) => topLabels(menu).some((label) => translatedLabels.has(label));
  const translateItem = (item) => {
    if (!item) return;
    const nextLabel = translations.get(item.label);
    if (nextLabel && item.label !== nextLabel) {
      item.label = nextLabel;
      changed += 1;
    }
    if (item.submenu?.items) {
      for (const child of item.submenu.items) translateItem(child);
    }
  };
  const translateMenu = (menu) => {
    if (!menu?.items) return menu;
    for (const item of menu.items) translateItem(item);
    return menu;
  };
  if (!globalThis.__codexPlusNativeMenuLocalizerInstalled) {
    globalThis.__codexPlusNativeMenuLocalizerInstalled = true;
    const originalSetApplicationMenu = Menu.setApplicationMenu.bind(Menu);
    Menu.setApplicationMenu = (menu) => {
      if (alreadyLocalized(menu)) globalThis.__codexPlusNativeMenuLocalizerNativeI18n = true;
      if (!globalThis.__codexPlusNativeMenuLocalizerNativeI18n) {
        try { translateMenu(menu); } catch {}
      }
      return originalSetApplicationMenu(menu);
    };
  }
  const menu = Menu.getApplicationMenu();
  if (menu && alreadyLocalized(menu)) {
    globalThis.__codexPlusNativeMenuLocalizerNativeI18n = true;
    return JSON.stringify({ status: "skipped", reason: "menu-already-localized", appVersion, topLabels: topLabels(menu) });
  }
  if (menu) {
    translateMenu(menu);
    // 只有真的改了 label 才重新 set；原样重新 set 同一个菜单没有意义，还会在新版本上触发渲染层崩溃。
    if (changed > 0) Menu.setApplicationMenu(menu);
  }
  return JSON.stringify({ status: "ok", changed, appVersion, topLabels: topLabels(menu) });
})()
"#;

pub fn native_menu_localizer_script() -> anyhow::Result<String> {
    let translations =
        serde_json::to_string(&MENU_LABEL_TRANSLATIONS.iter().copied().collect::<Vec<_>>())?;
    Ok(NATIVE_MENU_LOCALIZER_TEMPLATE
        .replace("__TRANSLATIONS__", &translations)
        .replace("__I18N_MAJOR__", &NATIVE_MENU_I18N_SINCE.0.to_string())
        .replace("__I18N_MINOR__", &NATIVE_MENU_I18N_SINCE.1.to_string()))
}

/// 从 `Runtime.evaluate` 的返回值里解出脚本自己序列化的 JSON 结果。
fn localizer_script_outcome(result: &serde_json::Value) -> Option<serde_json::Value> {
    let value = result
        .get("result")
        .and_then(|value| value.get("result"))
        .and_then(|value| value.get("value"))
        .and_then(serde_json::Value::as_str)?;
    serde_json::from_str(value).ok()
}

async fn try_install_native_menu_localizer(inspector_port: u16) -> anyhow::Result<()> {
    let targets = crate::cdp::list_targets(inspector_port).await?;
    let target = targets
        .iter()
        .find(|target| {
            target
                .web_socket_debugger_url
                .as_deref()
                .is_some_and(|url| !url.is_empty())
                && target.target_type == "node"
        })
        .or_else(|| {
            targets.iter().find(|target| {
                target
                    .web_socket_debugger_url
                    .as_deref()
                    .is_some_and(|url| !url.is_empty())
            })
        })
        .context("No Electron main-process inspector target found")?;
    let websocket_url = target
        .web_socket_debugger_url
        .as_deref()
        .context("selected inspector target has no websocket URL")?;
    let script = native_menu_localizer_script()?;
    let result = crate::bridge::evaluate_script_with_await_promise(websocket_url, &script, true)
        .await
        .context("failed to evaluate native menu localizer")?;
    if let Some(exception) = result
        .get("result")
        .and_then(|value| value.get("exceptionDetails"))
    {
        bail!("native menu localizer threw: {exception}");
    }
    let outcome = localizer_script_outcome(&result);
    let skipped = outcome
        .as_ref()
        .and_then(|outcome| outcome.get("status"))
        .and_then(serde_json::Value::as_str)
        == Some("skipped");
    let event = if skipped {
        "native_menu.localization_skipped"
    } else {
        "native_menu.localization_installed"
    };
    let _ = crate::diagnostic_log::append_diagnostic_log(
        event,
        json!({
            "inspector_port": inspector_port,
            "target_type": target.target_type,
            "target_title": target.title,
            "outcome": outcome,
            "result": result
        }),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_menu_localizer_script_uses_runtime_menu_patch() {
        let script = native_menu_localizer_script().unwrap();

        assert!(script.contains("Menu.setApplicationMenu"));
        assert!(script.contains("Toggle Sidebar"));
        assert!(script.contains("切换边栏"));
        assert!(!script.contains("app.asar"));
        assert!(!script.contains("__TRANSLATIONS__"));
    }

    #[test]
    fn native_menu_localizer_script_skips_codex_versions_with_native_menu_i18n() {
        let script = native_menu_localizer_script().unwrap();

        assert!(script.contains("electron.app?.getVersion?.()"));
        assert!(script.contains("const NATIVE_MENU_I18N_SINCE = [26, 908];"));
        assert!(script.contains(r#"reason: "native-menu-i18n""#));
        assert!(script.contains(r#"reason: "app-version-unknown""#));
        assert!(!script.contains("__I18N_MAJOR__"));
        assert!(!script.contains("__I18N_MINOR__"));
    }

    #[test]
    fn native_menu_localizer_script_never_resets_an_unchanged_menu() {
        let script = native_menu_localizer_script().unwrap();

        assert!(script.contains("if (changed > 0) Menu.setApplicationMenu(menu);"));
        assert!(script.contains(r#"reason: "menu-already-localized""#));
        assert!(script.contains("__codexPlusNativeMenuLocalizerNativeI18n"));
    }

    #[test]
    fn localizer_script_outcome_reads_serialized_status() {
        let result = json!({
            "id": 1,
            "result": {
                "result": {
                    "type": "string",
                    "value": r#"{"status":"skipped","reason":"native-menu-i18n","appVersion":"26.908.31748"}"#
                }
            }
        });

        let outcome = localizer_script_outcome(&result).unwrap();

        assert_eq!(outcome["status"], "skipped");
        assert_eq!(outcome["reason"], "native-menu-i18n");
        assert!(localizer_script_outcome(&json!({"id": 1})).is_none());
    }
}
