use crate::Workbench;
use crate::provider_view::{ModelCostRow, aggregate_model_costs, family_label};
use gpui_kit::component::ActiveTheme;
use gpui_kit::*;
use kanso_usage::{PriceEntry, builtin_price_table};

const PRICE_HEADERS: [&str; 6] = ["家族", "匹配规则", "输入", "缓存写入", "缓存读取", "输出"];
const PRICE_WIDTHS: [f32; 6] = [86.0, 220.0, 86.0, 86.0, 86.0, 86.0];
const COST_HEADERS: [&str; 5] = ["模型", "家族", "调用", "词元（Tokens）", "估算费用"];
const COST_WIDTHS: [f32; 5] = [220.0, 108.0, 86.0, 120.0, 116.0];

/// 渲染内置模型单价和近 30 天按模型汇总的会话费用。
pub fn render_pricing(
    this: &Workbench,
    _window: &mut Window,
    cx: &mut Context<Workbench>,
) -> AnyElement {
    let prices = builtin_price_table();
    let costs = this
        .state
        .usage
        .as_ref()
        .map_or_else(Vec::new, aggregate_model_costs);

    div()
        .id("page-pricing")
        .size_full()
        .overflow_y_scroll()
        .text_color(cx.theme().foreground)
        .child(
            div()
                .p_4()
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    div()
                        .text_size(px(20.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("价格"),
                )
                .child(builtin_prices_card(&prices, cx))
                .child(model_cost_card(&costs, cx)),
        )
        .test_support()
        .into_any_element()
}

fn builtin_prices_card(prices: &[PriceEntry], cx: &App) -> Div {
    let header = table_header("builtin-prices-header", PRICE_HEADERS, PRICE_WIDTHS, cx);
    let mut table = div().min_w(px(674.0)).child(header);
    if prices.is_empty() {
        table = table.child(empty_row("builtin-prices-empty", "暂无内置价格", cx));
    }
    table = table.children(prices.iter().enumerate().map(|(index, price)| {
        table_row(("builtin-price-row", index).into(), cx)
            .children(cells(
                [
                    family_label(&price.family).to_owned(),
                    price.pattern.clone(),
                    price.input.to_string(),
                    price.cache_write.to_string(),
                    price.cache_read.to_string(),
                    price.output.to_string(),
                ],
                PRICE_WIDTHS,
            ))
            .test_support()
    }));

    card(
        "builtin-prices-table",
        "内置单价（美元 / 百万词元）",
        "Built-in 估算价；可在 ~/.kanso/config.toml 的 [pricing] 节覆盖；Gemini 暂未收录单价",
        table,
        cx,
    )
}

fn model_cost_card(costs: &[ModelCostRow], cx: &App) -> Div {
    let header = table_header("model-costs-header", COST_HEADERS, COST_WIDTHS, cx);
    let mut table = div().min_w(px(674.0)).child(header);
    if costs.is_empty() {
        table = table.child(empty_row("model-costs-empty", "暂无会话费用记录", cx));
    }
    table = table.children(costs.iter().enumerate().map(|(index, cost)| {
        table_row(("model-cost-row", index).into(), cx)
            .children(cells(
                [
                    cost.model.clone(),
                    family_label(&cost.family).to_owned(),
                    cost.calls.to_string(),
                    cost.tokens.to_string(),
                    format!("${:.4}", cost.est_usd),
                ],
                COST_WIDTHS,
            ))
            .test_support()
    }));

    card(
        "model-costs-table",
        "按模型估算费用（近 30 天实测会话日志）",
        "",
        table,
        cx,
    )
}

fn card(
    table_id: &'static str,
    title: &'static str,
    note: &'static str,
    table: Div,
    cx: &App,
) -> Div {
    let title = div()
        .px_3()
        .py_3()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().font_weight(FontWeight::SEMIBOLD).child(title));
    let title = if note.is_empty() {
        title
    } else {
        title.child(
            div()
                .text_size(px(12.0))
                .text_color(cx.theme().muted_foreground)
                .child(note),
        )
    };

    div()
        .rounded(px(8.0))
        .border_1()
        .border_color(cx.theme().border)
        .overflow_hidden()
        .child(title)
        .child(
            div()
                .id(table_id)
                .w_full()
                .text_size(px(12.0))
                .overflow_x_scroll()
                .child(table)
                .test_support(),
        )
}

fn table_row(id: ElementId, cx: &App) -> Stateful<Div> {
    div()
        .id(id)
        .px_3()
        .py_2()
        .flex()
        .border_t_1()
        .border_color(cx.theme().border)
}

fn table_header<const N: usize>(
    id: &'static str,
    labels: [&'static str; N],
    widths: [f32; N],
    cx: &App,
) -> impl IntoElement {
    table_row(id.into(), cx)
        .bg(cx.theme().secondary)
        .text_color(cx.theme().muted_foreground)
        .font_weight(FontWeight::SEMIBOLD)
        .children(cells(labels, widths))
        .test_support()
}

fn cells<const N: usize>(
    values: [impl Into<SharedString>; N],
    widths: [f32; N],
) -> impl Iterator<Item = impl IntoElement> {
    values
        .into_iter()
        .zip(widths)
        .enumerate()
        .map(|(index, (value, width))| {
            let value = value.into();
            div()
                .id(("price-column", index))
                .w(px(width))
                .flex_shrink_0()
                .pr_2()
                .aria_label(value.clone())
                .child(value)
                .test_support()
        })
}

fn empty_row(id: &'static str, label: &'static str, cx: &App) -> impl IntoElement {
    table_row(id.into(), cx)
        .text_color(cx.theme().muted_foreground)
        .child(label)
        .test_support()
}

#[cfg(test)]
mod tests {
    use super::builtin_prices_card;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AppContext as _, Bounds, Context, IntoElement, Point, Render, TestAppContext, Window,
        WindowBounds, WindowOptions, px, size,
    };
    use kanso_usage::{Family, PriceEntry};

    struct PriceCard;

    impl Render for PriceCard {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            builtin_prices_card(
                &[PriceEntry {
                    family: Family::Anthropic,
                    pattern: "claude-*".into(),
                    input: 3.0,
                    cache_write: 3.75,
                    cache_read: 0.3,
                    output: 15.0,
                }],
                cx,
            )
        }
    }

    #[gpui_kit::test]
    fn builtin_price_card_displays_all_price_fields(cx: &mut TestAppContext) {
        let (handle, _) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(960.0), px(640.0)),
                    })),
                    ..Default::default()
                },
                cx,
                |_, cx| cx.new(|_| PriceCard),
            )
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let row = window.within(("builtin-price-row", 0usize));
            for (index, expected) in ["Anthropic", "claude-*", "3", "3.75", "0.3", "15"]
                .into_iter()
                .enumerate()
            {
                let cell = row.find(("price-column", index));
                assert!(cell.visible());
                assert_eq!(cell.label(), Some(expected));
            }
        })
        .unwrap();
    }
}
