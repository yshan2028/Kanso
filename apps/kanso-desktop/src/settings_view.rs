//! 设置与路由页面所用的纯逻辑。

use crate::Settings;
use kanso_config::{ConfigLayer, EffectiveEntry, KansoConfig};
use kanso_core::{Candidate, RolePolicy};
use kanso_usage::{Family, ProviderEntry, UsageSnapshot};

/// 策略选择器按此顺序展示全部路由策略。
pub const ALL_POLICIES: [RolePolicy; 6] = [
    RolePolicy::Sequential,
    RolePolicy::RoundRobin,
    RolePolicy::Weighted,
    RolePolicy::QuotaFirst,
    RolePolicy::SpeedFirst,
    RolePolicy::Sticky,
];

/// 读取路由策略键，未识别的值按默认的额度优先策略解释。
pub fn parse_policy(s: &str) -> RolePolicy {
    match s {
        "Sequential" => RolePolicy::Sequential,
        "RoundRobin" => RolePolicy::RoundRobin,
        "Weighted" => RolePolicy::Weighted,
        "SpeedFirst" => RolePolicy::SpeedFirst,
        "Sticky" => RolePolicy::Sticky,
        _ => RolePolicy::QuotaFirst,
    }
}

/// 返回写入设置的路由策略键。
pub fn policy_key(policy: RolePolicy) -> &'static str {
    match policy {
        RolePolicy::Sequential => "Sequential",
        RolePolicy::RoundRobin => "RoundRobin",
        RolePolicy::Weighted => "Weighted",
        RolePolicy::QuotaFirst => "QuotaFirst",
        RolePolicy::SpeedFirst => "SpeedFirst",
        RolePolicy::Sticky => "Sticky",
    }
}

/// 返回模型家族的显示名称及路由预览所用的家族标识。
pub(crate) fn family_name(family: &Family) -> &'static str {
    match family {
        Family::Anthropic => "Anthropic",
        Family::OpenAi => "OpenAI",
        Family::Google => "Google",
    }
}

/// 将桌面供应商转换为路由候选，未指定优先级的供应商按发现顺序排在后面。
pub fn build_candidates(
    providers: &[ProviderEntry],
    settings: &Settings,
    usage: Option<&UsageSnapshot>,
) -> Vec<Candidate> {
    providers
        .iter()
        .enumerate()
        .map(|(index, provider)| {
            let priority = settings
                .provider_order
                .iter()
                .position(|id| id == &provider.id)
                .unwrap_or(settings.provider_order.len() + index);
            Candidate {
                id: provider.id.clone(),
                family: family_name(&provider.family).into(),
                // 供应商摘要目前没有角色能力、权重和延迟字段。
                can_lead: true,
                can_work: true,
                can_review: true,
                enabled: !settings.provider_disabled.contains(&provider.id),
                priority: u32::try_from(priority + 1).unwrap_or(u32::MAX),
                recent_failure_rate: match provider.calls {
                    0 => 0.0,
                    calls => provider.errors.min(calls) as f64 / calls as f64,
                },
                quota_remaining_pct: quota_remaining(usage, &provider.family),
                weight: 1,
                avg_latency_ms: None,
            }
        })
        .collect()
}

fn quota_remaining(usage: Option<&UsageSnapshot>, family: &Family) -> Option<f64> {
    let mut latest_limits = std::collections::BTreeMap::new();
    for quota in usage?.quota.iter().filter(|quota| &quota.family == family) {
        let latest = latest_limits.entry(&quota.limit_id).or_insert(quota);
        if quota.captured_ts_ms > latest.captured_ts_ms {
            *latest = quota;
        }
    }
    latest_limits
        .into_values()
        .flat_map(|quota| [&quota.primary, &quota.secondary])
        .flatten()
        .map(|window| window.used_percent)
        .filter(|used| used.is_finite())
        .map(|used| (100.0 - used).clamp(0.0, 100.0))
        .reduce(f64::min)
}

/// 预览内置默认值与当前桌面用户改动合并后的配置来源。
pub fn effective_preview(settings: &Settings) -> Vec<EffectiveEntry> {
    let defaults = Settings::default();
    let user = KansoConfig {
        routing_policy: (settings.routing_policy != defaults.routing_policy)
            .then(|| settings.routing_policy.clone()),
        provider_order: (settings.provider_order != defaults.provider_order)
            .then(|| settings.provider_order.clone()),
        provider_disabled: (settings.provider_disabled != defaults.provider_disabled)
            .then(|| settings.provider_disabled.clone()),
        ..KansoConfig::default()
    };
    let builtin = KansoConfig {
        routing_policy: Some(defaults.routing_policy),
        provider_order: Some(defaults.provider_order),
        provider_disabled: Some(defaults.provider_disabled),
        ..KansoConfig::default()
    };

    kanso_config::effective(&[(ConfigLayer::Builtin, builtin), (ConfigLayer::User, user)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use kanso_usage::{Coverage, ProviderSource, QuotaSnapshot, QuotaWindow};

    #[test]
    fn policies_round_trip_through_their_settings_keys() {
        let keys = [
            "Sequential",
            "RoundRobin",
            "Weighted",
            "QuotaFirst",
            "SpeedFirst",
            "Sticky",
        ];
        for (policy, key) in ALL_POLICIES.into_iter().zip(keys) {
            assert_eq!(policy_key(policy), key);
            assert_eq!(parse_policy(key), policy);
        }
        assert_eq!(
            parse_policy(&Settings::default().routing_policy),
            RolePolicy::QuotaFirst
        );
    }

    fn provider(id: &str, family: Family, calls: u64, errors: u64) -> ProviderEntry {
        ProviderEntry {
            id: id.into(),
            name: id.into(),
            family,
            source: ProviderSource::CliProbe,
            tool: id.into(),
            version: None,
            models: Vec::new(),
            calls,
            errors,
            tokens: 0,
            last_day: None,
        }
    }

    fn quota(limit_id: &str, captured_ts_ms: i64, used: f64, secondary: f64) -> QuotaSnapshot {
        QuotaSnapshot {
            family: Family::OpenAi,
            limit_id: limit_id.into(),
            plan_type: None,
            primary: Some(QuotaWindow {
                used_percent: used,
                window_minutes: 300,
                resets_at: None,
            }),
            secondary: Some(QuotaWindow {
                used_percent: secondary,
                window_minutes: 10_080,
                resets_at: None,
            }),
            captured_ts_ms,
        }
    }

    #[test]
    fn candidate_mapping_uses_order_disabled_ids_and_tightest_current_quota() {
        let providers = [
            provider("codex", Family::OpenAi, 10, 2),
            provider("claude", Family::Anthropic, 0, 0),
            provider("gemini", Family::Google, 5, 0),
        ];
        let settings = Settings {
            provider_order: vec!["gemini".into(), "codex".into()],
            provider_disabled: vec!["claude".into()],
            ..Settings::default()
        };
        let usage = UsageSnapshot {
            requests: Vec::new(),
            total_requests: 0,
            total_errors: 0,
            daily: Default::default(),
            hourly_today: Vec::new(),
            coverage: Coverage {
                lookback_days: 0,
                first_day: String::new(),
                last_day: String::new(),
                active_days: 0,
            },
            quota: vec![
                quota("primary", 1, 100.0, 100.0),
                quota("primary", 2, 30.0, 75.0),
                quota("secondary", 1, 20.0, f64::NAN),
            ],
        };

        let candidates = build_candidates(&providers, &settings, Some(&usage));
        let [codex, claude, gemini] = candidates.as_slice() else {
            panic!("应保留三条供应商记录供路由筛选");
        };
        assert_eq!(codex.id, "codex");
        assert_eq!(codex.priority, 2);
        assert_eq!(gemini.priority, 1);
        assert!(claude.priority > codex.priority);
        assert!(codex.enabled && gemini.enabled);
        assert!(!claude.enabled);
        assert_eq!(codex.quota_remaining_pct, Some(25.0));
        assert_eq!(claude.quota_remaining_pct, None);
        assert_eq!(gemini.quota_remaining_pct, None);
        assert_eq!(codex.recent_failure_rate, 0.2);
        assert_eq!(claude.recent_failure_rate, 0.0);
        assert_eq!(
            build_candidates(&providers, &settings, None)[0].quota_remaining_pct,
            None
        );
    }
}
