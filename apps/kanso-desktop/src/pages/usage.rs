use super::title_card;
use crate::{Route, Workbench};
use gpui_kit::{AnyElement, Context, Window};

/// 渲染用量页面的标题卡片。
pub fn render_usage(
    _this: &Workbench,
    _window: &mut Window,
    cx: &mut Context<Workbench>,
) -> AnyElement {
    title_card("page-usage", Route::Usage.label(), cx)
}
