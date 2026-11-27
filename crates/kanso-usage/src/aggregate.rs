use crate::model::{
    Coverage, DailyBucket, HourlySlot, QuotaSnapshot, ReqStatus, UsageRequest, UsageSnapshot,
};
use crate::pricing::{builtin_price_table, est_cost};
use anyhow::Context;
use chrono::{DateTime, Duration, Local, TimeZone, Timelike};
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::path::Path;

/// 汇总全量请求和配额，并保留最近 600 条请求明细。
pub fn build_snapshot(
    mut reqs: Vec<UsageRequest>,
    quota: Vec<QuotaSnapshot>,
    now: DateTime<Local>,
    lookback_days: u32,
) -> UsageSnapshot {
    let total_requests = reqs.len() as u64;
    let mut total_errors = 0;
    let today = now.format("%Y-%m-%d").to_string();
    let today_date = now.date_naive();
    let mut daily: BTreeMap<String, Vec<DailyBucket>> = BTreeMap::new();
    let mut hourly_today: Vec<_> = (0..24)
        .map(|hour| HourlySlot {
            hour,
            calls: 0,
            input_uncached: 0,
            cache_read: 0,
            cache_write: 0,
            output: 0,
        })
        .collect();
    let price_table = builtin_price_table();

    for req in &mut reqs {
        let errors = u64::from(req.status == ReqStatus::Error);
        total_errors += errors;
        if req.est_usd == 0.0 {
            req.est_usd = est_cost(
                req.family.clone(),
                req.input_uncached,
                req.cache_write,
                req.cache_read,
                req.output,
                &price_table,
            );
        }

        let Some(ts) = Local.timestamp_millis_opt(req.ts_ms).single() else {
            continue;
        };
        let buckets = daily.entry(ts.format("%Y-%m-%d").to_string()).or_default();
        let matching_bucket = buckets.iter_mut().find(|bucket| {
            bucket.source_kind == req.source_kind
                && bucket.family == req.family
                && bucket.model == req.model
        });
        if let Some(bucket) = matching_bucket {
            bucket.calls += 1;
            bucket.errors += errors;
            bucket.input_uncached += req.input_uncached;
            bucket.cache_read += req.cache_read;
            bucket.cache_write += req.cache_write;
            bucket.output += req.output;
            bucket.reasoning += req.reasoning;
            bucket.est_usd += req.est_usd;
        } else {
            buckets.push(DailyBucket {
                source_kind: req.source_kind.clone(),
                family: req.family.clone(),
                model: req.model.clone(),
                calls: 1,
                errors,
                input_uncached: req.input_uncached,
                cache_read: req.cache_read,
                cache_write: req.cache_write,
                output: req.output,
                reasoning: req.reasoning,
                est_usd: req.est_usd,
            });
        }

        if ts.date_naive() == today_date {
            let slot = &mut hourly_today[ts.hour() as usize];
            slot.calls += 1;
            slot.input_uncached += req.input_uncached;
            slot.cache_read += req.cache_read;
            slot.cache_write += req.cache_write;
            slot.output += req.output;
        }
    }

    let coverage = Coverage {
        lookback_days,
        first_day: daily
            .first_key_value()
            .map_or_else(|| today.clone(), |(day, _)| day.clone()),
        last_day: daily
            .last_key_value()
            .map_or_else(|| today, |(day, _)| day.clone()),
        active_days: daily
            .values()
            .filter(|buckets| buckets.iter().any(|bucket| bucket.calls > 0))
            .count() as u32,
    };

    reqs.sort_by_key(|req| Reverse(req.ts_ms));
    reqs.truncate(600);
    UsageSnapshot {
        requests: reqs,
        total_requests,
        total_errors,
        daily,
        hourly_today,
        coverage,
        quota,
    }
}

/// 从指定主目录读取回溯窗口内的 Claude Code 和 Codex 用量及配额。
pub fn collect_snapshot(
    home: &Path,
    now: DateTime<Local>,
    lookback_days: u32,
) -> anyhow::Result<UsageSnapshot> {
    let cutoff_ms = now
        .checked_sub_signed(Duration::days(i64::from(lookback_days)))
        .context("回溯起始日期超出可表示范围")?
        .timestamp_millis();
    let mut reqs = crate::claude::parse_claude_logs(&home.join(".claude/projects"), cutoff_ms)?;
    let (codex_reqs, codex_quota) =
        crate::codex::parse_codex_logs(&home.join(".codex/sessions"), cutoff_ms)?;
    reqs.extend(codex_reqs);

    Ok(build_snapshot(
        reqs,
        codex_quota.into_iter().collect(),
        now,
        lookback_days,
    ))
}
