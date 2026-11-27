//! 用量、配额、价格和工具探测的共享数据类型。

use serde::{Deserialize, Serialize};

/// 模型家族。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Family {
    /// Anthropic 模型家族。
    Anthropic,
    /// OpenAI 模型家族。
    OpenAi,
    /// Google 模型家族。
    Google,
}

/// 请求的来源。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SourceKind {
    /// Claude Code 会话。
    ClaudeCode,
    /// Codex 命令行会话。
    CodexCli,
    /// Codex 桌面会话。
    CodexDesktop,
    /// Codex 非交互式执行。
    CodexExec,
}

/// 请求的执行结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ReqStatus {
    /// 请求成功。
    Success,
    /// 请求失败。
    Error,
}

/// 单次模型请求的用量与费用。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageRequest {
    /// 请求标识。
    pub id: String,
    /// 请求时间，使用 Unix 毫秒时间戳。
    pub ts_ms: i64,
    /// 请求来源。
    pub source_kind: SourceKind,
    /// 模型家族。
    pub family: Family,
    /// 模型名称。
    pub model: Option<String>,
    /// 会话标识。
    pub session_id: String,
    /// 项目标识。
    pub project: String,
    /// 未命中缓存的输入词元数。
    pub input_uncached: u64,
    /// 从缓存读取的词元数。
    pub cache_read: u64,
    /// 写入缓存的词元数。
    pub cache_write: u64,
    /// 输出词元数。
    pub output: u64,
    /// 推理词元数。
    pub reasoning: u64,
    /// 预估费用，单位为美元。
    pub est_usd: f64,
    /// 请求结果。
    pub status: ReqStatus,
}

/// 按日期、来源和模型汇总的用量。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DailyBucket {
    /// 请求来源。
    pub source_kind: SourceKind,
    /// 模型家族。
    pub family: Family,
    /// 模型名称。
    pub model: Option<String>,
    /// 请求总数。
    pub calls: u64,
    /// 失败请求数。
    pub errors: u64,
    /// 未命中缓存的输入词元数。
    pub input_uncached: u64,
    /// 从缓存读取的词元数。
    pub cache_read: u64,
    /// 写入缓存的词元数。
    pub cache_write: u64,
    /// 输出词元数。
    pub output: u64,
    /// 推理词元数。
    pub reasoning: u64,
    /// 预估费用，单位为美元。
    pub est_usd: f64,
}

/// 本地单小时的用量。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HourlySlot {
    /// 本地小时，范围为 0 至 23。
    pub hour: u8,
    /// 请求总数。
    pub calls: u64,
    /// 未命中缓存的输入词元数。
    pub input_uncached: u64,
    /// 从缓存读取的词元数。
    pub cache_read: u64,
    /// 写入缓存的词元数。
    pub cache_write: u64,
    /// 输出词元数。
    pub output: u64,
}

/// 采集范围与实际活跃日期。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Coverage {
    /// 查询的回溯天数。
    pub lookback_days: u32,
    /// 最早的活跃日期。
    pub first_day: String,
    /// 最近的活跃日期。
    pub last_day: String,
    /// 有请求记录的天数。
    pub active_days: u32,
}

/// 配额窗口中的用量与重置时间。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaWindow {
    /// 已使用的百分比。
    pub used_percent: f64,
    /// 窗口长度，单位为分钟。
    pub window_minutes: u32,
    /// 配额重置时间。
    pub resets_at: Option<i64>,
}

/// 某个限额的配额快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaSnapshot {
    /// 模型家族。
    pub family: Family,
    /// 限额标识。
    pub limit_id: String,
    /// 订阅计划类型。
    pub plan_type: Option<String>,
    /// 主要配额窗口。
    pub primary: Option<QuotaWindow>,
    /// 次要配额窗口。
    pub secondary: Option<QuotaWindow>,
    /// 采集时间，使用 Unix 毫秒时间戳。
    pub captured_ts_ms: i64,
}

/// 模型匹配规则及各类词元价格。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriceEntry {
    /// 模型家族。
    pub family: Family,
    /// 模型名称匹配规则。
    pub pattern: String,
    /// 每百万未缓存输入词元的美元价格。
    pub input: f64,
    /// 每百万缓存写入词元的美元价格。
    pub cache_write: f64,
    /// 每百万缓存读取词元的美元价格。
    pub cache_read: f64,
    /// 每百万输出词元的美元价格。
    pub output: f64,
}

/// 命令行工具的安装来源。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PathKind {
    /// 由 npm 安装。
    Npm,
    /// 由 Homebrew 安装。
    Brew,
    /// 独立可执行文件。
    Binary,
}

/// 本地命令行工具的安装状态。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolVersion {
    /// 工具名称。
    pub tool: String,
    /// 工具对应的模型家族。
    pub family: Family,
    /// 是否已安装。
    pub installed: bool,
    /// 已安装的版本。
    pub version: Option<String>,
    /// 安装来源。
    pub path_kind: Option<PathKind>,
    /// 升级命令。
    pub upgrade_cmd: Option<String>,
}

/// 桌面应用使用的更新机制。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UpdaterKind {
    /// 使用 Sparkle 更新。
    Sparkle,
    /// 更新机制未知。
    Unknown,
}

/// 桌面应用的安装和更新信息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesktopAppInfo {
    /// 是否已安装。
    pub installed: bool,
    /// 应用名称。
    pub name: String,
    /// 已安装的版本。
    pub version: Option<String>,
    /// 使用的更新机制。
    pub updater: UpdaterKind,
    /// 是否自动检查更新。
    pub auto_check: Option<bool>,
    /// 是否自动安装更新。
    pub auto_install: Option<bool>,
    /// 最后检查更新的时间。
    pub last_check: Option<String>,
}

/// 供应商信息的来源。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProviderSource {
    /// 从会话日志识别。
    SessionLog,
    /// 从命令行工具探测。
    CliProbe,
}

/// 供应商的工具和用量摘要。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderEntry {
    /// 供应商标识。
    pub id: String,
    /// 供应商显示名称。
    pub name: String,
    /// 模型家族。
    pub family: Family,
    /// 信息来源。
    pub source: ProviderSource,
    /// 关联工具名称。
    pub tool: String,
    /// 关联工具版本。
    pub version: Option<String>,
    /// 已观测的模型名称。
    pub models: Vec<String>,
    /// 请求总数。
    pub calls: u64,
    /// 失败请求数。
    pub errors: u64,
    /// 用量统计中的词元总数。
    pub tokens: u64,
    /// 最近活跃日期。
    pub last_day: Option<String>,
}

/// 请求明细、用量汇总与配额快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageSnapshot {
    /// 采集的请求明细。
    pub requests: Vec<UsageRequest>,
    /// 请求总数。
    pub total_requests: u64,
    /// 失败请求总数。
    pub total_errors: u64,
    /// 按本地日期索引的用量汇总。
    pub daily: std::collections::BTreeMap<String, Vec<DailyBucket>>,
    /// 本地当天的每小时用量。
    pub hourly_today: Vec<HourlySlot>,
    /// 采集范围。
    pub coverage: Coverage,
    /// 已采集的配额快照。
    pub quota: Vec<QuotaSnapshot>,
}
