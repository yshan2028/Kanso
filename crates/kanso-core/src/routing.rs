//! 模型路由与失败切换的共享契约。

use serde::{Deserialize, Serialize};

/// 候选供应商的排序策略。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RolePolicy {
    /// 按优先级依次分配。
    Sequential,
    /// 按轮询游标分配。
    RoundRobin,
    /// 按配置权重分配。
    Weighted,
    /// 优先使用剩余额度较高的供应商。
    QuotaFirst,
    /// 优先使用平均延迟较低的供应商。
    SpeedFirst,
    /// 优先沿用已绑定的供应商。
    Sticky,
}

impl RolePolicy {
    /// 返回策略的简体中文名称。
    pub fn label_zh(self) -> &'static str {
        match self {
            Self::Sequential => "顺序分配",
            Self::RoundRobin => "轮询",
            Self::Weighted => "加权分配",
            Self::QuotaFirst => "额度优先",
            Self::SpeedFirst => "速度优先",
            Self::Sticky => "固定供应商",
        }
    }
}

/// 任务中的模型职责。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Role {
    /// 拆解与协调任务。
    Lead,
    /// 执行任务。
    Worker,
    /// 审查任务结果。
    Reviewer,
}

/// 可参与模型路由的供应商及其能力和运行指标。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// 供应商标识。
    pub id: String,
    /// 模型家族。
    pub family: String,
    /// 是否能协调任务。
    pub can_lead: bool,
    /// 是否能执行任务。
    pub can_work: bool,
    /// 是否能审查任务。
    pub can_review: bool,
    /// 是否启用。
    pub enabled: bool,
    /// 分配优先级。
    pub priority: u32,
    /// 近期请求失败率。
    pub recent_failure_rate: f64,
    /// 剩余额度百分比。
    pub quota_remaining_pct: Option<f64>,
    /// 加权分配权重。
    pub weight: u32,
    /// 平均延迟，单位为毫秒。
    pub avg_latency_ms: Option<u64>,
}

/// 一次路由决策的角色、策略及上下文。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutingInput {
    /// 待分配的角色。
    pub role: Role,
    /// 候选排序策略。
    pub policy: RolePolicy,
    /// 执行者的模型家族，供审查分配使用。
    pub worker_family: Option<String>,
    /// 已绑定的供应商标识。
    pub sticky_id: Option<String>,
    /// 轮询游标。
    pub rr_cursor: usize,
}

/// 路由跳过供应商的原因。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SkipReason {
    /// 供应商已停用。
    Disabled,
    /// 缺少所需角色的能力。
    NoCapability,
    /// 与执行者属于同一模型家族。
    SameFamily,
    /// 供应商运行状态异常。
    Unhealthy,
    /// 剩余额度不足。
    LowQuota,
}

/// 一次路由的分配结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RouteOutcome {
    /// 已得到候选供应商顺序。
    Assigned {
        /// 按尝试顺序排列的供应商标识。
        ordered: Vec<String>,
        /// 跳过的供应商及原因。
        skipped: Vec<(String, SkipReason)>,
    },
    /// 需要升级处理。
    Escalate {
        /// 跳过的供应商及原因。
        skipped: Vec<(String, SkipReason)>,
    },
}

/// 失败切换的尝试记录和预算。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailoverState {
    /// 已尝试切换的次数。
    pub attempts: u32,
    /// 允许切换的次数。
    pub budget: u32,
}

/// 依次检查五项准入条件，再按策略排列通过的供应商。
/// 每个被跳过的供应商只记录首个未满足的条件。
pub fn plan_route(candidates: &[Candidate], input: &RoutingInput) -> RouteOutcome {
    let mut eligible = Vec::new();
    let mut skipped = Vec::new();
    for candidate in candidates {
        if let Some(reason) = admission_failure(candidate, input) {
            skipped.push((candidate.id.clone(), reason));
        } else {
            eligible.push(candidate);
        }
    }

    if eligible.is_empty() {
        return RouteOutcome::Escalate { skipped };
    }

    // 已知额度按数值排序，未知额度和 NaN 排在已知充足额度之后。
    let quota = |candidate: &Candidate| {
        candidate
            .quota_remaining_pct
            .filter(|quota| !quota.is_nan())
            .unwrap_or(-1.0)
    };
    let latency = |candidate: &Candidate| {
        (
            candidate.avg_latency_ms.unwrap_or(u64::MAX),
            candidate.avg_latency_ms.is_none(),
        )
    };
    let is_sticky =
        |candidate: &Candidate| input.sticky_id.as_deref() == Some(candidate.id.as_str());

    eligible.sort_by(|a, b| {
        let policy_order = match input.policy {
            RolePolicy::Sequential | RolePolicy::RoundRobin => std::cmp::Ordering::Equal,
            RolePolicy::Weighted => b.weight.cmp(&a.weight),
            RolePolicy::QuotaFirst => quota(b).total_cmp(&quota(a)),
            RolePolicy::SpeedFirst => latency(a).cmp(&latency(b)),
            RolePolicy::Sticky => is_sticky(b).cmp(&is_sticky(a)),
        };
        policy_order
            .then_with(|| a.priority.cmp(&b.priority))
            .then_with(|| a.id.cmp(&b.id))
    });

    if input.policy == RolePolicy::RoundRobin {
        let offset = input.rr_cursor % eligible.len();
        eligible.rotate_left(offset);
    }

    RouteOutcome::Assigned {
        ordered: eligible
            .iter()
            .map(|candidate| candidate.id.clone())
            .collect(),
        skipped,
    }
}

fn admission_failure(candidate: &Candidate, input: &RoutingInput) -> Option<SkipReason> {
    if !candidate.enabled {
        return Some(SkipReason::Disabled);
    }

    let capable = match input.role {
        Role::Lead => candidate.can_lead,
        Role::Worker => candidate.can_work,
        Role::Reviewer => candidate.can_review,
    };
    if !capable {
        return Some(SkipReason::NoCapability);
    }

    if input.role == Role::Reviewer && input.worker_family.as_ref() == Some(&candidate.family) {
        return Some(SkipReason::SameFamily);
    }

    if candidate.recent_failure_rate >= 0.05 || !candidate.recent_failure_rate.is_finite() {
        return Some(SkipReason::Unhealthy);
    }

    if candidate
        .quota_remaining_pct
        .is_some_and(|quota| quota < 10.0)
    {
        return Some(SkipReason::LowQuota);
    }

    None
}

/// 在预算内选择失败项之后的首个未耗尽供应商。
/// 若顺序中没有失败项，则从首项开始。仅在选中供应商时扣减可用次数。
pub fn next_after_failure(
    outcome: &RouteOutcome,
    failed_id: &str,
    state: &mut FailoverState,
) -> Option<String> {
    let RouteOutcome::Assigned { ordered, .. } = outcome else {
        return None;
    };
    if state.attempts >= state.budget {
        return None;
    }

    let next_index = ordered
        .iter()
        .position(|id| id == failed_id)
        .map_or(0, |index| index + 1);
    let (exhausted, remaining) = ordered.split_at(next_index);
    let next_id = remaining.iter().find(|id| !exhausted.contains(id))?;
    state.attempts += 1;
    Some(next_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str) -> Candidate {
        Candidate {
            id: id.into(),
            family: "family-a".into(),
            can_lead: true,
            can_work: true,
            can_review: true,
            enabled: true,
            priority: 10,
            recent_failure_rate: 0.0,
            quota_remaining_pct: Some(70.0),
            weight: 4,
            avg_latency_ms: Some(150),
        }
    }

    fn input(policy: RolePolicy) -> RoutingInput {
        RoutingInput {
            role: Role::Worker,
            policy,
            worker_family: None,
            sticky_id: None,
            rr_cursor: 0,
        }
    }

    #[test]
    fn records_the_first_failed_check_in_input_order() {
        let ineligible = |id| Candidate {
            family: "worker-family".into(),
            recent_failure_rate: 0.05,
            quota_remaining_pct: Some(9.9),
            ..candidate(id)
        };
        let candidates = [
            Candidate {
                enabled: false,
                can_review: false,
                ..ineligible("disabled")
            },
            Candidate {
                can_review: false,
                ..ineligible("no-capability")
            },
            ineligible("same-family"),
            Candidate {
                recent_failure_rate: 0.049,
                quota_remaining_pct: Some(10.0),
                ..candidate("eligible")
            },
            Candidate {
                family: "family-a".into(),
                ..ineligible("unhealthy")
            },
            Candidate {
                family: "family-a".into(),
                recent_failure_rate: 0.049,
                ..ineligible("low-quota")
            },
        ];
        let input = RoutingInput {
            role: Role::Reviewer,
            worker_family: Some("worker-family".into()),
            sticky_id: Some("same-family".into()),
            ..input(RolePolicy::Sticky)
        };

        assert_eq!(
            plan_route(&candidates, &input),
            RouteOutcome::Assigned {
                ordered: vec!["eligible".into()],
                skipped: vec![
                    ("disabled".into(), SkipReason::Disabled),
                    ("no-capability".into(), SkipReason::NoCapability),
                    ("same-family".into(), SkipReason::SameFamily),
                    ("unhealthy".into(), SkipReason::Unhealthy),
                    ("low-quota".into(), SkipReason::LowQuota),
                ],
            }
        );
    }

    #[test]
    fn checks_the_capability_for_the_requested_role() {
        let candidates = [
            Candidate {
                can_work: false,
                can_review: false,
                ..candidate("lead")
            },
            Candidate {
                can_lead: false,
                can_review: false,
                ..candidate("worker")
            },
            Candidate {
                can_lead: false,
                can_work: false,
                ..candidate("reviewer")
            },
        ];

        for (role, expected_id, worker_family) in [
            (Role::Lead, "lead", Some("family-a".into())),
            (Role::Worker, "worker", Some("family-a".into())),
            (Role::Reviewer, "reviewer", None),
        ] {
            let input = RoutingInput {
                role,
                worker_family,
                ..input(RolePolicy::Sequential)
            };
            let RouteOutcome::Assigned { ordered, skipped } = plan_route(&candidates, &input)
            else {
                panic!("应有且仅有一个候选具备所需能力");
            };
            assert_eq!(ordered, [expected_id]);
            assert_eq!(skipped.len(), 2);
            assert!(
                skipped
                    .iter()
                    .all(|(_, reason)| *reason == SkipReason::NoCapability)
            );
        }
    }

    #[test]
    fn escalates_when_every_candidate_has_a_non_finite_failure_rate() {
        let candidates = [
            Candidate {
                recent_failure_rate: f64::NAN,
                ..candidate("nan")
            },
            Candidate {
                recent_failure_rate: f64::INFINITY,
                ..candidate("positive-infinity")
            },
            Candidate {
                recent_failure_rate: f64::NEG_INFINITY,
                ..candidate("negative-infinity")
            },
        ];
        assert_eq!(
            plan_route(&candidates, &input(RolePolicy::RoundRobin)),
            RouteOutcome::Escalate {
                skipped: vec![
                    ("nan".into(), SkipReason::Unhealthy),
                    ("positive-infinity".into(), SkipReason::Unhealthy),
                    ("negative-infinity".into(), SkipReason::Unhealthy),
                ],
            }
        );
        assert_eq!(
            plan_route(&[], &input(RolePolicy::RoundRobin)),
            RouteOutcome::Escalate { skipped: vec![] }
        );
    }

    #[test]
    fn orders_candidates_for_each_policy() {
        let candidates = [
            Candidate {
                priority: 20,
                weight: 9,
                quota_remaining_pct: Some(90.0),
                avg_latency_ms: Some(200),
                ..candidate("z")
            },
            candidate("b"),
            Candidate {
                priority: 30,
                ..candidate("e")
            },
            Candidate {
                priority: 5,
                quota_remaining_pct: None,
                avg_latency_ms: None,
                ..candidate("d")
            },
            candidate("a"),
        ];

        for (policy, expected) in [
            (RolePolicy::Sequential, ["d", "a", "b", "z", "e"]),
            (RolePolicy::RoundRobin, ["b", "z", "e", "d", "a"]),
            (RolePolicy::Weighted, ["z", "d", "a", "b", "e"]),
            (RolePolicy::QuotaFirst, ["z", "a", "b", "e", "d"]),
            (RolePolicy::SpeedFirst, ["a", "b", "e", "z", "d"]),
            (RolePolicy::Sticky, ["b", "d", "a", "z", "e"]),
        ] {
            let input = RoutingInput {
                sticky_id: Some("b".into()),
                rr_cursor: 7,
                ..input(policy)
            };
            let RouteOutcome::Assigned { ordered, skipped } = plan_route(&candidates, &input)
            else {
                panic!("排序用候选应全部通过准入");
            };
            assert_eq!(ordered, expected, "{policy:?}");
            assert!(skipped.is_empty(), "{policy:?}");
        }
    }

    #[test]
    fn ranks_known_metrics_ahead_of_unknown_metrics() {
        let candidates = [
            Candidate {
                quota_remaining_pct: None,
                avg_latency_ms: None,
                ..candidate("none")
            },
            Candidate {
                quota_remaining_pct: Some(f64::NAN),
                avg_latency_ms: None,
                ..candidate("nan")
            },
            Candidate {
                priority: 20,
                quota_remaining_pct: Some(10.0),
                avg_latency_ms: Some(u64::MAX),
                ..candidate("known")
            },
        ];

        for policy in [RolePolicy::QuotaFirst, RolePolicy::SpeedFirst] {
            assert_eq!(
                plan_route(&candidates, &input(policy)),
                RouteOutcome::Assigned {
                    ordered: vec!["known".into(), "nan".into(), "none".into()],
                    skipped: vec![],
                }
            );
        }
    }

    #[test]
    fn permits_two_switches_then_stops_at_the_budget() {
        let outcome = RouteOutcome::Assigned {
            ordered: ["a", "b", "c", "d"].map(String::from).to_vec(),
            skipped: vec![],
        };
        let mut state = FailoverState {
            attempts: 0,
            budget: 2,
        };

        assert_eq!(
            next_after_failure(&outcome, "a", &mut state),
            Some("b".into())
        );
        assert_eq!(state.attempts, 1);
        assert_eq!(
            next_after_failure(&outcome, "b", &mut state),
            Some("c".into())
        );
        assert_eq!(state.attempts, 2);
        assert_eq!(next_after_failure(&outcome, "c", &mut state), None);
        assert_eq!(state.attempts, 2);
    }

    #[test]
    fn starts_at_the_first_candidate_for_an_unknown_failure_and_skips_exhausted_ids() {
        let outcome = RouteOutcome::Assigned {
            ordered: ["a", "b", "a", "b", "c"].map(String::from).to_vec(),
            skipped: vec![],
        };
        let mut state = FailoverState {
            attempts: 0,
            budget: 4,
        };

        assert_eq!(
            next_after_failure(&outcome, "unknown", &mut state),
            Some("a".into())
        );
        assert_eq!(
            next_after_failure(&outcome, "b", &mut state),
            Some("c".into())
        );
        assert_eq!(state.attempts, 2);
        assert_eq!(next_after_failure(&outcome, "c", &mut state), None);
        assert_eq!(
            next_after_failure(&RouteOutcome::Escalate { skipped: vec![] }, "a", &mut state),
            None
        );
        assert_eq!(state.attempts, 2);
    }
}
