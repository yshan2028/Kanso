use kanso_core::{RepoFailure, RepoStatus, WorkspaceSnapshot};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum EngineEvent {
    SnapshotReady(WorkspaceSnapshot),
    RepoUpdated(RepoStatus),
    RepoFailed(RepoFailure),
    ReposChanged {
        added: Vec<String>,
        removed: Vec<String>,
    },
    Error(String),
}

#[derive(Debug, Clone)]
pub enum EngineCommand {
    RefreshRepo(PathBuf),
    RefreshAll,
    MarkSelfWrite(PathBuf),
    Shutdown,
}
