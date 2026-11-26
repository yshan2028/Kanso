use kanso_core::WorkspaceConfig;
use kanso_engine::{EngineCommand, EngineConfig, EngineEvent, start};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_repo(dir: &Path, branch: &str) {
    fs::create_dir_all(dir).expect("create repo dir");
    git(dir, &["init", "-b", branch]);
    git(dir, &["config", "user.name", "Test User"]);
    git(dir, &["config", "user.email", "test@kanso.dev"]);
    fs::write(dir.join("README.md"), "# Test\n").expect("write readme");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "init"]);
}

fn test_config(root: &Path) -> EngineConfig {
    let ws = WorkspaceConfig {
        id: "ws-test".to_string(),
        name: "test-ws".to_string(),
        root_path: root.to_string_lossy().to_string(),
        default_trunk: Some("main".to_string()),
        ignore_dirs: Vec::new(),
    };
    let mut cfg = EngineConfig::new(ws);
    cfg.debounce = Duration::from_millis(100);
    cfg.min_interval = Duration::from_millis(200);
    cfg.self_write_cooldown = Duration::from_millis(800);
    cfg.calibrate_interval = Duration::from_secs(300);
    cfg.concurrency = 2;
    cfg
}

fn wait_for_snapshot(rx: &std::sync::mpsc::Receiver<EngineEvent>, timeout: Duration) -> kanso_core::WorkspaceSnapshot {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(EngineEvent::SnapshotReady(snap)) => return snap,
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    panic!("等待初始 SnapshotReady 超时");
}

#[test]
fn emits_initial_snapshot_with_two_repos() {
    let temp = TempDir::new().unwrap();
    init_repo(&temp.path().join("repo-a"), "main");
    init_repo(&temp.path().join("repo-b"), "main");

    let handle = start(test_config(temp.path())).expect("start engine");
    let snap = wait_for_snapshot(handle.events(), Duration::from_secs(5));
    assert_eq!(snap.repos.len(), 2);
    assert!(snap.failures.is_empty());

    handle.shutdown();
}

#[test]
fn modifying_repo_a_refreshes_only_repo_a() {
    let temp = TempDir::new().unwrap();
    let repo_a = temp.path().join("repo-a");
    let repo_b = temp.path().join("repo-b");
    init_repo(&repo_a, "main");
    init_repo(&repo_b, "main");

    let handle = start(test_config(temp.path())).expect("start engine");
    let _ = wait_for_snapshot(handle.events(), Duration::from_secs(5));

    // Give FSEvents a tiny settle window so creation events from setup don't mix in.
    std::thread::sleep(Duration::from_millis(250));
    while handle.events().try_recv().is_ok() {}

    fs::write(repo_a.join("change.txt"), "hello\n").unwrap();

    let mut updated_a = 0usize;
    let mut updated_b = 0usize;
    let deadline = Instant::now() + Duration::from_secs(6);
    while Instant::now() < deadline {
        let left = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(150));
        if let Ok(ev) = handle.events().recv_timeout(left) {
            if let EngineEvent::RepoUpdated(repo) = ev {
                if repo.name == "repo-a" {
                    updated_a += 1;
                } else if repo.name == "repo-b" {
                    updated_b += 1;
                }
            }
        }
        if updated_a >= 1 && Instant::now() + Duration::from_secs(1) >= deadline {
            break;
        }
    }

    assert!(updated_a >= 1, "expected repo-a refresh");
    assert_eq!(updated_b, 0, "repo-b must not be refreshed");

    handle.shutdown();
}

#[test]
fn burst_write_of_20_files_is_debounced_to_at_most_two_refreshes() {
    let temp = TempDir::new().unwrap();
    let repo_a = temp.path().join("repo-a");
    init_repo(&repo_a, "main");

    let handle = start(test_config(temp.path())).expect("start engine");
    let _ = wait_for_snapshot(handle.events(), Duration::from_secs(5));

    std::thread::sleep(Duration::from_millis(250));
    while handle.events().try_recv().is_ok() {}

    for i in 0..20 {
        fs::write(repo_a.join(format!("burst_{i}.txt")), format!("line {i}\n")).unwrap();
    }

    let mut refreshes = 0usize;
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        match handle.events().recv_timeout(left) {
            Ok(EngineEvent::RepoUpdated(r)) if r.name == "repo-a" => refreshes += 1,
            Ok(_) => {}
            Err(_) => break,
        }
    }

    assert!(
        (1..=2).contains(&refreshes),
        "expected 1..=2 refreshes for 20-file burst, got {refreshes}"
    );

    handle.shutdown();
}

#[test]
fn writes_to_ignored_build_dirs_trigger_zero_refreshes() {
    let temp = TempDir::new().unwrap();
    let repo_a = temp.path().join("repo-a");
    init_repo(&repo_a, "main");

    let handle = start(test_config(temp.path())).expect("start engine");
    let _ = wait_for_snapshot(handle.events(), Duration::from_secs(5));

    std::thread::sleep(Duration::from_millis(250));
    while handle.events().try_recv().is_ok() {}

    fs::create_dir_all(repo_a.join("target/debug")).unwrap();
    fs::create_dir_all(repo_a.join("node_modules/pkg")).unwrap();
    fs::write(repo_a.join("target/debug/out.o"), "bin").unwrap();
    fs::write(repo_a.join("node_modules/pkg/index.js"), "export default 1;").unwrap();

    let mut refreshes = 0usize;
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        match handle.events().recv_timeout(left) {
            Ok(EngineEvent::RepoUpdated(_)) | Ok(EngineEvent::RepoFailed(_)) => refreshes += 1,
            Ok(_) => {}
            Err(_) => break,
        }
    }

    assert_eq!(refreshes, 0, "ignored target/node_modules triggered refresh");

    handle.shutdown();
}

#[test]
fn self_write_cooldown_suppresses_during_window_and_refreshes_once_after() {
    let temp = TempDir::new().unwrap();
    let repo_a = temp.path().join("repo-a");
    init_repo(&repo_a, "main");

    let handle = start(test_config(temp.path())).expect("start engine");
    let _ = wait_for_snapshot(handle.events(), Duration::from_secs(5));

    std::thread::sleep(Duration::from_millis(250));
    while handle.events().try_recv().is_ok() {}

    let marked_at = Instant::now();
    handle
        .send(EngineCommand::MarkSelfWrite(repo_a.clone()))
        .unwrap();
    fs::write(repo_a.join("self_write.txt"), "generated\n").unwrap();

    // During cooldown (first 600ms out of 800ms), zero refreshes may occur.
    let mut during_cooldown = 0usize;
    let mut after_cooldown = 0usize;
    let total_deadline = marked_at + Duration::from_millis(2200);
    while Instant::now() < total_deadline {
        let left = total_deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(100));
        match handle.events().recv_timeout(left) {
            Ok(EngineEvent::RepoUpdated(r)) if r.name == "repo-a" => {
                if marked_at.elapsed() < Duration::from_millis(700) {
                    during_cooldown += 1;
                } else {
                    after_cooldown += 1;
                }
            }
            Ok(_) => {}
            Err(_) => {}
        }
    }

    assert_eq!(during_cooldown, 0, "must not refresh during cooldown window");
    assert_eq!(
        after_cooldown, 1,
        "must refresh exactly once after cooldown window ends"
    );

    handle.shutdown();
}

#[test]
fn refresh_all_discovers_newly_created_repo() {
    let temp = TempDir::new().unwrap();
    init_repo(&temp.path().join("repo-a"), "main");
    init_repo(&temp.path().join("repo-b"), "main");

    let handle = start(test_config(temp.path())).expect("start engine");
    let snap = wait_for_snapshot(handle.events(), Duration::from_secs(5));
    assert_eq!(snap.repos.len(), 2);

    let repo_c = temp.path().join("repo-c");
    init_repo(&repo_c, "main");
    let canonical_c = fs::canonicalize(&repo_c).unwrap();

    handle.send(EngineCommand::RefreshAll).unwrap();

    let mut found_c_added = false;
    let deadline = Instant::now() + Duration::from_secs(6);
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        match handle.events().recv_timeout(left) {
            Ok(EngineEvent::ReposChanged { added, .. }) => {
                if added
                    .iter()
                    .any(|p| Path::new(p) == canonical_c.as_path())
                {
                    found_c_added = true;
                    break;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }

    assert!(found_c_added, "expected ReposChanged added to contain repo-c");

    handle.shutdown();
}
