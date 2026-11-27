use chrono::{Days, Local, Timelike};
use gpui_kit::base::test_support::texts;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, Point, TestAppContext, Window, WindowBounds,
    WindowOptions, px, size,
};
use kanso_desktop::{AppState, Route, Settings, ThemeChoice, UsageRange, UsageTab, Workbench};
use kanso_usage::{
    Coverage, DailyBucket, Family, HourlySlot, QuotaSnapshot, QuotaWindow, ReqStatus, SourceKind,
    UsageRequest, UsageSnapshot,
};

fn synthetic_snapshot() -> UsageSnapshot {
    let now = Local::now();
    let today = now.date_naive();
    let earlier = (today - Days::new(29)).format("%Y-%m-%d").to_string();
    let bucket = DailyBucket {
        source_kind: SourceKind::CodexCli,
        family: Family::OpenAi,
        model: Some("gpt-ui-usage".to_string()),
        calls: 51,
        errors: 1,
        input_uncached: 5100,
        cache_read: 2550,
        cache_write: 510,
        output: 1020,
        reasoning: 0,
        est_usd: 0.51,
    };
    let mut requests: Vec<_> = (0..51)
        .map(|index| UsageRequest {
            id: format!("ui-usage-{index}"),
            ts_ms: now.timestamp_millis(),
            source_kind: bucket.source_kind.clone(),
            family: bucket.family.clone(),
            model: bucket.model.clone(),
            session_id: "ui-usage".into(),
            project: "测试项目".into(),
            input_uncached: 100,
            cache_read: 50,
            cache_write: 10,
            output: 20,
            reasoning: 0,
            est_usd: 0.01,
            status: if index == 50 {
                ReqStatus::Error
            } else {
                ReqStatus::Success
            },
        })
        .collect();
    let first_day = (today - Days::new(29))
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .single()
        .unwrap();
    let mut older = requests[0].clone();
    older.id = "ui-usage-older".into();
    older.ts_ms = first_day.timestamp_millis();
    requests.push(older);

    UsageSnapshot {
        requests,
        total_requests: 52,
        total_errors: 1,
        daily: [
            (
                earlier.clone(),
                vec![DailyBucket {
                    calls: 1,
                    errors: 0,
                    input_uncached: 100,
                    cache_read: 50,
                    cache_write: 10,
                    output: 20,
                    est_usd: 0.01,
                    ..bucket.clone()
                }],
            ),
            (today.format("%Y-%m-%d").to_string(), vec![bucket.clone()]),
        ]
        .into(),
        hourly_today: vec![HourlySlot {
            hour: now.hour() as u8,
            calls: bucket.calls,
            input_uncached: bucket.input_uncached,
            cache_read: bucket.cache_read,
            cache_write: bucket.cache_write,
            output: bucket.output,
        }],
        coverage: Coverage {
            lookback_days: 30,
            first_day: earlier,
            last_day: today.format("%Y-%m-%d").to_string(),
            active_days: 2,
        },
        quota: vec![QuotaSnapshot {
            family: Family::OpenAi,
            limit_id: "codex".into(),
            plan_type: None,
            primary: Some(QuotaWindow {
                used_percent: 10.0,
                window_minutes: 300,
                resets_at: None,
            }),
            secondary: Some(QuotaWindow {
                used_percent: 42.0,
                window_minutes: 10_080,
                resets_at: Some(now.timestamp() + 86_400),
            }),
            captured_ts_ms: now.timestamp_millis(),
        }],
    }
}

fn open_test_workbench(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workbench>) {
    let result = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1260.0), px(1040.0)),
                })),
                ..WindowOptions::default()
            },
            cx,
            |window, cx| {
                let state = AppState {
                    route: Route::Usage,
                    inspector_open: false,
                    usage: Some(synthetic_snapshot()),
                    ..AppState::default()
                };
                let settings = Settings {
                    theme: ThemeChoice::Dark,
                    ..Settings::default()
                };
                cx.new(|cx| Workbench::new(state, settings, None, None, None, window, cx))
            },
        )
        .expect("打开无头用量窗口")
    });
    cx.run_until_parked();
    result
}

fn shows_text(window: &Window, expected: &str) -> bool {
    texts(window)
        .iter()
        .any(|text| text.visible() && text.text().contains(expected))
}

#[gpui_kit::test]
fn usage_displays_calendar_quota_and_request_interactions(cx: &mut TestAppContext) {
    let (handle, workbench) = open_test_workbench(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("page-usage").visible());
        assert!(shows_text(window, "用量"));
        assert!(shows_text(window, "会话日志不含耗时"));

        let bars = window.find_all("usage-chart-bar");
        assert_eq!(bars.len(), 30);
        assert!(bars.iter().all(|bar| bar.visible()));
        assert!((bars[0].bounds().size.width - bars[29].bounds().size.width).abs() <= px(1.0));
        let baselines = window.find_all("usage-chart-baseline");
        assert_eq!(baselines.len(), 28);
        assert!(
            baselines
                .iter()
                .all(|bar| bar.bounds().size.height == px(2.0))
        );

        assert!(window.find("usage-quota-codex").visible());
        assert!(window.find("usage-quota-weekly").visible());
        assert!(window.find("usage-quota-claude").visible());
        assert!(shows_text(window, "已用 42%"));
        assert!(shows_text(window, "Codex 会话日志 token_count"));

        window
            .within(("usage-chart-column", 1_usize))
            .hover("usage-chart-bar", cx);
        window.render_frame(cx);
        assert_eq!(workbench.read(cx).state.usage_hover_idx, Some(1));
        assert!(shows_text(window, "没有请求"));

        window.click("usage-range-today", cx);
        window.render_frame(cx);
        assert_eq!(workbench.read(cx).state.usage_range, UsageRange::Today);
        assert_eq!(window.find_all("usage-chart-bar").len(), 24);
        assert!(shows_text(window, "每小时词元"));

        window.click("usage-tab-requests", cx);
        window.render_frame(cx);
        assert_eq!(workbench.read(cx).state.usage_tab, UsageTab::Requests);
        assert!(window.find("usage-requests-table").visible());
        assert!(window.try_find("usage-chart").is_none());
        assert!(shows_text(window, "51 条 · 50 条/页"));
        assert!(shows_text(window, "gpt-ui-usage"));
        assert_eq!(window.find_all("usage-request-row").len(), 50);

        window.click("usage-requests-next", cx);
        window.render_frame(cx);
        assert_eq!(workbench.read(cx).state.usage_req_page, 1);
        assert_eq!(window.find_all("usage-request-row").len(), 1);

        window.click("usage-requests-prev", cx);
        window.render_frame(cx);
        assert_eq!(workbench.read(cx).state.usage_req_page, 0);
        assert_eq!(window.find_all("usage-request-row").len(), 50);

        window.click("usage-requests-next", cx);
        window.click("usage-filter-error", cx);
        window.render_frame(cx);
        assert_eq!(workbench.read(cx).state.usage_req_page, 0);
        assert_eq!(
            workbench.read(cx).state.usage_req_filter,
            Some(ReqStatus::Error)
        );
        assert!(shows_text(window, "1 条 · 50 条/页"));
        assert_eq!(window.find_all("usage-request-row").len(), 1);
    })
    .unwrap();
}
