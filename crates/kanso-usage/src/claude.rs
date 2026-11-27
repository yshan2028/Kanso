/// 解析 Claude Code 会话日志，当前返回空的占位结果。
pub fn parse_claude_logs(
    _root: &std::path::Path,
    _cutoff_ms: i64,
) -> anyhow::Result<Vec<crate::model::UsageRequest>> {
    Ok(Vec::new())
}
