use anyhow::{Context, Result};
use chrono::Utc;
use kanso_core::{
    GuardrailAlert, GuardrailKind, RepoFailure, RepoStatus, TaskGroup, WorkspaceSnapshot,
    aggregate_tasks, detect_alerts,
};
use kanso_engine::EngineEvent;
use kanso_storage::WorkspaceRecord;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

pub const MAX_LOG_ENTRIES: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Overview,
    Repos,
    Tasks,
    CrossCommit,
    Pipeline,
}

impl Route {
    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "概览",
            Self::Repos => "仓库",
            Self::Tasks => "任务",
            Self::CrossCommit => "跨仓提交",
            Self::Pipeline => "流水线",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoFilter {
    All,
    Dirty,
    Alerts,
    OffTrunk,
    Errored,
}

impl RepoFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Dirty => "有改动",
            Self::Alerts => "仅告警",
            Self::OffTrunk => "非主干",
            Self::Errored => "刷新失败",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BottomTab {
    Tasks,
    Log,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub timestamp: i64,
    pub level: LogLevel,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BgTask {
    pub id: String,
    pub label: String,
    pub running: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "跟随系统",
            Self::Light => "浅色主题",
            Self::Dark => "深色主题",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::System => Self::Dark,
            Self::Dark => Self::Light,
            Self::Light => Self::System,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub theme: ThemeChoice,
}

pub fn app_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("kanso")
}

pub fn logs_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Library")
            .join("Logs")
            .join("Kanso")
    }
    #[cfg(not(target_os = "macos"))]
    {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Kanso")
            .join("logs")
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        app_data_dir().join("settings.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        let Ok(raw) = fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("创建设置目录失败: {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json).with_context(|| format!("写入设置文件失败: {}", path.display()))?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionItem {
    pub is_failure: bool,
    pub repo_path: String,
    pub repo_name: String,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteAction {
    RefreshAll,
    SetTheme(ThemeChoice),
    ToggleInspector,
    ToggleBottomPanel,
    OpenWorkspacePicker,
    OpenLogsDir,
    Navigate(Route),
    SelectRepo(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteItem {
    pub id: String,
    pub label: String,
    pub subtitle: String,
    pub shortcut: Option<&'static str>,
    pub action: PaletteAction,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub workspace: Option<WorkspaceRecord>,
    pub snapshot: Option<WorkspaceSnapshot>,
    pub verifying: bool,
    pub route: Route,
    pub filter: RepoFilter,
    pub selected: Option<String>,
    pub inspector_open: bool,
    pub bottom_open: bool,
    pub bottom_tab: BottomTab,
    pub log: VecDeque<LogEntry>,
    pub bg_tasks: Vec<BgTask>,
    pub last_refresh_at: Option<i64>,
    pub git_version: String,
    pub jj_version: Option<String>,
    pub db_error: Option<String>,
    pub recent_palette_ids: Vec<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            workspace: None,
            snapshot: None,
            verifying: false,
            route: Route::Repos,
            filter: RepoFilter::All,
            selected: None,
            inspector_open: true,
            bottom_open: false,
            bottom_tab: BottomTab::Log,
            log: VecDeque::with_capacity(MAX_LOG_ENTRIES),
            bg_tasks: Vec::new(),
            last_refresh_at: None,
            git_version: "未知".to_string(),
            jj_version: None,
            db_error: None,
            recent_palette_ids: Vec::new(),
        }
    }
}

impl AppState {
    pub fn push_log(&mut self, level: LogLevel, message: impl Into<String>) {
        if self.log.len() >= MAX_LOG_ENTRIES {
            self.log.pop_front();
        }
        self.log.push_back(LogEntry {
            timestamp: Utc::now().timestamp(),
            level,
            message: message.into(),
        });
    }

    pub fn set_bg_task(&mut self, id: &str, label: &str, running: bool, detail: impl Into<String>) {
        let detail = detail.into();
        if let Some(existing) = self.bg_tasks.iter_mut().find(|t| t.id == id) {
            existing.label = label.to_string();
            existing.running = running;
            existing.detail = detail;
        } else {
            self.bg_tasks.push(BgTask {
                id: id.to_string(),
                label: label.to_string(),
                running,
                detail,
            });
        }
    }

    fn ensure_snapshot_mut(&mut self) -> Option<&mut WorkspaceSnapshot> {
        if self.snapshot.is_none() {
            if let Some(ws) = &self.workspace {
                self.snapshot = Some(WorkspaceSnapshot {
                    workspace_id: ws.id.clone(),
                    workspace_name: ws.name.clone(),
                    root_path: ws.root_path.clone(),
                    scanned_at: Utc::now().timestamp(),
                    scan_duration_ms: 0,
                    repos: Vec::new(),
                    failures: Vec::new(),
                    tasks: Vec::new(),
                    alerts: Vec::new(),
                });
            }
        }
        self.snapshot.as_mut()
    }

    fn recompute_derived(&mut self) {
        if let Some(snap) = self.snapshot.as_mut() {
            snap.tasks = aggregate_tasks(&snap.repos);
            snap.alerts = detect_alerts(&snap.repos);
        }
    }

    pub fn apply_event(&mut self, event: EngineEvent) {
        let now = Utc::now().timestamp();
        match event {
            EngineEvent::SnapshotReady(snap) => {
                let repos_len = snap.repos.len();
                let fail_len = snap.failures.len();
                let dur = snap.scan_duration_ms;
                let first_path = snap.repos.first().map(|r| r.path.clone());
                self.snapshot = Some(snap);
                self.verifying = false;
                self.last_refresh_at = Some(now);
                if self.selected.is_none() {
                    self.selected = first_path;
                }
                self.set_bg_task(
                    "engine-sync",
                    "工作区全量校对",
                    false,
                    format!("{repos_len} 个仓库，{fail_len} 个失败 ({dur} ms)"),
                );
                self.push_log(
                    LogLevel::Info,
                    format!("全量扫描完成：{repos_len} 个仓库，{fail_len} 个失败，耗时 {dur} ms"),
                );
            }
            EngineEvent::RepoUpdated(repo) => {
                let repo_path = repo.path.clone();
                let repo_name = repo.name.clone();
                let branch = repo.current_branch.clone();
                if let Some(snap) = self.ensure_snapshot_mut() {
                    snap.failures.retain(|f| f.repo_path != repo_path);
                    if let Some(slot) = snap.repos.iter_mut().find(|r| r.path == repo_path) {
                        *slot = repo;
                    } else {
                        snap.repos.push(repo);
                    }
                    snap.repos.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
                    snap.scanned_at = now;
                }
                self.recompute_derived();
                self.last_refresh_at = Some(now);
                if self.selected.is_none() {
                    self.selected = Some(repo_path);
                }
                self.push_log(
                    LogLevel::Info,
                    format!("增量刷新：{repo_name} ({branch})"),
                );
            }
            EngineEvent::RepoFailed(fail) => {
                let repo_path = fail.repo_path.clone();
                let rel = fail.relative_path.clone();
                let err = fail.error.clone();
                if let Some(snap) = self.ensure_snapshot_mut() {
                    snap.repos.retain(|r| r.path != repo_path);
                    if let Some(slot) = snap.failures.iter_mut().find(|f| f.repo_path == repo_path) {
                        *slot = fail;
                    } else {
                        snap.failures.push(fail);
                    }
                    snap.failures
                        .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
                    snap.scanned_at = now;
                }
                self.recompute_derived();
                self.last_refresh_at = Some(now);
                self.push_log(LogLevel::Error, format!("仓库刷新失败 [{rel}]：{err}"));
            }
            EngineEvent::ReposChanged { added, removed } => {
                let add_n = added.len();
                let rem_n = removed.len();
                if let Some(snap) = self.snapshot.as_mut() {
                    if !removed.is_empty() {
                        let rem_set: HashSet<&str> = removed.iter().map(String::as_str).collect();
                        snap.repos.retain(|r| !rem_set.contains(r.path.as_str()));
                        snap.failures
                            .retain(|f| !rem_set.contains(f.repo_path.as_str()));
                    }
                }
                if rem_n > 0 {
                    self.recompute_derived();
                }
                self.push_log(
                    LogLevel::Info,
                    format!("仓库集合变更：新增 {add_n} 个，移除 {rem_n} 个"),
                );
            }
            EngineEvent::Error(msg) => {
                self.push_log(LogLevel::Error, format!("引擎告警：{msg}"));
            }
        }
    }

    pub fn filtered_repos(&self) -> Vec<&RepoStatus> {
        let Some(snap) = &self.snapshot else {
            return Vec::new();
        };
        let alert_paths: HashSet<&str> = snap
            .alerts
            .iter()
            .map(|a| a.repo_path.as_str())
            .collect();

        snap.repos
            .iter()
            .filter(|r| match self.filter {
                RepoFilter::All => true,
                RepoFilter::Dirty => r.is_dirty(),
                RepoFilter::Alerts => alert_paths.contains(r.path.as_str()),
                RepoFilter::OffTrunk => !r.is_on_trunk,
                RepoFilter::Errored => false,
            })
            .collect()
    }

    pub fn filtered_failures(&self) -> Vec<&RepoFailure> {
        let Some(snap) = &self.snapshot else {
            return Vec::new();
        };
        match self.filter {
            RepoFilter::All | RepoFilter::Errored => snap.failures.iter().collect(),
            RepoFilter::Dirty | RepoFilter::Alerts | RepoFilter::OffTrunk => Vec::new(),
        }
    }

    pub fn repo_tree(&self) -> Vec<(String, Vec<&RepoStatus>)> {
        let Some(snap) = &self.snapshot else {
            return Vec::new();
        };
        let mut groups: BTreeMap<String, Vec<&RepoStatus>> = BTreeMap::new();
        for repo in &snap.repos {
            let parent = Path::new(&repo.relative_path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .filter(|s| !s.is_empty() && s != ".")
                .unwrap_or_else(|| "根目录".to_string());
            groups.entry(parent).or_default().push(repo);
        }
        groups.into_iter().collect()
    }

    pub fn select_next(&mut self) {
        let paths: Vec<String> = self
            .filtered_repos()
            .into_iter()
            .map(|r| r.path.clone())
            .collect();
        if paths.is_empty() {
            return;
        }
        let next_idx = match &self.selected {
            Some(cur) => match paths.iter().position(|p| p == cur) {
                Some(idx) => (idx + 1).min(paths.len() - 1),
                None => 0,
            },
            None => 0,
        };
        self.selected = Some(paths[next_idx].clone());
    }

    pub fn select_prev(&mut self) {
        let paths: Vec<String> = self
            .filtered_repos()
            .into_iter()
            .map(|r| r.path.clone())
            .collect();
        if paths.is_empty() {
            return;
        }
        let prev_idx = match &self.selected {
            Some(cur) => match paths.iter().position(|p| p == cur) {
                Some(idx) => idx.saturating_sub(1),
                None => 0,
            },
            None => 0,
        };
        self.selected = Some(paths[prev_idx].clone());
    }

    pub fn selected_repo(&self) -> Option<&RepoStatus> {
        let path = self.selected.as_deref()?;
        self.snapshot
            .as_ref()?
            .repos
            .iter()
            .find(|r| r.path == path)
    }

    pub fn selected_failure(&self) -> Option<&RepoFailure> {
        let path = self.selected.as_deref()?;
        self.snapshot
            .as_ref()?
            .failures
            .iter()
            .find(|f| f.repo_path == path)
    }

    pub fn attention_queue(&self) -> Vec<AttentionItem> {
        let Some(snap) = &self.snapshot else {
            return Vec::new();
        };
        let mut items = Vec::with_capacity(snap.failures.len() + snap.alerts.len());

        for fail in &snap.failures {
            items.push(AttentionItem {
                is_failure: true,
                repo_path: fail.repo_path.clone(),
                repo_name: fail.relative_path.clone(),
                title: "仓库扫描失败".to_string(),
                detail: fail.error.clone(),
            });
        }

        let mut sorted_alerts: Vec<&GuardrailAlert> = snap.alerts.iter().collect();
        sorted_alerts.sort_by_key(|a| alert_priority(&a.kind));

        for alert in sorted_alerts {
            let (title, detail) = match &alert.kind {
                GuardrailKind::DirtyTrunk => (
                    "主干分支存在未提交改动".to_string(),
                    format!(
                        "{} 有 {} 个文件在主干({})被修改",
                        alert.repo_name,
                        alert.dirty_files,
                        alert.trunk_branch.as_deref().unwrap_or("trunk")
                    ),
                ),
                GuardrailKind::MergeConflict => (
                    "存在未解决的合并冲突".to_string(),
                    format!("{} 有 {} 个冲突文件", alert.repo_name, alert.dirty_files),
                ),
                GuardrailKind::DetachedHead => (
                    "处于游离 HEAD 状态".to_string(),
                    format!("{} 未检出命名分支", alert.repo_name),
                ),
            };
            items.push(AttentionItem {
                is_failure: false,
                repo_path: alert.repo_path.clone(),
                repo_name: alert.repo_name.clone(),
                title,
                detail,
            });
        }

        items
    }

    pub fn record_palette_use(&mut self, id: &str) {
        self.recent_palette_ids.retain(|x| x != id);
        self.recent_palette_ids.insert(0, id.to_string());
        if self.recent_palette_ids.len() > 20 {
            self.recent_palette_ids.truncate(20);
        }
    }

    pub fn palette_items(&self, query: &str) -> Vec<PaletteItem> {
        let mut all = vec![
            PaletteItem {
                id: "act:refresh_all".to_string(),
                label: "刷新全部仓库".to_string(),
                subtitle: "重新校准仓库列表并执行全量刷新".to_string(),
                shortcut: Some("⌘R"),
                action: PaletteAction::RefreshAll,
            },
            PaletteItem {
                id: "act:theme_system".to_string(),
                label: "切换为跟随系统主题".to_string(),
                subtitle: "外观设置：自动适配系统浅色/深色".to_string(),
                shortcut: None,
                action: PaletteAction::SetTheme(ThemeChoice::System),
            },
            PaletteItem {
                id: "act:theme_dark".to_string(),
                label: "切换为深色主题".to_string(),
                subtitle: "外观设置：立即应用 Dark 外观".to_string(),
                shortcut: None,
                action: PaletteAction::SetTheme(ThemeChoice::Dark),
            },
            PaletteItem {
                id: "act:theme_light".to_string(),
                label: "切换为浅色主题".to_string(),
                subtitle: "外观设置：立即应用 Light 外观".to_string(),
                shortcut: None,
                action: PaletteAction::SetTheme(ThemeChoice::Light),
            },
            PaletteItem {
                id: "act:toggle_inspector".to_string(),
                label: "切换右侧检查器".to_string(),
                subtitle: "显示或折叠选中仓库摘要面板".to_string(),
                shortcut: Some("⌘⌥I"),
                action: PaletteAction::ToggleInspector,
            },
            PaletteItem {
                id: "act:toggle_bottom".to_string(),
                label: "切换底部任务与日志面板".to_string(),
                subtitle: "显示或折叠后台任务与操作日志".to_string(),
                shortcut: Some("⌘J"),
                action: PaletteAction::ToggleBottomPanel,
            },
            PaletteItem {
                id: "act:open_workspace".to_string(),
                label: "打开工作区目录...".to_string(),
                subtitle: "选择新的多仓库根目录".to_string(),
                shortcut: None,
                action: PaletteAction::OpenWorkspacePicker,
            },
            PaletteItem {
                id: "act:open_logs".to_string(),
                label: "打开日志目录".to_string(),
                subtitle: "在文件管理器中查看滚动诊断日志".to_string(),
                shortcut: None,
                action: PaletteAction::OpenLogsDir,
            },
            PaletteItem {
                id: "nav:overview".to_string(),
                label: "跳转到：概览".to_string(),
                subtitle: "查看全局健康指标与待处理队列".to_string(),
                shortcut: None,
                action: PaletteAction::Navigate(Route::Overview),
            },
            PaletteItem {
                id: "nav:repos".to_string(),
                label: "跳转到：仓库列表".to_string(),
                subtitle: "浏览工作区多仓状态矩阵".to_string(),
                shortcut: None,
                action: PaletteAction::Navigate(Route::Repos),
            },
            PaletteItem {
                id: "nav:tasks".to_string(),
                label: "跳转到：任务".to_string(),
                subtitle: "按分支查看跨仓任务进度".to_string(),
                shortcut: None,
                action: PaletteAction::Navigate(Route::Tasks),
            },
            PaletteItem {
                id: "nav:commit".to_string(),
                label: "跳转到：跨仓提交".to_string(),
                subtitle: "查看过渡期命令行提示".to_string(),
                shortcut: None,
                action: PaletteAction::Navigate(Route::CrossCommit),
            },
            PaletteItem {
                id: "nav:pipeline".to_string(),
                label: "跳转到：流水线".to_string(),
                subtitle: "查看过渡期命令行提示".to_string(),
                shortcut: None,
                action: PaletteAction::Navigate(Route::Pipeline),
            },
        ];

        if let Some(snap) = &self.snapshot {
            for repo in &snap.repos {
                all.push(PaletteItem {
                    id: format!("repo:{}", repo.path),
                    label: format!("仓库：{}", repo.name),
                    subtitle: format!("分支 {} · {}", repo.current_branch, repo.relative_path),
                    shortcut: None,
                    action: PaletteAction::SelectRepo(repo.path.clone()),
                });
            }
        }

        let trimmed = query.trim();
        let mut matched: Vec<(usize, usize, PaletteItem)> = all
            .into_iter()
            .enumerate()
            .filter_map(|(orig_ix, item)| {
                let recent_rank = self
                    .recent_palette_ids
                    .iter()
                    .position(|id| id == &item.id)
                    .unwrap_or(usize::MAX);
                if trimmed.is_empty() {
                    return Some((recent_rank, orig_ix, item));
                }
                let haystack = format!("{} {}", item.label, item.subtitle);
                if fuzzy_match(trimmed, &haystack) {
                    Some((recent_rank, orig_ix, item))
                } else {
                    None
                }
            })
            .collect();

        matched.sort_by_key(|(recent_rank, orig_ix, _)| (*recent_rank, *orig_ix));
        matched.into_iter().map(|(_, _, item)| item).collect()
    }

    pub fn active_tasks(&self) -> Vec<TaskGroup> {
        self.snapshot
            .as_ref()
            .map(|s| s.tasks.clone())
            .unwrap_or_default()
    }
}

fn alert_priority(kind: &GuardrailKind) -> u8 {
    match kind {
        GuardrailKind::DirtyTrunk => 0,
        GuardrailKind::MergeConflict => 1,
        GuardrailKind::DetachedHead => 2,
    }
}

fn fuzzy_match(query: &str, haystack: &str) -> bool {
    let q_lower = query.to_lowercase();
    let h_lower = haystack.to_lowercase();
    if h_lower.contains(&q_lower) {
        return true;
    }
    let mut h_chars = h_lower.chars();
    for qc in q_lower.chars().filter(|c| !c.is_whitespace()) {
        if !h_chars.any(|hc| hc == qc) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use kanso_core::{VcsKind, WorktreeEntry};
    use tempfile::TempDir;

    fn make_repo(name: &str, rel: &str, branch: &str, dirty: u32, on_trunk: bool) -> RepoStatus {
        RepoStatus {
            id: format!("id-{name}"),
            name: name.to_string(),
            path: format!("/ws/{rel}"),
            relative_path: rel.to_string(),
            vcs_kind: VcsKind::Git,
            is_worktree: false,
            current_branch: branch.to_string(),
            is_detached: false,
            trunk_branch: Some("main".to_string()),
            is_on_trunk: on_trunk,
            has_dirty_on_trunk: on_trunk && dirty > 0,
            upstream: None,
            remotes: vec!["origin".to_string()],
            upstream_ahead: 0,
            upstream_behind: 0,
            trunk_ahead: u32::from(!on_trunk),
            trunk_behind: 0,
            staged_count: 0,
            modified_count: dirty,
            untracked_count: 0,
            conflicted_count: 0,
            last_commit_hash: Some("abc1234".to_string()),
            last_commit_message: Some("init".to_string()),
            last_commit_author: Some("dev".to_string()),
            last_commit_timestamp: Some(1000),
            worktrees: vec![WorktreeEntry {
                path: format!("/ws/{rel}"),
                branch: Some(branch.to_string()),
                head: Some("abc1234".to_string()),
                is_main: true,
                is_prunable: false,
            }],
        }
    }

    fn make_snapshot(repos: Vec<RepoStatus>, failures: Vec<RepoFailure>) -> WorkspaceSnapshot {
        let tasks = aggregate_tasks(&repos);
        let alerts = detect_alerts(&repos);
        WorkspaceSnapshot {
            workspace_id: "ws-1".to_string(),
            workspace_name: "demo".to_string(),
            root_path: "/ws".to_string(),
            scanned_at: 1000,
            scan_duration_ms: 42,
            repos,
            failures,
            tasks,
            alerts,
        }
    }

    #[test]
    fn apply_event_replaces_inserts_handles_failure_exclusivity_and_caps_log() {
        let mut state = AppState::default();
        let r1 = make_repo("alpha", "services/alpha", "main", 0, true);
        let r2 = make_repo("beta", "services/beta", "feat/x", 1, false);
        state.apply_event(EngineEvent::SnapshotReady(make_snapshot(
            vec![r1.clone(), r2],
            Vec::new(),
        )));
        assert_eq!(state.snapshot.as_ref().unwrap().repos.len(), 2);

        // Fail alpha -> removes from repos, inserts into failures
        state.apply_event(EngineEvent::RepoFailed(RepoFailure {
            repo_path: r1.path.clone(),
            relative_path: r1.relative_path.clone(),
            error: "timeout".to_string(),
        }));
        assert_eq!(state.snapshot.as_ref().unwrap().repos.len(), 1);
        assert_eq!(state.snapshot.as_ref().unwrap().failures.len(), 1);

        // Recover alpha via RepoUpdated -> moves back to repos
        let r1_fixed = make_repo("alpha", "services/alpha", "main", 2, true);
        state.apply_event(EngineEvent::RepoUpdated(r1_fixed));
        assert_eq!(state.snapshot.as_ref().unwrap().repos.len(), 2);
        assert!(state.snapshot.as_ref().unwrap().failures.is_empty());

        // Insert brand new gamma
        let r3 = make_repo("gamma", "apps/gamma", "dev", 0, false);
        state.apply_event(EngineEvent::RepoUpdated(r3));
        assert_eq!(state.snapshot.as_ref().unwrap().repos.len(), 3);

        for i in 0..600 {
            state.push_log(LogLevel::Info, format!("msg {i}"));
        }
        assert_eq!(state.log.len(), MAX_LOG_ENTRIES);
        assert_eq!(state.log.back().unwrap().message, "msg 599");
    }

    #[test]
    fn five_repo_filters_work() {
        let mut state = AppState::default();
        let clean_trunk = make_repo("clean", "clean", "main", 0, true);
        let dirty_trunk = make_repo("dirty-trunk", "dirty-trunk", "main", 2, true);
        let feature_clean = make_repo("feat", "feat", "feat/ok", 0, false);
        let fail = RepoFailure {
            repo_path: "/ws/broken".to_string(),
            relative_path: "broken".to_string(),
            error: "corrupt HEAD".to_string(),
        };
        state.snapshot = Some(make_snapshot(
            vec![clean_trunk, dirty_trunk, feature_clean],
            vec![fail],
        ));

        state.filter = RepoFilter::All;
        assert_eq!(state.filtered_repos().len(), 3);
        assert_eq!(state.filtered_failures().len(), 1);

        state.filter = RepoFilter::Dirty;
        assert_eq!(state.filtered_repos().len(), 1);
        assert_eq!(state.filtered_repos()[0].name, "dirty-trunk");

        state.filter = RepoFilter::Alerts;
        assert_eq!(state.filtered_repos().len(), 1);
        assert_eq!(state.filtered_repos()[0].name, "dirty-trunk");

        state.filter = RepoFilter::OffTrunk;
        assert_eq!(state.filtered_repos().len(), 1);
        assert_eq!(state.filtered_repos()[0].name, "feat");

        state.filter = RepoFilter::Errored;
        assert!(state.filtered_repos().is_empty());
        assert_eq!(state.filtered_failures().len(), 1);
    }

    #[test]
    fn repo_tree_groups_by_parent_dir() {
        let mut state = AppState::default();
        state.snapshot = Some(make_snapshot(
            vec![
                make_repo("a", "backend/a", "main", 0, true),
                make_repo("b", "backend/b", "main", 0, true),
                make_repo("root_repo", "root_repo", "main", 0, true),
            ],
            Vec::new(),
        ));
        let tree = state.repo_tree();
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].0, "backend");
        assert_eq!(tree[0].1.len(), 2);
        assert_eq!(tree[1].0, "根目录");
        assert_eq!(tree[1].1.len(), 1);
    }

    #[test]
    fn select_next_and_prev_clamp_to_bounds() {
        let mut state = AppState::default();
        state.snapshot = Some(make_snapshot(
            vec![
                make_repo("r1", "r1", "main", 0, true),
                make_repo("r2", "r2", "main", 0, true),
            ],
            Vec::new(),
        ));

        state.select_prev();
        assert_eq!(state.selected.as_deref(), Some("/ws/r1"));
        state.select_prev();
        assert_eq!(state.selected.as_deref(), Some("/ws/r1"));
        state.select_next();
        assert_eq!(state.selected.as_deref(), Some("/ws/r2"));
        state.select_next();
        assert_eq!(state.selected.as_deref(), Some("/ws/r2"));
    }

    #[test]
    fn attention_queue_orders_failures_first_then_by_alert_priority() {
        let mut state = AppState::default();
        let mut detached = make_repo("detached", "detached", "detached", 0, false);
        detached.is_detached = true;
        let dirty_trunk = make_repo("dirty", "dirty", "main", 3, true);
        let fail = RepoFailure {
            repo_path: "/ws/fail".to_string(),
            relative_path: "fail".to_string(),
            error: "timeout".to_string(),
        };
        state.snapshot = Some(make_snapshot(vec![detached, dirty_trunk], vec![fail]));

        let queue = state.attention_queue();
        assert!(queue[0].is_failure);
        assert_eq!(queue[0].repo_name, "fail");
        assert_eq!(queue[1].repo_name, "dirty");
        assert_eq!(queue[2].repo_name, "detached");
    }

    #[test]
    fn palette_fuzzy_matches_branch_and_prioritizes_recent() {
        let mut state = AppState::default();
        let r1 = make_repo("gw", "gw", "java/jv-fix-identity", 0, false);
        let r2 = make_repo("auth", "auth", "main", 0, true);
        state.snapshot = Some(make_snapshot(vec![r1, r2], Vec::new()));

        let items = state.palette_items("jvfix");
        assert!(
            items.iter().any(|i| i.id == "repo:/ws/gw"),
            "jvfix should fuzzy-match java/jv-fix-identity"
        );

        state.record_palette_use("act:theme_dark");
        let empty_query = state.palette_items("");
        assert_eq!(empty_query[0].id, "act:theme_dark");
    }

    #[test]
    fn settings_roundtrip_to_disk() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");

        let cfg = Settings {
            theme: ThemeChoice::Dark,
        };
        cfg.save_to(&path).unwrap();

        let loaded = Settings::load_from(&path);
        assert_eq!(loaded, cfg);
    }
}
