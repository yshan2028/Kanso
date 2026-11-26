use anyhow::{Context, Result};
use kanso_core::WorkspaceConfig;
use kanso_engine::{EngineConfig, EngineEvent, start};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let verbose = args.iter().any(|a| a == "--verbose");
    let positional: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| *a != "--verbose")
        .collect();

    let root = PathBuf::from(positional.first().copied().unwrap_or("."));
    let seconds: u64 = positional
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(15);

    let canonical_root =
        std::fs::canonicalize(&root).with_context(|| format!("工作区不存在: {}", root.display()))?;
    let name = canonical_root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "workspace".to_string());

    let ws = WorkspaceConfig {
        id: format!("watch-{name}"),
        name,
        root_path: canonical_root.to_string_lossy().to_string(),
        default_trunk: None,
        ignore_dirs: Vec::new(),
    };

    let handle = start(EngineConfig::new(ws))?;

    let mut initial_done = false;
    let init_deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < init_deadline {
        let left = init_deadline.saturating_duration_since(Instant::now());
        match handle.events().recv_timeout(left) {
            Ok(EngineEvent::SnapshotReady(snap)) => {
                println!(
                    "初始快照完成: {} 个仓库 (失败 {} 个), 耗时 {} ms",
                    snap.repos.len(),
                    snap.failures.len(),
                    snap.scan_duration_ms
                );
                initial_done = true;
                break;
            }
            Ok(EngineEvent::Error(err)) => {
                eprintln!("引擎错误: {err}");
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }

    if !initial_done {
        anyhow::bail!("等待初始快照超时");
    }

    let watch_deadline = Instant::now() + Duration::from_secs(seconds);
    let mut refresh_count = 0usize;
    while Instant::now() < watch_deadline {
        let left = watch_deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(250));
        match handle.events().recv_timeout(left) {
            Ok(EngineEvent::RepoUpdated(repo)) => {
                refresh_count += 1;
                if verbose {
                    println!(
                        "[verbose] 触发刷新: {} ({})",
                        repo.relative_path, repo.path
                    );
                }
            }
            Ok(EngineEvent::RepoFailed(fail)) => {
                refresh_count += 1;
                if verbose {
                    println!(
                        "[verbose] 刷新失败: {} ({}) -> {}",
                        fail.relative_path, fail.repo_path, fail.error
                    );
                }
            }
            Ok(EngineEvent::ReposChanged { added, removed }) if verbose => {
                println!(
                    "[verbose] 仓库列表变更: +{} -{}",
                    added.len(),
                    removed.len()
                );
            }
            Ok(EngineEvent::Error(err)) => {
                eprintln!("引擎事件错误: {err}");
            }
            Ok(_) => {}
            Err(_) => {}
        }
    }

    println!("空闲期间刷新次数: {refresh_count}");
    handle.shutdown();
    Ok(())
}
