use crate::theme;
use crate::views;
use chrono::Utc;
use iced::time::{self, Duration};
use iced::widget::{column, container, row, text};
use iced::{Element, Length, Subscription, Task, Theme};
use kanso_ci::{CiRunner, LocalCiRunner, PipelineDefinition, PipelineStep, discover_pipelines};
use kanso_core::{
    BatchOpResult, CommitRequest, CommitResult, GuardrailAlert, LandResult, PushResult,
    RepoChangeset, WorkspaceConfig, WorkspaceSnapshot, WorkspaceWatcher, batch_commit_and_push,
    batch_create_branch, batch_pull_trunk, commit_repo, detect_jj_version, enable_jj_colocated,
    inspect_repo_changes, jj_undo, land_branch_to_trunk, push_repo, rescue_dirty_trunk,
    scan_workspace,
};
use kanso_storage::{CiRunRecord, NewCiRun, StorageDb, WorkspaceRecord};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavTab {
    Radar,
    Tasks,
    Commit,
    Ci,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterMode {
    All,
    Dirty,
    OffTrunk,
    Alerts,
}

#[derive(Debug, Clone, Default)]
pub struct CiLiveBuffer {
    pub run_id: Option<String>,
    pub lines: Vec<String>,
    pub finished_status: Option<String>,
}

pub struct KansoApp {
    pub db: Arc<StorageDb>,
    pub workspaces: Vec<WorkspaceRecord>,
    pub active_workspace_id: Option<String>,
    pub snapshot: Option<WorkspaceSnapshot>,
    pub active_tab: NavTab,
    pub scanning: bool,

    pub watcher: Option<WorkspaceWatcher>,
    pub fs_dirty_flag: Arc<AtomicBool>,

    pub search_query: String,
    pub filter_mode: FilterMode,
    pub selected_repo_paths: HashSet<String>,

    pub show_manual_ws_form: bool,
    pub manual_ws_path: String,
    pub manual_ws_name: String,
    pub manual_ws_trunk: String,

    pub editing_trunk: bool,
    pub trunk_input: String,

    pub show_batch_branch_form: bool,
    pub batch_branch_name: String,
    pub batch_base_branch: String,
    pub batch_results: Option<Vec<BatchOpResult>>,

    pub rescue_target: Option<GuardrailAlert>,
    pub rescue_branch_input: String,

    pub selected_ci_repo: Option<String>,
    pub ci_pipelines: Vec<PipelineDefinition>,
    pub ci_runs: Vec<CiRunRecord>,
    pub ci_custom_cmd: String,
    pub active_ci_run_id: Option<String>,
    pub ci_running: bool,
    pub ci_live_buf: Arc<Mutex<CiLiveBuffer>>,
    pub ci_cancel_tx: Option<oneshot::Sender<()>>,
    pub displayed_logs: Vec<String>,
    pub displayed_run_status: Option<String>,
    pub repo_latest_ci: HashMap<String, String>,

    pub jj_version: Option<String>,
    pub selected_commit_repo: Option<String>,
    pub changeset: Option<RepoChangeset>,
    pub commit_loading: bool,
    pub selected_commit_files: HashSet<String>,
    pub commit_message: String,
    pub commit_allow_large: bool,
    pub selected_push_remote: Option<String>,
    pub land_require_ci_pass: bool,
    pub land_delete_source: bool,

    pub task_batch_commit_branch: Option<String>,
    pub task_batch_commit_msg: String,
    pub task_batch_push_remote: String,

    pub toast: Option<String>,
    pub copied_mcp: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    SelectTab(NavTab),
    PickWorkspaceFolder,
    FolderPicked(Option<PathBuf>),
    ToggleManualWsForm(bool),
    ManualWsPathChanged(String),
    ManualWsNameChanged(String),
    ManualWsTrunkChanged(String),
    SubmitManualWs,
    SelectWorkspace(String),
    RemoveWorkspace(String),

    TriggerScan,
    ScanCompleted(Result<WorkspaceSnapshot, String>),

    SearchChanged(String),
    SetFilter(FilterMode),
    ToggleRepoSelect(String),
    ToggleSelectAll(Vec<String>),

    StartEditTrunk,
    CancelEditTrunk,
    TrunkInputChanged(String),
    SaveTrunk,
    ApplySuggestedTrunk(String),
    FocusAlerts,

    ToggleBatchBranchForm(bool),
    BatchBranchNameChanged(String),
    BatchBaseBranchChanged(String),
    SubmitBatchBranch,
    BatchPullTrunk,
    BatchOpDone(Vec<BatchOpResult>),
    ClearBatchResults,

    OpenRescue(GuardrailAlert),
    CancelRescue,
    RescueBranchChanged(String),
    ConfirmRescue,
    RescueDone(Result<String, String>),

    OpenRepoCi(String),
    SelectCiRepo(String),
    CiCustomCmdChanged(String),
    StartCiPipeline(PipelineDefinition),
    StartCiCustom,
    CancelCi,
    CopyCiLogs,
    ClearCiLogs,
    LoadCiHistoryLog(String),
    CiLogLoaded(Result<(String, String), String>),

    OpenRepoCommit(String),
    SelectCommitRepo(String),
    RefreshChangeset,
    ChangesetLoaded(Result<RepoChangeset, String>),
    ToggleCommitFile(String),
    ToggleAllCommitFiles,
    CommitMessageChanged(String),
    PrependCommitPrefix(String),
    ToggleAllowLarge(bool),
    SubmitCommit,
    CommitDone(Result<CommitResult, String>),
    SelectPushRemote(String),
    SubmitPush,
    PushDone(Result<PushResult, String>),
    ToggleLandRequireCi(bool),
    ToggleLandDeleteSource(bool),
    SubmitLand,
    LandDone(Result<LandResult, String>),
    EnableJjColocated,
    RunJjUndo,
    JjOpDone(Result<String, String>),
    CopyDiffText,

    OpenTaskBatchCommit(String),
    CancelTaskBatchCommit,
    TaskBatchMsgChanged(String),
    TaskBatchRemoteChanged(String),
    SubmitTaskBatchCommit,

    CopyMcpConfig,
    McpCopied,
    ClearToast,
}

impl KansoApp {
    pub fn new() -> (Self, Task<Message>) {
        let data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("kanso");
        let db = Arc::new(
            StorageDb::open(&data_dir.join("kanso.db")).expect("无法初始化 Kanso SQLite 数据库"),
        );
        let workspaces = db.list_workspaces().unwrap_or_default();
        let active_workspace_id = workspaces.first().map(|w| w.id.clone());
        let repo_latest_ci = db.latest_ci_status_by_repo().unwrap_or_default();
        let jj_version = detect_jj_version();
        let fs_dirty_flag = Arc::new(AtomicBool::new(false));

        let mut app = Self {
            db,
            workspaces,
            active_workspace_id: active_workspace_id.clone(),
            snapshot: None,
            active_tab: NavTab::Radar,
            scanning: false,
            watcher: None,
            fs_dirty_flag,
            search_query: String::new(),
            filter_mode: FilterMode::All,
            selected_repo_paths: HashSet::new(),
            show_manual_ws_form: false,
            manual_ws_path: String::new(),
            manual_ws_name: String::new(),
            manual_ws_trunk: String::new(),
            editing_trunk: false,
            trunk_input: String::new(),
            show_batch_branch_form: false,
            batch_branch_name: String::new(),
            batch_base_branch: String::new(),
            batch_results: None,
            rescue_target: None,
            rescue_branch_input: String::new(),
            selected_ci_repo: None,
            ci_pipelines: Vec::new(),
            ci_runs: Vec::new(),
            ci_custom_cmd: String::new(),
            active_ci_run_id: None,
            ci_running: false,
            ci_live_buf: Arc::new(Mutex::new(CiLiveBuffer::default())),
            ci_cancel_tx: None,
            displayed_logs: Vec::new(),
            displayed_run_status: None,
            repo_latest_ci,
            jj_version,
            selected_commit_repo: None,
            changeset: None,
            commit_loading: false,
            selected_commit_files: HashSet::new(),
            commit_message: String::new(),
            commit_allow_large: false,
            selected_push_remote: None,
            land_require_ci_pass: true,
            land_delete_source: false,
            task_batch_commit_branch: None,
            task_batch_commit_msg: String::new(),
            task_batch_push_remote: String::new(),
            toast: None,
            copied_mcp: false,
        };

        let init_task = if let Some(ws_id) = active_workspace_id {
            app.restart_watcher(&ws_id);
            app.spawn_scan(&ws_id)
        } else {
            Task::none()
        };

        (app, init_task)
    }

    pub fn theme(&self) -> Theme {
        theme::kanso_theme()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        if self.ci_running {
            time::every(Duration::from_millis(150)).map(|_| Message::Tick)
        } else if self.active_workspace_id.is_some() {
            time::every(Duration::from_millis(500)).map(|_| Message::Tick)
        } else {
            Subscription::none()
        }
    }

    pub fn current_workspace(&self) -> Option<&WorkspaceRecord> {
        let id = self.active_workspace_id.as_deref()?;
        self.workspaces.iter().find(|w| w.id == id)
    }

    fn restart_watcher(&mut self, workspace_id: &str) {
        self.watcher = None;
        let Some(ws) = self.workspaces.iter().find(|w| w.id == workspace_id) else {
            return;
        };
        let flag = Arc::clone(&self.fs_dirty_flag);
        if let Ok(watcher) = WorkspaceWatcher::start(Path::new(&ws.root_path), move || {
            flag.store(true, Ordering::Relaxed);
        }) {
            self.watcher = Some(watcher);
        }
    }

    fn spawn_scan(&mut self, workspace_id: &str) -> Task<Message> {
        let Some(ws) = self.workspaces.iter().find(|w| w.id == workspace_id).cloned() else {
            return Task::none();
        };
        self.scanning = true;
        let cfg = WorkspaceConfig {
            id: ws.id,
            name: ws.name,
            root_path: ws.root_path,
            default_trunk: ws.default_trunk,
            ignore_dirs: Vec::new(),
        };
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || scan_workspace(&cfg))
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())
            },
            Message::ScanCompleted,
        )
    }

    fn spawn_load_changeset(&mut self, repo_path: &str) -> Task<Message> {
        self.commit_loading = true;
        let path = PathBuf::from(repo_path);
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    inspect_repo_changes(&path).map_err(|e| e.to_string())
                })
                .await
                .map_err(|e| e.to_string())?
            },
            Message::ChangesetLoaded,
        )
    }

    fn refresh_ci_repo_context(&mut self) {
        let Some(repo_path) = self.selected_ci_repo.as_deref() else {
            self.ci_pipelines.clear();
            self.ci_runs.clear();
            return;
        };
        self.ci_pipelines = discover_pipelines(Path::new(repo_path));
        self.ci_runs = self.db.list_ci_runs(Some(repo_path), 20).unwrap_or_default();
        if let Ok(latest) = self.db.latest_ci_status_by_repo() {
            self.repo_latest_ci = latest;
        }
    }

    fn launch_local_ci(&mut self, pipeline: PipelineDefinition) {
        let Some(ws_id) = self.active_workspace_id.clone() else {
            self.toast = Some("请先选择一个多仓工作区".to_string());
            return;
        };
        let Some(repo_path) = self.selected_ci_repo.clone() else {
            self.toast = Some("请先选择目标仓库".to_string());
            return;
        };
        if self.ci_running {
            self.toast = Some("已有流水线正在运行，请先中止或等待完成".to_string());
            return;
        }

        let run_id = Uuid::new_v4().to_string();
        let command_summary = pipeline
            .steps
            .iter()
            .map(|s| s.command.as_str())
            .collect::<Vec<_>>()
            .join(" && ");

        if let Err(err) = self.db.insert_ci_run(NewCiRun {
            id: run_id.clone(),
            workspace_id: ws_id,
            repo_path: repo_path.clone(),
            pipeline_name: pipeline.name.clone(),
            command: command_summary,
            started_at: Utc::now().timestamp(),
        }) {
            self.toast = Some(format!("记录 CI 运行失败: {err}"));
            return;
        }

        if let Ok(mut buf) = self.ci_live_buf.lock() {
            buf.run_id = Some(run_id.clone());
            buf.lines.clear();
            buf.finished_status = None;
        }

        let (log_tx, mut log_rx) = mpsc::unbounded_channel::<kanso_ci::CiLogEvent>();
        let (cancel_tx, cancel_rx) = oneshot::channel();
        self.ci_cancel_tx = Some(cancel_tx);
        self.active_ci_run_id = Some(run_id.clone());
        self.ci_running = true;
        self.displayed_logs.clear();
        self.displayed_run_status = Some("running".to_string());

        let buf_for_log = Arc::clone(&self.ci_live_buf);
        tokio::spawn(async move {
            while let Some(ev) = log_rx.recv().await {
                if let Ok(mut buf) = buf_for_log.lock() {
                    if buf.run_id.as_deref() == Some(ev.run_id.as_str()) {
                        buf.lines.push(ev.line);
                    }
                }
            }
        });

        let db = Arc::clone(&self.db);
        let buf_for_done = Arc::clone(&self.ci_live_buf);
        let run_id_task = run_id.clone();
        tokio::spawn(async move {
            let outcome = LocalCiRunner
                .run_pipeline(
                    &run_id_task,
                    Path::new(&repo_path),
                    &pipeline,
                    log_tx,
                    cancel_rx,
                )
                .await;

            let (status, exit_code, duration_ms, full_log) = match outcome {
                Ok(out) => (out.status, out.exit_code, out.duration_ms, out.full_log),
                Err(err) => (
                    "failed".to_string(),
                    None,
                    0,
                    format!("!! 执行器异常: {err}\n"),
                ),
            };

            let _ = db.finish_ci_run(
                &run_id_task,
                &status,
                exit_code,
                Utc::now().timestamp(),
                duration_ms,
                &full_log,
            );

            if let Ok(mut buf) = buf_for_done.lock() {
                if buf.run_id.as_deref() == Some(run_id_task.as_str()) {
                    buf.finished_status = Some(status);
                }
            }
        });

        self.refresh_ci_repo_context();
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => {
                if self.ci_running {
                    let mut finished = None;
                    if let Ok(buf) = self.ci_live_buf.lock() {
                        if buf.run_id == self.active_ci_run_id {
                            if buf.lines.len() > self.displayed_logs.len() {
                                self.displayed_logs
                                    .extend_from_slice(&buf.lines[self.displayed_logs.len()..]);
                            }
                            if let Some(st) = &buf.finished_status {
                                finished = Some(st.clone());
                            }
                        }
                    }
                    if let Some(st) = finished {
                        self.ci_running = false;
                        self.ci_cancel_tx = None;
                        self.displayed_run_status = Some(st);
                        self.refresh_ci_repo_context();
                    }
                }

                if !self.scanning && self.fs_dirty_flag.swap(false, Ordering::Relaxed) {
                    if let Some(ws_id) = self.active_workspace_id.clone() {
                        return self.spawn_scan(&ws_id);
                    }
                }
                Task::none()
            }

            Message::SelectTab(tab) => {
                self.active_tab = tab;
                if tab == NavTab::Ci && self.selected_ci_repo.is_none() {
                    if let Some(first) = self.snapshot.as_ref().and_then(|s| s.repos.first()) {
                        self.selected_ci_repo = Some(first.path.clone());
                        self.refresh_ci_repo_context();
                    }
                }
                if tab == NavTab::Commit {
                    if let Some(target) = self.selected_commit_repo.clone().or_else(|| {
                        self.snapshot.as_ref().and_then(|s| {
                            s.repos
                                .iter()
                                .find(|r| r.is_dirty())
                                .or_else(|| s.repos.first())
                                .map(|r| r.path.clone())
                        })
                    }) {
                        self.selected_commit_repo = Some(target.clone());
                        return self.spawn_load_changeset(&target);
                    }
                }
                Task::none()
            }

            Message::PickWorkspaceFolder => Task::perform(
                async move {
                    let handle = rfd::AsyncFileDialog::new()
                        .set_title("选择包含多个 Git 仓库的工作区根目录")
                        .pick_folder()
                        .await;
                    handle.map(|h| h.path().to_path_buf())
                },
                Message::FolderPicked,
            ),

            Message::FolderPicked(Some(path)) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "workspace".to_string());
                let path_str = path.to_string_lossy().to_string();
                match self.db.add_workspace(&name, &path_str, None) {
                    Ok(ws) => {
                        if !self.workspaces.iter().any(|w| w.id == ws.id) {
                            self.workspaces.push(ws.clone());
                        }
                        self.active_workspace_id = Some(ws.id.clone());
                        self.restart_watcher(&ws.id);
                        return self.spawn_scan(&ws.id);
                    }
                    Err(err) => self.toast = Some(format!("添加工作区失败: {err}")),
                }
                Task::none()
            }

            Message::FolderPicked(None) => Task::none(),

            Message::ToggleManualWsForm(show) => {
                self.show_manual_ws_form = show;
                Task::none()
            }
            Message::ManualWsPathChanged(v) => {
                self.manual_ws_path = v;
                Task::none()
            }
            Message::ManualWsNameChanged(v) => {
                self.manual_ws_name = v;
                Task::none()
            }
            Message::ManualWsTrunkChanged(v) => {
                self.manual_ws_trunk = v;
                Task::none()
            }

            Message::SubmitManualWs => {
                let path = self.manual_ws_path.trim().to_string();
                if path.is_empty() {
                    return Task::none();
                }
                let name = if self.manual_ws_name.trim().is_empty() {
                    Path::new(&path)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "workspace".to_string())
                } else {
                    self.manual_ws_name.trim().to_string()
                };
                let trunk = self.manual_ws_trunk.trim();
                let trunk_opt = if trunk.is_empty() { None } else { Some(trunk) };

                match self.db.add_workspace(&name, &path, trunk_opt) {
                    Ok(ws) => {
                        if !self.workspaces.iter().any(|w| w.id == ws.id) {
                            self.workspaces.push(ws.clone());
                        }
                        self.show_manual_ws_form = false;
                        self.manual_ws_path.clear();
                        self.manual_ws_name.clear();
                        self.manual_ws_trunk.clear();
                        self.active_workspace_id = Some(ws.id.clone());
                        self.restart_watcher(&ws.id);
                        return self.spawn_scan(&ws.id);
                    }
                    Err(err) => self.toast = Some(format!("添加工作区失败: {err}")),
                }
                Task::none()
            }

            Message::SelectWorkspace(id) => {
                self.active_workspace_id = Some(id.clone());
                self.selected_repo_paths.clear();
                self.batch_results = None;
                self.selected_ci_repo = None;
                self.selected_commit_repo = None;
                self.changeset = None;
                self.restart_watcher(&id);
                self.spawn_scan(&id)
            }

            Message::RemoveWorkspace(id) => {
                if let Err(err) = self.db.remove_workspace(&id) {
                    self.toast = Some(format!("移除失败: {err}"));
                    return Task::none();
                }
                self.workspaces.retain(|w| w.id != id);
                if self.active_workspace_id.as_deref() == Some(id.as_str()) {
                    self.active_workspace_id = self.workspaces.first().map(|w| w.id.clone());
                    self.snapshot = None;
                    if let Some(next_id) = self.active_workspace_id.clone() {
                        self.restart_watcher(&next_id);
                        return self.spawn_scan(&next_id);
                    } else {
                        self.watcher = None;
                    }
                }
                Task::none()
            }

            Message::TriggerScan => {
                if let Some(id) = self.active_workspace_id.clone() {
                    return self.spawn_scan(&id);
                }
                Task::none()
            }

            Message::ScanCompleted(res) => {
                self.scanning = false;
                if let Ok(latest) = self.db.latest_ci_status_by_repo() {
                    self.repo_latest_ci = latest;
                }
                match res {
                    Ok(snap) => {
                        if let Some(sel) = &self.selected_ci_repo {
                            if !snap.repos.iter().any(|r| &r.path == sel) {
                                self.selected_ci_repo = snap.repos.first().map(|r| r.path.clone());
                                self.refresh_ci_repo_context();
                            }
                        } else if let Some(first) = snap.repos.first() {
                            self.selected_ci_repo = Some(first.path.clone());
                            self.refresh_ci_repo_context();
                        }
                        if self.selected_commit_repo.is_none()
                            || !snap
                                .repos
                                .iter()
                                .any(|r| Some(r.path.as_str()) == self.selected_commit_repo.as_deref())
                        {
                            self.selected_commit_repo = snap
                                .repos
                                .iter()
                                .find(|r| r.is_dirty())
                                .or_else(|| snap.repos.first())
                                .map(|r| r.path.clone());
                        }
                        self.snapshot = Some(snap);
                    }
                    Err(err) => self.toast = Some(format!("扫描失败: {err}")),
                }
                Task::none()
            }

            Message::SearchChanged(q) => {
                self.search_query = q;
                Task::none()
            }

            Message::SetFilter(mode) => {
                self.filter_mode = mode;
                Task::none()
            }

            Message::ToggleRepoSelect(path) => {
                if !self.selected_repo_paths.remove(&path) {
                    self.selected_repo_paths.insert(path);
                }
                Task::none()
            }

            Message::ToggleSelectAll(visible_paths) => {
                let all_selected = !visible_paths.is_empty()
                    && visible_paths
                        .iter()
                        .all(|p| self.selected_repo_paths.contains(p));
                if all_selected {
                    for p in visible_paths {
                        self.selected_repo_paths.remove(&p);
                    }
                } else {
                    for p in visible_paths {
                        self.selected_repo_paths.insert(p);
                    }
                }
                Task::none()
            }

            Message::StartEditTrunk => {
                self.trunk_input = self
                    .current_workspace()
                    .and_then(|w| w.default_trunk.clone())
                    .unwrap_or_default();
                self.editing_trunk = true;
                Task::none()
            }

            Message::CancelEditTrunk => {
                self.editing_trunk = false;
                Task::none()
            }

            Message::TrunkInputChanged(v) => {
                self.trunk_input = v;
                Task::none()
            }

            Message::SaveTrunk => {
                if let Some(id) = self.active_workspace_id.clone() {
                    let trunk = self.trunk_input.trim();
                    let trunk_opt = if trunk.is_empty() { None } else { Some(trunk) };
                    match self.db.update_workspace_trunk(&id, trunk_opt) {
                        Ok(updated) => {
                            if let Some(slot) = self.workspaces.iter_mut().find(|w| w.id == id) {
                                *slot = updated;
                            }
                            self.editing_trunk = false;
                            return self.spawn_scan(&id);
                        }
                        Err(err) => self.toast = Some(format!("更新默认主干失败: {err}")),
                    }
                }
                Task::none()
            }

            Message::ApplySuggestedTrunk(branch) => {
                if let Some(id) = self.active_workspace_id.clone() {
                    match self.db.update_workspace_trunk(&id, Some(&branch)) {
                        Ok(updated) => {
                            if let Some(slot) = self.workspaces.iter_mut().find(|w| w.id == id) {
                                *slot = updated;
                            }
                            self.toast = Some(format!("已将默认主干设为 {branch}"));
                            return self.spawn_scan(&id);
                        }
                        Err(err) => self.toast = Some(format!("设置默认主干失败: {err}")),
                    }
                }
                Task::none()
            }

            Message::FocusAlerts => {
                self.active_tab = NavTab::Radar;
                self.filter_mode = FilterMode::Alerts;
                Task::none()
            }

            Message::ToggleBatchBranchForm(show) => {
                if show && self.selected_repo_paths.is_empty() {
                    self.toast = Some("请先在下方列表勾选需要创建分支的仓库".to_string());
                    self.show_batch_branch_form = false;
                } else {
                    self.show_batch_branch_form = show;
                }
                Task::none()
            }
            Message::BatchBranchNameChanged(v) => {
                self.batch_branch_name = v;
                Task::none()
            }
            Message::BatchBaseBranchChanged(v) => {
                self.batch_base_branch = v;
                Task::none()
            }

            Message::SubmitBatchBranch => {
                let branch = self.batch_branch_name.trim().to_string();
                if branch.is_empty() || self.selected_repo_paths.is_empty() {
                    return Task::none();
                }
                let paths: Vec<PathBuf> =
                    self.selected_repo_paths.iter().map(PathBuf::from).collect();
                let base = self.batch_base_branch.trim().to_string();
                let base_opt = if base.is_empty() { None } else { Some(base) };
                self.show_batch_branch_form = false;
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            batch_create_branch(&paths, &branch, base_opt.as_deref())
                        })
                        .await
                        .unwrap_or_default()
                    },
                    Message::BatchOpDone,
                )
            }

            Message::BatchPullTrunk => {
                let Some(snap) = &self.snapshot else {
                    return Task::none();
                };
                let targets: Vec<(PathBuf, String)> = snap
                    .repos
                    .iter()
                    .filter_map(|r| {
                        r.trunk_branch
                            .as_ref()
                            .map(|trunk| (PathBuf::from(&r.path), trunk.clone()))
                    })
                    .collect();
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || batch_pull_trunk(&targets))
                            .await
                            .unwrap_or_default()
                    },
                    Message::BatchOpDone,
                )
            }

            Message::BatchOpDone(results) => {
                self.batch_results = Some(results);
                self.selected_repo_paths.clear();
                self.batch_branch_name.clear();
                self.batch_base_branch.clear();
                if let Some(id) = self.active_workspace_id.clone() {
                    return self.spawn_scan(&id);
                }
                Task::none()
            }

            Message::ClearBatchResults => {
                self.batch_results = None;
                Task::none()
            }

            Message::OpenRescue(alert) => {
                self.rescue_branch_input = format!("feat/rescue-{}", alert.repo_name);
                self.rescue_target = Some(alert);
                Task::none()
            }

            Message::CancelRescue => {
                self.rescue_target = None;
                Task::none()
            }

            Message::RescueBranchChanged(v) => {
                self.rescue_branch_input = v;
                Task::none()
            }

            Message::ConfirmRescue => {
                let Some(alert) = self.rescue_target.take() else {
                    return Task::none();
                };
                let new_branch = self.rescue_branch_input.trim().to_string();
                if new_branch.is_empty() {
                    return Task::none();
                }
                let repo_path = PathBuf::from(alert.repo_path);
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            rescue_dirty_trunk(&repo_path, &new_branch).map_err(|e| e.to_string())
                        })
                        .await
                        .map_err(|e| e.to_string())?
                    },
                    Message::RescueDone,
                )
            }

            Message::RescueDone(res) => {
                match res {
                    Ok(msg) => {
                        self.toast = Some(msg);
                        if let Some(id) = self.active_workspace_id.clone() {
                            return self.spawn_scan(&id);
                        }
                    }
                    Err(err) => self.toast = Some(format!("主干救场失败: {err}")),
                }
                Task::none()
            }

            Message::OpenRepoCi(path) => {
                self.selected_ci_repo = Some(path);
                self.active_tab = NavTab::Ci;
                self.refresh_ci_repo_context();
                Task::none()
            }

            Message::SelectCiRepo(path) => {
                self.selected_ci_repo = Some(path);
                self.refresh_ci_repo_context();
                Task::none()
            }

            Message::CiCustomCmdChanged(cmd) => {
                self.ci_custom_cmd = cmd;
                Task::none()
            }

            Message::StartCiPipeline(pipeline) => {
                self.launch_local_ci(pipeline);
                Task::none()
            }

            Message::StartCiCustom => {
                let cmd = self.ci_custom_cmd.trim().to_string();
                if cmd.is_empty() {
                    return Task::none();
                }
                self.launch_local_ci(PipelineDefinition {
                    name: "自定义命令".to_string(),
                    source: "custom".to_string(),
                    steps: vec![PipelineStep {
                        name: "Shell".to_string(),
                        command: cmd,
                        working_dir: None,
                    }],
                });
                Task::none()
            }

            Message::CancelCi => {
                if let Some(tx) = self.ci_cancel_tx.take() {
                    let _ = tx.send(());
                }
                Task::none()
            }

            Message::CopyCiLogs => {
                if self.displayed_logs.is_empty() {
                    self.toast = Some("当前暂无可复制的日志".to_string());
                    return Task::none();
                }
                let text = self.displayed_logs.join("\n");
                self.toast = Some("已复制 CI 日志到剪贴板".to_string());
                iced::clipboard::write(text)
            }

            Message::ClearCiLogs => {
                self.displayed_logs.clear();
                self.displayed_run_status = None;
                self.active_ci_run_id = None;
                Task::none()
            }

            Message::LoadCiHistoryLog(run_id) => {
                let db = Arc::clone(&self.db);
                let run_status = self
                    .ci_runs
                    .iter()
                    .find(|r| r.id == run_id)
                    .map(|r| r.status.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                self.active_ci_run_id = Some(run_id.clone());
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            db.get_ci_run_log(&run_id)
                                .map(|log| (run_status, log))
                                .map_err(|e| e.to_string())
                        })
                        .await
                        .map_err(|e| e.to_string())?
                    },
                    Message::CiLogLoaded,
                )
            }

            Message::CiLogLoaded(res) => {
                match res {
                    Ok((status, log)) => {
                        self.displayed_run_status = Some(status);
                        self.displayed_logs = if log.is_empty() {
                            Vec::new()
                        } else {
                            log.lines().map(str::to_string).collect()
                        };
                    }
                    Err(err) => self.toast = Some(format!("读取压缩日志失败: {err}")),
                }
                Task::none()
            }

            Message::OpenRepoCommit(path) => {
                self.selected_commit_repo = Some(path.clone());
                self.active_tab = NavTab::Commit;
                self.spawn_load_changeset(&path)
            }

            Message::SelectCommitRepo(path) => {
                self.selected_commit_repo = Some(path.clone());
                self.spawn_load_changeset(&path)
            }

            Message::RefreshChangeset => {
                if let Some(path) = self.selected_commit_repo.clone() {
                    return self.spawn_load_changeset(&path);
                }
                Task::none()
            }

            Message::ChangesetLoaded(res) => {
                self.commit_loading = false;
                match res {
                    Ok(cs) => {
                        self.selected_commit_files =
                            cs.files.iter().map(|f| f.path.clone()).collect();
                        if self
                            .selected_push_remote
                            .as_deref()
                            .is_none_or(|rm| !cs.remotes.iter().any(|r| r == rm))
                        {
                            self.selected_push_remote = cs.remotes.first().cloned();
                        }
                        self.changeset = Some(cs);
                    }
                    Err(err) => self.toast = Some(format!("读取仓库变更失败: {err}")),
                }
                Task::none()
            }

            Message::ToggleCommitFile(path) => {
                if !self.selected_commit_files.remove(&path) {
                    self.selected_commit_files.insert(path);
                }
                Task::none()
            }

            Message::ToggleAllCommitFiles => {
                if let Some(cs) = &self.changeset {
                    let all = !cs.files.is_empty()
                        && cs
                            .files
                            .iter()
                            .all(|f| self.selected_commit_files.contains(&f.path));
                    if all {
                        self.selected_commit_files.clear();
                    } else {
                        self.selected_commit_files =
                            cs.files.iter().map(|f| f.path.clone()).collect();
                    }
                }
                Task::none()
            }

            Message::CommitMessageChanged(msg) => {
                self.commit_message = msg;
                Task::none()
            }

            Message::PrependCommitPrefix(prefix) => {
                if !self.commit_message.starts_with(&prefix) {
                    self.commit_message = format!("{prefix}{}", self.commit_message);
                }
                Task::none()
            }

            Message::ToggleAllowLarge(allow) => {
                self.commit_allow_large = allow;
                Task::none()
            }

            Message::SubmitCommit => {
                let Some(repo_path) = self.selected_commit_repo.clone() else {
                    return Task::none();
                };
                let selected_files: Vec<String> =
                    self.selected_commit_files.iter().cloned().collect();
                let req = CommitRequest {
                    repo_path,
                    message: self.commit_message.clone(),
                    selected_files,
                    author_name: None,
                    author_email: None,
                    allow_large_files: self.commit_allow_large,
                };
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            commit_repo(&req).map_err(|e| e.to_string())
                        })
                        .await
                        .map_err(|e| e.to_string())?
                    },
                    Message::CommitDone,
                )
            }

            Message::CommitDone(res) => match res {
                Ok(out) => {
                    self.commit_message.clear();
                    self.toast = Some(out.summary);
                    let mut tasks = Vec::new();
                    tasks.push(self.spawn_load_changeset(&out.repo_path));
                    if let Some(ws_id) = self.active_workspace_id.clone() {
                        tasks.push(self.spawn_scan(&ws_id));
                    }
                    Task::batch(tasks)
                }
                Err(err) => {
                    self.toast = Some(err);
                    Task::none()
                }
            },

            Message::SelectPushRemote(remote) => {
                self.selected_push_remote = Some(remote);
                Task::none()
            }

            Message::SubmitPush => {
                let Some(repo_path) = self.selected_commit_repo.clone() else {
                    return Task::none();
                };
                let Some(remote) = self.selected_push_remote.clone() else {
                    self.toast = Some("请先选择远端仓库".to_string());
                    return Task::none();
                };
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            push_repo(Path::new(&repo_path), &remote, None)
                                .map_err(|e| e.to_string())
                        })
                        .await
                        .map_err(|e| e.to_string())?
                    },
                    Message::PushDone,
                )
            }

            Message::PushDone(res) => match res {
                Ok(out) => {
                    self.toast = Some(out.summary);
                    if let Some(ws_id) = self.active_workspace_id.clone() {
                        return self.spawn_scan(&ws_id);
                    }
                    Task::none()
                }
                Err(err) => {
                    self.toast = Some(err);
                    Task::none()
                }
            },

            Message::ToggleLandRequireCi(v) => {
                self.land_require_ci_pass = v;
                Task::none()
            }

            Message::ToggleLandDeleteSource(v) => {
                self.land_delete_source = v;
                Task::none()
            }

            Message::SubmitLand => {
                let Some(cs) = &self.changeset else {
                    return Task::none();
                };
                let Some(trunk) = cs.trunk_branch.clone() else {
                    self.toast = Some("未识别该仓库主干分支，请先设置工作区默认主干".to_string());
                    return Task::none();
                };
                if self.land_require_ci_pass
                    && self.repo_latest_ci.get(&cs.repo_path).map(String::as_str) != Some("passed")
                {
                    self.toast = Some(
                        "已开启「CI 绿灯门禁」：请先通过本机 CI 验证后再合入主干".to_string(),
                    );
                    return Task::none();
                }
                let repo_path = cs.repo_path.clone();
                let delete_source = self.land_delete_source;
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            land_branch_to_trunk(Path::new(&repo_path), &trunk, delete_source)
                                .map_err(|e| e.to_string())
                        })
                        .await
                        .map_err(|e| e.to_string())?
                    },
                    Message::LandDone,
                )
            }

            Message::LandDone(res) => match res {
                Ok(out) => {
                    self.toast = Some(out.summary);
                    let mut tasks = Vec::new();
                    tasks.push(self.spawn_load_changeset(&out.repo_path));
                    if let Some(ws_id) = self.active_workspace_id.clone() {
                        tasks.push(self.spawn_scan(&ws_id));
                    }
                    Task::batch(tasks)
                }
                Err(err) => {
                    self.toast = Some(err);
                    Task::none()
                }
            },

            Message::EnableJjColocated => {
                let Some(repo_path) = self.selected_commit_repo.clone() else {
                    return Task::none();
                };
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            enable_jj_colocated(Path::new(&repo_path)).map_err(|e| e.to_string())
                        })
                        .await
                        .map_err(|e| e.to_string())?
                    },
                    Message::JjOpDone,
                )
            }

            Message::RunJjUndo => {
                let Some(repo_path) = self.selected_commit_repo.clone() else {
                    return Task::none();
                };
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            jj_undo(Path::new(&repo_path)).map_err(|e| e.to_string())
                        })
                        .await
                        .map_err(|e| e.to_string())?
                    },
                    Message::JjOpDone,
                )
            }

            Message::JjOpDone(res) => match res {
                Ok(msg) => {
                    self.toast = Some(msg);
                    let mut tasks = Vec::new();
                    if let Some(path) = self.selected_commit_repo.clone() {
                        tasks.push(self.spawn_load_changeset(&path));
                    }
                    if let Some(ws_id) = self.active_workspace_id.clone() {
                        tasks.push(self.spawn_scan(&ws_id));
                    }
                    Task::batch(tasks)
                }
                Err(err) => {
                    self.toast = Some(err);
                    Task::none()
                }
            },

            Message::CopyDiffText => {
                let Some(cs) = &self.changeset else {
                    return Task::none();
                };
                if cs.diff_text.trim().is_empty() {
                    self.toast = Some("当前无 Diff 可复制".to_string());
                    return Task::none();
                }
                self.toast = Some("已复制 Diff 到剪贴板".to_string());
                iced::clipboard::write(cs.diff_text.clone())
            }

            Message::OpenTaskBatchCommit(branch) => {
                self.task_batch_commit_branch = Some(branch);
                self.task_batch_commit_msg.clear();
                Task::none()
            }

            Message::CancelTaskBatchCommit => {
                self.task_batch_commit_branch = None;
                Task::none()
            }

            Message::TaskBatchMsgChanged(msg) => {
                self.task_batch_commit_msg = msg;
                Task::none()
            }

            Message::TaskBatchRemoteChanged(remote) => {
                self.task_batch_push_remote = remote;
                Task::none()
            }

            Message::SubmitTaskBatchCommit => {
                let Some(branch) = self.task_batch_commit_branch.clone() else {
                    return Task::none();
                };
                let Some(snap) = &self.snapshot else {
                    return Task::none();
                };
                let msg = self.task_batch_commit_msg.trim().to_string();
                if msg.is_empty() {
                    self.toast = Some("跨仓提交说明不能为空".to_string());
                    return Task::none();
                }
                let reqs: Vec<CommitRequest> = snap
                    .repos
                    .iter()
                    .filter(|r| !r.is_on_trunk && r.current_branch == branch && r.is_dirty())
                    .map(|r| CommitRequest {
                        repo_path: r.path.clone(),
                        message: msg.clone(),
                        selected_files: Vec::new(),
                        author_name: None,
                        author_email: None,
                        allow_large_files: false,
                    })
                    .collect();

                if reqs.is_empty() {
                    self.toast = Some("该任务下暂无需要提交的脏仓库".to_string());
                    return Task::none();
                }
                let remote = self.task_batch_push_remote.trim().to_string();
                let remote_opt = if remote.is_empty() { None } else { Some(remote) };
                self.task_batch_commit_branch = None;

                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            batch_commit_and_push(&reqs, remote_opt.as_deref())
                        })
                        .await
                        .unwrap_or_default()
                    },
                    Message::BatchOpDone,
                )
            }

            Message::CopyMcpConfig => {
                let snippet = serde_json::to_string_pretty(&serde_json::json!({
                    "mcpServers": {
                        "kanso": {
                            "command": "kanso",
                            "args": ["mcp"]
                        }
                    }
                }))
                .unwrap_or_default();
                self.copied_mcp = true;
                self.toast = Some("已复制 MCP 配置 JSON 到剪贴板".to_string());
                iced::clipboard::write(snippet).chain(Task::perform(
                    async {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    },
                    |_| Message::McpCopied,
                ))
            }

            Message::McpCopied => {
                self.copied_mcp = false;
                Task::none()
            }

            Message::ClearToast => {
                self.toast = None;
                Task::none()
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let sidebar = views::sidebar::view(self);
        let guardrail = views::guardrail::view(self);

        let body: Element<'_, Message> = match self.active_tab {
            NavTab::Radar => views::radar::view(self),
            NavTab::Tasks => views::tasks::view(self),
            NavTab::Commit => views::commit::view(self),
            NavTab::Ci => views::ci::view(self),
        };

        let main_col = column![guardrail, body]
            .width(Length::Fill)
            .height(Length::Fill);

        let main_area = container(main_col)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::panel_style);

        let layout = row![sidebar, theme::divider_v(), main_area]
            .width(Length::Fill)
            .height(Length::Fill);

        if let Some(toast) = &self.toast {
            let toast_bar = container(
                row![
                    text(toast.as_str()).size(12).color(theme::TEXT_PRIMARY),
                    container(text("").size(1)).width(Length::Fill),
                    iced::widget::button(text("关闭").size(11).color(theme::ACCENT_SKY))
                        .on_press(Message::ClearToast)
                        .style(theme::secondary_btn_style)
                        .padding([3, 8]),
                ]
                .align_y(iced::Alignment::Center),
            )
            .width(Length::Fill)
            .padding([8, 16])
            .style(theme::elevated_style);

            column![layout, toast_bar]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            layout.into()
        }
    }
}
