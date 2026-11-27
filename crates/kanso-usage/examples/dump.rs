//! 将最近 30 天的用量摘要以 JSON 输出到标准输出。

use anyhow::Context;
use chrono::Local;
use kanso_usage::{Family, collect_snapshot};
use serde::Serialize;
use serde_json::json;
use std::env;
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(Default, Serialize)]
struct FamilySummary {
    calls: u64,
    est_usd: f64,
}

fn main() -> anyhow::Result<()> {
    let home = env::args_os()
        .nth(1)
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .context("请传入主目录或设置 HOME")?;
    let snapshot = collect_snapshot(&home, Local::now(), 30)?;
    let mut anthropic = FamilySummary::default();
    let mut openai = FamilySummary::default();
    for bucket in snapshot.daily.values().flatten() {
        let summary = match bucket.family {
            Family::Anthropic => &mut anthropic,
            Family::OpenAi => &mut openai,
            Family::Google => continue,
        };
        summary.calls += bucket.calls;
        summary.est_usd += bucket.est_usd;
    }

    let output = json!({
        "total_requests": snapshot.total_requests,
        "total_errors": snapshot.total_errors,
        "active_days": snapshot.coverage.active_days,
        "first_day": snapshot.coverage.first_day,
        "last_day": snapshot.coverage.last_day,
        "quota": snapshot.quota,
        "by_family": { "anthropic": anthropic, "openai": openai },
    });
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &output)?;
    writeln!(stdout)?;
    Ok(())
}
