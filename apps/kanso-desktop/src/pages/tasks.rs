use crate::state::AppState;
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme;
use gpui_kit::*;
use kanso_core::TaskStage;

fn stage_label(stage: &TaskStage) -> &'static str {
    match stage {
        TaskStage::InProgress => "开发中",
        TaskStage::Dirty => "待提交",
        TaskStage::ReadyForCi => "待验证",
        TaskStage::CiFailed => "CI 失败",
        TaskStage::ReadyToLand => "可合入",
    }
}

pub fn render_tasks(state: &AppState, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let mono = theme.mono_font_family.clone();
    let tasks = state.active_tasks();

    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .px_4()
        .py_3()
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(IconName::ListTodo)
                .child(
                    div()
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .line_height(relative(1.4))
                        .child(format!("跨仓任务（{}）", tasks.len())),
                ),
        )
        .child(
            div()
                .text_size(px(12.0))
                .text_color(theme.muted_foreground)
                .child("按同分支名聚合非主干仓库（只读预览，交互工作台将于 M3 交付）"),
        );

    if tasks.is_empty() {
        return div()
            .id("tasks-page")
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.0))
                    .text_color(theme.muted_foreground)
                    .child("暂无非主干特性分支任务。"),
            )
            .into_any_element();
    }

    let mut list_col = div().flex_1().p_4().flex().flex_col().gap_2();
    for task in tasks {
        let dirty_total = task.dirty_repos;
        let unpushed_total = task.unpushed_repos;
        let repo_names = task.repo_names.join(" · ");

        list_col = list_col.child(
            div()
                .p_3()
                .rounded(px(8.0))
                .border_1()
                .border_color(theme.border)
                .bg(theme.secondary)
                .flex()
                .flex_col()
                .gap_1p5()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .font_family(mono.clone())
                                        .text_size(px(13.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(task.branch.clone()),
                                )
                                .child(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded(px(4.0))
                                        .bg(theme.background)
                                        .border_1()
                                        .border_color(theme.border)
                                        .text_size(px(11.0))
                                        .child(stage_label(&task.stage)),
                                ),
                        )
                        .child(
                            div()
                                .font_family(mono.clone())
                                .text_size(px(12.0))
                                .text_color(theme.muted_foreground)
                                .child(format!(
                                    "{} 仓 · 脏仓 {} · 待推 {}",
                                    task.total_repos,
                                    dirty_total,
                                    unpushed_total
                                )),
                        ),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(theme.muted_foreground)
                        .child(repo_names),
                ),
        );
    }

    div()
        .id("tasks-page")
        .size_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(header)
        .child(list_col)
        .into_any_element()
}
