//! Kanso 分层配置的共享契约。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 配置来源，声明顺序从最高优先级到最低优先级。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum ConfigLayer {
    /// 命令行传入的覆盖值。
    CliFlag,
    /// 单个任务的配置。
    Task,
    /// 仓库范围内的配置。
    Repo,
    /// 用户范围内的配置。
    User,
    /// 从外部只读导入的配置。
    Import,
    /// 应用内置的默认配置。
    Builtin,
}

impl ConfigLayer {
    /// 返回配置层的简体中文名称。
    pub fn label_zh(self) -> &'static str {
        match self {
            Self::CliFlag => "命令行标志",
            Self::Task => "任务级",
            Self::Repo => "仓库级",
            Self::User => "用户级",
            Self::Import => "只读导入",
            Self::Builtin => "内置默认",
        }
    }
}

/// 覆盖模型价格，每项均以美元每百万词元计。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PriceOverride {
    /// 未缓存输入价格。
    pub input: f64,
    /// 缓存写入价格。
    pub cache_write: f64,
    /// 缓存读取价格。
    pub cache_read: f64,
    /// 输出价格。
    pub output: f64,
}

/// 单个配置层声明的设置。
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct KansoConfig {
    /// 路由策略名称。
    pub routing_policy: Option<String>,
    /// 是否要求执行者和审查者属于不同模型家族。
    pub require_cross_family: Option<bool>,
    /// 供应商优先顺序。
    pub provider_order: Option<Vec<String>>,
    /// 停用的供应商。
    pub provider_disabled: Option<Vec<String>>,
    /// 按模型规则索引的价格覆盖。
    pub pricing: Option<BTreeMap<String, PriceOverride>>,
}

/// 合并后生效的设置及其来源。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EffectiveEntry {
    /// 设置键名。
    pub key: String,
    /// 设置的显示值。
    pub value: String,
    /// 提供生效值的配置层。
    pub layer: ConfigLayer,
}

/// 返回用户主目录下的配置路径。无法确定主目录时返回空路径。
pub fn user_config_path() -> PathBuf {
    dirs::home_dir()
        .into_iter()
        .map(|home| home.join(".kanso/config.toml"))
        .collect()
}

/// 加载指定配置文件，当前返回尚未接入读写的错误。
pub fn load_file(_path: &Path) -> anyhow::Result<KansoConfig> {
    anyhow::bail!("配置文件加载尚未实现")
}

/// 保存用户配置，当前返回尚未接入读写的错误。
pub fn save_user(_path: &Path, _cfg: &KansoConfig) -> anyhow::Result<()> {
    anyhow::bail!("用户配置保存尚未实现")
}

/// 合并配置层，当前返回空的占位结果。
pub fn effective(_layers: &[(ConfigLayer, KansoConfig)]) -> Vec<EffectiveEntry> {
    Vec::new()
}
