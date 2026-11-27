//! 供应商配置和模型费用汇总的纯逻辑。

use std::collections::{BTreeMap, HashMap};

use kanso_usage::{Family, ProviderEntry, UsageSnapshot};

/// 按已保存的优先级排列供应商，未指定优先级的供应商保持原顺序。
pub fn ordered_providers<'a>(
    providers: &'a [ProviderEntry],
    order: &[String],
) -> Vec<&'a ProviderEntry> {
    let mut positions = HashMap::new();
    for (index, id) in order.iter().enumerate() {
        positions.entry(id.as_str()).or_insert(index);
    }

    let mut ordered: Vec<_> = providers.iter().collect();
    ordered.sort_by_key(|provider| {
        positions
            .get(provider.id.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });
    ordered
}

/// 在当前供应商顺序中移动指定项，成功时将新顺序写入设置列表。
pub fn move_provider(
    providers: &[ProviderEntry],
    order: &mut Vec<String>,
    id: &str,
    delta: isize,
) -> bool {
    let mut ordered = ordered_providers(providers, order);
    let Some(current) = ordered.iter().position(|provider| provider.id == id) else {
        return false;
    };
    let Some(target) = current.checked_add_signed(delta) else {
        return false;
    };
    if target == current || target >= ordered.len() {
        return false;
    }

    let provider = ordered.remove(current);
    ordered.insert(target, provider);
    *order = ordered
        .into_iter()
        .map(|provider| provider.id.clone())
        .collect();
    true
}

/// 切换供应商的启用状态，并返回切换后是否已启用。
pub fn toggle_provider(disabled: &mut Vec<String>, id: &str) -> bool {
    let enabled = is_provider_enabled(disabled, id);
    if enabled {
        disabled.push(id.to_owned());
    } else {
        disabled.retain(|disabled_id| disabled_id != id);
    }
    !enabled
}

/// 判断指定供应商是否已启用。
pub fn is_provider_enabled(disabled: &[String], id: &str) -> bool {
    !disabled.iter().any(|disabled_id| disabled_id == id)
}

/// 单个模型在采集范围内的费用和用量汇总。
#[derive(Debug, Clone, PartialEq)]
pub struct ModelCostRow {
    /// 模型名称。
    pub model: String,
    /// 模型所属家族。
    pub family: Family,
    /// 调用次数。
    pub calls: u64,
    /// 输入、缓存读写和输出的词元总数。
    pub tokens: u64,
    /// 估算费用，单位为美元。
    pub est_usd: f64,
}

/// 跨日期和会话来源按家族、模型汇总每日用量，按估算费用降序排列。
pub fn aggregate_model_costs(snapshot: &UsageSnapshot) -> Vec<ModelCostRow> {
    let mut by_model = BTreeMap::new();
    for bucket in snapshot.daily.values().flatten() {
        let model = bucket.model.as_deref();
        let row = by_model
            .entry((model, family_label(&bucket.family)))
            .or_insert_with(|| ModelCostRow {
                model: model.unwrap_or("未知模型").to_owned(),
                family: bucket.family.clone(),
                calls: 0,
                tokens: 0,
                est_usd: 0.0,
            });
        row.calls = row.calls.saturating_add(bucket.calls);
        // 推理词元已计入输出。
        row.tokens = row
            .tokens
            .saturating_add(bucket.input_uncached)
            .saturating_add(bucket.cache_write)
            .saturating_add(bucket.cache_read)
            .saturating_add(bucket.output);
        row.est_usd += bucket.est_usd;
    }

    let mut rows: Vec<_> = by_model.into_values().collect();
    rows.sort_by(|left, right| right.est_usd.total_cmp(&left.est_usd));
    rows
}

/// 返回两页共用的模型家族名称。
pub(crate) fn family_label(family: &Family) -> &'static str {
    match family {
        Family::Anthropic => "Anthropic",
        Family::OpenAi => "OpenAI",
        Family::Google => "Google",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kanso_usage::{Coverage, DailyBucket, ProviderSource, SourceKind};

    fn providers() -> Vec<ProviderEntry> {
        ["a", "b", "c", "d"]
            .into_iter()
            .map(|id| ProviderEntry {
                id: id.into(),
                name: id.into(),
                family: Family::OpenAi,
                source: ProviderSource::SessionLog,
                tool: "codex".into(),
                version: None,
                models: Vec::new(),
                calls: 0,
                errors: 0,
                tokens: 0,
                last_day: None,
            })
            .collect()
    }

    #[test]
    fn saved_order_uses_first_position_and_keeps_unlisted_providers_in_input_order() {
        let providers = providers();
        let order = ["c", "removed", "b", "c"].map(String::from);
        let ordered = ordered_providers(&providers, &order);
        let ids: Vec<_> = ordered
            .iter()
            .map(|provider| provider.id.as_str())
            .collect();
        assert_eq!(ids, ["c", "b", "a", "d"]);
        assert!(ordered_providers(&[], &order).is_empty());
    }

    #[test]
    fn moving_materializes_the_displayed_order_in_settings() {
        let providers = providers();
        let mut order = vec!["c".into()];
        assert!(move_provider(&providers, &mut order, "c", 1));
        assert_eq!(order, ["a", "c", "b", "d"]);
        assert!(move_provider(&providers, &mut order, "d", -2));
        assert_eq!(order, ["a", "d", "c", "b"]);
        assert!(move_provider(&providers, &mut order, "d", -1));
        assert_eq!(order, ["d", "a", "c", "b"]);
    }

    #[test]
    fn invalid_moves_preserve_the_settings_list() {
        let providers = providers();
        let initial = vec!["b".into(), "removed".into()];
        let mut order = initial.clone();
        for (id, delta) in [
            ("b", -1),
            ("d", 1),
            ("a", 0),
            ("a", isize::MIN),
            ("a", isize::MAX),
            ("missing", 1),
        ] {
            assert!(!move_provider(&providers, &mut order, id, delta));
            assert_eq!(order, initial);
        }
        assert!(!move_provider(&[], &mut order, "b", 1));
        assert_eq!(order, initial);
    }

    #[test]
    fn toggle_updates_only_the_requested_provider() {
        let mut disabled = vec!["b".into()];
        assert!(is_provider_enabled(&disabled, "a"));
        assert!(!toggle_provider(&mut disabled, "a"));
        assert_eq!(disabled, ["b", "a"]);
        assert!(!is_provider_enabled(&disabled, "a"));

        disabled.push("a".into());
        assert!(toggle_provider(&mut disabled, "a"));
        assert_eq!(disabled, ["b"]);
        assert!(is_provider_enabled(&disabled, "a"));
    }

    fn bucket(family: Family, model: Option<&str>, calls: u64, est_usd: f64) -> DailyBucket {
        DailyBucket {
            source_kind: SourceKind::CodexCli,
            family,
            model: model.map(String::from),
            calls,
            errors: 0,
            input_uncached: 10,
            cache_read: 30,
            cache_write: 20,
            output: 40,
            reasoning: 15,
            est_usd,
        }
    }

    #[test]
    fn costs_sum_days_and_sources_and_keep_model_families_distinct() {
        let mut snapshot = UsageSnapshot {
            requests: Vec::new(),
            total_requests: 0,
            total_errors: 0,
            daily: BTreeMap::new(),
            hourly_today: Vec::new(),
            coverage: Coverage {
                lookback_days: 30,
                first_day: String::new(),
                last_day: String::new(),
                active_days: 0,
            },
            quota: Vec::new(),
        };
        assert!(aggregate_model_costs(&snapshot).is_empty());

        let mut desktop_bucket = bucket(Family::OpenAi, Some("gpt-5"), 3, 0.75);
        desktop_bucket.source_kind = SourceKind::CodexDesktop;
        snapshot.daily = BTreeMap::from([
            (
                "2026-11-26".into(),
                vec![
                    bucket(Family::OpenAi, Some("gpt-5"), 2, 0.5),
                    bucket(Family::Anthropic, Some("gpt-5"), 1, 0.125),
                    bucket(Family::Google, None, 4, 0.0),
                ],
            ),
            ("2026-11-27".into(), vec![desktop_bucket]),
        ]);

        assert_eq!(
            aggregate_model_costs(&snapshot),
            [
                ModelCostRow {
                    model: "gpt-5".into(),
                    family: Family::OpenAi,
                    calls: 5,
                    tokens: 200,
                    est_usd: 1.25,
                },
                ModelCostRow {
                    model: "gpt-5".into(),
                    family: Family::Anthropic,
                    calls: 1,
                    tokens: 100,
                    est_usd: 0.125,
                },
                ModelCostRow {
                    model: "未知模型".into(),
                    family: Family::Google,
                    calls: 4,
                    tokens: 100,
                    est_usd: 0.0,
                },
            ]
        );
    }
}
