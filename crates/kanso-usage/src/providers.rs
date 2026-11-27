/// 从用量快照和工具探测生成供应商摘要，当前返回空的占位结果。
pub fn derive_providers(
    _snapshot: &crate::model::UsageSnapshot,
    _tools: &[crate::model::ToolVersion],
) -> Vec<crate::model::ProviderEntry> {
    Vec::new()
}
