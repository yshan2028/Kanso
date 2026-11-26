use kanso_core::WorkspaceConfig;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub workspace: WorkspaceConfig,
    pub debounce: Duration,
    pub min_interval: Duration,
    pub self_write_cooldown: Duration,
    pub calibrate_interval: Duration,
    pub concurrency: usize,
}

impl EngineConfig {
    pub fn new(workspace: WorkspaceConfig) -> Self {
        Self {
            workspace,
            debounce: Duration::from_millis(300),
            min_interval: Duration::from_secs(1),
            self_write_cooldown: Duration::from_secs(1),
            calibrate_interval: Duration::from_secs(300),
            concurrency: 2,
        }
    }
}
