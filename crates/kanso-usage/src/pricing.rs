use crate::model::{Family, PriceEntry};

/// 返回内置标准估算单价，单位为美元／百万词元。
pub fn builtin_price_table() -> Vec<PriceEntry> {
    vec![
        PriceEntry {
            family: Family::Anthropic,
            pattern: "claude-*".into(),
            input: 3.0,
            cache_write: 3.75,
            cache_read: 0.30,
            output: 15.0,
        },
        PriceEntry {
            family: Family::OpenAi,
            pattern: "gpt-*".into(),
            input: 2.50,
            cache_write: 0.0,
            cache_read: 0.25,
            output: 10.0,
        },
    ]
}

/// 按模型家族及各类词元数估算美元费用。家族无价目时返回零。
pub fn est_cost(
    family: Family,
    in_u: u64,
    cw: u64,
    cr: u64,
    out: u64,
    table: &[PriceEntry],
) -> f64 {
    table
        .iter()
        .find(|entry| entry.family == family)
        .map_or(0.0, |entry| {
            (in_u as f64 * entry.input
                + cw as f64 * entry.cache_write
                + cr as f64 * entry.cache_read
                + out as f64 * entry.output)
                / 1_000_000.0
        })
}
