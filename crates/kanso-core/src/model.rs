use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceConfig {
    pub id: String,
    pub name: String,
    pub root_path: String,
    pub default_trunk: Option<String>,
    pub ignore_dirs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeEntry {
    pub path: String,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub is_main: bool,
    pub is_prunable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoStatus {
    pub id: String,
    pub name: String,
    pub path: String,
    pub relative_path: String,
    pub is_worktree: bool,
    pub current_branch: String,
    pub is_detached: bool,
    pub trunk_branch: Option<String>,
    pub is_on_trunk: bool,
    pub has_dirty_on_trunk: bool,
    pub upstream: Option<String>,
    pub upstream_ahead: u32,
    pub upstream_behind: u32,
    pub trunk_ahead: u32,
    pub trunk_behind: u32,
    pub staged_count: u32,
    pub modified_count: u32,
    pub untracked_count: u32,
    pub conflicted_count: u32,
    pub last_commit_hash: Option<String>,
    pub last_commit_message: Option<String>,
    pub last_commit_author: Option<String>,
    pub last_commit_timestamp: Option<i64>,
    pub worktrees: Vec<WorktreeEntry>,
}

impl RepoStatus {
    pub fn total_dirty(&self) -> u32 {
        self.staged_count + self.modified_count + self.untracked_count + self.conflicted_count
    }

    pub fn is_dirty(&self) -> bool {
        self.total_dirty() > 0
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TaskStage {
    InProgress,
    Dirty,
    ReadyForCi,
    CiFailed,
    ReadyToLand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskGroup {
    pub id: String,
    pub branch: String,
    pub stage: TaskStage,
    pub progress_percent: u8,
    pub repo_ids: Vec<String>,
    pub repo_names: Vec<String>,
    pub total_repos: usize,
    pub dirty_repos: usize,
    pub unpushed_repos: usize,
    pub behind_trunk_repos: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GuardrailKind {
    DirtyTrunk,
    DetachedHead,
    MergeConflict,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardrailAlert {
    pub repo_id: String,
    pub repo_name: String,
    pub repo_path: String,
    pub trunk_branch: Option<String>,
    pub dirty_files: u32,
    pub kind: GuardrailKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSnapshot {
    pub workspace_id: String,
    pub workspace_name: String,
    pub root_path: String,
    pub scanned_at: i64,
    pub scan_duration_ms: u64,
    pub repos: Vec<RepoStatus>,
    pub tasks: Vec<TaskGroup>,
    pub alerts: Vec<GuardrailAlert>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchOpResult {
    pub repo_path: String,
    pub repo_name: String,
    pub success: bool,
    pub message: String,
}
