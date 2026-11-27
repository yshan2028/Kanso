use crate::state::AppState;
use crate::workbench::Workbench;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

fn metric_card(title: &str, value: String, color: Hsla, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .flex_1()
        .p_3()
        .rounded(px(8.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.secondary)
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(12.0))
                .line_height(relative(1.4))
                .text_color(theme.muted_foreground)
                .child(title.to_string()),
        )
        .child(
            div()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(20.0))
                .font_weight(FontWeight::SEMIBOLD)
                .line_height(relative(1.4))
                .text_color(color)
                .child(value),
        )
        .into_any_element()
}

pub fn render_overview(state: &AppState, cx: &mut Context<Workbench>) -> AnyElement {
    let theme = cx.theme();
    let fg = theme.foreground;
    let muted = theme.muted_foreground;
    let border = theme.border;
    let sec = theme.secondary;
    let blue = theme.blue;
    let yellow = theme.yellow;
    let red = theme.red;
    let green = theme.green;

    let Some(snap) = &state.snapshot else {
        return div()
            .id("overview-page")
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(13.0))
            .text_color(muted)
            .child("尚未加载工作区快照，请先选择目录。")
            .into_any_element();
    };

    let total_repos = snap.repos.len();
    let off_trunk = snap.repos.iter().filter(|r| !r.is_on_trunk).count();
    let dirty_repos = snap.repos.iter().filter(|r| r.is_dirty()).count();
    let failed_repos = snap.failures.len();
    let attention = state.attention_queue();

    let cards = div()
        .flex()
        .gap_3()
        .child(metric_card("仓库总数", total_repos.to_string(), fg, cx))
        .child(metric_card("非主干仓库", off_trunk.to_string(), blue, cx))
        .child(metric_card(
            "有改动仓库",
            dirty_repos.to_string(),
            yellow,
            cx,
        ))
        .child(metric_card("刷新失败", failed_repos.to_string(), red, cx));

    let queue_header = div()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(IconName::TriangleAlert)
                .child(
                    div()
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .line_height(relative(1.4))
                        .child(format!("需要你处理（{}）", attention.len())),
                ),
        )
        .child(
            div()
                .text_size(px(12.0))
                .text_color(muted)
                .child("异常与主干风险优先排在队列顶部"),
        );

    let queue_body: AnyElement = if attention.is_empty() {
        div()
            .p_5()
            .rounded(px(8.0))
            .border_1()
            .border_color(border)
            .bg(sec)
            .flex()
            .items_center()
            .gap_2()
            .text_color(green)
            .child(IconName::CircleCheck)
            .child(
                div()
                    .text_size(px(13.0))
                    .line_height(relative(1.4))
                    .child("工作区状态健康"),
            )
            .into_any_element()
    } else {
        let mut col = div().flex().flex_col().gap_2();
        for (ix, item) in attention.into_iter().take(40).enumerate() {
            let badge_color = if item.is_failure { red } else { yellow };
            let repo_path = item.repo_path.clone();
            let repo_path_btn = item.repo_path.clone();
            col = col.child(
                div()
                    .id(SharedString::from(format!("attention-item-{ix}")))
                    .px_3()
                    .py_2p5()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(border)
                    .bg(sec)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2p5()
                            .min_w_0()
                            .child(div().text_color(badge_color).child(if item.is_failure {
                                IconName::CircleAlert
                            } else {
                                IconName::TriangleAlert
                            }))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_size(px(13.0))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child(item.repo_name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(12.0))
                                                    .text_color(badge_color)
                                                    .child(item.title),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .text_color(muted)
                                            .truncate()
                                            .child(item.detail),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .child(
                                Button::new(SharedString::from(format!("att-locate-{ix}")))
                                    .small()
                                    .ghost()
                                    .label("定位")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.focus_repo(&repo_path, cx);
                                    })),
                            )
                            .when(item.is_failure, |row| {
                                row.child(
                                    Button::new(SharedString::from(format!("att-retry-{ix}")))
                                        .small()
                                        .danger()
                                        .label("重试")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.refresh_repo(&repo_path_btn, cx);
                                        })),
                                )
                            }),
                    ),
            );
        }
        col.into_any_element()
    };

    div()
        .id("overview-page")
        .size_full()
        .p_4()
        .flex()
        .flex_col()
        .gap_4()
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(20.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .line_height(relative(1.4))
                        .child(format!("{} · 工作区概览", snap.workspace_name)),
                )
                .when(state.verifying, |d| {
                    d.child(
                        div()
                            .px_2p5()
                            .py_1()
                            .rounded(px(6.0))
                            .bg(sec)
                            .border_1()
                            .border_color(border)
                            .text_size(px(12.0))
                            .text_color(yellow)
                            .child("正在校验缓存..."),
                    )
                }),
        )
        .child(cards)
        .child(queue_header)
        .child(
            div()
                .id("overview-queue-scroll")
                .flex_1()
                .overflow_y_scroll()
                .child(queue_body),
        )
        .into_any_element()
}
