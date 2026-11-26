use crate::app::{KansoApp, Message};
use crate::theme;
use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Color, Element, Font, Length};
use kanso_core::{RepoStatus, TaskStage};

pub fn view(app: &KansoApp) -> Element<'_, Message> {
    let Some(snap) = &app.snapshot else {
        return container(text("请先在左侧添加多仓工作区").size(13).color(theme::TEXT_MUTED))
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
    };

    if snap.tasks.is_empty() {
        return container(
            column![
                text("当前所有仓库均位于主干分支").size(15).color(theme::TEXT_PRIMARY),
                text("当 AI 或开发者切出特性分支时，Kanso 将自动按分支聚合跨仓任务卡片，无需任何手工填表。")
                    .size(12)
                    .color(theme::TEXT_MUTED),
            ]
            .spacing(8)
            .align_x(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into();
    }

    let mut cards = column![].spacing(12);
    for task in &snap.tasks {
        let member_repos: Vec<&RepoStatus> = snap
            .repos
            .iter()
            .filter(|r| task.repo_ids.contains(&r.id))
            .collect();

        let has_ci_failed = member_repos
            .iter()
            .any(|r| app.repo_latest_ci.get(&r.path).map(String::as_str) == Some("failed"));
        let effective_stage = if has_ci_failed {
            TaskStage::CiFailed
        } else {
            task.stage
        };

        let (stage_label, stage_color) = stage_meta(effective_stage);
        let pct = task.progress_percent.clamp(1, 100) as u16;

        let mut bar_row = row![].height(6);
        bar_row = bar_row.push(
            container(text("").size(1))
                .width(Length::FillPortion(pct))
                .height(6)
                .style(move |_| container::Style {
                    background: Some(iced::Background::Color(stage_color)),
                    border: iced::border::Border {
                        radius: 4.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        );
        if pct < 100 {
            bar_row = bar_row.push(
                container(text("").size(1))
                    .width(Length::FillPortion(100 - pct))
                    .height(6)
                    .style(|_| container::Style {
                        background: Some(iced::Background::Color(theme::BG_MAIN)),
                        ..Default::default()
                    }),
            );
        }

        let mut member_list = column![].spacing(6);
        for repo in member_repos {
            let dirty_text = if repo.is_dirty() {
                format!("{} 处改动", repo.total_dirty())
            } else {
                "干净".to_string()
            };
            let dirty_color = if repo.is_dirty() {
                theme::ACCENT_AMBER
            } else {
                theme::ACCENT_GREEN
            };

            let mut member_row = row![
                text(repo.name.as_str()).size(12).color(theme::TEXT_PRIMARY),
                text(format!("+{}/-{}", repo.trunk_ahead, repo.trunk_behind))
                    .size(11)
                    .font(Font::MONOSPACE)
                    .color(theme::TEXT_MUTED),
                container(text("").size(1)).width(Length::Fill),
            ]
            .spacing(10)
            .align_y(Alignment::Center);

            if let Some(ci_st) = app.repo_latest_ci.get(&repo.path) {
                let (ci_label, ci_color) = match ci_st.as_str() {
                    "passed" => ("CI 通过", theme::ACCENT_GREEN),
                    "failed" => ("CI 失败", theme::ACCENT_ROSE),
                    "running" => ("CI 运行中", theme::ACCENT_SKY),
                    "cancelled" => ("CI 已中止", theme::TEXT_MUTED),
                    _ => ("CI 未知", theme::TEXT_MUTED),
                };
                member_row = member_row.push(
                    container(text(ci_label).size(11).color(ci_color))
                        .padding([2, 6])
                        .style(theme::badge_style(ci_color)),
                );
            }

            if repo.is_dirty() {
                member_row = member_row.push(
                    button(text("提交").size(11))
                        .on_press(Message::OpenRepoCommit(repo.path.clone()))
                        .style(theme::primary_btn_style)
                        .padding([4, 9]),
                );
            }

            member_row = member_row
                .push(text(dirty_text).size(11).color(dirty_color))
                .push(
                    button(text("跑 CI").size(11))
                        .on_press(Message::OpenRepoCi(repo.path.clone()))
                        .style(theme::secondary_btn_style)
                        .padding([4, 10]),
                );

            member_list = member_list.push(
                container(member_row)
                    .padding([7, 10])
                    .style(theme::elevated_style),
            );
        }

        let mut header_row = row![
            text(task.branch.as_str())
                .size(14)
                .font(Font::MONOSPACE)
                .color(theme::TEXT_PRIMARY),
            container(text("").size(1)).width(Length::Fill),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        if task.dirty_repos > 0 {
            header_row = header_row.push(
                button(text(format!("跨仓批量提交 ({})", task.dirty_repos)).size(11))
                    .on_press(Message::OpenTaskBatchCommit(task.branch.clone()))
                    .style(theme::primary_btn_style)
                    .padding([4, 10]),
            );
        }

        header_row = header_row.push(
            container(text(stage_label).size(11).color(stage_color))
                .padding([3, 9])
                .style(theme::badge_style(stage_color)),
        );

        let mut card_col = column![
            header_row,
            row![
                text(format!(
                    "涉及 {} 个仓库 · 脏仓 {} · 未推 {} · 落后主干 {}",
                    task.total_repos,
                    task.dirty_repos,
                    task.unpushed_repos,
                    task.behind_trunk_repos
                ))
                .size(11)
                .color(theme::TEXT_MUTED),
                container(text("").size(1)).width(Length::Fill),
                text(format!("{}%", task.progress_percent))
                    .size(11)
                    .color(theme::TEXT_SECONDARY),
            ],
            bar_row,
        ]
        .spacing(10);

        if app.task_batch_commit_branch.as_deref() == Some(task.branch.as_str()) {
            let batch_box = container(
                row![
                    text_input("统一提交说明（如 feat(order): 联调完成）", &app.task_batch_commit_msg)
                        .on_input(Message::TaskBatchMsgChanged)
                        .on_submit(Message::SubmitTaskBatchCommit)
                        .style(theme::input_style)
                        .size(12)
                        .padding([6, 10]),
                    text_input("推送远端（可选，如 origin/forgejo）", &app.task_batch_push_remote)
                        .on_input(Message::TaskBatchRemoteChanged)
                        .on_submit(Message::SubmitTaskBatchCommit)
                        .style(theme::input_style)
                        .size(12)
                        .width(200)
                        .padding([6, 10]),
                    button(text("提交并执行").size(11))
                        .on_press(Message::SubmitTaskBatchCommit)
                        .style(theme::primary_btn_style)
                        .padding([6, 12]),
                    button(text("取消").size(11))
                        .on_press(Message::CancelTaskBatchCommit)
                        .style(theme::secondary_btn_style)
                        .padding([6, 10]),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .padding(10)
            .style(theme::elevated_style);
            card_col = card_col.push(batch_box);
        }

        card_col = card_col.push(member_list);

        let card = container(card_col)
            .width(Length::Fill)
            .padding(16)
            .style(theme::card_style);

        cards = cards.push(card);
    }

    column![
        column![
            text(format!("自动聚合任务视图 ({})", snap.tasks.len()))
                .size(17)
                .color(theme::TEXT_PRIMARY),
            text("根据当前分支、工作区脏状态与主干偏离度自动推导，无需手工维护状态")
                .size(12)
                .color(theme::TEXT_MUTED),
        ]
        .spacing(4),
        scrollable(cards).height(Length::Fill),
    ]
    .spacing(14)
    .padding(20)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn stage_meta(stage: TaskStage) -> (&'static str, Color) {
    match stage {
        TaskStage::Dirty => ("开发改动中", theme::ACCENT_AMBER),
        TaskStage::InProgress => ("分支进行中", theme::ACCENT_SKY),
        TaskStage::ReadyForCi => ("待验 / 待推送", theme::ACCENT_VIOLET),
        TaskStage::CiFailed => ("CI 失败", theme::ACCENT_ROSE),
        TaskStage::ReadyToLand => ("可合入主干", theme::ACCENT_GREEN),
    }
}
