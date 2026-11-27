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

/// 规划供应商顺序，当前仅提供占位结果。
pub fn plan_route(_candidates: &[Candidate], _input: &RoutingInput) -> RouteOutcome {
    RouteOutcome::Escalate {
        skipped: Vec::new(),
    }
}

/// 获取失败后的下一个供应商，当前仅提供占位结果。
pub fn next_after_failure(
    _outcome: &RouteOutcome,
    _failed_id: &str,
    _state: &mut FailoverState,
) -> Option<String> {
    None
}
