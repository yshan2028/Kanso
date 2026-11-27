use crate::state::RepoFilter;
use crate::workbench::Workbench;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kanso_core::{RepoFailure, RepoStatus};
use std::ops::Range;

#[derive(Clone)]
enum RepoListRow {
    Healthy(RepoStatus),
    Failed(RepoFailure),
}

pub fn render_repos(
    workbench: &Workbench,
    _window: &mut Window,
    cx: &mut Context<Workbench>,
) -> AnyElement {
    let theme = cx.theme();
    let border = theme.border;
    let muted = theme.muted_foreground;
    let sec = theme.secondary;
    let yellow = theme.yellow;

    let Some(snap) = &workbench.state.snapshot else {
        return div()
            .id("repos-page")
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(13.0))
            .text_color(muted)
            .child("暂无仓库数据，请先选择或打开一个工作区目录。")
            .into_any_element();
    };

    let healthy_rows: Vec<RepoListRow> = workbench
        .state
        .filtered_repos()
        .into_iter()
        .cloned()
        .map(RepoListRow::Healthy)
        .collect();
    let failed_rows: Vec<RepoListRow> = workbench
        .state
        .filtered_failures()
        .into_iter()
        .cloned()
        .map(RepoListRow::Failed)
        .collect();

    let mut rows = Vec::with_capacity(healthy_rows.len() + failed_rows.len());
    rows.extend(healthy_rows);
    rows.extend(failed_rows);
    let total_rows = rows.len();

    let filters = [
        RepoFilter::All,
        RepoFilter::Dirty,
        RepoFilter::Alerts,
        RepoFilter::OffTrunk,
        RepoFilter::Errored,
    ];

    let mut filter_bar = div().flex().items_center().gap_1p5();
    for f in filters {
        let active = workbench.state.filter == f;
        let btn = Button::new(SharedString::from(format!("filter-btn-{f:?}")))
            .small()
            .label(f.label());
        let btn = if active { btn.primary() } else { btn.ghost() };
        filter_bar = filter_bar.child(btn.on_click(cx.listener(move |this, _, _, cx| {
            this.state.filter = f;
            cx.notify();
        })));
    }

    let header = div()
        .px_4()
        .py_2p5()
        .border_b_1()
        .border_color(border)
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2p5()
                .child(IconName::GitBranch)
                .child(
                    div()
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .line_height(relative(1.4))
                        .child(format!(
                            "仓库列表（{} 正常 / {} 失败）",
                            snap.repos.len(),
                            snap.failures.len()
                        )),
                )
                .when(workbench.state.verifying, |d| {
                    d.child(
                        div()
                            .id("verifying-cache-badge")
                            .px_2()
                            .py_0p5()
                            .rounded(px(4.0))
                            .bg(sec)
                            .border_1()
                            .border_color(border)
                            .text_size(px(11.0))
                            .text_color(yellow)
                            .child("正在校验缓存"),
                    )
                }),
        )
        .child(filter_bar);

    let table_head = div()
        .h(px(26.0))
        .px_3()
        .border_b_1()
        .border_color(border)
        .bg(sec)
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(11.0))
        .text_color(muted)
        .child(div().w(px(170.0)).child("仓库名称"))
        .child(div().w(px(175.0)).child("当前分支"))
        .child(div().w(px(90.0)).child("主干偏移"))
        .child(div().w(px(80.0)).child("远端"))
        .child(div().w(px(110.0)).child("工作区状态"))
        .child(div().flex_1().child("最近提交"));

    let selected_path = workbench.state.selected.clone();

    let body: AnyElement = if total_rows == 0 {
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(13.0))
            .text_color(muted)
            .child("当前筛选条件下没有匹配的仓库。")
            .into_any_element()
    } else {
        uniform_list(
            "repos-uniform-list",
            total_rows,
            cx.processor(move |_this, range: Range<usize>, _window, cx| {
                let row_theme = cx.theme();
                let row_border = row_theme.border;
                let row_fg = row_theme.foreground;
                let row_muted = row_theme.muted_foreground;
                let row_active = row_theme.list_active;
                let row_red = row_theme.red;
                let row_red_light = row_theme.red_light;
                let row_yellow = row_theme.yellow;
                let row_green = row_theme.green;
                let row_blue = row_theme.blue;
                let row_mono = row_theme.mono_font_family.clone();

                range
                    .map(|ix| match &rows[ix] {
                        RepoListRow::Healthy(repo) => {
                            let is_sel = selected_path.as_deref() == Some(repo.path.as_str());
                            let click_path = repo.path.clone();

                            let branch_color = if repo.is_detached {
                                row_red
                            } else if repo.is_on_trunk {
                                row_muted
                            } else {
                                row_blue
                            };

                            let trunk_delta = if repo.is_on_trunk {
                                repo.trunk_branch
                                    .clone()
                                    .unwrap_or_else(|| "主干".to_string())
                            } else {
                                format!("+{} / -{}", repo.trunk_ahead, repo.trunk_behind)
                            };

                            let remote_str = if repo.upstream.is_some() {
                                format!("↑{} ↓{}", repo.upstream_ahead, repo.upstream_behind)
                            } else {
                                "未绑定".to_string()
                            };

                            let (dirty_str, dirty_color) = if !repo.is_dirty() {
                                ("干净".to_string(), row_green)
                            } else if repo.conflicted_count > 0 {
                                (format!("冲突 {}", repo.conflicted_count), row_red)
                            } else if repo.has_dirty_on_trunk {
                                (format!("主干脏 {}", repo.total_dirty()), row_yellow)
                            } else {
                                (
                                    format!(
                                        "S:{} M:{} U:{}",
                                        repo.staged_count,
                                        repo.modified_count,
                                        repo.untracked_count
                                    ),
                                    row_yellow,
                                )
                            };

                            let commit_text =
                                match (&repo.last_commit_hash, &repo.last_commit_message) {
                                    (Some(h), Some(m)) => format!("{h} {m}"),
                                    _ => "-".to_string(),
                                };

                            div()
                                .id(SharedString::from(format!("repo-row-{ix}")))
                                .h(px(28.0))
                                .px_3()
                                .border_b_1()
                                .border_color(row_border)
                                .when(is_sel, |d| d.bg(row_active))
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_size(px(12.0))
                                .text_color(row_fg)
                                .cursor_pointer()
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.state.selected = Some(click_path.clone());
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .w(px(170.0))
                                        .truncate()
                                        .font_weight(if is_sel {
                                            FontWeight::SEMIBOLD
                                        } else {
                                            FontWeight::NORMAL
                                        })
                                        .child(repo.name.clone()),
                                )
                                .child(
                                    div()
                                        .w(px(175.0))
                                        .truncate()
                                        .font_family(row_mono.clone())
                                        .text_color(branch_color)
                                        .child(repo.current_branch.clone()),
                                )
                                .child(
                                    div()
                                        .w(px(90.0))
                                        .truncate()
                                        .font_family(row_mono.clone())
                                        .text_color(row_muted)
                                        .child(trunk_delta),
                                )
                                .child(
                                    div()
                                        .w(px(80.0))
                                        .truncate()
                                        .font_family(row_mono.clone())
                                        .text_color(row_muted)
                                        .child(remote_str),
                                )
                                .child(
                                    div()
                                        .w(px(110.0))
                                        .truncate()
                                        .font_family(row_mono.clone())
                                        .text_color(dirty_color)
                                        .child(dirty_str),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .font_family(row_mono.clone())
                                        .text_color(row_muted)
                                        .child(commit_text),
                                )
                                .test_support()
                                .into_any_element()
                        }
                        RepoListRow::Failed(fail) => {
                            let retry_path = fail.repo_path.clone();
                            let sel_path = fail.repo_path.clone();
                            let is_sel = selected_path.as_deref() == Some(fail.repo_path.as_str());
                            let short_err: String = fail.error.chars().take(72).collect();

                            div()
                                .id(SharedString::from(format!("repo-row-{ix}")))
                                .h(px(28.0))
                                .px_3()
                                .border_b_1()
                                .border_color(row_border)
                                .bg(if is_sel {
                                    row_active
                                } else {
                                    row_red_light.opacity(0.14)
                                })
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .text_size(px(12.0))
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.state.selected = Some(sel_path.clone());
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .min_w_0()
                                        .flex_1()
                                        .text_color(row_red)
                                        .child(IconName::CircleAlert)
                                        .child(
                                            div()
                                                .w(px(150.0))
                                                .truncate()
                                                .font_weight(FontWeight::MEDIUM)
                                                .child(fail.relative_path.clone()),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .truncate()
                                                .font_family(row_mono.clone())
                                                .child(format!("失败: {short_err}")),
                                        ),
                                )
                                .child(
                                    Button::new(SharedString::from(format!("retry-repo-{ix}")))
                                        .small()
                                        .danger()
                                        .label("重试")
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.refresh_repo(&retry_path, cx);
                                        })),
                                )
                                .test_support()
                                .into_any_element()
                        }
                    })
                    .collect::<Vec<AnyElement>>()
            }),
        )
        .size_full()
        .track_scroll(&workbench.repos_scroll)
        .into_any_element()
    };

    div()
        .id("repos-page")
        .key_context("KansoRepos")
        .size_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(header)
        .child(table_head)
        .child(div().flex_1().w_full().overflow_hidden().child(body))
        .into_any_element()
}
