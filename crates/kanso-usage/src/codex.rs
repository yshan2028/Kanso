use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde_json::Value;

use crate::claude::{event_timestamp, nonempty_text, project_name, session_logs};
use crate::model::{Family, QuotaSnapshot, QuotaWindow, ReqStatus, SourceKind, UsageRequest};
use crate::pricing::{builtin_price_table, est_cost};

/// 递归读取 Codex 会话日志，采集指定时间以来的模型请求和最新配额。
pub fn parse_codex_logs(
    root: &Path,
    cutoff_ms: i64,
) -> anyhow::Result<(Vec<UsageRequest>, Option<QuotaSnapshot>)> {
    let prices = builtin_price_table();
    let mut requests = Vec::new();
    let mut latest_quota: Option<QuotaSnapshot> = None;

    for (path, modified_ms) in session_logs(root, cutoff_ms)? {
        let file_name = path.file_name().unwrap_or_default().to_string_lossy();
        let mut session_id = file_name.to_string();
        let mut project = "-".to_owned();
        let mut source_kind = SourceKind::CodexCli;
        let mut model = None;
        let mut seen_totals = HashSet::new();
        let reader = BufReader::new(File::open(&path)?);

        for (index, line) in reader.split(b'\n').enumerate() {
            let Ok(record) = serde_json::from_slice::<Value>(&line?) else {
                continue;
            };
            let payload = &record["payload"];
            match record["type"].as_str() {
                Some("session_meta") => {
                    let cwd = payload["cwd"].as_str().unwrap_or_default();
                    if cwd.contains("scratch/mcp_probe") {
                        break;
                    }
                    session_id = nonempty_text(&payload["id"])
                        .unwrap_or(&file_name)
                        .to_owned();
                    project = project_name(cwd);
                    source_kind = match payload["originator"].as_str() {
                        Some("Codex Desktop") => SourceKind::CodexDesktop,
                        Some("codex_exec") => SourceKind::CodexExec,
                        _ => SourceKind::CodexCli,
                    };
                    continue;
                }
                Some("turn_context") => {
                    model = nonempty_text(&payload["model"])
                        .or_else(|| {
                            nonempty_text(&payload["collaboration_mode"]["settings"]["model"])
                        })
                        .map(str::to_owned);
                    continue;
                }
                Some("event_msg") if payload["type"] == "token_count" => {}
                _ => continue,
            }
            let Some(ts_ms) = event_timestamp(&record, modified_ms).filter(|&ts| ts >= cutoff_ms)
            else {
                continue;
            };
            if latest_quota
                .as_ref()
                .is_none_or(|quota| ts_ms >= quota.captured_ts_ms)
                && let Some(quota) = parse_quota(&payload["rate_limits"], ts_ms)
            {
                latest_quota = Some(quota);
            }

            let info = &payload["info"];
            let usage = &info["last_token_usage"];
            if !usage.as_object().is_some_and(|fields| !fields.is_empty()) {
                continue;
            }
            let total = &info["total_token_usage"];
            let signature = (
                total["total_tokens"].as_u64(),
                total["output_tokens"].as_u64(),
            );
            if signature != (None, None) && !seen_totals.insert(signature) {
                continue;
            }

            let raw_in = usage["input_tokens"].as_u64().unwrap_or_default();
            let cache_read = usage["cached_input_tokens"].as_u64().unwrap_or_default();
            let input_uncached = raw_in.saturating_sub(cache_read);
            let cache_write = usage["cache_write_input_tokens"]
                .as_u64()
                .unwrap_or_default();
            let output = usage["output_tokens"].as_u64().unwrap_or_default();
            let reasoning = usage["reasoning_output_tokens"]
                .as_u64()
                .unwrap_or_default();
            requests.push(UsageRequest {
                id: format!("{session_id}:r{}", index + 1),
                ts_ms,
                source_kind: source_kind.clone(),
                family: Family::OpenAi,
                model: model.clone(),
                session_id: session_id.clone(),
                project: project.clone(),
                input_uncached,
                cache_read,
                cache_write,
                output,
                reasoning,
                est_usd: est_cost(
                    Family::OpenAi,
                    input_uncached,
                    cache_write,
                    cache_read,
                    output,
                    &prices,
                ),
                status: ReqStatus::Success,
            });
        }
    }
    Ok((requests, latest_quota))
}

fn parse_quota(rate_limits: &Value, ts_ms: i64) -> Option<QuotaSnapshot> {
    let primary = parse_window(&rate_limits["primary"]);
    let secondary = parse_window(&rate_limits["secondary"]);
    if primary.is_none() && secondary.is_none() {
        return None;
    }
    Some(QuotaSnapshot {
        family: Family::OpenAi,
        limit_id: nonempty_text(&rate_limits["limit_id"])
            .unwrap_or("codex")
            .to_owned(),
        plan_type: nonempty_text(&rate_limits["plan_type"]).map(str::to_owned),
        primary,
        secondary,
        captured_ts_ms: ts_ms,
    })
}

fn parse_window(window: &Value) -> Option<QuotaWindow> {
    let fields = window.as_object()?;
    if fields.is_empty() {
        return None;
    }
    Some(QuotaWindow {
        used_percent: window["used_percent"].as_f64().unwrap_or_default(),
        window_minutes: window["window_minutes"]
            .as_u64()
            .and_then(|minutes| u32::try_from(minutes).ok())
            .unwrap_or_default(),
        resets_at: window["resets_at"].as_i64(),
    })
}
