pub mod overview;
pub mod placeholder;
/// 模型价格页面。
pub mod pricing;
/// 模型供应商页面。
pub mod providers;
pub mod repos;
/// 模型路由页面。
pub mod routing;
/// 应用设置页面。
pub mod settings;
pub mod tasks;
/// 模型用量页面。
pub mod usage;

use gpui_kit::component::ActiveTheme;
use gpui_kit::{
    AnyElement, App, FontWeight, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, TestSupportExt as _, div, px,
};

fn title_card(id: &'static str, title: &'static str, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .size_full()
        .p_6()
        .child(
            div()
                .id(id)
                .p_4()
                .rounded(px(8.0))
                .border_1()
                .border_color(theme.border)
                .bg(theme.secondary)
                .text_color(theme.foreground)
                .text_size(px(16.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title)
                .test_support(),
        )
        .into_any_element()
}
