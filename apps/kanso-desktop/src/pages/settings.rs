use crate::settings_view::{effective_preview, family_name};
use crate::state::LogLevel;
use crate::{Settings, SettingsSub, Workbench};
use gpui_kit::base::{Selectable as _, TestTextExt as _};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClipboardItem, Context, Div, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, Window, div, px,
};
use kanso_usage::{PathKind, UpdaterKind};

const DEMO_VERSION: &str = "0.2.0";

/// 渲染设置页及其当前选中的配置或更新子页。
pub fn render_settings(
    this: &Workbench,
    _window: &mut Window,
    cx: &mut Context<Workbench>,
) -> AnyElement {
    let theme = cx.theme();
    let foreground = theme.foreground;
    let tabs = div().flex().flex_wrap().gap_2().children(
        [
            (
                "settings-tab-config",
                "配置文件与优先级",
                SettingsSub::Config,
            ),
            ("settings-tab-update", "关于与更新", SettingsSub::Update),
        ]
        .into_iter()
        .map(|(id, label, sub)| {
            Button::new(id)
                .small()
                .outline()
                .selected(sub == this.state.settings_sub)
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.state.settings_sub = sub;
                    this.state.ui_notice = None;
                    cx.notify();
                }))
        }),
    );
    let body = match this.state.settings_sub {
        SettingsSub::Config => render_config(this, cx),
        SettingsSub::Update => render_update(this, cx),
    };

    div()
        .id("page-settings")
        .size_full()
        .flex()
        .flex_col()
        .gap_3()
        .text_color(foreground)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .px_6()
                .pt_6()
                .child(
                    div()
                        .text_size(px(22.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("设置"),
                )
                .child(tabs)
                .children(page_notice(this, cx)),
        )
        .child(body)
        .test_support()
        .into_any_element()
}

fn render_config(this: &Workbench, cx: &App) -> AnyElement {
    let layers = [
        ("CLI 参数", "本次命令传入的参数"),
        ("任务级", "当前任务的配置"),
        ("仓库级", ".kanso/config.toml"),
        ("用户级", "~/.kanso/config.toml"),
        (
            "只读外部导入",
            "~/.codex/config.toml、Claude ~/.claude/settings.json",
        ),
        ("内置默认", "Kanso 自带的默认设置"),
    ];
    let theme = cx.theme();
    let priority =
        section_card("配置优先级（从高到低）", cx).children(layers.into_iter().enumerate().map(
            |(index, (source, detail))| {
                div()
                    .flex()
                    .gap_3()
                    .child(
                        div()
                            .w(px(20.0))
                            .flex_shrink_0()
                            .text_color(theme.muted_foreground)
                            .child((index + 1).to_string()),
                    )
                    .child(div().w(px(110.0)).flex_shrink_0().child(source))
                    .child(
                        div()
                            .min_w_0()
                            .text_color(theme.muted_foreground)
                            .child(detail),
                    )
            },
        ));
    let effective = effective_preview(&this.settings, &this.state.config_layers);
    let table = section_card("生效配置", cx)
        .child(
            div()
                .text_color(theme.muted_foreground)
                .child("kanso config list --show-origin"),
        )
        .child(div().text_color(theme.muted_foreground).child(
            "合并来源：内置默认、~/.kanso/config.toml、工作区 .kanso/config.toml 及桌面设置。",
        ))
        .when_some(this.state.config_error.clone(), |card, err| {
            card.child(
                div()
                    .id("settings-config-error")
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(theme.red)
                    .bg(theme.secondary)
                    .px_3()
                    .py_2()
                    .text_color(theme.red)
                    .child(err)
                    .test_support(),
            )
        })
        .child(
            div()
                .flex()
                .gap_3()
                .text_color(theme.muted_foreground)
                .child(div().flex_1().child("配置项"))
                .child(div().flex_1().child("生效值"))
                .child(div().w(px(96.0)).flex_shrink_0().child("来源")),
        )
        .when(effective.is_empty(), |table| {
            table.child(
                div()
                    .border_t_1()
                    .border_color(theme.border)
                    .pt_2()
                    .text_color(theme.muted_foreground)
                    .child("暂无可显示的生效配置。"),
            )
        })
        .children(effective.into_iter().map(|entry| {
            div()
                .flex()
                .gap_3()
                .border_t_1()
                .border_color(theme.border)
                .pt_2()
                .child(div().min_w_0().flex_1().child(entry.key))
                .child(div().min_w_0().flex_1().child(entry.value))
                .child(
                    div()
                        .w(px(96.0))
                        .flex_shrink_0()
                        .child(entry.layer.label_zh()),
                )
        }));

    settings_body(
        "settings-config",
        [priority.into_any_element(), table.into_any_element()],
    )
}

fn render_update(this: &Workbench, cx: &mut Context<Workbench>) -> AnyElement {
    let mut cards = vec![kanso_update(this, cx).into_any_element()];
    if this.state.update_modal_open {
        cards.insert(0, update_modal(cx));
    }
    cards.push(cli_versions(this, cx).into_any_element());
    cards.push(desktop_app(this, cx).into_any_element());
    settings_body("settings-update", cards)
}

fn settings_body(id: &'static str, cards: impl IntoIterator<Item = AnyElement>) -> AnyElement {
    div()
        .id(id)
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_4()
                .px_6()
                .pb_6()
                .children(cards),
        )
        .test_support()
        .into_any_element()
}

fn kanso_update(this: &Workbench, cx: &mut Context<Workbench>) -> Div {
    let muted = cx.theme().muted_foreground;
    let secondary = cx.theme().secondary;
    let settings = &this.settings;

    section_card("Kanso 自身更新（Sparkle 模型 · 演示）", cx)
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(format!("当前版本 {}", env!("CARGO_PKG_VERSION")))
                .when_some(settings.update_skipped_version.clone(), |row, version| {
                    row.child(
                        div()
                            .id("update-skipped-version")
                            .rounded_full()
                            .px_2()
                            .py_1()
                            .bg(secondary)
                            .child(format!("已跳过 {version}"))
                            .test_support(),
                    )
                }),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(div().text_color(muted).child("更新渠道"))
                .children([("stable", "稳定版"), ("beta", "测试版")].into_iter().map(
                    |(channel, label)| {
                        Button::new(SharedString::from(format!("update-channel-{channel}")))
                            .small()
                            .outline()
                            .selected(settings.update_channel == channel)
                            .label(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                persist_settings(this, |settings| {
                                    settings.update_channel = channel.to_owned();
                                });
                                cx.notify();
                            }))
                    },
                )),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_4()
                .child(
                    Switch::new("update-auto-check")
                        .checked(settings.update_auto_check)
                        .label("自动检查更新")
                        .on_click(cx.listener(|this, checked, _, cx| {
                            persist_settings(this, |settings| {
                                settings.update_auto_check = *checked
                            });
                            cx.notify();
                        })),
                )
                .child(
                    Switch::new("update-auto-install")
                        .checked(settings.update_auto_install)
                        .label("自动安装更新")
                        .on_click(cx.listener(|this, checked, _, cx| {
                            persist_settings(this, |settings| {
                                settings.update_auto_install = *checked
                            });
                            cx.notify();
                        })),
                ),
        )
        .child(
            div().child(
                Button::new("check-for-updates")
                    .small()
                    .primary()
                    .label("立即检查更新")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.state.ui_notice = None;
                        this.state.update_modal_open = true;
                        cx.notify();
                    })),
            ),
        )
}

fn update_modal(cx: &mut Context<Workbench>) -> AnyElement {
    let accent = cx.theme().primary;
    section_card(format!("发现演示版本 {DEMO_VERSION}"), cx)
        .id("update-modal")
        .border_color(accent)
        .children(
            [
                "新增六种任务路由策略。",
                "按角色展示候选顺序与筛选原因。",
                "按来源展示各层配置的生效值。",
            ]
            .map(|note| div().child(format!("• {note}"))),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(
                    Button::new("update-skip")
                        .small()
                        .outline()
                        .label("跳过此版本")
                        .on_click(cx.listener(|this, _, _, cx| {
                            let saved = persist_settings(this, |settings| {
                                settings.update_skipped_version = Some(DEMO_VERSION.into());
                            });
                            if saved {
                                this.state.update_modal_open = false;
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("update-later")
                        .small()
                        .outline()
                        .label("稍后提醒")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.state.update_modal_open = false;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("update-install")
                        .small()
                        .primary()
                        .label("安装并重启")
                        .on_click(cx.listener(|this, _, _, cx| {
                            let notice = format!("已模拟安装版本 {DEMO_VERSION} 并重启。");
                            this.state.push_log(LogLevel::Info, notice.clone());
                            this.state.ui_notice = Some(notice);
                            this.state.update_modal_open = false;
                            cx.notify();
                        })),
                ),
        )
        .test_support()
        .into_any_element()
}

fn cli_versions(this: &Workbench, cx: &mut Context<Workbench>) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    let border = cx.theme().border;

    section_card("Agent CLI 本地版本与升级（实测）", cx)
        .when(this.state.tool_versions.is_empty(), |card| {
            card.child(
                div()
                    .text_color(muted)
                    .child("暂无本地命令行工具探测结果。"),
            )
        })
        .children(
            this.state
                .tool_versions
                .iter()
                .enumerate()
                .map(|(index, tool)| {
                    let version = match (tool.installed, tool.version.as_deref()) {
                        (true, Some(version)) => version,
                        (true, None) => "未知",
                        (false, _) => "未安装",
                    };
                    let method = match tool.path_kind {
                        Some(PathKind::Npm) => "npm",
                        Some(PathKind::Brew) => "Homebrew",
                        Some(PathKind::Binary) => "独立可执行文件",
                        None => "未知",
                    };

                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .pt_2()
                        .border_t_1()
                        .border_color(border)
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_x_4()
                                .gap_y_1()
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(tool.tool.clone().test_text(tool.tool.clone())),
                                )
                                .child(format!("模型家族 {}", family_name(&tool.family)))
                                .child(format!("已安装版本 {version}"))
                                .child(format!("安装方式 {method}")),
                        )
                        .child(div().text_color(muted).child("需联网检查最新版本（离线）"))
                        .when_some(tool.upgrade_cmd.clone(), |row, command| {
                            row.child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .items_center()
                                    .child(div().text_color(muted).child(command.clone()))
                                    .child(
                                        Button::new(("copy-upgrade-command", index))
                                            .small()
                                            .outline()
                                            .label("复制升级命令")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    command.clone(),
                                                ));
                                                this.state.ui_notice =
                                                    Some("已复制升级命令。".into());
                                                cx.notify();
                                            })),
                                    ),
                            )
                        })
                }),
        )
        .id("settings-cli-versions")
        .test_support()
}

fn desktop_app(this: &Workbench, cx: &App) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    let card = section_card("桌面客户端（实测）", cx);
    let card = match &this.state.desktop_app {
        Some(app) => {
            let version = match (app.installed, app.version.as_deref()) {
                (true, Some(version)) => version,
                (true, None) => "未知",
                (false, _) => "未安装",
            };
            let updater = match app.updater {
                UpdaterKind::Sparkle => "Sparkle",
                UpdaterKind::Unknown => "未知",
            };
            let updater_label = format!("更新引擎 {updater}");
            card.child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(app.name.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_x_4()
                    .gap_y_1()
                    .child(format!("已安装版本 {version}"))
                    .child(updater_label.clone().test_text(updater_label))
                    .child(format!("自动检查 {}", switch_status(app.auto_check)))
                    .child(format!("自动安装 {}", switch_status(app.auto_install))),
            )
            .child(format!(
                "最后检查时间 {}",
                app.last_check.as_deref().unwrap_or("未知")
            ))
        }
        None => card.child(div().text_color(muted).child("暂无桌面客户端探测结果。")),
    };
    card.id("settings-desktop-app").test_support()
}

fn switch_status(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "已开启",
        Some(false) => "已关闭",
        None => "未知",
    }
}

/// 构造与当前主题一致的页面内容卡片。
pub(super) fn section_card(title: impl Into<SharedString>, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .flex()
        .flex_col()
        .gap_3()
        .rounded(px(8.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .p_4()
        .text_size(px(13.0))
        .child(
            div()
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.into()),
        )
}

/// 显示当前操作的页内提示。
pub(super) fn page_notice(this: &Workbench, cx: &App) -> Option<Div> {
    this.state.ui_notice.as_ref().map(|notice| {
        div()
            .rounded(px(6.0))
            .bg(cx.theme().secondary)
            .px_3()
            .py_2()
            .child(notice.clone())
    })
}

/// 保存设置改动，写盘成功后提交页面状态，失败时显示提示并记录原因。
pub(super) fn persist_settings(this: &mut Workbench, edit: impl FnOnce(&mut Settings)) -> bool {
    let mut settings = this.settings.clone();
    edit(&mut settings);
    this.state.ui_notice = None;

    match settings.save() {
        Ok(()) => {
            this.settings = settings;
            true
        }
        Err(err) => {
            this.state
                .push_log(LogLevel::Warn, format!("保存设置失败：{err}"));
            this.state.ui_notice = Some("保存设置失败，请查看日志。".into());
            false
        }
    }
}
