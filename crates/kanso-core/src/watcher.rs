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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tempfile::TempDir;

    #[test]
    fn should_ignore_path_filters_noise_and_watcher_detects_changes() {
        assert!(should_ignore_path(Path::new("/repo/.git/objects/ab/cd")));
        assert!(should_ignore_path(Path::new("/repo/node_modules/pkg/index.js")));
        assert!(should_ignore_path(Path::new("/repo/target/debug/app")));
        assert!(should_ignore_path(Path::new("/ws/.kanso/kanso.db")));
        assert!(!should_ignore_path(Path::new("/repo/src/main.rs")));

        let temp = TempDir::new().unwrap();
        let triggered = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&triggered);

        let _watcher = WorkspaceWatcher::start(temp.path(), move || {
            flag.store(true, Ordering::Relaxed);
        })
        .expect("start watcher");

        std::thread::sleep(Duration::from_millis(100));
        fs::write(temp.path().join("hello.rs"), "fn main() {}\n").unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if triggered.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(triggered.load(Ordering::Relaxed));
    }
}
