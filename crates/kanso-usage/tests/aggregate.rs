use chrono::{DateTime, Local, TimeZone};
use kanso_usage::aggregate::build_snapshot;
use kanso_usage::pricing::{builtin_price_table, est_cost};
use kanso_usage::{Family, QuotaSnapshot, ReqStatus, SourceKind, UsageRequest};

fn local_time(day: u32, hour: u32) -> DateTime<Local> {
    Local
        .with_ymd_and_hms(2026, 1, day, hour, 0, 0)
        .single()
        .unwrap()
}

fn request(ts_ms: i64) -> UsageRequest {
    UsageRequest {
        id: ts_ms.to_string(),
        ts_ms,
        source_kind: SourceKind::ClaudeCode,
        family: Family::Anthropic,
        model: Some("claude-test".into()),
        session_id: "test-session".into(),
        project: "test-project".into(),
        input_uncached: 1_000,
        cache_read: 2_000,
        cache_write: 3_000,
        output: 4_000,
        reasoning: 5_000,
        est_usd: 0.0,
        status: ReqStatus::Success,
    }
}

fn assert_cost(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-10, "{actual} != {expected}");
}

#[test]
fn estimates_cost_by_family() {
    let table = builtin_price_table();
    assert_eq!(table.len(), 2);
    assert_eq!(table[0].pattern, "claude-*");
    assert_eq!(table[1].pattern, "gpt-*");
    let cache_write = 3_000_000;
    let cache_read = 2_000_000;
    let cost = |family| {
        est_cost(
            family,
            1_000_000,
            cache_write,
            cache_read,
            4_000_000,
            &table,
        )
    };
    assert_cost(cost(Family::Anthropic), 74.85);
    assert_cost(cost(Family::OpenAi), 43.0);
    assert_eq!(cost(Family::Google), 0.0);
}

#[test]
fn truncates_details_after_counting_all_requests() {
    let now = local_time(12, 12);
    let first_ts = now.timestamp_millis();
    let mut requests: Vec<_> = (0..601).map(|offset| request(first_ts + offset)).collect();
    requests[0].status = ReqStatus::Error;

    let snapshot = build_snapshot(requests, vec![], now, 30);

    assert_eq!(snapshot.total_requests, 601);
    assert_eq!(snapshot.total_errors, 1);
    assert_eq!(snapshot.requests.len(), 600);
    for (req, offset) in snapshot.requests.iter().zip((1..=600).rev()) {
        assert_eq!(req.ts_ms, first_ts + offset);
        assert_eq!(req.status, ReqStatus::Success);
    }
    let bucket = &snapshot.daily["2026-01-12"][0];
    assert_eq!(bucket.calls, 601);
    assert_eq!(bucket.errors, 1);
    assert_cost(bucket.est_usd, 601.0 * 0.07485);
    assert_eq!(snapshot.hourly_today[12].calls, 601);
}

#[test]
fn groups_local_dates_sources_families_models_and_hours() {
    let now = local_time(12, 18);
    let today = request(local_time(12, 8).timestamp_millis());
    let mut priced = request(local_time(12, 8).timestamp_millis() + 1);
    priced.est_usd = 0.5;
    priced.status = ReqStatus::Error;
    let mut other_source = today.clone();
    other_source.source_kind = SourceKind::CodexCli;
    let mut other_family = today.clone();
    other_family.family = Family::OpenAi;
    let mut other_model = request(local_time(12, 9).timestamp_millis());
    other_model.model = None;
    let prior = request(local_time(10, 8).timestamp_millis());
    let quota = vec![QuotaSnapshot {
        family: Family::OpenAi,
        limit_id: "test-limit".into(),
        plan_type: None,
        primary: None,
        secondary: None,
        captured_ts_ms: now.timestamp_millis(),
    }];

    let snapshot = build_snapshot(
        vec![
            today,
            priced,
            other_source,
            other_family,
            other_model,
            prior,
        ],
        quota.clone(),
        now,
        30,
    );

    assert_eq!(snapshot.quota, quota);
    assert_eq!(snapshot.coverage.lookback_days, 30);
    assert_eq!(snapshot.coverage.first_day, "2026-01-10");
    assert_eq!(snapshot.coverage.last_day, "2026-01-12");
    assert_eq!(snapshot.coverage.active_days, 2);
    assert_eq!(snapshot.daily.len(), 2);
    assert_eq!(snapshot.daily["2026-01-10"][0].calls, 1);
    let today_buckets = &snapshot.daily["2026-01-12"];
    assert_eq!(today_buckets.len(), 4);
    let combined = today_buckets
        .iter()
        .find(|bucket| {
            bucket.source_kind == SourceKind::ClaudeCode
                && bucket.family == Family::Anthropic
                && bucket.model.as_deref() == Some("claude-test")
        })
        .unwrap();
    assert_eq!(combined.calls, 2);
    assert_eq!(combined.errors, 1);
    assert_eq!(combined.input_uncached, 2_000);
    assert_eq!(combined.cache_read, 4_000);
    assert_eq!(combined.cache_write, 6_000);
    assert_eq!(combined.output, 8_000);
    assert_eq!(combined.reasoning, 10_000);
    assert_cost(combined.est_usd, 0.57485);
    let openai = today_buckets
        .iter()
        .find(|bucket| bucket.family == Family::OpenAi)
        .unwrap();
    assert_eq!(openai.calls, 1);
    assert_cost(openai.est_usd, 0.043);
    let priced = snapshot
        .requests
        .iter()
        .find(|req| req.status == ReqStatus::Error)
        .unwrap();
    assert_cost(priced.est_usd, 0.5);
    assert_cost(snapshot.requests[0].est_usd, 0.07485);

    assert_eq!(snapshot.hourly_today.len(), 24);
    for (slot, hour) in snapshot.hourly_today.iter().zip(0..24) {
        assert_eq!(slot.hour, hour);
        let calls = match hour {
            8 => 4,
            9 => 1,
            _ => 0,
        };
        assert_eq!(slot.calls, calls);
        assert_eq!(slot.input_uncached, calls * 1_000);
        assert_eq!(slot.cache_read, calls * 2_000);
        assert_eq!(slot.cache_write, calls * 3_000);
        assert_eq!(slot.output, calls * 4_000);
    }
}

#[test]
fn empty_snapshot_uses_today_for_coverage() {
    let snapshot = build_snapshot(vec![], vec![], local_time(12, 12), 30);

    assert!(snapshot.daily.is_empty());
    assert_eq!(snapshot.coverage.first_day, "2026-01-12");
    assert_eq!(snapshot.coverage.last_day, "2026-01-12");
    assert_eq!(snapshot.coverage.active_days, 0);
    assert_eq!(snapshot.hourly_today.len(), 24);
    assert!(snapshot.hourly_today.iter().all(|slot| slot.calls == 0));
}
