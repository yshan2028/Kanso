use std::path::PathBuf;

use crate::Workbench;
use crate::provider_view::{
    family_label, is_provider_enabled, move_provider, ordered_providers, toggle_provider,
};
use crate::state::LogLevel;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme, Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kanso_usage::{Family, ProviderEntry, ProviderSource, QuotaWindow};

const COLUMNS: [(&str, f32); 10] = [
    ("优先级", 100.0),
    ("供应商", 104.0),
    ("家族", 72.0),
    ("来源", 76.0),
    ("CLI 版本", 76.0),
    ("模型", 152.0),
    ("调用", 52.0),
    ("错误率", 62.0),
    ("额度", 60.0),
    ("启用", 60.0),
];

/// 为当前应用实例指定供应商设置的保存文件，可用于隔离无头测试。
pub struct ProviderSettingsPath(
    /// 设置文件的完整路径。
    pub PathBuf,
);

impl Global for ProviderSettingsPath {}

/// 渲染供应商统计、额度和可编辑的供应商配置草案。
pub fn render_providers(
    this: &Workbench,
    _window: &mut Window,
    cx: &mut Context<Workbench>,
) -> AnyElement {
    let theme = cx.theme();
    let border = theme.border;
    let secondary = theme.secondary;
    let muted = theme.muted_foreground;
    let foreground = theme.foreground;
    let quotas = this.state.usage.iter().flat_map(|usage| &usage.quota);
    let minimum_remaining = quotas
        .clone()
        .flat_map(|quota| quota.primary.iter().chain(&quota.secondary))
        .filter_map(remaining)
        .reduce(f64::min);
    let openai_remaining = quotas
        .filter(|quota| quota.family == Family::OpenAi)
        .filter_map(|quota| quota.primary.as_ref().and_then(remaining))
        .reduce(f64::min);
    let providers = ordered_providers(&this.state.providers, &this.settings.provider_order);
    let active = providers
        .iter()
        .filter(|provider| provider.calls > 0)
        .count();
    let metrics = [
        (
            "provider-count",
            "已识别供应商数",
            providers.len().to_string(),
        ),
        ("provider-active", "近 30 天活跃数", active.to_string()),
        (
            "provider-min-quota",
            "最低剩余额度",
            percent(minimum_remaining),
        ),
    ];

    let table_header = div()
        .id("providers-table-header")
        .bg(secondary)
        .px_3()
        .py_2()
        .flex()
        .text_color(muted)
        .font_weight(FontWeight::SEMIBOLD)
        .children(
            COLUMNS
                .iter()
                .enumerate()
                .map(|(index, (label, _))| column_text(index, *label)),
        )
        .test_support();
    let table_width = COLUMNS.iter().map(|(_, width)| width).sum::<f32>() + 24.0;
    let mut table = div().min_w(px(table_width)).child(table_header);
    if providers.is_empty() {
        table = table.child(
            div()
                .px_3()
                .py_3()
                .text_color(muted)
                .child("暂无供应商记录"),
        );
    }
    for (index, provider) in providers.iter().enumerate() {
        table = table.child(provider_row(
            this,
            provider,
            index,
            providers.len(),
            openai_remaining,
            cx,
        ));
    }

    div()
        .id("page-providers")
        .size_full()
        .overflow_y_scroll()
        .text_color(foreground)
        .child(
            div()
                .p_4()
                .flex()
                .flex_col()
                .gap_4()
                .child(div().text_size(px(20.0)).font_weight(FontWeight::SEMIBOLD).child("供应商"))
                .child(
                    div()
                        .id("providers-kpis")
                        .flex()
                        .gap_3()
                        .children(metrics.map(|(id, label, value)| {
                            div()
                                .id(id)
                                .flex_1()
                                .min_w_0()
                                .p_3()
                                .rounded(px(8.0))
                                .border_1()
                                .border_color(border)
                                .bg(secondary)
                                .text_size(px(12.0))
                                .child(div().text_color(muted).child(label))
                                .child(
                                    div()
                                        .id("provider-kpi-value")
                                        .text_size(px(20.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .aria_label(value.clone())
                                        .child(value)
                                        .test_support(),
                                )
                                .test_support()
                        }))
                        .test_support(),
                )
                .child(
                    div()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(border)
                        .overflow_hidden()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_3()
                                .py_3()
                                .child(div().font_weight(FontWeight::SEMIBOLD).child("供应商池"))
                                .child(div().text_size(px(12.0)).text_color(muted).child("配置草案")),
                        )
                        .child(
                            div()
                                .id("providers-table")
                                .w_full()
                                .text_size(px(12.0))
                                .overflow_x_scroll()
                                .child(table)
                                .test_support(),
                        ),
                )
                .child(
                    div().text_size(px(12.0)).text_color(muted).child(
                        "Kanso 不读取任何账号凭据；账号登录由各 CLI 自行管理，因此供应商只能识别到工具/家族级",
                    ),
                ),
        )
        .test_support()
        .into_any_element()
}

fn provider_row(
    this: &Workbench,
    provider: &ProviderEntry,
    index: usize,
    count: usize,
    openai_remaining: Option<f64>,
    cx: &Context<Workbench>,
) -> impl IntoElement {
    let enabled = is_provider_enabled(&this.settings.provider_disabled, &provider.id);
    let id = provider.id.clone();
    let source = match provider.source {
        ProviderSource::SessionLog => "会话日志",
        ProviderSource::CliProbe => "CLI 探测",
    };
    let models = if provider.models.is_empty() {
        "—".into()
    } else {
        provider.models.join("、")
    };
    let error_rate =
        (provider.calls > 0).then(|| 100.0 * provider.errors as f64 / provider.calls as f64);
    let remaining = if provider.family == Family::OpenAi {
        openai_remaining
    } else {
        None
    };

    div()
        .id(SharedString::from(format!("provider-row-{id}")))
        .px_3()
        .py_2()
        .border_t_1()
        .border_color(cx.theme().border)
        .flex()
        .items_center()
        .child(
            column(0)
                .flex()
                .items_center()
                .gap_1()
                .child(observed_text(div(), 0, format!("#{}", index + 1)))
                .child(move_button(provider, -1, index == 0, cx))
                .child(move_button(provider, 1, index + 1 == count, cx)),
        )
        .child(column_text(1, provider.name.clone()))
        .child(column_text(2, family_label(&provider.family)))
        .child(column_text(3, source))
        .child(column_text(
            4,
            provider.version.as_deref().unwrap_or("—").to_owned(),
        ))
        .child(column_text(5, models))
        .child(column_text(6, provider.calls.to_string()))
        .child(column_text(7, percent(error_rate)))
        .child(column_text(8, percent(remaining)))
        .child(
            column(9).child(
                Button::new("provider-toggle")
                    .small()
                    .when(enabled, |button| button.outline())
                    .when(!enabled, |button| button.ghost())
                    .label(if enabled { "已启用" } else { "已停用" })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        toggle_provider(&mut this.settings.provider_disabled, &id);
                        save_provider_settings(this, cx);
                    })),
            ),
        )
        .test_support()
}

fn column(index: usize) -> Div {
    let width = px(COLUMNS[index].1);
    div()
        .min_w(width)
        .w(width)
        .pr_1()
        .when(index == 5, |column| column.flex_1())
}

fn column_text(index: usize, value: impl Into<SharedString>) -> impl IntoElement {
    observed_text(column(index), index, value)
}

fn observed_text(cell: Div, index: usize, value: impl Into<SharedString>) -> impl IntoElement {
    let value = value.into();
    cell.id(("provider-column", index))
        .aria_label(value.clone())
        .child(value)
        .test_support()
}

fn move_button(
    provider: &ProviderEntry,
    delta: isize,
    disabled: bool,
    cx: &Context<Workbench>,
) -> Button {
    let id = provider.id.clone();
    let (button_id, arrow, action) = if delta < 0 {
        ("provider-move-up", "↑", "上移")
    } else {
        ("provider-move-down", "↓", "下移")
    };
    Button::new(button_id)
        .small()
        .ghost()
        .label(arrow)
        .accessibility_label(format!("{action}{}", provider.name))
        .disabled(disabled)
        .on_click(cx.listener(move |this, _, _, cx| {
            if move_provider(
                &this.state.providers,
                &mut this.settings.provider_order,
                &id,
                delta,
            ) {
                save_provider_settings(this, cx);
            }
        }))
}

fn save_provider_settings(this: &mut Workbench, cx: &mut Context<Workbench>) {
    let saved = match cx.try_global::<ProviderSettingsPath>() {
        Some(path) => this.settings.save_to(&path.0),
        None => this.settings.save(),
    };
    if let Err(error) = saved {
        this.state
            .push_log(LogLevel::Warn, format!("保存供应商设置失败：{error}"));
    }
    cx.notify();
}

fn remaining(window: &QuotaWindow) -> Option<f64> {
    window
        .used_percent
        .is_finite()
        .then(|| (100.0 - window.used_percent).clamp(0.0, 100.0))
}

fn percent(value: Option<f64>) -> String {
    value.map_or_else(|| "—".into(), |value| format!("{value:.1}%"))
}
