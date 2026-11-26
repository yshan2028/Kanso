use chrono::Utc;
use gpui_kit::assets::AllAssets;
use gpui_kit::component::TitleBar;
use gpui_kit::*;
use kanso_core::{WorkspaceConfig, WorkspaceSnapshot, commit::detect_jj_version, exec};
use kanso_desktop::engine_link::EngineLink;
use kanso_desktop::state::{AppState, LogLevel, Settings, app_data_dir, logs_dir};
use kanso_desktop::{Workbench, bind_keys};
use kanso_storage::{StorageDb, WorkspaceRecord};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tracing_subscriber::EnvFilter;

fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let dir = logs_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let file_appender = tracing_appender::rolling::daily(&dir, "kanso-desktop.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(non_blocking)
        .try_init();
    Some(guard)
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("Kanso Desktop 发生 panic: {info}");
        default_hook(info);
    }));
}

fn read_git_version() -> String {
    exec::run_raw(
        "git",
        &[OsString::from("--version")],
        &[],
        exec::GitAccess::Read,
    )
    .ok()
    .filter(|o| o.success)
    .map(|o| o.stdout.trim().to_string())
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| "git (未检测到)".to_string())
}

fn main() {
    let t0 = Instant::now();
    let _log_guard = init_logging();
    install_panic_hook();

    let cli_root = std::env::args().nth(1).map(PathBuf::from);
    let settings = Settings::load();

    let db_path = app_data_dir().join("kanso.db");
    let (db, db_err) = match StorageDb::open(&db_path) {
        Ok(db) => (Some(Arc::new(db)), None),
        Err(err) => {
            tracing::error!("打开 SQLite 失败 ({}): {err}", db_path.display());
            (None, Some(err.to_string()))
        }
    };

    let mut state = AppState::default();
    state.db_error = db_err;
    state.git_version = read_git_version();
    state.jj_version = detect_jj_version();

    let target_ws: Option<WorkspaceRecord> = if let Some(path) = cli_root {
        let canonical = std::fs::canonicalize(&path).unwrap_or(path);
        let name = canonical
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "workspace".to_string());
        let root_str = canonical.to_string_lossy().to_string();
        if let Some(db_ref) = &db {
            match db_ref.add_workspace(&name, &root_str, None) {
                Ok(rec) => Some(rec),
                Err(err) => {
                    state.push_log(LogLevel::Warn, format!("登记工作区失败：{err}"));
                    Some(WorkspaceRecord {
                        id: format!("mem-{name}"),
                        name,
                        root_path: root_str,
                        default_trunk: None,
                        created_at: Utc::now().timestamp(),
                    })
                }
            }
        } else {
            Some(WorkspaceRecord {
                id: format!("mem-{name}"),
                name,
                root_path: root_str,
                default_trunk: None,
                created_at: Utc::now().timestamp(),
            })
        }
    } else {
        db.as_ref()
            .and_then(|d| d.list_workspaces().ok())
            .and_then(|mut list| (!list.is_empty()).then(|| list.remove(0)))
    };

    if let Some(ws) = &target_ws {
        state.workspace = Some(ws.clone());
        if let Some(db_ref) = &db {
            if let Ok(Some(json)) = db_ref.load_snapshot(&ws.id) {
                if let Ok(snap) = serde_json::from_str::<WorkspaceSnapshot>(&json) {
                    state.selected = snap.repos.first().map(|r| r.path.clone());
                    state.snapshot = Some(snap);
                    state.verifying = true;
                    state.push_log(LogLevel::Info, "已从本地缓存恢复上次工作区快照");
                }
            }
        }
    }

    let engine = target_ws.as_ref().and_then(|ws| {
        let cfg = WorkspaceConfig {
            id: ws.id.clone(),
            name: ws.name.clone(),
            root_path: ws.root_path.clone(),
            default_trunk: ws.default_trunk.clone(),
            ignore_dirs: Vec::new(),
        };
        match EngineLink::start(cfg) {
            Ok(pair) => {
                state.set_bg_task("engine-sync", "工作区全量校对", true, "后台扫描中...");
                Some(pair)
            }
            Err(err) => {
                state.push_log(LogLevel::Error, format!("启动后台刷新引擎失败：{err}"));
                None
            }
        }
    });

    gpui_kit::application()
        .with_assets(AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            bind_keys(cx);

            let opts = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1260.0), px(820.0)),
                    cx,
                ))),
                ..TitleBar::window_options()
            };

            if let Err(err) = gpui_kit::open_window(opts, cx, move |window, cx| {
                cx.new(|cx| Workbench::new(state, settings, db, engine, Some(t0), window, cx))
            }) {
                tracing::error!("打开 Kanso 主窗口失败: {err}");
            }
        });
}
