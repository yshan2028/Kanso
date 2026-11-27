//! Kanso 分层配置的共享契约。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::{fs, io};

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

/// 配置文件读取、写入或 TOML 转换时发生的错误。
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// 指定的配置文件不存在。
    #[error("配置文件不存在 {path:?}")]
    NotFound {
        /// 缺失的配置文件路径。
        path: PathBuf,
        /// 底层文件读取错误。
        #[source]
        source: io::Error,
    },
    /// 无法读取指定的配置文件。
    #[error("读取配置文件失败 {path:?}")]
    Read {
        /// 读取失败的配置文件路径。
        path: PathBuf,
        /// 底层文件读取错误。
        #[source]
        source: io::Error,
    },
    /// 无法创建配置文件的父目录。
    #[error("创建配置目录失败 {path:?}")]
    CreateDirectory {
        /// 创建失败的目录路径。
        path: PathBuf,
        /// 底层目录创建错误。
        #[source]
        source: io::Error,
    },
    /// 无法写入指定的配置文件。
    #[error("写入配置文件失败 {path:?}")]
    Write {
        /// 写入失败的配置文件路径。
        path: PathBuf,
        /// 底层文件写入错误。
        #[source]
        source: io::Error,
    },
    /// 配置文件中的 TOML 内容无法解析为配置。
    #[error("解析 TOML 配置失败 {path:?}")]
    Parse {
        /// 解析失败的配置文件路径。
        path: PathBuf,
        /// 底层 TOML 解析错误。
        #[source]
        source: toml::de::Error,
    },
    /// 配置无法序列化为 TOML。
    #[error("序列化 TOML 配置失败 {path:?}")]
    Serialize {
        /// 配置文件的目标路径。
        path: PathBuf,
        /// 底层 TOML 序列化错误。
        #[source]
        source: toml::ser::Error,
    },
}

/// 返回用户主目录下的配置路径。无法确定主目录时返回空路径。
pub fn user_config_path() -> PathBuf {
    dirs::home_dir()
        .into_iter()
        .map(|home| home.join(".kanso/config.toml"))
        .collect()
}

/// 加载指定的 TOML 配置文件，文件缺失时返回 [`ConfigError::NotFound`]。
pub fn load_file(path: &Path) -> anyhow::Result<KansoConfig> {
    let contents = fs::read_to_string(path).map_err(|source| {
        let path = path.to_path_buf();
        match source.kind() {
            io::ErrorKind::NotFound => ConfigError::NotFound { path, source },
            _ => ConfigError::Read { path, source },
        }
    })?;
    let config = toml::from_str(&contents).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(config)
}

/// 创建缺失的父目录并以格式化的 TOML 覆盖目标配置文件。
pub fn save_user(path: &Path, cfg: &KansoConfig) -> anyhow::Result<()> {
    let contents = toml::to_string_pretty(cfg).map_err(|source| ConfigError::Serialize {
        path: path.to_path_buf(),
        source,
    })?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| ConfigError::CreateDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(path, contents).map_err(|source| ConfigError::Write {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

/// 按优先级合并配置。同层取最先传入的已设置值，价格按模型规则分别合并。
/// 返回的条目按来源层优先级排列。
pub fn effective(layers: &[(ConfigLayer, KansoConfig)]) -> Vec<EffectiveEntry> {
    let mut sorted_layers: Vec<_> = layers.iter().collect();
    sorted_layers.sort_by_key(|(layer, _)| *layer);

    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();
    for (layer, cfg) in sorted_layers {
        let mut add_entry = |key: String, value: String| {
            if seen.insert(key.clone()) {
                entries.push(EffectiveEntry {
                    key,
                    value,
                    layer: *layer,
                });
            }
        };

        let settings = [
            ("routing.worker.policy", cfg.routing_policy.clone()),
            (
                "routing.reviewer.require_cross_family",
                cfg.require_cross_family.map(|value| value.to_string()),
            ),
            (
                "providers.order",
                cfg.provider_order.as_ref().map(|values| values.join(", ")),
            ),
            (
                "providers.disabled",
                cfg.provider_disabled
                    .as_ref()
                    .map(|values| values.join(", ")),
            ),
        ];
        for (key, value) in settings {
            if let Some(value) = value {
                add_entry(key.to_owned(), value);
            }
        }

        if let Some(pricing) = &cfg.pricing {
            for (pattern, price) in pricing {
                add_entry(
                    format!("pricing.{pattern}"),
                    format!(
                        "in=${} cw=${} cr=${} out=${}",
                        price.input, price.cache_write, price.cache_read, price.output
                    ),
                );
            }
        }
    }
    entries
}
