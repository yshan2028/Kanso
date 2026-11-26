use crate::app::{KansoApp, Message};
use crate::theme;
use iced::widget::{button, column, container, pick_list, row, scrollable, text, text_input};
use iced::{Alignment, Element, Length};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
struct RepoOption {
    path: String,
    label: String,
}

impl fmt::Display for RepoOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label)
    }
}

pub fn view(app: &KansoApp) -> Element<'_, Message> {
    let Some(snap) = &app.snapshot else {
        return container(text("请先添加工作区并选择仓库运行本机 CI").size(13).color(theme::TEXT_MUTED))
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
    };

    let options: Vec<RepoOption> = snap
        .repos
        .iter()
        .map(|r| RepoOption {
            path: r.path.clone(),
            label: format!("{} ({})", r.name, r.current_branch),
        })
        .collect();

    let selected_option = app
        .selected_ci_repo
        .as_deref()
        .and_then(|p| options.iter().find(|o| o.path == p).cloned());

    let selected_name = selected_option
        .as_ref()
        .map(|o| o.label.clone())
        .unwrap_or_else(|| "未选择仓库".to_string());

    let mut pipelines_col = column![].spacing(8);
    if app.ci_pipelines.is_empty() {
        pipelines_col = pipelines_col.push(
            container(
                text("未检测到 .kanso.yaml 或常见语言工程文件，可在下方输入自定义 Shell 命令运行。")
                    .size(11)
                    .color(theme::TEXT_MUTED),
            )
            .padding(10)
            .style(theme::card_style),
        );
    } else {
        for p in &app.ci_pipelines {
            let mut steps_col = column![].spacing(2);
            for s in &p.steps {
                steps_col = steps_col.push(
                    text(format!("$ {}", s.command))
                        .size(11)
                        .color(theme::TEXT_SECONDARY),
                );
            }

            let card = container(
                column![
                    row![
                        column![
                            text(p.name.as_str()).size(12).color(theme::TEXT_PRIMARY),
                            text(p.source.as_str()).size(11).color(theme::TEXT_MUTED),
                        ]
                        .spacing(2),
                        container(text("").size(1)).width(Length::Fill),
                        button(text("运行").size(11))
                            .on_press(Message::StartCiPipeline(p.clone()))
                            .style(theme::primary_btn_style)
                            .padding([4, 10]),
                    ]
                    .align_y(Alignment::Center),
                    steps_col,
                ]
                .spacing(6),
            )
            .padding(10)
            .style(theme::card_style);
            pipelines_col = pipelines_col.push(card);
        }
    }

    let mut history_col = column![].spacing(6);
    if app.ci_runs.is_empty() {
        history_col = history_col.push(text("暂无历史执行记录").size(11).color(theme::TEXT_MUTED));
    } else {
        for run in &app.ci_runs {
            let status_color = match run.status.as_str() {
                "passed" => theme::ACCENT_GREEN,
                "failed" => theme::ACCENT_ROSE,
                _ => theme::ACCENT_SKY,
            };
            let dur_label = run
                .duration_ms
                .map(|d| format!("{d} ms"))
                .unwrap_or_else(|| "运行中".to_string());
            let active = app.active_ci_run_id.as_deref() == Some(run.id.as_str());

            history_col = history_col.push(
                button(
                    row![
                        column![
                            text(run.pipeline_name.as_str())
                                .size(12)
                                .color(theme::TEXT_PRIMARY),
                            text(format!("{dur_label} · {} B (Zstd)", run.compressed_log_bytes))
                                .size(11)
                                .color(theme::TEXT_MUTED),
                        ]
                        .spacing(2),
                        container(text("").size(1)).width(Length::Fill),
                        container(text(run.status.as_str()).size(11).color(status_color))
                            .padding([2, 7])
                            .style(theme::badge_style(status_color)),
                    ]
                    .align_y(Alignment::Center),
                )
                .on_press(Message::LoadCiHistoryLog(run.id.clone()))
                .style(theme::nav_btn_style(active))
                .width(Length::Fill)
                .padding([7, 10]),
            );
        }
    }

    let left_panel = container(
        column![
            text("目标仓库").size(11).color(theme::TEXT_MUTED),
            pick_list(options, selected_option, |opt| {
                Message::SelectCiRepo(opt.path)
            })
            .width(Length::Fill),
            text("探测到的流水线").size(11).color(theme::TEXT_MUTED),
            pipelines_col,
            text("自定义命令").size(11).color(theme::TEXT_MUTED),
            row![
                text_input("如 mvn test / cargo check", &app.ci_custom_cmd)
                    .on_input(Message::CiCustomCmdChanged)
                    .style(theme::input_style)
                    .size(12)
                    .padding([6, 8]),
                button(text("执行").size(12))
                    .on_press(Message::StartCiCustom)
                    .style(theme::secondary_btn_style)
                    .padding([6, 12]),
            ]
            .spacing(6),
            text("Zstd 压缩历史记录（点击回放）")
                .size(11)
                .color(theme::TEXT_MUTED),
            scrollable(history_col).height(Length::Fill),
        ]
        .spacing(10),
    )
    .width(340)
    .height(Length::Fill)
    .padding(16)
    .style(theme::sidebar_style);

    let mut log_header = row![
        text(format!("本机 CI 控制台 · {selected_name}"))
            .size(13)
            .color(theme::TEXT_PRIMARY),
        container(text("").size(1)).width(Length::Fill),
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    if let Some(status) = &app.displayed_run_status {
        let color = match status.as_str() {
            "passed" => theme::ACCENT_GREEN,
            "failed" => theme::ACCENT_ROSE,
            _ => theme::ACCENT_SKY,
        };
        log_header = log_header.push(
            container(text(status.as_str()).size(11).color(color))
                .padding([3, 8])
                .style(theme::badge_style(color)),
        );
    }

    if app.ci_running {
        log_header = log_header.push(
            button(text("中止运行").size(11))
                .on_press(Message::CancelCi)
                .style(theme::danger_btn_style)
                .padding([4, 10]),
        );
    }

    let mut log_lines = column![].spacing(3);
    if app.displayed_logs.is_empty() {
        log_lines = log_lines.push(
            text("在左侧选择流水线点击「运行」，或点击历史记录解压回放 Zstd 压缩日志...")
                .size(12)
                .color(theme::TEXT_MUTED),
        );
    } else {
        for line in &app.displayed_logs {
            let color = if line.starts_with("==>") {
                theme::ACCENT_SKY
            } else if line.starts_with("!!") {
                theme::ACCENT_ROSE
            } else {
                theme::TEXT_PRIMARY
            };
            log_lines = log_lines.push(text(line.as_str()).size(12).color(color));
        }
    }

    let right_panel = column![
        container(log_header).padding([12, 18]),
        theme::divider(),
        scrollable(container(log_lines).padding(16)).height(Length::Fill),
    ]
    .width(Length::Fill)
    .height(Length::Fill);

    row![left_panel, theme::divider_v(), right_panel]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
