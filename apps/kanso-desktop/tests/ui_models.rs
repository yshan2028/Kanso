use std::collections::BTreeMap;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, Point, TestAppContext, WindowBounds,
    WindowOptions, px, size,
};
use kanso_config::{ConfigLayer, KansoConfig, PriceOverride};
use kanso_desktop::pages::providers::ProviderSettingsPath;
use kanso_desktop::{AppState, Route, Settings, ThemeChoice, Workbench, bind_keys};
use kanso_usage::{
    Coverage, DailyBucket, Family, ProviderEntry, ProviderSource, QuotaSnapshot, QuotaWindow,
    SourceKind, UsageSnapshot, builtin_price_table,
};

fn snapshot() -> UsageSnapshot {
    let codex = DailyBucket {
        source_kind: SourceKind::CodexCli,
        family: Family::OpenAi,
        model: Some("gpt-5".into()),
        calls: 8,
        errors: 1,
        input_uncached: 100,
        cache_read: 50,
        cache_write: 0,
        output: 40,
        reasoning: 10,
        est_usd: 1.25,
    };
    let claude = DailyBucket {
        source_kind: SourceKind::ClaudeCode,
        family: Family::Anthropic,
        model: Some("claude-sonnet".into()),
        calls: 3,
        errors: 0,
        input_uncached: 25,
        cache_read: 10,
        cache_write: 5,
        output: 12,
        reasoning: 0,
        est_usd: 0.1,
    };
    UsageSnapshot {
        requests: Vec::new(),
        total_requests: 11,
        total_errors: 1,
        daily: BTreeMap::from([("2026-11-27".into(), vec![claude, codex])]),
        hourly_today: Vec::new(),
        coverage: Coverage {
            lookback_days: 30,
            first_day: "2026-11-27".into(),
            last_day: "2026-11-27".into(),
            active_days: 1,
        },
        quota: vec![QuotaSnapshot {
            family: Family::OpenAi,
            limit_id: "codex".into(),
            plan_type: None,
            primary: Some(QuotaWindow {
                used_percent: 32.5,
                window_minutes: 300,
                resets_at: None,
            }),
            secondary: Some(QuotaWindow {
                used_percent: 74.0,
                window_minutes: 10080,
                resets_at: None,
            }),
            captured_ts_ms: 0,
        }],
    }
}

fn providers() -> Vec<ProviderEntry> {
    vec![
        ProviderEntry {
            id: "codex".into(),
            name: "Codex".into(),
            family: Family::OpenAi,
            source: ProviderSource::SessionLog,
            tool: "codex".into(),
            version: Some("1.2.3".into()),
            models: vec!["gpt-5".into()],
            calls: 8,
            errors: 1,
            tokens: 190,
            last_day: Some("2026-11-27".into()),
        },
        ProviderEntry {
            id: "gemini".into(),
            name: "Gemini CLI".into(),
            family: Family::Google,
            source: ProviderSource::CliProbe,
            tool: "gemini".into(),
            version: Some("2.3.4".into()),
            models: Vec::new(),
            calls: 0,
            errors: 0,
            tokens: 0,
            last_day: None,
        },
    ]
}

fn open_workbench(
    cx: &mut TestAppContext,
    route: Route,
    settings_path: ProviderSettingsPath,
) -> (AnyWindowHandle, Entity<Workbench>) {
    let result = cx.update(|cx| {
        gpui_kit::init(cx);
        bind_keys(cx);
        cx.set_global(settings_path);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1320.0), px(960.0)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let state = AppState {
                    route,
                    usage: Some(snapshot()),
                    providers: providers(),
                    inspector_open: false,
                    ..AppState::default()
                };
                let settings = Settings {
                    theme: ThemeChoice::Dark,
                    ..Settings::default()
                };
                cx.new(|cx| Workbench::new(state, settings, None, None, None, window, cx))
            },
        )
        .unwrap()
    });
    cx.run_until_parked();
    result
}

#[gpui_kit::test]
fn providers_displays_metrics_and_persists_clicked_changes(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let (handle, wb) = open_workbench(cx, Route::Providers, ProviderSettingsPath(path.clone()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("page-providers").visible());
        let mut kpis = window.within("providers-kpis");
        assert_eq!(kpis.find_all("provider-kpi-value").len(), 3);
        for (id, expected) in [
            ("provider-count", "2"),
            ("provider-active", "1"),
            ("provider-min-quota", "26.0%"),
        ] {
            let metric = kpis.within(id).find("provider-kpi-value");
            assert!(metric.visible());
            assert_eq!(metric.label(), Some(expected));
        }

        assert!(window.find("providers-table").visible());
        let header = window.within("providers-table-header");
        for (index, expected) in [
            "优先级",
            "供应商",
            "家族",
            "来源",
            "CLI 版本",
            "模型",
            "调用",
            "错误率",
            "额度",
            "启用",
        ]
        .into_iter()
        .enumerate()
        {
            let cell = header.find(("provider-column", index));
            assert!(cell.visible());
            assert_eq!(cell.label(), Some(expected));
        }

        let codex = window.within("provider-row-codex");
        for (index, expected) in [
            "#1",
            "Codex",
            "OpenAI",
            "会话日志",
            "1.2.3",
            "gpt-5",
            "8",
            "12.5%",
            "67.5%",
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(
                codex.find(("provider-column", index)).label(),
                Some(expected)
            );
        }
        assert_eq!(codex.find("provider-move-up").disabled(), Some(true));
        let gemini = window.within("provider-row-gemini");
        assert_eq!(
            gemini.find(("provider-column", 3usize)).label(),
            Some("CLI 探测")
        );
        assert_eq!(gemini.find(("provider-column", 8usize)).label(), Some("—"));
        assert_eq!(gemini.find("provider-move-down").disabled(), Some(true));

        window
            .within("provider-row-codex")
            .click("provider-move-down", cx);
        window.render_frame(cx);
        assert_eq!(wb.read(cx).settings.provider_order, ["gemini", "codex"]);
        assert_eq!(
            Settings::load_from(&path).provider_order,
            ["gemini", "codex"]
        );
        assert!(
            window.find("provider-row-gemini").bounds().origin.y
                < window.find("provider-row-codex").bounds().origin.y
        );

        window
            .within("provider-row-codex")
            .click("provider-move-up", cx);
        window.render_frame(cx);
        assert_eq!(wb.read(cx).settings.provider_order, ["codex", "gemini"]);
        assert_eq!(
            Settings::load_from(&path).provider_order,
            ["codex", "gemini"]
        );

        window
            .within("provider-row-codex")
            .click("provider-toggle", cx);
        window.render_frame(cx);
        assert_eq!(wb.read(cx).settings.provider_disabled, ["codex"]);
        assert_eq!(Settings::load_from(&path).provider_disabled, ["codex"]);
        assert_eq!(
            window
                .within("provider-row-codex")
                .find("provider-toggle")
                .label(),
            Some("已停用")
        );

        window
            .within("provider-row-codex")
            .click("provider-toggle", cx);
        window.render_frame(cx);
        assert!(wb.read(cx).settings.provider_disabled.is_empty());
        assert!(Settings::load_from(&path).provider_disabled.is_empty());
        assert_eq!(
            window
                .within("provider-row-codex")
                .find("provider-toggle")
                .label(),
            Some("已启用")
        );

        wb.update(cx, |wb, cx| {
            wb.state.usage = None;
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(
            window
                .within("provider-min-quota")
                .find("provider-kpi-value")
                .label(),
            Some("—")
        );
        assert_eq!(
            window
                .within("provider-row-codex")
                .find(("provider-column", 8usize))
                .label(),
            Some("—")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn providers_reports_settings_write_failure(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (handle, wb) = open_workbench(
        cx,
        Route::Providers,
        ProviderSettingsPath(dir.path().into()),
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .within("provider-row-gemini")
            .click("provider-toggle", cx);
        window.render_frame(cx);
        let app = wb.read(cx);
        assert_eq!(app.settings.provider_disabled, ["gemini"]);
        assert!(
            app.state
                .log
                .back()
                .is_some_and(|entry| entry.message.starts_with("保存供应商设置失败"))
        );
        assert_eq!(
            window
                .within("provider-row-gemini")
                .find("provider-toggle")
                .label(),
            Some("已停用")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn pricing_displays_builtin_prices_and_sorted_model_costs(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let settings_path = ProviderSettingsPath(dir.path().join("settings.json"));
    let (handle, wb) = open_workbench(cx, Route::Pricing, settings_path);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("page-pricing").visible());
        assert!(window.find("builtin-prices-table").visible());
        assert!(window.find("model-costs-table").visible());

        for (header, labels) in [
            (
                "builtin-prices-header",
                &["家族", "匹配规则", "输入", "缓存写入", "缓存读取", "输出"][..],
            ),
            (
                "model-costs-header",
                &["模型", "家族", "调用", "词元（Tokens）", "估算费用"][..],
            ),
        ] {
            let table_header = window.within(header);
            for (index, label) in labels.iter().enumerate() {
                let cell = table_header.find(("price-column", index));
                assert!(cell.visible());
                assert_eq!(cell.label(), Some(*label));
            }
        }

        let prices = builtin_price_table();
        if prices.is_empty() {
            assert!(window.find("builtin-prices-empty").visible());
        }
        for (index, price) in prices.iter().enumerate() {
            let row = window.within(("builtin-price-row", index));
            assert_eq!(
                row.find(("price-column", 1usize)).label(),
                Some(price.pattern.as_str())
            );
        }

        for (index, labels) in [
            ["gpt-5", "OpenAI", "8", "190", "$1.2500"],
            ["claude-sonnet", "Anthropic", "3", "52", "$0.1000"],
        ]
        .into_iter()
        .enumerate()
        {
            let row = window.within(("model-cost-row", index));
            for (column, expected) in labels.into_iter().enumerate() {
                assert_eq!(row.find(("price-column", column)).label(), Some(expected));
            }
        }

        wb.update(cx, |wb, cx| {
            wb.state.usage = None;
            wb.state.config_layers = vec![(
                ConfigLayer::Repo,
                KansoConfig {
                    pricing: Some(BTreeMap::from([(
                        "claude-*".into(),
                        PriceOverride {
                            input: 9.9,
                            cache_write: 3.75,
                            cache_read: 0.3,
                            output: 15.0,
                        },
                    )])),
                    ..KansoConfig::default()
                },
            )];
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find(("model-cost-row", 0usize)).is_none());
        assert!(window.try_find("model-costs-empty").is_some());
        assert_eq!(
            window
                .within(("builtin-price-row", 0usize))
                .find(("price-column", 2usize))
                .label(),
            Some("9.9")
        );
    })
    .unwrap();
}
