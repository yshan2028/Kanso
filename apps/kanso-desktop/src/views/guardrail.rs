use crate::app::{KansoApp, Message};
use crate::theme;
use iced::widget::{button, column, container, row, text, text_input};
use iced::{Alignment, Element, Length};
use kanso_core::GuardrailKind;

pub fn view(app: &KansoApp) -> Element<'_, Message> {
    let Some(snap) = &app.snapshot else {
        return container(text("").size(1)).height(0).into();
    };
    if snap.alerts.is_empty() && app.rescue_target.is_none() {
        return container(text("").size(1)).height(0).into();
    }

    let mut content = column![].spacing(8);

    if !snap.alerts.is_empty() {
        let mut alert_row = row![text(format!("AI 护栏告警 ({})：", snap.alerts.len()))
            .size(12)
            .color(theme::ACCENT_AMBER)]
        .spacing(8)
        .align_y(Alignment::Center);

        for alert in snap.alerts.iter().take(3) {
            let desc = match alert.kind {
                GuardrailKind::DirtyTrunk => format!(
                    "主干({})有 {} 处未提交",
                    alert.trunk_branch.as_deref().unwrap_or("trunk"),
                    alert.dirty_files
                ),
                GuardrailKind::MergeConflict => format!("{} 个冲突文件", alert.dirty_files),
                GuardrailKind::DetachedHead => "处于 Detached HEAD".to_string(),
            };

            let mut chip = row![
                text(alert.repo_name.as_str())
                    .size(12)
                    .color(theme::TEXT_PRIMARY),
                text(desc).size(11).color(theme::ACCENT_AMBER),
            ]
            .spacing(6)
            .align_y(Alignment::Center);

            if alert.kind == GuardrailKind::DirtyTrunk {
                chip = chip.push(
                    button(text("一键救场").size(11))
                        .on_press(Message::OpenRescue(alert.clone()))
                        .style(theme::warning_btn_style)
                        .padding([3, 8]),
                );
            }

            alert_row = alert_row.push(
                container(chip)
                    .padding([4, 8])
                    .style(theme::badge_style(theme::ACCENT_AMBER)),
            );
        }

        if snap.alerts.len() > 3 {
            alert_row = alert_row.push(
                button(text(format!("查看全部 {} 项告警", snap.alerts.len())).size(11))
                    .on_press(Message::FocusAlerts)
                    .style(theme::secondary_btn_style)
                    .padding([4, 10]),
            );
        }

        content = content.push(alert_row);
    }

    if let Some(target) = &app.rescue_target {
        let rescue_bar = container(
            row![
                text(format!("将 {} 的主干未提交改动无损迁移到新分支：", target.repo_name))
                    .size(12)
                    .color(theme::TEXT_PRIMARY),
                text_input("输入新分支名（如 feat/rescue）", &app.rescue_branch_input)
                    .on_input(Message::RescueBranchChanged)
                    .on_submit(Message::ConfirmRescue)
                    .style(theme::input_style)
                    .size(12)
                    .width(240)
                    .padding([6, 10]),
                button(text("立即迁移").size(12))
                    .on_press(Message::ConfirmRescue)
                    .style(theme::warning_btn_style)
                    .padding([6, 12]),
                button(text("取消").size(12))
                    .on_press(Message::CancelRescue)
                    .style(theme::secondary_btn_style)
                    .padding([6, 10]),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([8, 12])
        .style(theme::card_style);

        content = content.push(rescue_bar);
    }

    container(content)
        .width(Length::Fill)
        .padding([10, 20])
        .style(theme::alert_banner_style)
        .into()
}
