use crate::config::EngineConfig;
use crate::event::{EngineCommand, EngineEvent};
use anyhow::{Context, Result, anyhow};
use kanso_core::{
    DEFAULT_MAX_DEPTH, GitCliBackend, RepoFailure, RepoStatus, VcsBackend, WorkspaceConfig,
    discover_repos, resolve_workspace_settings, scan_workspace,
};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::panic::AssertUnwindSafe;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::Instant;

enum ActorMsg {
    FsEvents(Vec<PathBuf>),
    WatcherError(String),
    Command(EngineCommand),
    WorkerDone {
        repo: PathBuf,
        outcome: std::result::Result<RepoStatus, RepoFailure>,
    },
}

struct RefreshJob {
    repo: PathBuf,
    workspace_root: PathBuf,
    effective_trunk: Option<String>,
}

pub struct EngineHandle {
    tx: mpsc::Sender<ActorMsg>,
    event_rx: Option<mpsc::Receiver<EngineEvent>>,
    actor_thread: Option<JoinHandle<()>>,
}

impl EngineHandle {
    pub fn send(&self, cmd: EngineCommand) -> Result<()> {
        self.tx
            .send(ActorMsg::Command(cmd))
            .map_err(|_| anyhow!("后台刷新引擎已关闭"))
    }

    pub fn events(&self) -> &mpsc::Receiver<EngineEvent> {
        self.event_rx
            .as_ref()
            .expect("事件接收端已被 take_events 移走")
    }

    pub fn take_events(&mut self) -> Option<mpsc::Receiver<EngineEvent>> {
        self.event_rx.take()
    }

    pub fn shutdown(mut self) {
        let _ = self.tx.send(ActorMsg::Command(EngineCommand::Shutdown));
        if let Some(handle) = self.actor_thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        if let Some(handle) = self.actor_thread.take() {
            let _ = self.tx.send(ActorMsg::Command(EngineCommand::Shutdown));
            let _ = handle.join();
        }
    }
}

pub fn should_ignore_path(path: &Path) -> bool {
    let mut in_git = false;
    for comp in path.components() {
        if let Component::Normal(name) = comp {
            let s = name.to_string_lossy();
            if in_git {
                if matches!(s.as_ref(), "objects" | "logs") {
                    return true;
                }
            }
            if matches!(
                s.as_ref(),
                "node_modules"
                    | "target"
                    | "build"
                    | "dist"
                    | "out"
                    | ".gradle"
                    | ".idea"
                    | ".kanso"
            ) {
                return true;
            }
            if s == ".git" {
                in_git = true;
            }
        }
    }

    if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
        if file_name.ends_with(".lock") {
            return true;
        }
    }

    false
}

fn canonicalize_or_keep(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn sort_repos_by_prefix_specificity(repos: &mut Vec<PathBuf>) {
    repos.sort_by(|a, b| {
        b.components()
            .count()
            .cmp(&a.components().count())
            .then_with(|| b.as_os_str().len().cmp(&a.as_os_str().len()))
            .then_with(|| a.cmp(b))
    });
}

fn match_repo_for_path(known_repos: &[PathBuf], event_path: &Path) -> Option<PathBuf> {
    if let Some(found) = known_repos.iter().find(|repo| event_path.starts_with(repo)) {
        return Some(found.clone());
    }
    let canonical = canonicalize_or_keep(event_path);
    known_repos
        .iter()
        .find(|repo| canonical.starts_with(repo))
        .cloned()
}

fn compute_relative_path(repo_path: &Path, root: &Path) -> String {
    repo_path
        .strip_prefix(root)
        .map(|rel| {
            let s = rel.to_string_lossy().to_string();
            if s.is_empty() {
                String::from(".")
            } else {
                s
            }
        })
        .unwrap_or_else(|_| repo_path.to_string_lossy().to_string())
}

fn panic_payload_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

fn spawn_workers(
    concurrency: usize,
    actor_tx: mpsc::Sender<ActorMsg>,
) -> (mpsc::Sender<RefreshJob>, Vec<JoinHandle<()>>) {
    let (job_tx, job_rx) = mpsc::channel::<RefreshJob>();
    let shared_rx = Arc::new(Mutex::new(job_rx));
    let mut handles = Vec::with_capacity(concurrency);

    for idx in 0..concurrency {
        let rx = Arc::clone(&shared_rx);
        let tx = actor_tx.clone();
        let handle = std::thread::Builder::new()
            .name(format!("kanso-engine-worker-{idx}"))
            .spawn(move || {
                let backend = GitCliBackend;
                loop {
                    let job = {
                        let Ok(guard) = rx.lock() else {
                            break;
                        };
                        guard.recv()
                    };
                    let Ok(job) = job else {
                        break;
                    };

                    let res = std::panic::catch_unwind(AssertUnwindSafe(|| {
                        backend.inspect_repo(
                            &job.repo,
                            &job.workspace_root,
                            job.effective_trunk.as_deref(),
                        )
                    }));

                    let outcome = match res {
                        Ok(Ok(status)) => Ok(status),
                        Ok(Err(err)) => Err(RepoFailure {
                            repo_path: job.repo.to_string_lossy().to_string(),
                            relative_path: compute_relative_path(&job.repo, &job.workspace_root),
                            error: err.to_string(),
                        }),
                        Err(payload) => Err(RepoFailure {
                            repo_path: job.repo.to_string_lossy().to_string(),
                            relative_path: compute_relative_path(&job.repo, &job.workspace_root),
                            error: format!("刷新异常 panic: {}", panic_payload_message(payload)),
                        }),
                    };

                    if tx
                        .send(ActorMsg::WorkerDone {
                            repo: job.repo,
                            outcome,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .expect("spawn kanso-engine worker");
        handles.push(handle);
    }

    (job_tx, handles)
}

pub fn start(cfg: EngineConfig) -> Result<EngineHandle> {
    let raw_root = PathBuf::from(&cfg.workspace.root_path);
    if !raw_root.exists() || !raw_root.is_dir() {
        return Err(anyhow!(
            "工作区目录不存在: {}",
            cfg.workspace.root_path
        ));
    }
    let canonical_root = fs::canonicalize(&raw_root)
        .with_context(|| format!("规范化工作区路径失败: {}", raw_root.display()))?;

    let mut canonical_ws: WorkspaceConfig = cfg.workspace.clone();
    canonical_ws.root_path = canonical_root.to_string_lossy().to_string();

    let (actor_tx, actor_rx) = mpsc::channel::<ActorMsg>();
    let (event_tx, event_rx) = mpsc::channel::<EngineEvent>();

    let watcher_tx = actor_tx.clone();
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(
        move |res: std::result::Result<Event, notify::Error>| match res {
            Ok(ev) => {
                if matches!(ev.kind, notify::EventKind::Access(_)) {
                    return;
                }
                let relevant: Vec<PathBuf> = ev
                    .paths
                    .into_iter()
                    .filter(|p| !should_ignore_path(p))
                    .collect();
                if !relevant.is_empty() {
                    let _ = watcher_tx.send(ActorMsg::FsEvents(relevant));
                }
            }
            Err(err) => {
                let _ = watcher_tx.send(ActorMsg::WatcherError(err.to_string()));
            }
        },
    )
    .context("创建文件系统监听器失败")?;

    watcher
        .watch(&canonical_root, RecursiveMode::Recursive)
        .with_context(|| format!("监听工作区目录失败: {}", canonical_root.display()))?;

    let actor_tx_for_workers = actor_tx.clone();
    let actor_thread = std::thread::Builder::new()
        .name("kanso-engine-actor".to_string())
        .spawn(move || {
            run_actor(
                cfg,
                canonical_ws,
                canonical_root,
                watcher,
                actor_tx_for_workers,
                actor_rx,
                event_tx,
            );
        })
        .context("启动 kanso-engine actor 线程失败")?;

    Ok(EngineHandle {
        tx: actor_tx,
        event_rx: Some(event_rx),
        actor_thread: Some(actor_thread),
    })
}

fn discover_canonical_repos(
    canonical_ws: &WorkspaceConfig,
    canonical_root: &Path,
) -> (Vec<PathBuf>, Option<String>) {
    let (ignore_dirs, effective_trunk) = resolve_workspace_settings(canonical_ws);
    let mut repos: Vec<PathBuf> = discover_repos(canonical_root, &ignore_dirs, DEFAULT_MAX_DEPTH)
        .into_iter()
        .map(|p| canonicalize_or_keep(&p))
        .collect();
    sort_repos_by_prefix_specificity(&mut repos);
    (repos, effective_trunk)
}

fn schedule_after_event(
    repo: PathBuf,
    now: Instant,
    cfg: &EngineConfig,
    cooldown_until: &HashMap<PathBuf, Instant>,
    last_refresh_at: &HashMap<PathBuf, Instant>,
    pending_deadlines: &mut HashMap<PathBuf, Instant>,
    in_flight: &HashSet<PathBuf>,
    dirty_while_in_flight: &mut HashSet<PathBuf>,
) {
    if in_flight.contains(&repo) {
        dirty_while_in_flight.insert(repo);
        return;
    }

    if let Some(&cd_until) = cooldown_until.get(&repo) {
        if now < cd_until {
            // Self-write cooldown active: ignore intermediate events, keep single cooldown-end refresh.
            return;
        }
    }

    let base = now + cfg.debounce;
    let min_next = last_refresh_at
        .get(&repo)
        .map(|t| *t + cfg.min_interval)
        .unwrap_or(now);
    let deadline = base.max(min_next);
    pending_deadlines.insert(repo, deadline);
}

fn run_actor(
    cfg: EngineConfig,
    canonical_ws: WorkspaceConfig,
    canonical_root: PathBuf,
    _watcher: RecommendedWatcher,
    actor_tx: mpsc::Sender<ActorMsg>,
    actor_rx: mpsc::Receiver<ActorMsg>,
    event_tx: mpsc::Sender<EngineEvent>,
) {
    let (job_tx, worker_handles) = spawn_workers(cfg.concurrency.max(1), actor_tx);

    let (mut known_repos, mut effective_trunk) =
        discover_canonical_repos(&canonical_ws, &canonical_root);

    match scan_workspace(&canonical_ws) {
        Ok(snapshot) => {
            if event_tx.send(EngineEvent::SnapshotReady(snapshot)).is_err() {
                drop(job_tx);
                for h in worker_handles {
                    let _ = h.join();
                }
                return;
            }
        }
        Err(err) => {
            let _ = event_tx.send(EngineEvent::Error(format!("初始扫描失败: {err}")));
        }
    }

    // Drop any stale pre-snapshot FS events that were queued during initial scan.
    let mut pending_deadlines: HashMap<PathBuf, Instant> = HashMap::new();
    let mut last_refresh_at: HashMap<PathBuf, Instant> = HashMap::new();
    let mut cooldown_until: HashMap<PathBuf, Instant> = HashMap::new();
    let mut in_flight: HashSet<PathBuf> = HashSet::new();
    let mut dirty_while_in_flight: HashSet<PathBuf> = HashSet::new();
    let mut next_calibrate = Instant::now() + cfg.calibrate_interval;

    loop {
        let now = Instant::now();

        // Clean expired cooldowns and compute next wakeup deadline.
        cooldown_until.retain(|_, until| *until > now);

        let earliest_pending = pending_deadlines.values().copied().min();
        let next_wakeup = match earliest_pending {
            Some(d) => d.min(next_calibrate),
            None => next_calibrate,
        };
        let wait_dur = next_wakeup.saturating_duration_since(now);

        match actor_rx.recv_timeout(wait_dur) {
            Ok(msg) => {
                if !handle_actor_msg(
                    msg,
                    &cfg,
                    &canonical_ws,
                    &canonical_root,
                    &mut known_repos,
                    &mut effective_trunk,
                    &mut pending_deadlines,
                    &mut last_refresh_at,
                    &mut cooldown_until,
                    &mut in_flight,
                    &mut dirty_while_in_flight,
                    &mut next_calibrate,
                    &event_tx,
                ) {
                    break;
                }
                // Drain any immediately ready messages before dispatching.
                let mut should_stop = false;
                while let Ok(more) = actor_rx.try_recv() {
                    if !handle_actor_msg(
                        more,
                        &cfg,
                        &canonical_ws,
                        &canonical_root,
                        &mut known_repos,
                        &mut effective_trunk,
                        &mut pending_deadlines,
                        &mut last_refresh_at,
                        &mut cooldown_until,
                        &mut in_flight,
                        &mut dirty_while_in_flight,
                        &mut next_calibrate,
                        &event_tx,
                    ) {
                        should_stop = true;
                        break;
                    }
                }
                if should_stop {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        let now = Instant::now();

        if now >= next_calibrate {
            calibrate_repos(
                &canonical_ws,
                &canonical_root,
                &mut known_repos,
                &mut effective_trunk,
                &mut pending_deadlines,
                &in_flight,
                &mut dirty_while_in_flight,
                &event_tx,
                now,
                false,
            );
            next_calibrate = now + cfg.calibrate_interval;
        }

        let due_repos: Vec<PathBuf> = pending_deadlines
            .iter()
            .filter_map(|(repo, &deadline)| (deadline <= now).then(|| repo.clone()))
            .collect();

        for repo in due_repos {
            pending_deadlines.remove(&repo);
            if !known_repos.contains(&repo) {
                continue;
            }
            if in_flight.contains(&repo) {
                dirty_while_in_flight.insert(repo);
                continue;
            }
            in_flight.insert(repo.clone());
            if job_tx
                .send(RefreshJob {
                    repo,
                    workspace_root: canonical_root.clone(),
                    effective_trunk: effective_trunk.clone(),
                })
                .is_err()
            {
                break;
            }
        }
    }

    drop(job_tx);
    for h in worker_handles {
        let _ = h.join();
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_actor_msg(
    msg: ActorMsg,
    cfg: &EngineConfig,
    canonical_ws: &WorkspaceConfig,
    canonical_root: &Path,
    known_repos: &mut Vec<PathBuf>,
    effective_trunk: &mut Option<String>,
    pending_deadlines: &mut HashMap<PathBuf, Instant>,
    last_refresh_at: &mut HashMap<PathBuf, Instant>,
    cooldown_until: &mut HashMap<PathBuf, Instant>,
    in_flight: &mut HashSet<PathBuf>,
    dirty_while_in_flight: &mut HashSet<PathBuf>,
    next_calibrate: &mut Instant,
    event_tx: &mpsc::Sender<EngineEvent>,
) -> bool {
    let now = Instant::now();
    match msg {
        ActorMsg::FsEvents(paths) => {
            let mut matched = HashSet::new();
            for p in paths {
                if let Some(repo) = match_repo_for_path(known_repos, &p) {
                    matched.insert(repo);
                }
            }
            for repo in matched {
                schedule_after_event(
                    repo,
                    now,
                    cfg,
                    cooldown_until,
                    last_refresh_at,
                    pending_deadlines,
                    in_flight,
                    dirty_while_in_flight,
                );
            }
        }
        ActorMsg::WatcherError(err) => {
            let _ = event_tx.send(EngineEvent::Error(format!("文件监听异常: {err}")));
        }
        ActorMsg::WorkerDone { repo, outcome } => {
            in_flight.remove(&repo);
            last_refresh_at.insert(repo.clone(), now);
            let event = match outcome {
                Ok(status) => EngineEvent::RepoUpdated(status),
                Err(failure) => EngineEvent::RepoFailed(failure),
            };
            if event_tx.send(event).is_err() {
                return false;
            }
            if dirty_while_in_flight.remove(&repo) && known_repos.contains(&repo) {
                let next_at = (now + cfg.debounce).max(now + cfg.min_interval);
                let effective_at = cooldown_until
                    .get(&repo)
                    .copied()
                    .map(|cd| next_at.max(cd))
                    .unwrap_or(next_at);
                pending_deadlines.insert(repo, effective_at);
            }
        }
        ActorMsg::Command(cmd) => match cmd {
            EngineCommand::RefreshRepo(path) => {
                let target = canonicalize_or_keep(&path);
                if known_repos.contains(&target) {
                    if in_flight.contains(&target) {
                        dirty_while_in_flight.insert(target);
                    } else {
                        pending_deadlines.insert(target, now);
                    }
                }
            }
            EngineCommand::RefreshAll => {
                calibrate_repos(
                    canonical_ws,
                    canonical_root,
                    known_repos,
                    effective_trunk,
                    pending_deadlines,
                    in_flight,
                    dirty_while_in_flight,
                    event_tx,
                    now,
                    true,
                );
                *next_calibrate = now + cfg.calibrate_interval;
            }
            EngineCommand::MarkSelfWrite(path) => {
                let target = canonicalize_or_keep(&path);
                let cd_end = now + cfg.self_write_cooldown;
                cooldown_until.insert(target.clone(), cd_end);
                if known_repos.contains(&target) {
                    pending_deadlines.insert(target, cd_end);
                }
            }
            EngineCommand::Shutdown => return false,
        },
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn calibrate_repos(
    canonical_ws: &WorkspaceConfig,
    canonical_root: &Path,
    known_repos: &mut Vec<PathBuf>,
    effective_trunk: &mut Option<String>,
    pending_deadlines: &mut HashMap<PathBuf, Instant>,
    in_flight: &HashSet<PathBuf>,
    dirty_while_in_flight: &mut HashSet<PathBuf>,
    event_tx: &mpsc::Sender<EngineEvent>,
    now: Instant,
    schedule_all: bool,
) {
    let (new_repos, new_trunk) = discover_canonical_repos(canonical_ws, canonical_root);
    *effective_trunk = new_trunk;

    let old_set: BTreeSet<PathBuf> = known_repos.iter().cloned().collect();
    let new_set: BTreeSet<PathBuf> = new_repos.iter().cloned().collect();

    let added: Vec<String> = new_set
        .difference(&old_set)
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let removed: Vec<String> = old_set
        .difference(&new_set)
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    *known_repos = new_repos;

    if !added.is_empty() || !removed.is_empty() {
        let _ = event_tx.send(EngineEvent::ReposChanged {
            added: added.clone(),
            removed,
        });
    }

    if schedule_all {
        for repo in known_repos.iter() {
            if in_flight.contains(repo) {
                dirty_while_in_flight.insert(repo.clone());
            } else {
                pending_deadlines.insert(repo.clone(), now);
            }
        }
    } else {
        for added_path in added {
            let p = PathBuf::from(added_path);
            pending_deadlines.insert(p, now);
        }
    }
}
