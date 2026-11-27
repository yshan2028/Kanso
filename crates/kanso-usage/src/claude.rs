use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, ErrorKind};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::model::{Family, ReqStatus, SourceKind, UsageRequest};
use crate::pricing::{builtin_price_table, est_cost};

/// 递归读取 Claude Code 会话日志并采集指定时间以来的模型请求。
pub fn parse_claude_logs(root: &Path, cutoff_ms: i64) -> anyhow::Result<Vec<UsageRequest>> {
    let prices = builtin_price_table();
    let mut requests = Vec::new();

    for (path, modified_ms) in session_logs(root, cutoff_ms)? {
        let file_name = path.file_name().unwrap_or_default().to_string_lossy();
        let mut session_id = file_name.to_string();
        let mut project = "-".to_owned();
        let mut seen_ids = HashSet::new();
        let reader = BufReader::new(File::open(&path)?);

        for (index, line) in reader.split(b'\n').enumerate() {
            let line = line?;
            if let Ok(text) = std::str::from_utf8(&line)
                && !text.contains("assistant")
                && !text.contains("\"cwd\"")
                && !text.contains("\"sessionId\"")
            {
                continue;
            }
            let Ok(record) = serde_json::from_slice::<Value>(&line) else {
                continue;
            };
            if let Some(cwd) = nonempty_text(&record["cwd"]) {
                if cwd.contains("scratch/mcp_probe") {
                    break;
                }
                project = project_name(cwd);
            }
            if let Some(id) = nonempty_text(&record["sessionId"]) {
                session_id = id.to_owned();
            }
            if record["type"] != "assistant" {
                continue;
            }
            let message = &record["message"];
            let usage = &message["usage"];
            if !usage.as_object().is_some_and(|fields| !fields.is_empty()) {
                continue;
            }
            let Some(ts_ms) = event_timestamp(&record, modified_ms).filter(|&ts| ts >= cutoff_ms)
            else {
                continue;
            };
            let id = nonempty_text(&message["id"])
                .or_else(|| nonempty_text(&record["requestId"]))
                .or_else(|| nonempty_text(&record["uuid"]));
            if let Some(id) = id
                && !seen_ids.insert(id.to_owned())
            {
                continue;
            }

            let raw_model = nonempty_text(&message["model"]);
            let (status, model) = if record["isApiErrorMessage"].as_bool() == Some(true)
                || raw_model == Some("<synthetic>")
            {
                (ReqStatus::Error, None)
            } else {
                (ReqStatus::Success, raw_model.map(str::to_owned))
            };
            let input_uncached = usage["input_tokens"].as_u64().unwrap_or_default();
            let cache_write = usage["cache_creation_input_tokens"]
                .as_u64()
                .unwrap_or_default();
            let cache_read = usage["cache_read_input_tokens"]
                .as_u64()
                .unwrap_or_default();
            let output = usage["output_tokens"].as_u64().unwrap_or_default();
            let reasoning = usage["output_tokens_details"]["thinking_tokens"]
                .as_u64()
                .unwrap_or_default();
            requests.push(UsageRequest {
                id: id.map_or_else(|| format!("{file_name}:{}", index + 1), str::to_owned),
                ts_ms,
                source_kind: SourceKind::ClaudeCode,
                family: Family::Anthropic,
                model,
                session_id: session_id.clone(),
                project: project.clone(),
                input_uncached,
                cache_read,
                cache_write,
                output,
                reasoning,
                est_usd: est_cost(
                    Family::Anthropic,
                    input_uncached,
                    cache_write,
                    cache_read,
                    output,
                    &prices,
                ),
                status,
            });
        }
    }
    Ok(requests)
}

/// 查找修改时间达到截断点的 JSONL 日志，跳过探针路径和符号链接条目。
pub(super) fn session_logs(root: &Path, cutoff_ms: i64) -> anyhow::Result<Vec<(PathBuf, i64)>> {
    let mut directories = vec![root.to_path_buf()];
    let mut logs = Vec::new();
    while let Some(directory) = directories.pop() {
        if directory.to_string_lossy().contains("mcp_probe") {
            continue;
        }
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(err) if err.kind() == ErrorKind::NotFound => continue,
            Err(err) => return Err(err.into()),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if entry.file_name().to_string_lossy().contains("mcp_probe") {
                continue;
            }
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                directories.push(path);
            } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "jsonl") {
                let modified_ms =
                    DateTime::<Utc>::from(entry.metadata()?.modified()?).timestamp_millis();
                if modified_ms >= cutoff_ms {
                    logs.push((path, modified_ms));
                }
            }
        }
    }
    Ok(logs)
}

/// 将日志时间转成 Unix 毫秒，时间缺省时采用文件修改时间。
pub(super) fn event_timestamp(record: &Value, modified_ms: i64) -> Option<i64> {
    match &record["timestamp"] {
        Value::Null => Some(modified_ms),
        timestamp => Some(
            DateTime::parse_from_rfc3339(timestamp.as_str()?)
                .ok()?
                .timestamp_millis(),
        ),
    }
}

/// 读取非空字符串字段。
pub(super) fn nonempty_text(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| !text.is_empty())
}

/// 取工作目录的最后一级名称，缺省时返回短横线。
pub(super) fn project_name(cwd: &str) -> String {
    Path::new(cwd).file_name().map_or_else(
        || "-".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}
