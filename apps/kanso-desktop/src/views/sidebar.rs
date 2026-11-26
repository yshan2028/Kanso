use crate::app::{KansoApp, Message, NavTab};
use crate::theme;
use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Element, Length};

pub fn view(app: &KansoApp) -> Element<'_, Message> {
    let repo_count = app.snapshot.as_ref().map_or(0, |s| s.repos.len());
    let alert_count = app.snapshot.as_ref().map_or(0, |s| s.alerts.len());
    let task_count = app.snapshot.as_ref().map_or(0, |s| s.tasks.len());

    let brand = container(
        row![
            container(text("簡").size(15).color(theme::ACCENT_SKY))
                .padding([6, 10])
                .style(theme::badge_style(theme::ACCENT_SKY)),
            column![
                row![
                    text("Kanso").size(15).color(theme::TEXT_PRIMARY),
                    container(text("簡素").size(11).color(theme::TEXT_SECONDARY))
                        .padding([2, 6])
                        .style(theme::elevated_style),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
                text("AI 多仓工程控制塔").size(11).color(theme::TEXT_MUTED),
            ]
            .spacing(2),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .padding([16, 16]);

    let nav_section = column![
        text("控制视图").size(11).color(theme::TEXT_MUTED),
        nav_button(
            "全景雷达",
            if alert_count > 0 {
                format!("{alert_count} 告警 · {repo_count}")
            } else {
                format!("{repo_count}")
            },
            app.active_tab == NavTab::Radar,
            NavTab::Radar,
        ),
        nav_button(
            "任务进度",
            format!("{task_count}"),
            app.active_tab == NavTab::Tasks,
            NavTab::Tasks,
        ),
        nav_button(
            "本机 CI 流水线",
            if app.ci_running {
                "运行中".to_string()
            } else {
                String::new()
            },
            app.active_tab == NavTab::Ci,
            NavTab::Ci,
        ),
    ]
    .spacing(6)
    .padding(12);

    let ws_header = row![
        text("多仓工作区").size(11).color(theme::TEXT_MUTED),
        container(text("").size(1)).width(Length::Fill),
        button(text("路径").size(11))
            .on_press(Message::ToggleManualWsForm(!app.show_manual_ws_form))
            .style(theme::secondary_btn_style)
            .padding([4, 8]),
        button(text("+ 添加").size(11))
            .on_press(Message::PickWorkspaceFolder)
            .style(theme::primary_btn_style)
            .padding([4, 10]),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let mut ws_items = column![].spacing(6);
    if app.show_manual_ws_form {
        let form = container(
            column![
                text("手动输入工作区路径").size(12).color(theme::TEXT_PRIMARY),
                text_input("/Users/you/Code/project", &app.manual_ws_path)
                    .on_input(Message::ManualWsPathChanged)
                    .style(theme::input_style)
                    .size(12)
                    .padding([6, 8]),
                text_input("显示名称（可选）", &app.manual_ws_name)
                    .on_input(Message::ManualWsNameChanged)
                    .style(theme::input_style)
                    .size(12)
                    .padding([6, 8]),
                text_input("默认主干（如 dev-Java）", &app.manual_ws_trunk)
                    .on_input(Message::ManualWsTrunkChanged)
                    .style(theme::input_style)
                    .size(12)
                    .padding([6, 8]),
                row![
                    button(text("取消").size(11))
                        .on_press(Message::ToggleManualWsForm(false))
                        .style(theme::secondary_btn_style)
                        .padding([5, 10]),
                    button(text("添加并扫描").size(11))
                        .on_press(Message::SubmitManualWs)
                        .style(theme::primary_btn_style)
                        .padding([5, 10]),
                ]
                .spacing(6),
            ]
            .spacing(8),
        )
        .padding(10)
        .style(theme::card_style);
        ws_items = ws_items.push(form);
    }

    if app.workspaces.is_empty() {
        ws_items = ws_items.push(
            container(
                text("点击「+ 添加」选择包含多个 Git 仓库的目录")
                    .size(12)
                    .color(theme::TEXT_MUTED),
            )
            .width(Length::Fill)
            .padding(12)
            .style(theme::card_style),
        );
    } else {
        for ws in &app.workspaces {
            let active = app.active_workspace_id.as_deref() == Some(ws.id.as_str());
            let trunk_label = ws
                .default_trunk
                .as_deref()
                .map(|t| format!("主干: {t}"))
                .unwrap_or_else(|| "自动识别主干".to_string());

            let item_row = row![
                button(
                    column![
                        text(ws.name.as_str()).size(12).color(theme::TEXT_PRIMARY),
                        text(trunk_label).size(11).color(theme::TEXT_MUTED),
                    ]
                    .spacing(2)
                    .width(Length::Fill),
                )
                .on_press(Message::SelectWorkspace(ws.id.clone()))
                .style(theme::nav_btn_style(active))
                .width(Length::Fill)
                .padding([7, 10]),
                button(text("×").size(12))
                    .on_press(Message::RemoveWorkspace(ws.id.clone()))
                    .style(theme::secondary_btn_style)
                    .padding([6, 8]),
            ]
            .spacing(4)
            .align_y(Alignment::Center);

            ws_items = ws_items.push(item_row);
        }
    }

    let ws_section = column![ws_header, scrollable(ws_items).height(Length::Fill)]
        .spacing(10)
        .padding(12)
        .height(Length::Fill);

    let mcp_label = if app.copied_mcp {
        "已复制 MCP 配置"
    } else {
        "复制 AI MCP 配置"
    };

    let footer = column![
        container(
            column![
                row![
                    text("渲染引擎").size(11).color(theme::TEXT_MUTED),
                    container(text("").size(1)).width(Length::Fill),
                    text("纯 Rust · Iced GPU").size(11).color(theme::ACCENT_GREEN),
                ],
                row![
                    text("压缩底座").size(11).color(theme::TEXT_MUTED),
                    container(text("").size(1)).width(Length::Fill),
                    text("Zstd + FastCDC").size(11).color(theme::TEXT_SECONDARY),
                ],
            ]
            .spacing(4),
        )
        .padding(10)
        .style(theme::card_style),
        button(
            row![
                text(mcp_label).size(12),
                container(text("").size(1)).width(Length::Fill),
                text("JSON").size(11).color(theme::TEXT_MUTED),
            ]
            .align_y(Alignment::Center),
        )
        .on_press(Message::CopyMcpConfig)
        .style(theme::secondary_btn_style)
        .width(Length::Fill)
        .padding([8, 12]),
    ]
    .spacing(8)
    .padding(12);

    container(
        column![
            brand,
            theme::divider(),
            nav_section,
            theme::divider(),
            ws_section,
            theme::divider(),
            footer,
        ]
        .height(Length::Fill),
    )
    .width(260)
    .height(Length::Fill)
    .style(theme::sidebar_style)
    .into()
}

fn nav_button<'a>(
    label: &'a str,
    badge: String,
    active: bool,
    tab: NavTab,
) -> Element<'a, Message> {
    button(
        row![
            text(label).size(12),
            container(text("").size(1)).width(Length::Fill),
            text(badge).size(11).color(theme::TEXT_MUTED),
        ]
        .align_y(Alignment::Center),
    )
    .on_press(Message::SelectTab(tab))
    .style(theme::nav_btn_style(active))
    .width(Length::Fill)
    .padding([8, 10])
    .into()
}
