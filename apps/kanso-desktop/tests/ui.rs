use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::test::{TestInput, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, ElementInputHandler, Entity, Point,
    SharedString, TestAppContext, WindowBounds, WindowOptions, px, size,
};
use kanso_core::{
    RepoFailure, RepoStatus, VcsKind, WorkspaceSnapshot, WorktreeEntry, aggregate_tasks,
    detect_alerts,
};
use kanso_desktop::{AppState, Route, Settings, ThemeChoice, Workbench, bind_keys};

fn make_repo(name: &str, branch: &str, dirty: u32, on_trunk: bool) -> RepoStatus {
    RepoStatus {
        id: format!("id-{name}"),
        name: name.to_string(),
        path: format!("/mock/{name}"),
        relative_path: name.to_string(),
        vcs_kind: VcsKind::Git,
        is_worktree: false,
        current_branch: branch.to_string(),
        is_detached: false,
        trunk_branch: Some("main".to_string()),
        is_on_trunk: on_trunk,
        has_dirty_on_trunk: on_trunk && dirty > 0,
        upstream: Some(format!("origin/{branch}")),
        remotes: vec!["origin".to_string()],
        upstream_ahead: 0,
        upstream_behind: 0,
        trunk_ahead: u32::from(!on_trunk),
        trunk_behind: 0,
        staged_count: 0,
        modified_count: dirty,
        untracked_count: 0,
        conflicted_count: 0,
        last_commit_hash: Some("a1b2c3d".to_string()),
        last_commit_message: Some(format!("feat: {name}")),
        last_commit_author: Some("dev".to_string()),
        last_commit_timestamp: Some(1700000000),
        worktrees: vec![WorktreeEntry {
            path: format!("/mock/{name}"),
            branch: Some(branch.to_string()),
            head: Some("a1b2c3d".to_string()),
            is_main: true,
            is_prunable: false,
        }],
    }
}

fn synthetic_state(theme: ThemeChoice) -> (AppState, Settings) {
    let repos = vec![
        make_repo("alpha", "main", 0, true),
        make_repo("beta", "feat/ui", 2, false),
        make_repo("gamma", "main", 1, true),
        make_repo("delta", "feat/engine", 0, false),
    ];
    let failures = vec![RepoFailure {
        repo_path: "/mock/broken".to_string(),
        relative_path: "broken".to_string(),
        error: "fatal: corrupt HEAD".to_string(),
    }];
    let tasks = aggregate_tasks(&repos);
    let alerts = detect_alerts(&repos);

    let mut state = AppState::default();
    state.route = Route::Repos;
    state.git_version = "git version 2.54.0".to_string();
    state.snapshot = Some(WorkspaceSnapshot {
        workspace_id: "ws-ui".to_string(),
        workspace_name: "mock-ws".to_string(),
        root_path: "/mock".to_string(),
        scanned_at: 1700000000,
        scan_duration_ms: 15,
        repos,
        failures,
        tasks,
        alerts,
    });
    state.selected = Some("/mock/alpha".to_string());
    (
        state,
        Settings {
            theme,
            ..Settings::default()
        },
    )
}

fn open_test_workbench(
    cx: &mut TestAppContext,
    theme: ThemeChoice,
) -> (AnyWindowHandle, Entity<Workbench>) {
    let (handle, wb) = cx.update(|cx| {
        gpui_kit::init(cx);
        bind_keys(cx);
        let bounds = Bounds {
            origin: Point::default(),
            size: size(px(1260.0), px(820.0)),
        };
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let (state, settings) = synthetic_state(theme);
                cx.new(|cx| Workbench::new(state, settings, None, None, None, window, cx))
            },
        )
        .expect("open test window")
    });
    cx.run_until_parked();
    (handle, wb)
}

fn cmd_k() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-k"
    } else {
        "ctrl-k"
    }
}

fn cmd_alt_i() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-alt-i"
    } else {
        "ctrl-alt-i"
    }
}

fn cmd_j() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-j"
    } else {
        "ctrl-j"
    }
}

#[gpui_kit::test]
fn ui_model_pages_open_without_a_workspace(cx: &mut TestAppContext) {
    let (handle, wb) = open_test_workbench(cx, ThemeChoice::Dark);
    cx.update_window(handle, |_, window, cx| {
        wb.update(cx, |wb, cx| {
            wb.state.snapshot = None;
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("empty-open-workspace-btn").visible());

        for (route, page) in [
            (Route::Providers, "page-providers"),
            (Route::Routing, "page-routing"),
            (Route::Pricing, "page-pricing"),
            (Route::Usage, "page-usage"),
            (Route::Settings, "page-settings"),
        ] {
            let nav = SharedString::from(format!("sidebar-nav-{route:?}"));
            assert!(window.find(nav.clone()).visible());
            window.click(nav, cx);
            window.render_frame(cx);
            assert_eq!(wb.read(cx).state.route, route);
            assert!(window.find(page).visible());
            assert!(window.try_find("empty-open-workspace-btn").is_none());
        }

        assert!(wb.read(cx).state.usage.is_none());
        assert!(wb.read(cx).state.desktop_app.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn ui_renders_five_rows_with_visible_failure_row(cx: &mut TestAppContext) {
    let (handle, _wb) = open_test_workbench(cx, ThemeChoice::Dark);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for ix in 0..5 {
            let row = window.find(SharedString::from(format!("repo-row-{ix}")));
            assert!(row.visible(), "repo-row-{ix} should be visible");
        }
        assert!(window.try_find("retry-repo-4").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn ui_command_palette_opens_filters_and_closes(cx: &mut TestAppContext) {
    let (handle, wb) = open_test_workbench(cx, ThemeChoice::Dark);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("command-palette").is_none());

        window.press(cmd_k(), cx);
        window.render_frame(cx);
        assert!(window.try_find("command-palette").is_some());

        window.input("alpha", cx);
        window.render_frame(cx);
        assert_eq!(wb.read(cx).palette_input.read(cx).value(), "alpha");
        assert!(window.try_find("palette-item-0").is_some());
        assert!(
            window.try_find("palette-item-1").is_none(),
            "filtering by 'alpha' should only leave matching alpha item"
        );

        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("command-palette").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn ui_chinese_ime_composition_commits_cleanly(cx: &mut TestAppContext) {
    let (handle, wb) = open_test_workbench(cx, ThemeChoice::Dark);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press(cmd_k(), cx);
        window.render_frame(cx);

        let input_state = wb.read(cx).palette_input.clone();
        let input_id: ElementId = ("input", input_state.entity_id()).into();
        let bounds = window.find(input_id).bounds();
        let mut ime = TestInput::new(
            ElementInputHandler::new(bounds, input_state.clone()),
            window,
        );

        ime.compose_text(None, "chong", Some(0..5), cx);
        assert!(
            ime.marked_text_range(cx).is_some(),
            "IME marked_text_range must be active during composition"
        );

        ime.commit_text("重构", cx);
        assert_eq!(ime.marked_text_range(cx), None);
        drop(ime);

        assert_eq!(wb.read(cx).palette_input.read(cx).value(), "重构");
    })
    .unwrap();
}

#[gpui_kit::test]
fn ui_jk_navigation_moves_selection_to_second_row(cx: &mut TestAppContext) {
    let (handle, wb) = open_test_workbench(cx, ThemeChoice::Dark);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(wb.read(cx).state.selected.as_deref(), Some("/mock/alpha"));

        window.press("j", cx);
        window.press("j", cx);
        window.press("k", cx);
        window.render_frame(cx);

        assert_eq!(
            wb.read(cx).state.selected.as_deref(),
            Some("/mock/beta"),
            "after j j k from row 0, selected should be row 2 (/mock/beta)"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn ui_shortcuts_toggle_inspector_and_bottom_panel(cx: &mut TestAppContext) {
    let (handle, _wb) = open_test_workbench(cx, ThemeChoice::Dark);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("inspector-panel").is_some());
        assert!(window.try_find("bottom-panel").is_none());

        window.press(cmd_alt_i(), cx);
        window.render_frame(cx);
        assert!(window.try_find("inspector-panel").is_none());

        window.press(cmd_alt_i(), cx);
        window.render_frame(cx);
        assert!(window.try_find("inspector-panel").is_some());

        window.press(cmd_j(), cx);
        window.render_frame(cx);
        assert!(window.try_find("bottom-panel").is_some());

        window.press(cmd_j(), cx);
        window.render_frame(cx);
        assert!(window.try_find("bottom-panel").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn ui_dark_theme_command_changes_global_theme_mode(cx: &mut TestAppContext) {
    let (handle, _wb) = open_test_workbench(cx, ThemeChoice::Light);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(Theme::global(cx).mode, ThemeMode::Light);

        window.press(cmd_k(), cx);
        window.render_frame(cx);
        window.input("深色主题", cx);
        window.render_frame(cx);
        window.click("palette-item-0", cx);
        window.render_frame(cx);

        assert_eq!(Theme::global(cx).mode, ThemeMode::Dark);
        assert!(window.try_find("command-palette").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn ui_clicking_retry_on_failed_row_logs_without_crash(cx: &mut TestAppContext) {
    let (handle, wb) = open_test_workbench(cx, ThemeChoice::Dark);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let logs_before = wb.read(cx).state.log.len();

        window.click("retry-repo-4", cx);
        window.render_frame(cx);

        let state = &wb.read(cx).state;
        assert!(state.log.len() > logs_before);
        assert!(
            state
                .log
                .back()
                .is_some_and(|e| e.message.contains("/mock/broken")),
            "retry click should log broken repo path"
        );
    })
    .unwrap();
}
