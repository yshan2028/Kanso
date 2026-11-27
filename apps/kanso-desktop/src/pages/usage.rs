//! 会话日志用量的统计和请求视图。

use crate::usage_view::{
    RangeSummary, SeriesBar, SeriesSummary, build_series, family_label, format_short_num,
    format_usd, in_range, model_label, percent, range_start, source_label, summarize_range,
};
use crate::{UsageDim, UsageRange, UsageTab, Workbench};
use chrono::{DateTime, Local, NaiveDate};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme, Disableable as _, Selectable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kanso_usage::{Family, QuotaSnapshot, ReqStatus, UsageRequest, UsageSnapshot};
use std::cmp::Reverse;

const CHART_HEIGHT: f32 = 168.0;
const REQUESTS_PER_PAGE: usize = 50;
const SEVEN_DAYS_MINUTES: u32 = 7 * 24 * 60;
const CHART_LABELS: [&str; 3] = ["输入（含缓存写入）", "缓存读取", "输出"];
const BREAKDOWN_COLUMNS: [(&str, f32); 7] = [
    ("分组", 2.5),
    ("词元占比", 1.6),
    ("调用", 0.7),
    ("词元", 0.8),
    ("缓存命中", 0.9),
    ("错误", 0.7),
    ("估算费用", 1.0),
];
const REQUEST_COLUMNS: [(&str, f32); 8] = [
    ("时间", 1.4),
    ("状态", 0.6),
    ("来源", 1.6),
    ("模型", 1.6),
    ("输入", 0.8),
    ("输出", 0.8),
    ("缓存", 0.8),
    ("估算费用", 1.0),
];

/// 渲染基于会话日志的用量页面。
pub fn render_usage(
    this: &Workbench,
    _window: &mut Window,
    cx: &mut Context<Workbench>,
) -> AnyElement {
    let state = &this.state;
    let today = Local::now().date_naive();
    let header = div()
        .flex()
        .flex_col()
        .gap_3()
        .flex_shrink_0()
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(20.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(IconName::Activity)
                        .child(observed_text("用量")),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            pills(cx).children(
                                [
                                    (UsageRange::Today, "usage-range-today", "今天"),
                                    (UsageRange::D7, "usage-range-7d", "7 天"),
                                    (UsageRange::D30, "usage-range-30d", "30 天"),
                                    (UsageRange::All, "usage-range-all", "全部"),
                                ]
                                .map(|(range, id, label)| {
                                    pill(id, label, state.usage_range == range).on_click(
                                        cx.listener(move |this, _, _, cx| {
                                            this.state.usage_range = range;
                                            this.state.usage_req_page = 0;
                                            this.state.usage_hover_idx = None;
                                            cx.notify();
                                        }),
                                    )
                                }),
                            ),
                        )
                        .child(
                            Button::new("usage-refresh")
                                .small()
                                .outline()
                                .disabled(state.usage_loading)
                                .label(if state.usage_loading {
                                    "刷新中..."
                                } else {
                                    "刷新"
                                })
                                .on_click(cx.listener(|this, _, _, cx| this.refresh_usage(cx))),
                        ),
                ),
        )
        .child(
            div().child(
                pills(cx).children(
                    [
                        (UsageTab::Stats, "usage-tab-stats", "统计"),
                        (UsageTab::Requests, "usage-tab-requests", "请求"),
                    ]
                    .map(|(tab, id, label)| {
                        pill(id, label, state.usage_tab == tab).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.state.usage_tab = tab;
                                this.state.usage_hover_idx = None;
                                cx.notify();
                            },
                        ))
                    }),
                ),
            ),
        );

    let body = match (&state.usage, &state.usage_error, state.usage_tab) {
        (Some(snapshot), _, UsageTab::Stats) => stats(this, snapshot, today, cx),
        (Some(snapshot), _, UsageTab::Requests) => requests(this, snapshot, today, cx),
        (None, Some(error), _) => card(cx).child(muted(error.clone(), cx)),
        (None, None, _) => card(cx).child(muted("尚无会话日志用量", cx)),
    };

    div()
        .id("page-usage")
        .size_full()
        .p_4()
        .flex()
        .flex_col()
        .gap_3()
        .text_color(cx.theme().foreground)
        .text_size(px(13.0))
        .child(header)
        .child(
            div()
                .id(match state.usage_tab {
                    UsageTab::Stats => "usage-stats-scroll",
                    UsageTab::Requests => "usage-requests-scroll",
                })
                .min_h_0()
                .flex_1()
                .overflow_y_scroll()
                .child(body),
        )
        .test_support()
        .into_any_element()
}

fn pills(cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .p_1()
        .rounded(px(18.0))
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().secondary)
}

fn pill(id: &'static str, label: &'static str, active: bool) -> Button {
    Button::new(id)
        .small()
        .ghost()
        .rounded(px(14.0))
        .label(label)
        .selected(active)
}

fn card(cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_3()
        .rounded(px(8.0))
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
}

fn muted(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_size(px(12.0))
        .text_color(cx.theme().muted_foreground)
        .child(observed_text(text))
}

fn observed_text(text: impl Into<SharedString>) -> impl IntoElement {
    let text = text.into();
    text.clone().test_text(text)
}

fn section_title(title: &'static str) -> Div {
    div()
        .text_size(px(14.0))
        .font_weight(FontWeight::SEMIBOLD)
        .child(observed_text(title))
}

fn stats(
    this: &Workbench,
    snapshot: &UsageSnapshot,
    today: NaiveDate,
    cx: &Context<Workbench>,
) -> Div {
    let range = this.state.usage_range;
    let series = build_series(snapshot, range, today);
    let summary = summarize_range(snapshot, range, today, this.state.usage_dim);
    let totals = &summary.totals;
    let coverage = &snapshot.coverage;
    let first = if coverage.first_day.is_empty() {
        "—"
    } else {
        &coverage.first_day
    };
    let last = if coverage.last_day.is_empty() {
        "—"
    } else {
        &coverage.last_day
    };
    let kpis = [
        (
            "估算费用",
            format_usd(totals.est_usd),
            "按内置价目估算".to_string(),
        ),
        (
            "词元（Tokens）",
            format_short_num(totals.tokens),
            format!(
                "输入 {} · 输出 {}",
                format_short_num(totals.input),
                format_short_num(totals.output)
            ),
        ),
        (
            "缓存命中",
            format!("{:.1}%", totals.cache_hit_pct()),
            format!("读取 {} 词元", format_short_num(totals.cache_read)),
        ),
        (
            "调用",
            format_short_num(totals.calls),
            format!("{} 次失败", totals.errors),
        ),
        ("平均耗时", "—".into(), "会话日志不含耗时".into()),
    ];

    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .gap_2()
                .children(kpis.map(|(name, value, detail)| {
                    card(cx)
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .bg(cx.theme().secondary)
                        .child(muted(name, cx))
                        .child(
                            div()
                                .truncate()
                                .text_size(px(20.0))
                                .font_family(cx.theme().mono_font_family.clone())
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(value),
                        )
                        .child(muted(detail, cx))
                })),
        )
        .child(muted(
            format!(
                "~/.claude/projects + ~/.codex/sessions · {first} → {last} · {} 天有数据",
                coverage.active_days
            ),
            cx,
        ))
        .child(chart(&series, this.state.usage_hover_idx, cx))
        .child(breakdown(&summary, this.state.usage_dim, cx))
        .child(quotas(&snapshot.quota, cx))
}

fn chart(
    series: &SeriesSummary,
    hovered: Option<usize>,
    cx: &Context<Workbench>,
) -> impl IntoElement {
    let theme = cx.theme();
    let colors = [theme.blue, theme.cyan, theme.green];
    let (title, unit, average) = if series.is_hourly {
        ("每小时词元", "小时", "时均")
    } else {
        ("每日词元", "天", "日均")
    };
    let peak = format!(
        "峰值 {} · {} · {} {unit}有请求 · {average} {}",
        series.peak_label,
        format_short_num(series.peak_tokens),
        series.active_count,
        format_short_num(series.avg_active)
    );
    let detail = match hovered.and_then(|index| series.bars.get(index)) {
        Some(bar) if bar.calls == 0 => format!("{} · 没有请求", bar.label),
        Some(bar) => format!(
            "{} · {} 次请求 · 输入 {} · 缓存读取 {} · 输出 {} · 合计 {}",
            bar.label,
            bar.calls,
            format_short_num(bar.in_u),
            format_short_num(bar.cache_read),
            format_short_num(bar.output),
            format_short_num(bar.total)
        ),
        None => "悬停柱列查看详情".to_string(),
    };

    let columns = series.bars.iter().enumerate().map(|(index, bar)| {
        let show_tick = series.tick_indices.contains(&index);
        let plot_column = div()
            .id("usage-chart-bar")
            .h(px(CHART_HEIGHT))
            .w_full()
            .flex()
            .items_end()
            .justify_center()
            .cursor_crosshair()
            .when(hovered == Some(index), |column| {
                column.bg(theme.secondary_hover)
            })
            .child(chart_stack(bar, series.y_max, colors, cx))
            .on_hover(cx.listener(move |this, inside: &bool, _, cx| {
                let current = &mut this.state.usage_hover_idx;
                if *inside && *current != Some(index) {
                    *current = Some(index);
                    cx.notify();
                } else if !inside && *current == Some(index) {
                    *current = None;
                    cx.notify();
                }
            }))
            .test_support();
        div()
            .id(("usage-chart-column", index))
            .relative()
            .flex_1()
            .min_w_0()
            .child(plot_column)
            .when(show_tick, |column| {
                column.child(
                    muted(bar.label.clone(), cx)
                        .absolute()
                        .top(px(CHART_HEIGHT + 4.0))
                        .left(relative(0.5))
                        .ml(px(-26.0))
                        .w(px(52.0))
                        .text_size(px(11.0))
                        .text_center(),
                )
            })
    });
    let chart_plot = div()
        .flex()
        .gap_1()
        .child(
            div()
                .id("usage-chart-scroll")
                .min_w_0()
                .flex_1()
                .overflow_x_scroll()
                .child(
                    div()
                        .relative()
                        .w_full()
                        .min_w(px(series.bars.len() as f32 * 5.0 + 52.0))
                        .h(px(CHART_HEIGHT + 24.0))
                        .px(px(26.0))
                        .children([0.0, 0.5, 1.0].map(|fraction| {
                            div()
                                .absolute()
                                .top(px(fraction * (CHART_HEIGHT - 1.0)))
                                .left(px(26.0))
                                .right(px(26.0))
                                .h(px(1.0))
                                .bg(theme.border)
                        }))
                        .child(div().flex().children(columns)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .justify_between()
                .h(px(CHART_HEIGHT))
                .w(px(43.0))
                .flex_shrink_0()
                .text_right()
                .children(
                    [series.y_max, series.y_max / 2 + series.y_max % 2, 0]
                        .map(|tick| muted(format_short_num(tick), cx)),
                ),
        );

    card(cx)
        .id("usage-chart")
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .child(section_title(title))
                .child(muted(peak, cx)),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_3()
                .children(CHART_LABELS.into_iter().zip(colors).map(|(label, color)| {
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(div().size(px(7.0)).rounded_full().bg(color))
                        .child(muted(label, cx))
                })),
        )
        .child(chart_plot)
        .child(
            muted(detail, cx)
                .id("usage-chart-detail")
                .px_2()
                .py_1()
                .rounded(px(4.0))
                .bg(theme.secondary)
                .test_support(),
        )
        .test_support()
}

fn chart_stack(bar: &SeriesBar, y_max: u64, colors: [Hsla; 3], cx: &App) -> Div {
    let stack = div()
        .w(relative(0.62))
        .min_w(px(2.0))
        .max_w(px(34.0))
        .flex()
        .flex_col()
        .rounded(px(2.0))
        .overflow_hidden();
    if bar.total == 0 {
        return stack.child(
            div()
                .id("usage-chart-baseline")
                .h(px(2.0))
                .bg(cx.theme().muted_foreground)
                .test_support(),
        );
    }

    stack.children(
        [bar.in_u, bar.cache_read, bar.output]
            .into_iter()
            .zip(colors)
            .rev()
            .filter(|(tokens, _)| *tokens > 0)
            .map(|(tokens, color)| {
                let height = (tokens as f32 / y_max as f32) * CHART_HEIGHT;
                div().h(px(height.max(1.0))).flex_shrink_0().bg(color)
            }),
    )
}

fn table_cell(content: Div, width: f32) -> Div {
    content
        .min_w_0()
        .flex_basis(px(0.0))
        .flex_grow(width)
        .truncate()
}

fn table_row(cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .border_b_1()
        .border_color(cx.theme().border)
}

fn table_header(columns: &[(&'static str, f32)], cx: &App) -> Div {
    table_row(cx).bg(cx.theme().secondary).children(
        columns
            .iter()
            .map(|&(label, width)| table_cell(muted(label, cx), width)),
    )
}

fn progress(percent: f64, color: Hsla, cx: &App) -> Div {
    div()
        .h(px(6.0))
        .flex_1()
        .rounded_full()
        .overflow_hidden()
        .bg(cx.theme().border)
        .child(
            div()
                .h_full()
                .w(relative((percent / 100.0) as f32))
                .bg(color),
        )
}

fn breakdown(summary: &RangeSummary, dim: UsageDim, cx: &Context<Workbench>) -> Div {
    let rows = summary.breakdown.iter().map(|row| {
        let share = percent(row.totals.tokens, summary.totals.tokens);
        let name = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().min_w_0().truncate().child(row.label.clone()))
            .when(dim == UsageDim::Provider, |cell| {
                cell.child(
                    muted("家族级", cx)
                        .flex_shrink_0()
                        .px_1()
                        .rounded(px(4.0))
                        .bg(cx.theme().secondary),
                )
            });
        let share_cell = div()
            .flex()
            .items_center()
            .gap_2()
            .child(progress(share, cx.theme().muted_foreground, cx))
            .child(muted(format!("{share:.0}%"), cx));
        let cells = [
            name,
            share_cell,
            div().child(format_short_num(row.totals.calls)),
            div().child(format_short_num(row.totals.tokens)),
            div().child(format!("{:.0}%", row.totals.cache_hit_pct())),
            div().child(row.totals.errors.to_string()),
            div().child(format_usd(row.totals.est_usd)),
        ];
        table_row(cx).children(
            cells
                .into_iter()
                .zip(BREAKDOWN_COLUMNS)
                .map(|(cell, (_, width))| table_cell(cell, width)),
        )
    });
    card(cx)
        .child(
            div()
                .flex()
                .flex_wrap()
                .justify_between()
                .items_center()
                .gap_2()
                .child(section_title("用量明细"))
                .child(
                    pills(cx).children(
                        [
                            (UsageDim::Provider, "usage-dim-provider", "按供应商"),
                            (UsageDim::Model, "usage-dim-model", "按模型"),
                            (UsageDim::Agent, "usage-dim-agent", "按 Agent"),
                        ]
                        .map(|(value, id, label)| {
                            pill(id, label, dim == value).on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.state.usage_dim = value;
                                    cx.notify();
                                },
                            ))
                        }),
                    ),
                ),
        )
        .child(
            div()
                .id("usage-breakdown-scroll")
                .overflow_x_scroll()
                .child(
                    div()
                        .min_w(px(580.0))
                        .child(table_header(&BREAKDOWN_COLUMNS, cx))
                        .children(rows)
                        .when(summary.breakdown.is_empty(), |table| {
                            table.child(muted("此范围没有用量明细", cx).p_2())
                        }),
                ),
        )
}

fn quotas(quota: &[QuotaSnapshot], cx: &App) -> Div {
    card(cx)
        .child(section_title("额度"))
        .children(
            quota
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.family == Family::OpenAi)
                .map(|(index, entry)| {
                    let captured =
                        local_timestamp(DateTime::from_timestamp_millis(entry.captured_ts_ms));
                    let weekly = entry
                        .primary
                        .iter()
                        .chain(entry.secondary.iter())
                        .find(|window| window.window_minutes == SEVEN_DAYS_MINUTES);
                    let row = div()
                        .id("usage-quota-codex")
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(div().child(family_label(&entry.family)).child(muted(
                            format!("Codex 会话日志 token_count · 采集 {captured}"),
                            cx,
                        )))
                        .test_support();

                    let row = if let Some(weekly) = weekly {
                        let used = weekly.used_percent.clamp(0.0, 100.0);
                        let reset = local_timestamp(
                            weekly
                                .resets_at
                                .and_then(|ts| DateTime::from_timestamp(ts, 0)),
                        );
                        let theme = cx.theme();
                        let color = if used >= 95.0 {
                            theme.red
                        } else if used >= 80.0 {
                            theme.yellow
                        } else {
                            theme.green
                        };
                        row.child(
                            div()
                                .id("usage-quota-weekly")
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_2()
                                .child(muted("7d", cx))
                                .child(div().flex().w(px(108.0)).child(progress(used, color, cx)))
                                .child(observed_text(format!("已用 {used:.0}%")))
                                .child(muted(format!("重置 {reset}"), cx))
                                .test_support(),
                        )
                    } else {
                        row.child(muted("会话日志未提供 7d 额度", cx))
                    };
                    div().id(("usage-quota-entry", index)).child(row)
                }),
        )
        .child(
            muted("Anthropic · Claude Code — 会话日志未提供额度字段", cx)
                .id("usage-quota-claude")
                .test_support(),
        )
}

fn local_timestamp(timestamp: Option<DateTime<chrono::Utc>>) -> String {
    timestamp.map_or_else(
        || "—".to_string(),
        |time| time.with_timezone(&Local).format("%m-%d %H:%M").to_string(),
    )
}

fn requests(
    this: &Workbench,
    snapshot: &UsageSnapshot,
    today: NaiveDate,
    cx: &Context<Workbench>,
) -> Div {
    let state = &this.state;
    let start = range_start(state.usage_range, today);
    let mut filtered: Vec<_> = snapshot
        .requests
        .iter()
        .filter(|row| {
            state
                .usage_req_filter
                .as_ref()
                .is_none_or(|status| row.status == *status)
        })
        .filter(|row| {
            DateTime::from_timestamp_millis(row.ts_ms)
                .is_some_and(|time| in_range(time.with_timezone(&Local).date_naive(), start, today))
        })
        .collect();
    filtered.sort_by_key(|row| Reverse(row.ts_ms));
    let total_pages = filtered.len().div_ceil(REQUESTS_PER_PAGE).max(1);
    let page = state.usage_req_page.min(total_pages - 1);
    let rows = filtered
        .iter()
        .skip(page * REQUESTS_PER_PAGE)
        .take(REQUESTS_PER_PAGE)
        .enumerate()
        .map(|(index, row)| {
            div()
                .id(("usage-request-entry", index))
                .child(request_row(row, cx))
        });

    card(cx)
        .child(
            div()
                .flex()
                .flex_wrap()
                .justify_between()
                .items_center()
                .gap_2()
                .child(
                    pills(cx).children(
                        [
                            (None, "usage-filter-all", "全部"),
                            (Some(ReqStatus::Success), "usage-filter-success", "成功"),
                            (Some(ReqStatus::Error), "usage-filter-error", "失败"),
                        ]
                        .map(|(status, id, label)| {
                            pill(id, label, state.usage_req_filter == status).on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.state.usage_req_filter = status.clone();
                                    this.state.usage_req_page = 0;
                                    cx.notify();
                                },
                            ))
                        }),
                    ),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(muted(
                            format!(
                                "{} 条 · {REQUESTS_PER_PAGE} 条/页 · 第 {} / {total_pages} 页",
                                filtered.len(),
                                page + 1
                            ),
                            cx,
                        ))
                        .child(
                            Button::new("usage-requests-prev")
                                .small()
                                .outline()
                                .label("上一页")
                                .disabled(page == 0)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.state.usage_req_page = page.saturating_sub(1);
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("usage-requests-next")
                                .small()
                                .outline()
                                .label("下一页")
                                .disabled(page + 1 == total_pages)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.state.usage_req_page = page + 1;
                                    cx.notify();
                                })),
                        ),
                ),
        )
        .child(
            div()
                .id("usage-requests-table")
                .overflow_x_scroll()
                .child(
                    div()
                        .min_w(px(630.0))
                        .child(table_header(&REQUEST_COLUMNS, cx))
                        .children(rows)
                        .when(filtered.is_empty(), |table| {
                            table.child(muted("此范围没有符合条件的请求", cx).p_2())
                        }),
                )
                .test_support(),
        )
}

fn request_row(row: &UsageRequest, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let (status, color) = match row.status {
        ReqStatus::Success => ("成功", theme.green),
        ReqStatus::Error => ("失败", theme.red),
    };
    let input = row
        .input_uncached
        .saturating_add(row.cache_write)
        .saturating_add(row.cache_read);
    let cells = [
        div().child(local_timestamp(DateTime::from_timestamp_millis(row.ts_ms))),
        div().text_color(color).child(status),
        div().child(source_label(&row.source_kind)),
        div().child(observed_text(model_label(row.model.as_deref()).to_string())),
        div().child(format_short_num(input)),
        div().child(format_short_num(row.output)),
        div().child(format_short_num(row.cache_read)),
        div().child(format_usd(row.est_usd)),
    ];
    table_row(cx)
        .id("usage-request-row")
        .children(
            cells
                .into_iter()
                .zip(REQUEST_COLUMNS)
                .map(|(cell, (_, width))| table_cell(cell, width)),
        )
        .test_support()
}
