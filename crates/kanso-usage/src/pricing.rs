/// 提供内置模型价格表，当前返回空的占位价格表。
pub fn builtin_price_table() -> Vec<crate::model::PriceEntry> {
    Vec::new()
}

/// 估算请求费用，当前返回占位金额。
pub fn est_cost(
    _family: crate::model::Family,
    _in_u: u64,
    _cw: u64,
    _cr: u64,
    _out: u64,
    _table: &[crate::model::PriceEntry],
) -> f64 {
    0.0
}
