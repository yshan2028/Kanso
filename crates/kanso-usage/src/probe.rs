/// 探测本地命令行工具，当前返回空的占位结果。
pub fn probe_tools() -> Vec<crate::model::ToolVersion> {
    Vec::new()
}

/// 探测桌面应用和更新机制，当前仅返回占位状态。
pub fn probe_desktop_app(
    _app_dir: &std::path::Path,
    _defaults_domain: &str,
) -> crate::model::DesktopAppInfo {
    crate::model::DesktopAppInfo {
        installed: false,
        name: "ChatGPT".to_string(),
        version: None,
        updater: crate::model::UpdaterKind::Unknown,
        auto_check: None,
        auto_install: None,
        last_check: None,
    }
}
