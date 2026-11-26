use kanso_core::WorkspaceConfig;
use kanso_engine::{EngineCommand, EngineConfig, EngineEvent, start};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

fn init_repo(dir: &Path) {
    fs::create_dir_all(dir).expect("mkdir repo");
    git(dir, &["init", "-b", "main"]);
    git(dir, &["config", "user.name", "Stress Bot"]);
    git(dir, &["config", "user.email", "stress@kanso.dev"]);
    fs::write(dir.join("src.txt"), "base\n").expect("write src");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "init"]);
}

fn count_open_fds() -> usize {
    #[cfg(target_os = "macos")]
    let dir = "/dev/fd";
    #[cfg(not(target_os = "macos"))]
    let dir = "/proc/self/fd";

    fs::read_dir(dir)
        .map(|rd| rd.count())
        .unwrap_or(0)
}

#[test]
#[ignore = "长时间高压稳定性测试，需手动或带 --ignored 运行"]
fn engine_10min_high_frequency_stress() {
    let duration_secs: u64 = std::env::var("KANSO_STRESS_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);
    let total_dur = Duration::from_secs(duration_secs);

    let temp = TempDir::new().expect("temp dir");
    let mut repos: Vec<PathBuf> = Vec::with_capacity(30);
    for i in 0..30 {
        let dir = temp.path().join(format!("service-{i:02}"));
        init_repo(&dir);
        repos.push(dir);
    }

    let ws = WorkspaceConfig {
        id: "stress-ws".to_string(),
        name: "stress-ws".to_string(),
        root_path: temp.path().to_string_lossy().to_string(),
        default_trunk: Some("main".to_string()),
        ignore_dirs: Vec::new(),
    };
    let mut cfg = EngineConfig::new(ws);
    cfg.debounce = Duration::from_millis(100);
    cfg.min_interval = Duration::from_millis(250);
    cfg.self_write_cooldown = Duration::from_millis(400);
    cfg.calibrate_interval = Duration::from_secs(60);
    cfg.concurrency = 4;

    let handle = start(cfg).expect("start engine");

    // 等待初始快照
    let init_deadline = Instant::now() + Duration::from_secs(30);
    let mut got_snap = false;
    while Instant::now() < init_deadline {
        match handle.events().recv_timeout(Duration::from_millis(200)) {
            Ok(EngineEvent::SnapshotReady(snap)) => {
                assert_eq!(snap.repos.len(), 30);
                got_snap = true;
                break;
            }
            Ok(_) => {}
            Err(_) => {}
        }
    }
    assert!(got_snap, "未收到初始快照");

    let fd_start = count_open_fds();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let mut writers = Vec::new();

    for t in 0..4 {
        let stop = Arc::clone(&stop_flag);
        let worker_repos = repos.clone();
        writers.push(std::thread::spawn(move || {
            let mut rng_state: u64 = 0x9E37_79B9_7F4A_7C15 ^ (t as u64 + 1);
            let mut iter = 0u64;
            while !stop.load(Ordering::Relaxed) {
                rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
                let idx = (rng_state as usize) % worker_repos.len();
                let repo = &worker_repos[idx];
                let file_idx = iter % 5;
                let file = repo.join(format!("work_{t}_{file_idx}.txt"));
                let _ = fs::write(&file, format!("worker={t} iter={iter}\n"));
                if iter % 40 == 0 {
                    // 偶尔执行 git add 引发 .git/index 变动
                    let _ = Command::new("git")
                        .arg("-C")
                        .arg(repo)
                        .args(["add", "."])
                        .output();
                }
                iter += 1;
                std::thread::sleep(Duration::from_millis(20));
            }
        }));
    }

    let started = Instant::now();
    let mut next_probe = Instant::now() + Duration::from_secs(10);
    let mut probes_passed = 0usize;
    let mut updated_total = 0usize;
    let mut failed_total = 0usize;

    while started.elapsed() < total_dur {
        let now = Instant::now();
        if now >= next_probe && started.elapsed() + Duration::from_secs(4) <= total_dur {
            let target_repo = &repos[probes_passed % repos.len()];
            let canonical_target = fs::canonicalize(target_repo).unwrap_or_else(|_| target_repo.clone());
            handle
                .send(EngineCommand::RefreshRepo(target_repo.clone()))
                .expect("send RefreshRepo");

            let probe_deadline = Instant::now() + Duration::from_secs(3);
            let mut probe_ok = false;
            while Instant::now() < probe_deadline {
                match handle.events().recv_timeout(Duration::from_millis(100)) {
                    Ok(EngineEvent::RepoUpdated(r)) => {
                        updated_total += 1;
                        if Path::new(&r.path) == canonical_target.as_path() {
                            probe_ok = true;
                            break;
                        }
                    }
                    Ok(EngineEvent::RepoFailed(_)) => {
                        failed_total += 1;
                    }
                    Ok(_) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        panic!("engine events channel disconnected unexpectedly");
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
            assert!(
                probe_ok,
                "第 {} 次 RefreshRepo 未在 3s 内收到 {:?} 的 RepoUpdated",
                probes_passed + 1,
                canonical_target
            );
            probes_passed += 1;
            next_probe = Instant::now() + Duration::from_secs(10);
        } else {
            match handle.events().recv_timeout(Duration::from_millis(100)) {
                Ok(EngineEvent::RepoUpdated(_)) => updated_total += 1,
                Ok(EngineEvent::RepoFailed(_)) => failed_total += 1,
                Ok(_) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("engine events channel disconnected unexpectedly");
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }

    stop_flag.store(true, Ordering::Relaxed);
    for w in writers {
        let _ = w.join();
    }

    // 清空残余队列事件
    while let Ok(ev) = handle.events().try_recv() {
        match ev {
            EngineEvent::RepoUpdated(_) => updated_total += 1,
            EngineEvent::RepoFailed(_) => failed_total += 1,
            _ => {}
        }
    }

    let fd_end = count_open_fds();
    let fd_diff = fd_end.abs_diff(fd_start);

    println!(
        "STRESS_DONE duration={}s probes_passed={} updated_total={} failed_total={} fd_start={} fd_end={} fd_diff={}",
        duration_secs, probes_passed, updated_total, failed_total, fd_start, fd_end, fd_diff
    );

    assert!(
        fd_diff <= 20,
        "打开文件描述符数量异常增长: fd_start={fd_start}, fd_end={fd_end}, diff={fd_diff}"
    );
    assert_eq!(failed_total, 0, "压测期间出现意外失败仓库数: {failed_total}");

    handle.shutdown();
}
