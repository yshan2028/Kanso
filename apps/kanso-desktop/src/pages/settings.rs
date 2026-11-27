use super::title_card;
use crate::{Route, Workbench};
use gpui_kit::{AnyElement, Context, Window};

/// 渲染设置页面的标题卡片。
pub fn render_settings(
    _this: &Workbench,
    _window: &mut Window,
    cx: &mut Context<Workbench>,
) -> AnyElement {
    title_card("page-settings", Route::Settings.label(), cx)
}
