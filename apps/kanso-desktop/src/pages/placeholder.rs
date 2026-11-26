use crate::state::Route;
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme;
use gpui_kit::*;

pub fn render_placeholder(route: Route, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (icon, title, cli_cmd) = match route {
        Route::CrossCommit => (IconName::GitBranch, "跨仓提交", "kanso commit"),
        Route::Pipeline => (IconName::Workflow, "本地流水线", "kanso ci"),
        _ => (IconName::Folder, route.label(), "kanso --help"),
    };

    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .p_6()
        .child(
            div()
                .max_w(px(480.0))
                .w_full()
                .p_6()
                .rounded(px(8.0))
                .border_1()
                .border_color(theme.border)
                .bg(theme.secondary)
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_color(theme.foreground)
                        .child(icon)
                        .child(
                            div()
                                .text_size(px(15.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .line_height(relative(1.4))
                                .child(title),
                        ),
                )
                .child(
                    div()
                        .text_size(px(13.0))
                        .line_height(relative(1.4))
                        .text_color(theme.muted_foreground)
                        .child(format!(
                            "将在 M3 重建，过渡期请使用 `{cli_cmd}` 命令行。"
                        )),
                )
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded(px(6.0))
                        .bg(theme.background)
                        .border_1()
                        .border_color(theme.border)
                        .font_family(theme.mono_font_family.clone())
                        .text_size(px(12.0))
                        .text_color(theme.foreground)
                        .child(format!("$ {cli_cmd}")),
                ),
        )
        .into_any_element()
}
