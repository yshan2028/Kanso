use crate::state::{AppState, PaletteAction, PaletteItem};
use crate::workbench::{CancelAction, PaletteConfirm, PaletteDown, PaletteUp, Workbench};
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

pub fn render_palette_overlay(
    workbench: &Workbench,
    _window: &mut Window,
    cx: &mut Context<Workbench>,
) -> AnyElement {
    let theme = cx.theme();
    let query = workbench.palette_input.read(cx).value().to_string();
    let items: Vec<PaletteItem> = workbench
        .state
        .palette_items(&query)
        .into_iter()
        .take(12)
        .collect();
    let cursor = if items.is_empty() {
        0
    } else {
        workbench.palette_cursor.min(items.len() - 1)
    };

    let border = theme.border;
    let bg = theme.popover;
    let fg = theme.popover_foreground;
    let muted = theme.muted_foreground;
    let list_active = theme.list_active;
    let overlay_bg = theme.overlay.opacity(0.45);
    let mono = theme.mono_font_family.clone();

    let mut list_el = div()
        .id("palette-results")
        .max_h(px(340.0))
        .overflow_y_scroll()
        .flex()
        .flex_col();

    if items.is_empty() {
        list_el = list_el.child(
            div()
                .px_3()
                .py_4()
                .text_size(px(12.0))
                .text_color(muted)
                .child("未找到匹配的命令、仓库或分支"),
        );
    } else {
        for (ix, item) in items.iter().enumerate() {
            let selected = ix == cursor;
            let action = item.action.clone();
            let item_id = item.id.clone();
            let shortcut = item.shortcut;
            list_el = list_el.child(
                div()
                    .id(SharedString::from(format!("palette-item-{ix}")))
                    .px_3()
                    .py_2()
                    .when(selected, |d| d.bg(list_active))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.execute_palette_action(&item_id, action.clone(), window, cx);
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .text_color(fg)
                                    .truncate()
                                    .child(item.label.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(muted)
                                    .truncate()
                                    .child(item.subtitle.clone()),
                            ),
                    )
                    .when_some(shortcut, |row, sc| {
                        row.child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(px(4.0))
                                .border_1()
                                .border_color(border)
                                .font_family(mono.clone())
                                .text_size(px(11.0))
                                .text_color(muted)
                                .child(sc),
                        )
                    })
                    .test_support(),
            );
        }
    }

    div()
        .id("command-palette-backdrop")
        .absolute()
        .inset_0()
        .bg(overlay_bg)
        .flex()
        .justify_center()
        .items_start()
        .pt(px(64.0))
        .on_click(cx.listener(|this, _, window, cx| {
            this.close_palette(window, cx);
        }))
        .child(
            div()
                .id("command-palette")
                .key_context("KansoPalette")
                .on_action(cx.listener(|this, _: &CancelAction, window, cx| {
                    this.close_palette(window, cx);
                }))
                .on_action(cx.listener(|this, _: &PaletteUp, _, cx| {
                    this.move_palette_cursor(-1, cx);
                }))
                .on_action(cx.listener(|this, _: &PaletteDown, _, cx| {
                    this.move_palette_cursor(1, cx);
                }))
                .on_action(cx.listener(|this, _: &PaletteConfirm, window, cx| {
                    this.confirm_palette(window, cx);
                }))
                .on_click(|_, _, cx| {
                    cx.stop_propagation();
                })
                .w(px(560.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(border)
                .bg(bg)
                .shadow_lg()
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .border_b_1()
                        .border_color(border)
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().text_color(muted).child(IconName::Search))
                        .child(
                            div()
                                .flex_1()
                                .child(Input::new(&workbench.palette_input).cleanable(false)),
                        ),
                )
                .child(list_el)
                .test_support(),
        )
        .into_any_element()
}

pub fn selected_palette_item(state: &AppState, query: &str, cursor: usize) -> Option<PaletteItem> {
    let items: Vec<PaletteItem> = state.palette_items(query).into_iter().take(12).collect();
    if items.is_empty() {
        None
    } else {
        let idx = cursor.min(items.len() - 1);
        Some(items[idx].clone())
    }
}

pub fn run_open_dir_command(dir: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let prog = "open";
    #[cfg(target_os = "windows")]
    let prog = "explorer";
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let prog = "xdg-open";

    let args = [dir.as_os_str().to_os_string()];
    let _ = kanso_core::exec::run_raw(prog, &args, &[], kanso_core::exec::GitAccess::Read);
}

pub fn palette_action_label(action: &PaletteAction) -> &'static str {
    match action {
        PaletteAction::RefreshAll => "刷新全部仓库",
        PaletteAction::SetTheme(_) => "切换主题",
        PaletteAction::ToggleInspector => "切换检查器",
        PaletteAction::ToggleBottomPanel => "切换底部面板",
        PaletteAction::OpenWorkspacePicker => "打开工作区",
        PaletteAction::OpenLogsDir => "打开日志目录",
        PaletteAction::Navigate(_) => "页面跳转",
        PaletteAction::SelectRepo(_) => "定位仓库",
    }
}
