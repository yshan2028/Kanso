/// 解析 Codex 会话日志和配额，当前返回空的占位结果。
pub fn parse_codex_logs(
    _root: &std::path::Path,
    _cutoff_ms: i64,
) -> anyhow::Result<(
    Vec<crate::model::UsageRequest>,
    Option<crate::model::QuotaSnapshot>,
)> {
    Ok((Vec::new(), None))
}
