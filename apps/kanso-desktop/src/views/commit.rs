use crate::app::{KansoApp, Message};
use crate::theme;
use iced::widget::{button, column, container, pick_list, row, scrollable, text, text_input};
use iced::{Alignment, Color, Element, Font, Length};
use kanso_core::{FileChangeStatus, VcsKind, run_preflight_checks};
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
        return container(
            text("请先在左侧添加多仓工作区，再进入提交与交付工作台")
                .size(13)
                .color(theme::TEXT_MUTED),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into();
    };

    let options: Vec<RepoOption> = snap
        .repos
        .iter()
        .map(|r| {
            let suffix = if r.is_dirty() {
                format!(" · {} 改动", r.total_dirty())
            } else {
                String::new()
            };
            RepoOption {
                path: r.path.clone(),
                label: format!("{} ({}){suffix}", r.name, r.current_branch),
            }
        })
        .collect();

    let selected_option = app
        .selected_commit_repo
        .as_deref()
        .and_then(|p| options.iter().find(|o| o.path == p).cloned());

    let header_left = row![
        pick_list(options, selected_option, |opt| {
            Message::SelectCommitRepo(opt.path)
        })
        .width(Length::Fill),
        button(text(if app.commit_loading { "读取中" } else { "刷新" }).size(11))
            .on_press(Message::RefreshChangeset)
            .style(theme::secondary_btn_style)
            .padding([6, 10]),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let mut left_col = column![
        text("目标仓库").size(11).color(theme::TEXT_MUTED),
        header_left,
    ]
    .spacing(8);

    if let Some(cs) = &app.changeset {
        let (vcs_label, vcs_color) = match cs.vcs_kind {
            VcsKind::Git => ("标准 Git 引擎", theme::ACCENT_SKY),
            VcsKind::JjColocated => ("jj + Git 共存", theme::ACCENT_GREEN),
            VcsKind::Jj => ("Jujutsu (jj)", theme::ACCENT_GREEN),
        };

        let mut engine_row = row![
            container(text(vcs_label).size(11).color(vcs_color))
                .padding([3, 8])
                .style(theme::badge_style(vcs_color)),
            container(text("").size(1)).width(Length::Fill),
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        if matches!(cs.vcs_kind, VcsKind::JjColocated | VcsKind::Jj) {
            engine_row = engine_row.push(
                button(text("jj undo 撤销").size(11))
                    .on_press(Message::RunJjUndo)
                    .style(theme::warning_btn_style)
                    .padding([3, 8]),
            );
        } else if app.jj_version.is_some() {
            engine_row = engine_row.push(
                button(text("开启 jj 共存").size(11))
                    .on_press(Message::EnableJjColocated)
                    .style(theme::secondary_btn_style)
                    .padding([3, 8]),
            );
        } else {
            engine_row = engine_row.push(
                text("装 jj 可用无暂存区共存")
                    .size(11)
                    .color(theme::TEXT_MUTED),
            );
        }

        left_col = left_col.push(engine_row);

        let all_selected = !cs.files.is_empty()
            && cs
                .files
                .iter()
                .all(|f| app.selected_commit_files.contains(&f.path));

        let files_header = row![
            text(format!("变更文件 ({})", cs.files.len()))
                .size(11)
                .color(theme::TEXT_MUTED),
            container(text("").size(1)).width(Length::Fill),
            button(text(if all_selected { "取消全选" } else { "全选" }).size(11))
                .on_press(Message::ToggleAllCommitFiles)
                .style(theme::secondary_btn_style)
                .padding([2, 8]),
        ]
        .align_y(Alignment::Center);

        let mut files_list = column![].spacing(4);
        if cs.files.is_empty() {
            files_list = files_list.push(
                text("当前工作区干净，可直接推送已有提交或合入主干。")
                    .size(11)
                    .color(theme::TEXT_MUTED),
            );
        } else {
            for file in &cs.files {
                let checked = app.selected_commit_files.contains(&file.path);
                let (tag, color) = file_status_meta(file.status);
                files_list = files_list.push(
                    button(
                        row![
                            text(if checked { "☑" } else { "☐" }).size(12),
                            text(tag)
                                .size(11)
                                .font(Font::MONOSPACE)
                                .color(color),
                            text(file.path.as_str())
                                .size(11)
                                .font(Font::MONOSPACE)
                                .color(theme::TEXT_PRIMARY),
                            container(text("").size(1)).width(Length::Fill),
                            text(format_bytes(file.size_bytes))
                                .size(11)
                                .color(theme::TEXT_MUTED),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    )
                    .on_press(Message::ToggleCommitFile(file.path.clone()))
                    .style(theme::nav_btn_style(checked))
                    .width(Length::Fill)
                    .padding([5, 8]),
                );
            }
        }

        left_col = left_col
            .push(files_header)
            .push(scrollable(files_list).height(155));

        let live_issues = run_preflight_checks(
            std::path::Path::new(&cs.repo_path),
            &cs.files,
            &cs.diff_text,
            Some(&app.commit_message),
        );

        let preflight_box: Element<'_, Message> = if live_issues.is_empty() {
            container(
                text("✓ 提交前安检通过：无敏感凭证、无超标大文件或 AI 署名")
                    .size(11)
                    .color(theme::ACCENT_GREEN),
            )
            .width(Length::Fill)
            .padding(8)
            .style(theme::badge_style(theme::ACCENT_GREEN))
            .into()
        } else {
            let mut iss_col = column![].spacing(3);
            for issue in live_issues.iter().take(3) {
                let color = if issue.severity == "error" {
                    theme::ACCENT_ROSE
                } else {
                    theme::ACCENT_AMBER
                };
                iss_col = iss_col.push(
                    text(format!("• [{}] {}", issue.code, issue.message))
                        .size(11)
                        .color(color),
                );
            }
            container(iss_col)
                .width(Length::Fill)
                .padding(8)
                .style(theme::badge_style(theme::ACCENT_ROSE))
                .into()
        };

        let prefixes = row![
            prefix_btn("feat"),
            prefix_btn("fix"),
            prefix_btn("refactor"),
            prefix_btn("test"),
            prefix_btn("chore"),
        ]
        .spacing(4);

        let commit_box = container(
            column![
                text("提交说明（建议：<type>(<scope>): <中文说明>）")
                    .size(11)
                    .color(theme::TEXT_MUTED),
                prefixes,
                text_input("例如：feat(order): 完善退款幂等校验", &app.commit_message)
                    .on_input(Message::CommitMessageChanged)
                    .on_submit(Message::SubmitCommit)
                    .style(theme::input_style)
                    .size(12)
                    .padding([7, 10]),
                row![
                    button(
                        text(if app.commit_allow_large {
                            "☑ 允许 >10MB 文件"
                        } else {
                            "☐ 允许 >10MB 文件"
                        })
                        .size(11),
                    )
                    .on_press(Message::ToggleAllowLarge(!app.commit_allow_large))
                    .style(theme::secondary_btn_style)
                    .padding([4, 8]),
                    container(text("").size(1)).width(Length::Fill),
                    button(text("提交代码").size(12))
                        .on_press(Message::SubmitCommit)
                        .style(theme::primary_btn_style)
                        .padding([6, 14]),
                ]
                .align_y(Alignment::Center),
            ]
            .spacing(8),
        )
        .padding(10)
        .style(theme::card_style);

        let remote_pick: Element<'_, Message> = if cs.remotes.is_empty() {
            text("当前仓未配置远端（可配置私有 Forgejo / origin 后推送）")
                .size(11)
                .color(theme::TEXT_MUTED)
                .into()
        } else {
            row![
                pick_list(
                    cs.remotes.clone(),
                    app.selected_push_remote.clone(),
                    Message::SelectPushRemote,
                )
                .width(Length::Fill),
                button(text("推送").size(12))
                    .on_press(Message::SubmitPush)
                    .style(theme::secondary_btn_style)
                    .padding([6, 14]),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
        };

        let trunk_label = cs.trunk_branch.as_deref().unwrap_or("未识别主干");
        let land_box = container(
            column![
                text("推送与合入主干").size(11).color(theme::TEXT_MUTED),
                remote_pick,
                theme::divider(),
                row![
                    text(format!("目标主干: {trunk_label}"))
                        .size(11)
                        .font(Font::MONOSPACE)
                        .color(theme::TEXT_SECONDARY),
                    container(text("").size(1)).width(Length::Fill),
                    button(text("合入主干").size(11))
                        .on_press(Message::SubmitLand)
                        .style(theme::warning_btn_style)
                        .padding([5, 12]),
                ]
                .align_y(Alignment::Center),
                row![
                    button(
                        text(if app.land_require_ci_pass {
                            "☑ CI 绿灯门禁"
                        } else {
                            "☐ CI 绿灯门禁"
                        })
                        .size(11),
                    )
                    .on_press(Message::ToggleLandRequireCi(!app.land_require_ci_pass))
                    .style(theme::secondary_btn_style)
                    .padding([3, 7]),
                    button(
                        text(if app.land_delete_source {
                            "☑ 合入后删源分支"
                        } else {
                            "☐ 合入后删源分支"
                        })
                        .size(11),
                    )
                    .on_press(Message::ToggleLandDeleteSource(!app.land_delete_source))
                    .style(theme::secondary_btn_style)
                    .padding([3, 7]),
                ]
                .spacing(6),
            ]
            .spacing(8),
        )
        .padding(10)
        .style(theme::card_style);

        left_col = left_col
            .push(preflight_box)
            .push(commit_box)
            .push(land_box);
    }

    let left_panel = container(scrollable(left_col).height(Length::Fill))
        .width(385)
        .height(Length::Fill)
        .padding(16)
        .style(theme::sidebar_style);

    let (diff_title, diff_body): (String, Element<'_, Message>) = match &app.changeset {
        None => (
            "变更 Diff 预览".to_string(),
            text("请选择左侧仓库查看实时文件变更与 Diff...")
                .size(12)
                .color(theme::TEXT_MUTED)
                .into(),
        ),
        Some(cs) if cs.diff_text.trim().is_empty() => (
            format!("变更 Diff · {} ({})", cs.repo_name, cs.current_branch),
            text("当前仓库工作区干净，暂无未提交 Diff 差异。")
                .size(12)
                .color(theme::TEXT_MUTED)
                .into(),
        ),
        Some(cs) => {
            let mut diff_lines = column![].spacing(2);
            for line in cs.diff_text.lines() {
                let color = if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff --git") {
                    theme::TEXT_PRIMARY
                } else if line.starts_with("@@") {
                    theme::ACCENT_SKY
                } else if line.starts_with('+') {
                    theme::ACCENT_GREEN
                } else if line.starts_with('-') {
                    theme::ACCENT_ROSE
                } else {
                    theme::TEXT_SECONDARY
                };
                diff_lines = diff_lines.push(
                    text(line)
                        .size(12)
                        .font(Font::MONOSPACE)
                        .color(color),
                );
            }
            (
                format!(
                    "变更 Diff · {} ({}) · {} 个文件",
                    cs.repo_name,
                    cs.current_branch,
                    cs.files.len()
                ),
                diff_lines.into(),
            )
        }
    };

    let right_header = row![
        text(diff_title).size(13).color(theme::TEXT_PRIMARY),
        container(text("").size(1)).width(Length::Fill),
        button(text("复制 Diff").size(11))
            .on_press(Message::CopyDiffText)
            .style(theme::secondary_btn_style)
            .padding([4, 10]),
    ]
    .align_y(Alignment::Center);

    let right_panel = column![
        container(right_header).padding([12, 18]),
        theme::divider(),
        scrollable(container(diff_body).padding(16)).height(Length::Fill),
    ]
    .width(Length::Fill)
    .height(Length::Fill);

    row![left_panel, theme::divider_v(), right_panel]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn prefix_btn(kind: &'static str) -> Element<'static, Message> {
    button(text(kind).size(11))
        .on_press(Message::PrependCommitPrefix(format!("{kind}(): ")))
        .style(theme::secondary_btn_style)
        .padding([3, 7])
        .into()
}

fn file_status_meta(status: FileChangeStatus) -> (&'static str, Color) {
    match status {
        FileChangeStatus::Modified => ("M", theme::ACCENT_AMBER),
        FileChangeStatus::Added => ("A", theme::ACCENT_GREEN),
        FileChangeStatus::Deleted => ("D", theme::ACCENT_ROSE),
        FileChangeStatus::Renamed => ("R", theme::ACCENT_SKY),
        FileChangeStatus::Untracked => ("?", theme::TEXT_SECONDARY),
        FileChangeStatus::Conflicted => ("!", theme::ACCENT_ROSE),
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
