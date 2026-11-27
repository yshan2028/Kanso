use crate::model::{Coverage, QuotaSnapshot, UsageRequest, UsageSnapshot};
use chrono::{DateTime, Local};
use std::collections::BTreeMap;

/// 汇总请求和配额，当前返回空的占位快照。
pub fn build_snapshot(
    _reqs: Vec<UsageRequest>,
    _quota: Vec<QuotaSnapshot>,
    _now: DateTime<Local>,
    lookback_days: u32,
) -> UsageSnapshot {
    UsageSnapshot {
        requests: Vec::new(),
        total_requests: 0,
        total_errors: 0,
        daily: BTreeMap::new(),
        hourly_today: Vec::new(),
        coverage: Coverage {
            lookback_days,
            first_day: String::new(),
            last_day: String::new(),
            active_days: 0,
        },
        quota: Vec::new(),
    }
}

/// 从指定主目录采集用量，当前返回空的占位快照。
pub fn collect_snapshot(
    _home: &std::path::Path,
    now: DateTime<Local>,
    lookback_days: u32,
) -> anyhow::Result<UsageSnapshot> {
    Ok(build_snapshot(Vec::new(), Vec::new(), now, lookback_days))
}
