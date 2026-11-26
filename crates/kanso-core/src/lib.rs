pub mod config;
pub mod model;
pub mod ops;
pub mod scanner;
pub mod task;
pub mod watcher;

pub use config::{KansoYaml, PipelineSpec, PipelineStepSpec, load_kanso_yaml};
pub use model::{
    BatchOpResult, GuardrailAlert, GuardrailKind, RepoStatus, TaskGroup, TaskStage,
    WorkspaceConfig, WorkspaceSnapshot, WorktreeEntry,
};
pub use ops::{batch_create_branch, batch_pull_trunk, rescue_dirty_trunk};
pub use scanner::{GitCliBackend, VcsBackend, discover_git_repos, scan_workspace};
pub use task::{aggregate_tasks, detect_alerts};
pub use watcher::WorkspaceWatcher;
