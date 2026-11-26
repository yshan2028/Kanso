use anyhow::Result;
use notify::RecursiveMode;
use notify_debouncer_mini::{DebouncedEventKind, Debouncer, new_debouncer};
use std::path::{Component, Path};
use std::time::Duration;

pub struct WorkspaceWatcher {
    _debouncer: Debouncer<notify::RecommendedWatcher>,
}

fn should_ignore_path(path: &Path) -> bool {
    let mut prev_git = false;
    for comp in path.components() {
        if let Component::Normal(name) = comp {
            let s = name.to_string_lossy();
            if prev_git && (s == "objects" || s == "logs") {
                return true;
            }
            if matches!(
                s.as_ref(),
                "node_modules" | "target" | "dist" | "build" | ".next" | ".kanso"
            ) {
                return true;
            }
            prev_git = s == ".git";
        }
    }
    false
}

impl WorkspaceWatcher {
    pub fn start<F>(root: &Path, on_change: F) -> Result<Self>
    where
        F: Fn() + Send + Sync + 'static,
    {
        let mut debouncer = new_debouncer(
            Duration::from_millis(250),
            move |res: std::result::Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>| {
                if let Ok(events) = res {
                    let relevant = events.iter().any(|ev| {
                        matches!(ev.kind, DebouncedEventKind::Any) && !should_ignore_path(&ev.path)
                    });
                    if relevant {
                        on_change();
                    }
                }
            },
        )?;

        debouncer
            .watcher()
            .watch(root, RecursiveMode::Recursive)?;

        Ok(Self {
            _debouncer: debouncer,
        })
    }
}
