use crate::engine_link::EngineLink;
use crate::pages;
use crate::palette;
use crate::state::{
    AppState, BottomTab, LogLevel, PaletteAction, RepoFilter, Route, Settings, ThemeChoice,
    logs_dir,
};
use chrono::{DateTime, Local, Utc};
use futures::StreamExt;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::{
    ActiveTheme, Sizable as _, Theme, ThemeMode, TitleBar, h_resizable, resizable_panel,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kanso_core::{WorkspaceConfig, WorkspaceSnapshot};
use kanso_engine::{EngineCommand, EngineEvent};
use kanso_storage::StorageDb;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

actions!(
    kanso_desktop,
    [
        TogglePalette,
        ToggleInspector,
        ToggleBottomPanel,
        RefreshAllAction,
        MoveDownAction,
        MoveUpAction,
        ToggleInspectorSpace,
        CancelAction,
        PaletteUp,
        PaletteDown,
        PaletteConfirm,
    ]
);

pub fn bind_keys(cx: &mut App) {
    #[cfg(target_os = "macos")]
    {
        cx.bind_keys([
            KeyBinding::new("cmd-k", TogglePalette, None),
            KeyBinding::new("cmd-alt-i", ToggleInspector, Some("KansoWorkbench")),
            KeyBinding::new("cmd-j", ToggleBottomPanel, Some("KansoWorkbench")),
            KeyBinding::new("cmd-r", RefreshAllAction, Some("KansoWorkbench")),
            KeyBinding::new("escape", CancelAction, Some("KansoWorkbench")),
            KeyBinding::new("j", MoveDownAction, Some("KansoWorkbench")),
            KeyBinding::new("k", MoveUpAction, Some("KansoWorkbench")),
            KeyBinding::new("space", ToggleInspectorSpace, Some("KansoWorkbench")),
            KeyBinding::new("up", PaletteUp, Some("KansoPalette")),
            KeyBinding::new("down", PaletteDown, Some("KansoPalette")),
            KeyBinding::new("enter", PaletteConfirm, Some("KansoPalette")),
            KeyBinding::new("escape", CancelAction, Some("KansoPalette")),
        ]);
    }
    #[cfg(not(target_os = "macos"))]
    {
        cx.bind_keys([
            KeyBinding::new("ctrl-k", TogglePalette, None),
            KeyBinding::new("ctrl-alt-i", ToggleInspector, Some("KansoWorkbench")),
            KeyBinding::new("ctrl-j", ToggleBottomPanel, Some("KansoWorkbench")),
            KeyBinding::new("ctrl-r", RefreshAllAction, Some("KansoWorkbench")),
            KeyBinding::new("escape", CancelAction, Some("KansoWorkbench")),
            KeyBinding::new("j", MoveDownAction, Some("KansoWorkbench")),
            KeyBinding::new("k", MoveUpAction, Some("KansoWorkbench")),
            KeyBinding::new("space", ToggleInspectorSpace, Some("KansoWorkbench")),
            KeyBinding::new("up", PaletteUp, Some("KansoPalette")),
            KeyBinding::new("down", PaletteDown, Some("KansoPalette")),
            KeyBinding::new("enter", PaletteConfirm, Some("KansoPalette")),
            KeyBinding::new("escape", CancelAction, Some("KansoPalette")),
        ]);
    }
}

pub fn apply_theme_choice(choice: ThemeChoice, window: Option<&mut Window>, cx: &mut App) {
    match choice {
        ThemeChoice::System => Theme::sync_system_appearance(window, cx),
        ThemeChoice::Light => Theme::change(ThemeMode::Light, window, cx),
        ThemeChoice::Dark => Theme::change(ThemeMode::Dark, window, cx),
    }
}

pub struct Workbench {
    pub state: AppState,
    pub settings: Settings,
    pub db: Option<Arc<StorageDb>>,
    pub engine: Option<EngineLink>,
    pub palette_open: bool,
    pub palette_input: Entity<InputState>,
    pub palette_cursor: usize,
    pub repos_scroll: UniformListScrollHandle,
    pub logs_scroll: UniformListScrollHandle,
    focus_handle: FocusHandle,
    started_at: Option<Instant>,
    first_frame_logged: bool,
    last_snapshot_save: Option<Instant>,
    _subscriptions: Vec<Subscription>,
    _engine_task: Option<Task<()>>,
}

impl Workbench {
    pub fn new(
        mut state: AppState,
        settings: Settings,
        db: Option<Arc<StorageDb>>,
        engine: Option<(EngineLink, futures::channel::mpsc::UnboundedReceiver<EngineEvent>)>,
        started_at: Option<Instant>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        apply_theme_choice(settings.theme, Some(window), cx);

        let palette_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("搜索命令、仓库或分支..."));

        let mut subs = Vec::new();
        subs.push(cx.subscribe_in(
            &palette_input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.palette_cursor = 0;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    this.confirm_palette(window, cx);
                }
                _ => {}
            },
        ));

        subs.push(window.observe_window_appearance(|window, cx| {
            let cfg = Settings::load();
            if cfg.theme == ThemeChoice::System {
                Theme::sync_system_appearance(Some(window), cx);
            }
        }));

        if state.selected.is_none() {
            if let Some(first) = state
                .snapshot
                .as_ref()
                .and_then(|s| s.repos.first())
                .map(|r| r.path.clone())
            {
                state.selected = Some(first);
            }
        }

        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);

        let (engine_link, engine_task) = match engine {
            Some((link, rx)) => {
                let task = Self::spawn_engine_listener(rx, cx);
                (Some(link), Some(task))
            }
            None => (None, None),
        };

        Self {
            state,
            settings,
            db,
            engine: engine_link,
            palette_open: false,
            palette_input,
            palette_cursor: 0,
            repos_scroll: UniformListScrollHandle::new(),
            logs_scroll: UniformListScrollHandle::new(),
            focus_handle,
            started_at,
            first_frame_logged: false,
            last_snapshot_save: None,
            _subscriptions: subs,
            _engine_task: engine_task,
        }
    }

    fn spawn_engine_listener(
        mut rx: futures::channel::mpsc::UnboundedReceiver<EngineEvent>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            while let Some(ev) = rx.next().await {
                let ok = this.update(cx, |view, cx| {
                    view.handle_engine_event(ev);
                    cx.notify();
                });
                if ok.is_err() {
                    break;
                }
            }
        })
    }

    fn handle_engine_event(&mut self, ev: EngineEvent) {
        let should_save = matches!(ev, EngineEvent::SnapshotReady(_));
        let is_incremental = matches!(
            ev,
            EngineEvent::RepoUpdated(_) | EngineEvent::RepoFailed(_) | EngineEvent::ReposChanged { .. }
        );
        self.state.apply_event(ev);

        if should_save {
            self.persist_snapshot(true);
        } else if is_incremental {
            let elapsed_ok = self
                .last_snapshot_save
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(5));
            if elapsed_ok {
                self.persist_snapshot(false);
            }
        }
    }

    fn persist_snapshot(&mut self, force: bool) {
        let Some(db) = &self.db else {
            return;
        };
        let Some(snap) = &self.state.snapshot else {
            return;
        };
        if !force && self.state.verifying {
            return;
        }
        if let Ok(json) = serde_json::to_string(snap) {
            if let Err(err) = db.save_snapshot(&snap.workspace_id, &json) {
                self.state
                    .push_log(LogLevel::Warn, format!("保存工作区缓存快照失败：{err}"));
            } else {
                self.last_snapshot_save = Some(Instant::now());
            }
        }
    }

    pub fn open_workspace_path(&mut self, root_path: PathBuf, cx: &mut Context<Self>) {
        let canonical = std::fs::canonicalize(&root_path).unwrap_or(root_path);
        let name = canonical
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "workspace".to_string());
        let root_str = canonical.to_string_lossy().to_string();

        let ws_record = if let Some(db) = &self.db {
            match db.add_workspace(&name, &root_str, None) {
                Ok(rec) => rec,
                Err(err) => {
                    self.state
                        .push_log(LogLevel::Error, format!("写入工作区记录失败：{err}"));
                    kanso_storage::WorkspaceRecord {
                        id: format!("mem-{name}"),
                        name,
                        root_path: root_str,
                        default_trunk: None,
                        created_at: Utc::now().timestamp(),
                    }
                }
            }
        } else {
            kanso_storage::WorkspaceRecord {
                id: format!("mem-{name}"),
                name,
                root_path: root_str,
                default_trunk: None,
                created_at: Utc::now().timestamp(),
            }
        };

        let cached_snap = self
            .db
            .as_ref()
            .and_then(|db| db.load_snapshot(&ws_record.id).ok().flatten())
            .and_then(|json| serde_json::from_str::<WorkspaceSnapshot>(&json).ok());

        if let Some(old_engine) = self.engine.take() {
            old_engine.shutdown();
        }
        self._engine_task = None;

        self.state.workspace = Some(ws_record.clone());
        if let Some(snap) = cached_snap {
            self.state.selected = snap.repos.first().map(|r| r.path.clone());
            self.state.snapshot = Some(snap);
            self.state.verifying = true;
        } else {
            self.state.snapshot = None;
            self.state.selected = None;
            self.state.verifying = false;
        }
        self.state
            .set_bg_task("engine-sync", "工作区全量校对", true, "后台扫描中...");
        self.state
            .push_log(LogLevel::Info, format!("打开工作区：{}", ws_record.root_path));

        let ws_cfg = WorkspaceConfig {
            id: ws_record.id,
            name: ws_record.name,
            root_path: ws_record.root_path,
            default_trunk: ws_record.default_trunk,
            ignore_dirs: Vec::new(),
        };

        match EngineLink::start(ws_cfg) {
            Ok((link, rx)) => {
                self._engine_task = Some(Self::spawn_engine_listener(rx, cx));
                self.engine = Some(link);
            }
            Err(err) => {
                self.state
                    .push_log(LogLevel::Error, format!("启动后台引擎失败：{err}"));
            }
        }
        cx.notify();
    }

    pub fn prompt_open_workspace(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择多仓库工作区目录".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(first) = paths.into_iter().next() {
                    let _ = this.update(cx, |view, cx| {
                        view.open_workspace_path(first, cx);
                    });
                }
            }
        })
        .detach();
    }

    pub fn refresh_all(&mut self, cx: &mut Context<Self>) {
        if let Some(engine) = &self.engine {
            if let Err(err) = engine.send(EngineCommand::RefreshAll) {
                self.state
                    .push_log(LogLevel::Error, format!("发送全量刷新失败：{err}"));
            } else {
                self.state
                    .set_bg_task("engine-sync", "工作区全量校对", true, "刷新中...");
                self.state.push_log(LogLevel::Info, "已触发全量工作区刷新");
            }
        } else {
            self.state
                .push_log(LogLevel::Warn, "未挂载后台引擎，跳过全量刷新请求");
        }
        cx.notify();
    }

    pub fn refresh_repo(&mut self, repo_path: &str, cx: &mut Context<Self>) {
        if let Some(engine) = &self.engine {
            if let Err(err) = engine.send(EngineCommand::RefreshRepo(PathBuf::from(repo_path))) {
                self.state
                    .push_log(LogLevel::Error, format!("触发单仓刷新失败：{err}"));
            } else {
                self.state
                    .push_log(LogLevel::Info, format!("已请求刷新仓库：{repo_path}"));
            }
        } else {
            self.state
                .push_log(LogLevel::Info, format!("未挂载后台引擎，记录单仓重试日志：{repo_path}"));
        }
        cx.notify();
    }

    pub fn focus_repo(&mut self, repo_path: &str, cx: &mut Context<Self>) {
        self.state.selected = Some(repo_path.to_string());
        self.state.route = Route::Repos;
        self.state.inspector_open = true;
        cx.notify();
    }

    pub fn set_theme(&mut self, choice: ThemeChoice, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.theme = choice;
        apply_theme_choice(choice, Some(window), cx);
        if let Err(err) = self.settings.save() {
            self.state
                .push_log(LogLevel::Warn, format!("保存外观设置失败：{err}"));
        }
        self.state
            .push_log(LogLevel::Info, format!("主题切换为：{}", choice.label()));
        cx.notify();
    }

    pub fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = true;
        self.palette_cursor = 0;
        self.palette_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        let input_focus = self.palette_input.read(cx).focus_handle(cx);
        input_focus.focus(window, cx);
        cx.notify();
    }

    pub fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = false;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    pub fn move_palette_cursor(&mut self, delta: isize, cx: &mut Context<Self>) {
        let query = self.palette_input.read(cx).value().to_string();
        let len = self.state.palette_items(&query).len().min(12);
        if len == 0 {
            self.palette_cursor = 0;
            return;
        }
        let next = if delta < 0 {
            self.palette_cursor.saturating_sub((-delta) as usize)
        } else {
            (self.palette_cursor + delta as usize).min(len - 1)
        };
        self.palette_cursor = next;
        cx.notify();
    }

    pub fn confirm_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.palette_input.read(cx).value().to_string();
        if let Some(item) = palette::selected_palette_item(&self.state, &query, self.palette_cursor)
        {
            self.execute_palette_action(&item.id, item.action, window, cx);
        } else {
            self.close_palette(window, cx);
        }
    }

    pub fn execute_palette_action(
        &mut self,
        item_id: &str,
        action: PaletteAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.record_palette_use(item_id);
        self.state.push_log(
            LogLevel::Info,
            format!("命令面板执行：{}", palette::palette_action_label(&action)),
        );
        self.close_palette(window, cx);
        match action {
            PaletteAction::RefreshAll => self.refresh_all(cx),
            PaletteAction::SetTheme(choice) => self.set_theme(choice, window, cx),
            PaletteAction::ToggleInspector => {
                self.state.inspector_open = !self.state.inspector_open;
                cx.notify();
            }
            PaletteAction::ToggleBottomPanel => {
                self.state.bottom_open = !self.state.bottom_open;
                cx.notify();
            }
            PaletteAction::OpenWorkspacePicker => self.prompt_open_workspace(cx),
            PaletteAction::OpenLogsDir => palette::run_open_dir_command(&logs_dir()),
            PaletteAction::Navigate(route) => {
                self.state.route = route;
                cx.notify();
            }
            PaletteAction::SelectRepo(path) => self.focus_repo(&path, cx),
        }
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let border = theme.sidebar_border;
        let bg = theme.sidebar;
        let fg = theme.sidebar_foreground;
        let muted = theme.muted_foreground;
        let active_bg = theme.sidebar_accent;
        let active_fg = theme.sidebar_accent_foreground;
        let red = theme.red;
        let yellow = theme.yellow;
        let green = theme.green;

        let nav_items = [
            (Route::Overview, IconName::LayoutDashboard),
            (Route::Repos, IconName::GitBranch),
            (Route::Tasks, IconName::ListTodo),
            (Route::CrossCommit, IconName::Terminal),
            (Route::Pipeline, IconName::Workflow),
        ];

        let mut nav_col = div().flex().flex_col().gap_0p5();
        for (route, icon) in nav_items {
            let active = self.state.route == route;
            nav_col = nav_col.child(
                div()
                    .id(SharedString::from(format!("sidebar-nav-{route:?}")))
                    .px_2p5()
                    .py_1p5()
                    .rounded(px(6.0))
                    .when(active, |d| d.bg(active_bg).text_color(active_fg))
                    .when(!active, |d| d.text_color(fg))
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(13.0))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.state.route = route;
                        cx.notify();
                    }))
                    .child(icon)
                    .child(route.label()),
            );
        }

        let filters = [
            RepoFilter::Dirty,
            RepoFilter::Alerts,
            RepoFilter::OffTrunk,
            RepoFilter::Errored,
        ];
        let mut filter_col = div().flex().flex_col().gap_0p5();
        for f in filters {
            let active = self.state.route == Route::Repos && self.state.filter == f;
            filter_col = filter_col.child(
                div()
                    .id(SharedString::from(format!("sidebar-filter-{f:?}")))
                    .px_2p5()
                    .py_1()
                    .rounded(px(6.0))
                    .when(active, |d| d.bg(active_bg))
                    .text_size(px(12.0))
                    .text_color(muted)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.state.route = Route::Repos;
                        this.state.filter = f;
                        cx.notify();
                    }))
                    .child(f.label()),
            );
        }

        let mut tree_col = div().flex().flex_col().gap_2();
        for (group, repos) in self.state.repo_tree() {
            let mut group_box = div().flex().flex_col().gap_0p5().child(
                div()
                    .px_2()
                    .text_size(px(11.0))
                    .text_color(muted)
                    .truncate()
                    .child(group),
            );
            for repo in repos {
                let selected = self.state.selected.as_deref() == Some(repo.path.as_str());
                let (dot_color, tag) = if repo.conflicted_count > 0 || repo.is_detached {
                    (red, "告警")
                } else if repo.is_dirty() {
                    (yellow, "改动")
                } else {
                    (green, "正常")
                };
                let click_path = repo.path.clone();
                group_box = group_box.child(
                    div()
                        .id(SharedString::from(format!("tree-repo-{}", repo.id)))
                        .px_2()
                        .py_1()
                        .rounded(px(6.0))
                        .when(selected, |d| d.bg(active_bg))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_1p5()
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.focus_repo(&click_path, cx);
                        }))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .min_w_0()
                                .child(div().text_size(px(10.0)).text_color(dot_color).child("●"))
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(fg)
                                        .truncate()
                                        .child(repo.name.clone()),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(muted)
                                .child(tag),
                        ),
                );
            }
            tree_col = tree_col.child(group_box);
        }

        div()
            .id("sidebar-panel")
            .size_full()
            .bg(bg)
            .border_r_1()
            .border_color(border)
            .p_2p5()
            .flex()
            .flex_col()
            .gap_3()
            .overflow_hidden()
            .child(nav_col)
            .child(div().h(px(1.0)).bg(border))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .px_2()
                            .text_size(px(11.0))
                            .text_color(muted)
                            .child("快速筛选"),
                    )
                    .child(filter_col),
            )
            .child(div().h(px(1.0)).bg(border))
            .child(
                div()
                    .id("sidebar-repo-tree")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .px_2()
                            .text_size(px(11.0))
                            .text_color(muted)
                            .child("仓库目录树"),
                    )
                    .child(tree_col),
            )
            .into_any_element()
    }

    fn render_inspector(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let border = theme.border;
        let bg = theme.sidebar;
        let muted = theme.muted_foreground;
        let mono = theme.mono_font_family.clone();
        let red = theme.red;

        let body: AnyElement = if let Some(repo) = self.state.selected_repo() {
            let repo_path = repo.path.clone();
            let dirty_text = format!(
                "已暂存 {} / 已修改 {} / 未跟踪 {} / 冲突 {}",
                repo.staged_count, repo.modified_count, repo.untracked_count, repo.conflicted_count
            );
            let trunk_text = match &repo.trunk_branch {
                Some(t) => format!("{t} (+{} / -{})", repo.trunk_ahead, repo.trunk_behind),
                None => "未识别主干".to_string(),
            };
            let remote_text = repo
                .upstream
                .clone()
                .map(|u| format!("{u} (↑{} ↓{})", repo.upstream_ahead, repo.upstream_behind))
                .unwrap_or_else(|| "未关联上游".to_string());

            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .child(
                            div()
                                .text_size(px(15.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(repo.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .font_family(mono.clone())
                                .text_color(muted)
                                .truncate()
                                .child(repo.relative_path.clone()),
                        ),
                )
                .child(inspector_kv("当前分支", repo.current_branch.clone(), mono.clone(), cx))
                .child(inspector_kv("主干偏离", trunk_text, mono.clone(), cx))
                .child(inspector_kv("远端跟踪", remote_text, mono.clone(), cx))
                .child(inspector_kv("工作区状态", dirty_text, mono.clone(), cx))
                .child(inspector_kv(
                    "最近提交",
                    format!(
                        "{} {}",
                        repo.last_commit_hash.as_deref().unwrap_or("-"),
                        repo.last_commit_message.as_deref().unwrap_or("-")
                    ),
                    mono,
                    cx,
                ))
                .child(
                    Button::new("inspector-refresh-btn")
                        .primary()
                        .small()
                        .label("刷新此仓")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.refresh_repo(&repo_path, cx);
                        })),
                )
                .into_any_element()
        } else if let Some(fail) = self.state.selected_failure() {
            let repo_path = fail.repo_path.clone();
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(red)
                        .child(fail.relative_path.clone()),
                )
                .child(
                    div()
                        .p_2p5()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(red)
                        .font_family(mono)
                        .text_size(px(12.0))
                        .text_color(red)
                        .child(fail.error.clone()),
                )
                .child(
                    Button::new("inspector-retry-btn")
                        .danger()
                        .small()
                        .label("重试刷新")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.refresh_repo(&repo_path, cx);
                        })),
                )
                .into_any_element()
        } else {
            div()
                .text_size(px(12.0))
                .text_color(muted)
                .child("在仓库列表中点击任意仓库以查看检查器摘要。")
                .into_any_element()
        };

        div()
            .id("inspector-panel")
            .w(px(280.0))
            .h_full()
            .bg(bg)
            .border_l_1()
            .border_color(border)
            .p_3()
            .flex()
            .flex_col()
            .gap_3()
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(IconName::PanelRight)
                            .child("检查器"),
                    )
                    .child(
                        Button::new("close-inspector-btn")
                            .small()
                            .ghost()
                            .label("✕")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.state.inspector_open = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(div().h(px(1.0)).bg(border))
            .child(body)
            .test_support()
            .into_any_element()
    }

    fn render_bottom_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let border = theme.border;
        let sec = theme.secondary;
        let muted = theme.muted_foreground;

        let tab_btn = |id: &'static str, tab: BottomTab, active: bool| {
            let b = Button::new(id)
                .small()
                .label(match tab {
                    BottomTab::Tasks => "后台任务",
                    BottomTab::Log => "操作日志",
                });
            if active { b.primary() } else { b.ghost() }
        };

        let header = div()
            .h(px(32.0))
            .px_3()
            .border_b_1()
            .border_color(border)
            .bg(sec)
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        tab_btn(
                            "bottom-tab-tasks",
                            BottomTab::Tasks,
                            self.state.bottom_tab == BottomTab::Tasks,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.state.bottom_tab = BottomTab::Tasks;
                            cx.notify();
                        })),
                    )
                    .child(
                        tab_btn(
                            "bottom-tab-log",
                            BottomTab::Log,
                            self.state.bottom_tab == BottomTab::Log,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.state.bottom_tab = BottomTab::Log;
                            cx.notify();
                        })),
                    ),
            )
            .child(
                Button::new("close-bottom-btn")
                    .small()
                    .ghost()
                    .label("✕")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.state.bottom_open = false;
                        cx.notify();
                    })),
            );

        let body: AnyElement = match self.state.bottom_tab {
            BottomTab::Tasks => {
                if self.state.bg_tasks.is_empty() {
                    div()
                        .p_3()
                        .text_size(px(12.0))
                        .text_color(muted)
                        .child("当前没有后台任务。")
                        .into_any_element()
                } else {
                    let mut col = div().p_3().flex().flex_col().gap_1p5();
                    for task in &self.state.bg_tasks {
                        col = col.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .text_size(px(12.0))
                                .child(format!(
                                    "{} ({})",
                                    task.label,
                                    if task.running { "运行中" } else { "完成" }
                                ))
                                .child(div().text_color(muted).child(task.detail.clone())),
                        );
                    }
                    col.into_any_element()
                }
            }
            BottomTab::Log => {
                let count = self.state.log.len();
                if count == 0 {
                    div()
                        .p_3()
                        .text_size(px(12.0))
                        .text_color(muted)
                        .child("尚无操作日志。")
                        .into_any_element()
                } else {
                    uniform_list(
                        "bottom-log-list",
                        count,
                        cx.processor(move |this, range: Range<usize>, _window, cx| {
                            let t = cx.theme();
                            let mono = t.mono_font_family.clone();
                            let red = t.red;
                            let yellow = t.yellow;
                            let fg = t.foreground;
                            let m = t.muted_foreground;
                            range
                                .map(|ix| {
                                    // 最新日志展示在最上方
                                    let rev_ix = this.state.log.len().saturating_sub(1 + ix);
                                    let entry = &this.state.log[rev_ix];
                                    let (lvl_text, lvl_color) = match entry.level {
                                        LogLevel::Info => ("INFO", m),
                                        LogLevel::Warn => ("WARN", yellow),
                                        LogLevel::Error => ("ERR ", red),
                                    };
                                    let ts_str = DateTime::<Utc>::from_timestamp(entry.timestamp, 0)
                                        .map(|dt| {
                                            dt.with_timezone(&Local).format("%H:%M:%S").to_string()
                                        })
                                        .unwrap_or_else(|| "--:--:--".to_string());
                                    div()
                                        .h(px(22.0))
                                        .px_3()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .font_family(mono.clone())
                                        .text_size(px(11.0))
                                        .child(div().text_color(m).child(ts_str))
                                        .child(div().text_color(lvl_color).child(lvl_text))
                                        .child(
                                            div()
                                                .flex_1()
                                                .truncate()
                                                .text_color(fg)
                                                .child(entry.message.clone()),
                                        )
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .size_full()
                    .track_scroll(&self.logs_scroll)
                    .into_any_element()
                }
            }
        };

        div()
            .id("bottom-panel")
            .h(px(190.0))
            .w_full()
            .border_t_1()
            .border_color(border)
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(header)
            .child(div().flex_1().w_full().overflow_hidden().child(body))
            .test_support()
            .into_any_element()
    }

    fn render_empty_workspace(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        div()
            .id("empty-workspace-state")
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .child(
                div()
                    .text_size(px(20.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("欢迎使用 Kanso v2 多仓库工程控制塔"),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(theme.muted_foreground)
                    .child("请选择一个包含多个 Git 仓库的根目录开始监听与调度。"),
            )
            .child(
                Button::new("empty-open-workspace-btn")
                    .primary()
                    .label("打开工作区目录")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.prompt_open_workspace(cx);
                    })),
            )
            .into_any_element()
    }
}

fn inspector_kv(label: &str, value: String, mono: SharedString, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .flex()
        .flex_col()
        .gap_0p5()
        .child(
            div()
                .text_size(px(11.0))
                .text_color(theme.muted_foreground)
                .child(label.to_string()),
        )
        .child(
            div()
                .font_family(mono)
                .text_size(px(12.0))
                .text_color(theme.foreground)
                .child(value),
        )
        .into_any_element()
}

impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.first_frame_logged {
            self.first_frame_logged = true;
            let elapsed_ms = self
                .started_at
                .map(|t| t.elapsed().as_millis())
                .unwrap_or(0);
            let cached_repos = self
                .state
                .snapshot
                .as_ref()
                .map(|s| s.repos.len())
                .unwrap_or(0);
            println!(
                "KANSO_FIRST_FRAME_MS={} CACHED_REPOS={}",
                elapsed_ms, cached_repos
            );
            tracing::info!(
                "KANSO_FIRST_FRAME_MS={} CACHED_REPOS={}",
                elapsed_ms,
                cached_repos
            );
        }

        let theme = cx.theme();
        let ui_font = theme.font_family.clone();
        let bg = theme.background;
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let sec = theme.secondary;
        let red = theme.red;
        let mono = theme.mono_font_family.clone();

        let ws_name = self
            .state
            .workspace
            .as_ref()
            .map(|w| w.name.clone())
            .or_else(|| self.state.snapshot.as_ref().map(|s| s.workspace_name.clone()))
            .unwrap_or_else(|| "未选择工作区".to_string());

        let any_running = self.state.bg_tasks.iter().any(|t| t.running) || self.state.verifying;
        let theme_icon = match self.settings.theme {
            ThemeChoice::System => IconName::Laptop,
            ThemeChoice::Light => IconName::Sun,
            ThemeChoice::Dark => IconName::Moon,
        };

        let title_bar = TitleBar::new().child(
            div()
                .w_full()
                .px_3()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(13.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Kanso"),
                        )
                        .child(div().text_color(muted).child("·"))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(muted)
                                .child(ws_name),
                        ),
                )
                .child(
                    div()
                        .id("open-palette-btn")
                        .w(px(300.0))
                        .h(px(24.0))
                        .px_2p5()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(border)
                        .bg(sec)
                        .flex()
                        .items_center()
                        .justify_between()
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_palette(window, cx);
                        }))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .text_size(px(12.0))
                                .text_color(muted)
                                .child(IconName::Search)
                                .child("搜索仓库、分支或命令..."),
                        )
                        .child(
                            div()
                                .font_family(mono.clone())
                                .text_size(px(11.0))
                                .text_color(muted)
                                .child("⌘K"),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child(
                            div()
                                .px_2()
                                .py_0p5()
                                .rounded(px(4.0))
                                .bg(sec)
                                .text_size(px(11.0))
                                .text_color(muted)
                                .child(if any_running { "扫描校验中" } else { "引擎就绪" }),
                        )
                        .child(
                            Button::new("open-workspace-btn")
                                .small()
                                .ghost()
                                .label("打开目录")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.prompt_open_workspace(cx);
                                })),
                        )
                        .child(
                            Button::new("refresh-all-btn")
                                .small()
                                .ghost()
                                .label("刷新")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.refresh_all(cx);
                                })),
                        )
                        .child(
                            Button::new("theme-toggle-btn")
                                .small()
                                .ghost()
                                .child(theme_icon)
                                .label(self.settings.theme.label())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    let next = this.settings.theme.next();
                                    this.set_theme(next, window, cx);
                                })),
                        ),
                ),
        );

        let main_content: AnyElement = if self.state.workspace.is_none() && self.state.snapshot.is_none() {
            self.render_empty_workspace(cx)
        } else {
            match self.state.route {
                Route::Overview => pages::overview::render_overview(&self.state, cx),
                Route::Repos => pages::repos::render_repos(self, window, cx),
                Route::Tasks => pages::tasks::render_tasks(&self.state, cx),
                Route::CrossCommit | Route::Pipeline => {
                    pages::placeholder::render_placeholder(self.state.route, cx)
                }
            }
        };

        let center_column = div()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .when_some(self.state.db_error.clone(), |col, err| {
                col.child(
                    div()
                        .id("db-error-banner")
                        .px_4()
                        .py_2()
                        .border_b_1()
                        .border_color(red)
                        .bg(sec)
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(red)
                                .child(format!("数据库初始化失败（已降级为内存只读模式）：{err}")),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_1p5()
                                .child(
                                    Button::new("db-err-open-logs")
                                        .small()
                                        .ghost()
                                        .label("打开日志目录")
                                        .on_click(|_, _, _| {
                                            palette::run_open_dir_command(&logs_dir());
                                        }),
                                )
                                .child(
                                    Button::new("db-err-dismiss")
                                        .small()
                                        .ghost()
                                        .label("忽略继续")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.state.db_error = None;
                                            cx.notify();
                                        })),
                                ),
                        ),
                )
            })
            .child(div().flex_1().w_full().overflow_hidden().child(main_content))
            .when(self.state.bottom_open, |col| {
                col.child(self.render_bottom_panel(cx))
            });

        let workspace_area = div()
            .flex_1()
            .w_full()
            .flex()
            .flex_row()
            .overflow_hidden()
            .child(
                div().flex_1().h_full().overflow_hidden().child(
                    h_resizable("workbench-h-split")
                        .child(
                            resizable_panel()
                                .size(px(236.0))
                                .size_range(px(180.0)..px(340.0))
                                .child(self.render_sidebar(cx)),
                        )
                        .child(resizable_panel().child(center_column)),
                ),
            )
            .when(self.state.inspector_open, |row| {
                row.child(self.render_inspector(cx))
            });

        let repo_total = self
            .state
            .snapshot
            .as_ref()
            .map(|s| s.repos.len())
            .unwrap_or(0);
        let refresh_desc = match self.state.last_refresh_at {
            Some(ts) => {
                let secs = (Utc::now().timestamp() - ts).max(0);
                format!("{secs} 秒前刷新")
            }
            None => "等待首次扫描".to_string(),
        };
        let status_left = if self.state.workspace.is_some() || self.state.snapshot.is_some() {
            format!("监听中 · {repo_total} 仓 · {refresh_desc}")
        } else {
            "空闲 · 未打开工作区".to_string()
        };

        let sel_branch = self
            .state
            .selected_repo()
            .map(|r| r.current_branch.clone())
            .unwrap_or_else(|| "-".to_string());
        let jj_desc = self
            .state
            .jj_version
            .as_deref()
            .map(|v| format!(" · {v}"))
            .unwrap_or_default();
        let status_right = format!(
            "分支 {sel_branch} · {}{} · 并发 2",
            self.state.git_version, jj_desc
        );

        let status_bar = StatusBar::new()
            .h(px(24.0))
            .border_t_1()
            .border_color(border)
            .left(
                div()
                    .text_size(px(11.0))
                    .text_color(muted)
                    .child(status_left),
            )
            .right(
                div()
                    .font_family(mono)
                    .text_size(px(11.0))
                    .text_color(muted)
                    .child(status_right),
            );

        div()
            .id("kanso-workbench")
            .key_context("KansoWorkbench")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &TogglePalette, window, cx| {
                if this.palette_open {
                    this.close_palette(window, cx);
                } else {
                    this.open_palette(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleInspector, _, cx| {
                this.state.inspector_open = !this.state.inspector_open;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleBottomPanel, _, cx| {
                this.state.bottom_open = !this.state.bottom_open;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &RefreshAllAction, _, cx| {
                this.refresh_all(cx);
            }))
            .on_action(cx.listener(|this, _: &MoveDownAction, _, cx| {
                if !this.palette_open && this.state.route == Route::Repos {
                    this.state.select_next();
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &MoveUpAction, _, cx| {
                if !this.palette_open && this.state.route == Route::Repos {
                    this.state.select_prev();
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleInspectorSpace, _, cx| {
                if !this.palette_open && this.state.route == Route::Repos {
                    this.state.inspector_open = !this.state.inspector_open;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &CancelAction, window, cx| {
                if this.palette_open {
                    this.close_palette(window, cx);
                }
            }))
            .relative()
            .size_full()
            .bg(bg)
            .text_color(fg)
            .font_family(ui_font)
            .flex()
            .flex_col()
            .child(title_bar)
            .child(workspace_area)
            .child(status_bar)
            .when(self.palette_open, |root| {
                root.child(palette::render_palette_overlay(self, window, cx))
            })
    }
}
