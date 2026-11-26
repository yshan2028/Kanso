use crate::app::{FilterMode, KansoApp, Message};
use crate::theme;
use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Color, Element, Font, Length};
use kanso_core::{GuardrailAlert, GuardrailKind, RepoStatus};
use std::collections::HashMap;

pub fn view(app: &KansoApp) -> Element<'_, Message> {
    let Some(snap) = &app.snapshot else {
        return container(
            column![
                text("尚未选择多仓工作区").size(16).color(theme::TEXT_PRIMARY),
                text("请在左侧点击「+ 添加」选择包含多个 Git 仓库的目录，Kanso 将毫秒级扫描全部分支与任务状态。")
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
    };

    let q = app.search_query.trim().to_lowercase();
    let filtered: Vec<&RepoStatus> = snap
        .repos
        .iter()
        .filter(|r| {
            if !q.is_empty()
                && !r.name.to_lowercase().contains(&q)
                && !r.current_branch.to_lowercase().contains(&q)
                && !r.relative_path.to_lowercase().contains(&q)
            {
                return false;
            }
            match app.filter_mode {
                FilterMode::All => true,
                FilterMode::Dirty => r.is_dirty(),
                FilterMode::OffTrunk => !r.is_on_trunk,
                FilterMode::Alerts => {
                    r.has_dirty_on_trunk || r.is_detached || r.conflicted_count > 0
                }
            }
        })
        .collect();

    let dirty_count = snap.repos.iter().filter(|r| r.is_dirty()).count();
    let off_trunk_count = snap.repos.iter().filter(|r| !r.is_on_trunk).count();
    let unpushed_count = snap
        .repos
        .iter()
        .filter(|r| !r.is_on_trunk && (r.upstream.is_none() || r.upstream_ahead > 0))
        .count();

    let has_configured_trunk = app
        .current_workspace()
        .and_then(|w| w.default_trunk.as_ref())
        .is_some();

    let suggested_trunk = if !has_configured_trunk && !app.editing_trunk {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for r in &snap.repos {
            if !r.is_detached {
                *counts.entry(r.current_branch.as_str()).or_default() += 1;
            }
        }
        counts
            .into_iter()
            .max_by_key(|(_, c)| *c)
            .filter(|(branch, count)| {
                *count >= 2
                    && snap
                        .repos
                        .iter()
                        .any(|r| r.trunk_branch.as_deref() != Some(*branch))
            })
            .map(|(branch, _)| branch.to_string())
    } else {
        None
    };

    let trunk_control: Element<'_, Message> = if app.editing_trunk {
        row![
            text_input("默认主干（如 dev-Java）", &app.trunk_input)
                .on_input(Message::TrunkInputChanged)
                .on_submit(Message::SaveTrunk)
                .style(theme::input_style)
                .size(12)
                .width(150)
                .padding([4, 8]),
            button(text("保存").size(11))
                .on_press(Message::SaveTrunk)
                .style(theme::primary_btn_style)
                .padding([4, 10]),
            button(text("取消").size(11))
                .on_press(Message::CancelEditTrunk)
                .style(theme::secondary_btn_style)
                .padding([4, 8]),
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .into()
    } else {
        let mut trunk_row = row![
            button(text("设置默认主干").size(11))
                .on_press(Message::StartEditTrunk)
                .style(theme::secondary_btn_style)
                .padding([4, 10]),
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        if let Some(suggested) = suggested_trunk {
            trunk_row = trunk_row.push(
                button(text(format!("推荐设为主干: {suggested}")).size(11))
                    .on_press(Message::ApplySuggestedTrunk(suggested))
                    .style(theme::warning_btn_style)
                    .padding([4, 10]),
            );
        }
        trunk_row.into()
    };

    let header = row![
        column![
            row![
                text(snap.workspace_name.as_str())
                    .size(17)
                    .color(theme::TEXT_PRIMARY),
                container(
                    text(format!("并发扫描 {} ms", snap.scan_duration_ms))
                        .size(11)
                        .color(theme::ACCENT_GREEN),
                )
                .padding([2, 8])
                .style(theme::badge_style(theme::ACCENT_GREEN)),
                trunk_control,
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            text(snap.root_path.as_str())
                .size(12)
                .color(theme::TEXT_MUTED),
        ]
        .spacing(4),
        container(text("").size(1)).width(Length::Fill),
        row![
            button(text(format!("批量开分支 ({})", app.selected_repo_paths.len())).size(12))
                .on_press(Message::ToggleBatchBranchForm(!app.show_batch_branch_form))
                .style(theme::secondary_btn_style)
                .padding([7, 12]),
            button(text("全仓拉主干").size(12))
                .on_press(Message::BatchPullTrunk)
                .style(theme::secondary_btn_style)
                .padding([7, 12]),
            button(text(if app.scanning { "扫描中..." } else { "刷新" }).size(12))
                .on_press(Message::TriggerScan)
                .style(theme::primary_btn_style)
                .padding([7, 14]),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .align_y(Alignment::Center);

    let stats = row![
        stat_card("发现仓库", format!("{}", snap.repos.len()), theme::TEXT_PRIMARY),
        stat_card("活跃特性分支", format!("{off_trunk_count}"), theme::ACCENT_SKY),
        stat_card("有未提交改动", format!("{dirty_count}"), theme::ACCENT_AMBER),
        stat_card("待推送仓库", format!("{unpushed_count}"), theme::ACCENT_GREEN),
    ]
    .spacing(12);

    let filters = row![
        text_input("搜索仓库名、路径或分支...", &app.search_query)
            .on_input(Message::SearchChanged)
            .style(theme::input_style)
            .size(12)
            .width(280)
            .padding([7, 10]),
        container(text("").size(1)).width(Length::Fill),
        filter_chip("全部", FilterMode::All, app.filter_mode == FilterMode::All),
        filter_chip("有改动", FilterMode::Dirty, app.filter_mode == FilterMode::Dirty),
        filter_chip("非主干", FilterMode::OffTrunk, app.filter_mode == FilterMode::OffTrunk),
        filter_chip("仅告警", FilterMode::Alerts, app.filter_mode == FilterMode::Alerts),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let mut top_section = column![header, stats, filters].spacing(14);

    if app.show_batch_branch_form {
        let batch_form = container(
            row![
                text(format!("已选 {} 个仓库，批量创建任务分支：", app.selected_repo_paths.len()))
                    .size(12)
                    .color(theme::TEXT_PRIMARY),
                text_input("分支名（如 feat/ai-sync）", &app.batch_branch_name)
                    .on_input(Message::BatchBranchNameChanged)
                    .on_submit(Message::SubmitBatchBranch)
                    .style(theme::input_style)
                    .size(12)
                    .width(210)
                    .padding([6, 10]),
                text_input("基线分支（可选）", &app.batch_base_branch)
                    .on_input(Message::BatchBaseBranchChanged)
                    .on_submit(Message::SubmitBatchBranch)
                    .style(theme::input_style)
                    .size(12)
                    .width(150)
                    .padding([6, 10]),
                button(text("创建并切换").size(12))
                    .on_press(Message::SubmitBatchBranch)
                    .style(theme::primary_btn_style)
                    .padding([6, 12]),
                button(text("取消").size(12))
                    .on_press(Message::ToggleBatchBranchForm(false))
                    .style(theme::secondary_btn_style)
                    .padding([6, 10]),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding(10)
        .style(theme::card_style);
        top_section = top_section.push(batch_form);
    }

    if let Some(results) = &app.batch_results {
        let ok_count = results.iter().filter(|r| r.success).count();
        let skip_count = results.len() - ok_count;
        let mut res_col = column![].spacing(4);
        for item in results {
            let color = if item.success {
                theme::ACCENT_GREEN
            } else {
                theme::TEXT_SECONDARY
            };
            res_col = res_col.push(
                text(format!("• {}: {}", item.repo_name, item.message))
                    .size(11)
                    .color(color),
            );
        }
        let box_el = container(
            column![
                row![
                    text(format!(
                        "批量操作结果：成功 {ok_count} 项 · 跳过或失败 {skip_count} 项（共 {} 项）",
                        results.len()
                    ))
                    .size(12)
                    .color(theme::TEXT_PRIMARY),
                    container(text("").size(1)).width(Length::Fill),
                    button(text("关闭").size(11))
                        .on_press(Message::ClearBatchResults)
                        .style(theme::secondary_btn_style)
                        .padding([3, 8]),
                ],
                scrollable(res_col).height(110),
            ]
            .spacing(6),
        )
        .padding(10)
        .style(theme::card_style);
        top_section = top_section.push(box_el);
    }

    let visible_paths: Vec<String> = filtered.iter().map(|r| r.path.clone()).collect();
    let all_checked = !visible_paths.is_empty()
        && visible_paths
            .iter()
            .all(|p| app.selected_repo_paths.contains(p));

    let table_header = container(
        row![
            button(text(if all_checked { "☑" } else { "☐" }).size(12))
                .on_press(Message::ToggleSelectAll(visible_paths))
                .style(theme::secondary_btn_style)
                .width(34)
                .padding([3, 6]),
            text("仓库").size(11).color(theme::TEXT_MUTED).width(Length::FillPortion(3)),
            text("当前分支").size(11).color(theme::TEXT_MUTED).width(Length::FillPortion(3)),
            text("主干偏离").size(11).color(theme::TEXT_MUTED).width(Length::FillPortion(2)),
            text("远程同步").size(11).color(theme::TEXT_MUTED).width(Length::FillPortion(2)),
            text("工作区状态").size(11).color(theme::TEXT_MUTED).width(Length::FillPortion(2)),
            text("最近提交").size(11).color(theme::TEXT_MUTED).width(Length::FillPortion(4)),
            text("操作").size(11).color(theme::TEXT_MUTED).width(120),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .padding([8, 12])
    .style(theme::elevated_style);

    let mut rows_col = column![].spacing(4);
    for repo in filtered {
        let checked = app.selected_repo_paths.contains(&repo.path);
        let branch_color = if repo.is_detached {
            theme::ACCENT_ROSE
        } else if repo.is_on_trunk {
            theme::TEXT_SECONDARY
        } else {
            theme::ACCENT_SKY
        };

        let trunk_text = match &repo.trunk_branch {
            Some(trunk) if repo.is_on_trunk => format!("在 {trunk}"),
            Some(trunk) => format!("{trunk} +{}/-{}", repo.trunk_ahead, repo.trunk_behind),
            None => "-".to_string(),
        };

        let remote_text = if repo.upstream.is_some() {
            format!("↑{} ↓{}", repo.upstream_ahead, repo.upstream_behind)
        } else {
            "未关联".to_string()
        };

        let (dirty_label, dirty_color) = if !repo.is_dirty() {
            ("干净".to_string(), theme::ACCENT_GREEN)
        } else if repo.conflicted_count > 0 {
            (format!("冲突 {}", repo.conflicted_count), theme::ACCENT_ROSE)
        } else if repo.has_dirty_on_trunk {
            (format!("主干脏 {}", repo.total_dirty()), theme::ACCENT_AMBER)
        } else {
            (
                format!("S:{} M:{} U:{}", repo.staged_count, repo.modified_count, repo.untracked_count),
                theme::ACCENT_AMBER,
            )
        };

        let commit_summary = match (&repo.last_commit_hash, &repo.last_commit_message) {
            (Some(h), Some(m)) => format!("{h} {m}"),
            _ => "-".to_string(),
        };

        let name_label = if repo.is_worktree {
            format!("{} [WT]", repo.name)
        } else {
            repo.name.clone()
        };

        let row_card = container(
            row![
                button(text(if checked { "☑" } else { "☐" }).size(12))
                    .on_press(Message::ToggleRepoSelect(repo.path.clone()))
                    .style(theme::secondary_btn_style)
                    .width(34)
                    .padding([3, 6]),
                column![
                    text(name_label).size(12).color(theme::TEXT_PRIMARY),
                    text(repo.relative_path.as_str()).size(11).color(theme::TEXT_MUTED),
                ]
                .spacing(1)
                .width(Length::FillPortion(3)),
                container(
                    text(repo.current_branch.as_str())
                        .size(11)
                        .font(Font::MONOSPACE)
                        .color(branch_color),
                )
                .padding([3, 7])
                .style(theme::badge_style(branch_color))
                .width(Length::FillPortion(3)),
                text(trunk_text)
                    .size(11)
                    .font(Font::MONOSPACE)
                    .color(theme::TEXT_SECONDARY)
                    .width(Length::FillPortion(2)),
                text(remote_text)
                    .size(11)
                    .font(Font::MONOSPACE)
                    .color(theme::TEXT_SECONDARY)
                    .width(Length::FillPortion(2)),
                container(text(dirty_label).size(11).color(dirty_color))
                    .padding([3, 7])
                    .style(theme::badge_style(dirty_color))
                    .width(Length::FillPortion(2)),
                text(commit_summary)
                    .size(11)
                    .font(Font::MONOSPACE)
                    .color(theme::TEXT_SECONDARY)
                    .width(Length::FillPortion(4)),
                {
                    let mut actions = row![].spacing(4).width(120).align_y(Alignment::Center);
                    if repo.has_dirty_on_trunk {
                        actions = actions.push(
                            button(text("救场").size(11))
                                .on_press(Message::OpenRescue(GuardrailAlert {
                                    repo_id: repo.id.clone(),
                                    repo_name: repo.name.clone(),
                                    repo_path: repo.path.clone(),
                                    trunk_branch: repo.trunk_branch.clone(),
                                    dirty_files: repo.total_dirty(),
                                    kind: GuardrailKind::DirtyTrunk,
                                }))
                                .style(theme::warning_btn_style)
                                .padding([4, 8]),
                        );
                    }
                    actions = actions.push(
                        button(text("CI").size(11))
                            .on_press(Message::OpenRepoCi(repo.path.clone()))
                            .style(theme::secondary_btn_style)
                            .padding([4, 10]),
                    );
                    actions
                },
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .padding([8, 12])
        .style(theme::card_style);

        rows_col = rows_col.push(row_card);
    }

    column![
        top_section,
        table_header,
        scrollable(rows_col).height(Length::Fill),
    ]
    .spacing(10)
    .padding(20)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn stat_card<'a>(label: &'a str, value: String, color: Color) -> Element<'a, Message> {
    container(
        column![
            text(label).size(11).color(theme::TEXT_MUTED),
            text(value).size(20).color(color),
        ]
        .spacing(4),
    )
    .width(Length::Fill)
    .padding([12, 14])
    .style(theme::card_style)
    .into()
}

fn filter_chip<'a>(label: &'a str, mode: FilterMode, active: bool) -> Element<'a, Message> {
    button(text(label).size(12))
        .on_press(Message::SetFilter(mode))
        .style(theme::nav_btn_style(active))
        .padding([5, 10])
        .into()
}
