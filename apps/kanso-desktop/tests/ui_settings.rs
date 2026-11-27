//! 路由与设置页面的无头交互测试。

use gpui_kit::base::test_support::texts;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, Point, TestAppContext, Window, WindowBounds,
    WindowOptions, px, size,
};
use kanso_core::RolePolicy;
use kanso_desktop::{AppState, Route, Settings, SettingsSub, ThemeChoice, Workbench, bind_keys};
use kanso_usage::{DesktopAppInfo, Family, PathKind, ToolVersion, UpdaterKind};
use std::path::PathBuf;
use std::process::Command;

const TEST_DATA_DIR: &str = "KANSO_UI_SETTINGS_TEST_DIR";
const UPGRADE_COMMAND: &str = "npm install -g @openai/codex";

fn with_isolated_settings(test: &str, run: impl FnOnce()) {
    if let Some(test_dir) = std::env::var_os(TEST_DATA_DIR) {
        assert!(Settings::path().starts_with(PathBuf::from(test_dir)));
        run();
        return;
    }

    // 子进程拥有独立的临时用户目录，点击保存时走正式写盘路径。
    let temp = tempfile::tempdir().expect("创建测试配置目录");
    let output = Command::new(std::env::current_exe().expect("找到当前测试程序"))
        .args(["--exact", test, "--nocapture"])
        .env(TEST_DATA_DIR, temp.path())
        .env("HOME", temp.path())
        .env("XDG_DATA_HOME", temp.path())
        .output()
        .expect("运行隔离的界面测试");
    assert!(
        output.status.success(),
        "隔离测试失败：\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn open_workbench(cx: &mut TestAppContext, route: Route) -> (AnyWindowHandle, Entity<Workbench>) {
    let (handle, workbench) = cx.update(|cx| {
        gpui_kit::init(cx);
        bind_keys(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1260.0), px(1000.0)),
                })),
                ..WindowOptions::default()
            },
            cx,
            |window, cx| {
                let state = AppState {
                    route,
                    tool_versions: vec![ToolVersion {
                        tool: "Codex".into(),
                        family: Family::OpenAi,
                        installed: true,
                        version: Some("1.2.3".into()),
                        path_kind: Some(PathKind::Npm),
                        upgrade_cmd: Some(UPGRADE_COMMAND.into()),
                    }],
                    desktop_app: Some(DesktopAppInfo {
                        installed: true,
                        name: "Codex 桌面客户端".into(),
                        version: Some("4.5.6".into()),
                        updater: UpdaterKind::Sparkle,
                        auto_check: Some(true),
                        auto_install: Some(false),
                        last_check: Some("2026-11-27 09:00".into()),
                    }),
                    ..AppState::default()
                };
                let settings = Settings {
                    theme: ThemeChoice::Dark,
                    ..Settings::default()
                };
                cx.new(|cx| Workbench::new(state, settings, None, None, None, window, cx))
            },
        )
        .expect("打开无头工作台")
    });
    cx.run_until_parked();
    (handle, workbench)
}

fn assert_step_five(window: &Window, policy: &str) {
    let step = window.find(("routing-step", 5_usize));
    assert!(step.visible());
    assert!(
        texts(window).iter().any(|text| {
            text.visible() && text.path().starts_with(step.path()) && text.text() == policy
        }),
        "第五步应显示「{policy}」"
    );
}

#[gpui_kit::test]
fn routing_policy_updates_step_five_and_saves(cx: &mut TestAppContext) {
    with_isolated_settings("routing_policy_updates_step_five_and_saves", || {
        let (handle, workbench) = open_workbench(cx, Route::Routing);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_step_five(window, "额度优先");
            for (policy, key, label) in [
                (RolePolicy::Sequential, "Sequential", "顺序分配"),
                (RolePolicy::RoundRobin, "RoundRobin", "轮询"),
                (RolePolicy::Weighted, "Weighted", "加权分配"),
                (RolePolicy::SpeedFirst, "SpeedFirst", "速度优先"),
                (RolePolicy::Sticky, "Sticky", "固定供应商"),
                (RolePolicy::QuotaFirst, "QuotaFirst", "额度优先"),
            ] {
                let id = gpui_kit::SharedString::from(format!("routing-policy-{key}"));
                assert_eq!(window.find(id.clone()).label(), Some(label));
                window.click(id, cx);
                window.render_frame(cx);
                assert_step_five(window, label);
                assert_eq!(workbench.read(cx).settings.routing_policy, policy);
                assert_eq!(Settings::load().routing_policy, policy);
            }
        })
        .expect("验证路由策略交互");
    });
}

#[gpui_kit::test]
fn settings_tabs_show_updates_and_save_skipped_version(cx: &mut TestAppContext) {
    with_isolated_settings(
        "settings_tabs_show_updates_and_save_skipped_version",
        || {
            let (handle, workbench) = open_workbench(cx, Route::Settings);
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert_eq!(workbench.read(cx).state.settings_sub, SettingsSub::Config);
                assert!(window.find("settings-config").visible());
                assert!(window.try_find("settings-update").is_none());

                window.click("settings-tab-update", cx);
                window.render_frame(cx);
                assert_eq!(workbench.read(cx).state.settings_sub, SettingsSub::Update);
                assert!(window.find("settings-update").visible());
                assert!(window.try_find("settings-config").is_none());
                for (id, expected) in [
                    ("settings-cli-versions", "Codex"),
                    ("settings-desktop-app", "更新引擎 Sparkle"),
                ] {
                    let card = window.find(id);
                    assert!(card.visible());
                    assert!(texts(window).iter().any(|text| {
                        text.visible()
                            && text.path().starts_with(card.path())
                            && text.text() == expected
                    }));
                }

                window.click(("copy-upgrade-command", 0_usize), cx);
                window.render_frame(cx);
                assert_eq!(
                    workbench.read(cx).state.ui_notice.as_deref(),
                    Some("已复制升级命令。")
                );
                assert_eq!(
                    cx.read_from_clipboard()
                        .and_then(|item| item.text())
                        .as_deref(),
                    Some(UPGRADE_COMMAND)
                );

                window.click("update-channel-beta", cx);
                window.click("update-auto-check", cx);
                window.click("update-auto-install", cx);
                let saved = Settings::load();
                assert_eq!(saved.update_channel, "beta");
                assert!(!saved.update_auto_check);
                assert!(saved.update_auto_install);

                window.click("check-for-updates", cx);
                window.render_frame(cx);
                assert!(workbench.read(cx).state.update_modal_open);
                assert!(window.find("update-modal").visible());

                window.click("update-skip", cx);
                window.render_frame(cx);
                assert!(!workbench.read(cx).state.update_modal_open);
                assert!(window.try_find("update-modal").is_none());
                assert!(window.find("update-skipped-version").visible());
                assert_eq!(
                    workbench
                        .read(cx)
                        .settings
                        .update_skipped_version
                        .as_deref(),
                    Some("0.2.0")
                );
                assert_eq!(
                    Settings::load().update_skipped_version.as_deref(),
                    Some("0.2.0")
                );

                window.click("check-for-updates", cx);
                window.click("update-later", cx);
                assert!(!workbench.read(cx).state.update_modal_open);

                window.click("check-for-updates", cx);
                window.click("update-install", cx);
                let state = &workbench.read(cx).state;
                assert!(!state.update_modal_open);
                assert_eq!(
                    state.ui_notice.as_deref(),
                    Some("已模拟安装版本 0.2.0 并重启。")
                );
                assert_eq!(
                    state.log.back().map(|entry| entry.message.as_str()),
                    state.ui_notice.as_deref()
                );

                window.click("settings-tab-config", cx);
                window.render_frame(cx);
                assert_eq!(workbench.read(cx).state.settings_sub, SettingsSub::Config);
                assert!(window.find("settings-config").visible());
                assert!(window.try_find("settings-update").is_none());
            })
            .expect("验证设置页交互");
        },
    );
}
