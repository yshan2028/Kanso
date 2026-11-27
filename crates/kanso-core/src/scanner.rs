use crate::commit::detect_vcs_kind;
use crate::config::load_kanso_yaml;
use crate::exec::GitAccess;
use crate::model::{
    RepoFailure, RepoStatus, VcsKind, WorkspaceConfig, WorkspaceSnapshot, WorktreeEntry,
};
use crate::task::{aggregate_tasks, detect_alerts};
use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use rayon::prelude::*;
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

const DEFAULT_IGNORED_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".venv",
    "venv",
    ".gradle",
    ".idea",
    ".vscode",
    "vendor",
    ".kanso",
    ".cargo",
];

pub const DEFAULT_MAX_DEPTH: usize = 6;
const TRUNK_CANDIDATES: &[&str] = &["main", "master", "develop", "dev"];

pub trait VcsBackend: Send + Sync {
    fn inspect_repo(
        &self,
        repo_path: &Path,
        workspace_root: &Path,
        default_trunk: Option<&str>,
    ) -> Result<RepoStatus>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct GitCliBackend;

impl GitCliBackend {
    fn run_git(&self, repo_path: &Path, args: &[&str]) -> Result<String> {
        crate::exec::git(repo_path, args, GitAccess::Read)
    }

    fn list_refs(&self, repo_path: &Path) -> HashSet<String> {
        let Ok(output) = crate::exec::git_output(repo_path, &["show-ref"], GitAccess::Read) else {
            return HashSet::new();
        };
        if !output.success {
            return HashSet::new();
        }
        output
            .stdout
            .lines()
            .filter_map(|line| line.split_whitespace().nth(1).map(str::to_string))
            .collect()
    }

    fn resolve_trunk(
        &self,
        repo_path: &Path,
        preferred: Option<&str>,
        refs: &HashSet<String>,
    ) -> Option<(String, String)> {
        if let Some(pref) = preferred.map(str::trim).filter(|s| !s.is_empty()) {
            let local_ref = format!("refs/heads/{pref}");
            if refs.contains(&local_ref) {
                return Some((pref.to_string(), local_ref));
            }
            let remote_ref = format!("refs/remotes/origin/{pref}");
            if refs.contains(&remote_ref) {
                return Some((pref.to_string(), remote_ref));
            }
        }

        if let Some(sym) = refs
            .contains("refs/remotes/origin/HEAD")
            .then(|| self.run_git(repo_path, &["symbolic-ref", "refs/remotes/origin/HEAD"]).ok())
            .flatten()
        {
            let sym = sym.trim();
            if let Some(branch) = sym.strip_prefix("refs/remotes/origin/") {
                let local_ref = format!("refs/heads/{branch}");
                if refs.contains(&local_ref) {
                    return Some((branch.to_string(), local_ref));
                }
                return Some((branch.to_string(), sym.to_string()));
            }
        }

        for candidate in TRUNK_CANDIDATES {
            let local_ref = format!("refs/heads/{candidate}");
            if refs.contains(&local_ref) {
                return Some(((*candidate).to_string(), local_ref));
            }
        }

        for candidate in TRUNK_CANDIDATES {
            let remote_ref = format!("refs/remotes/origin/{candidate}");
            if refs.contains(&remote_ref) {
                return Some(((*candidate).to_string(), remote_ref));
            }
        }

        None
    }

    fn parse_remotes(&self, repo_path: &Path, refs: &HashSet<String>) -> Vec<String> {
        let mut remotes = BTreeSet::new();
        for r in refs {
            if let Some(rest) = r.strip_prefix("refs/remotes/") {
                if let Some((remote_name, _)) = rest.split_once('/') {
                    remotes.insert(remote_name.to_string());
                }
            }
        }

        let dot_git = repo_path.join(".git");
        let config_path = if dot_git.is_dir() {
            Some(dot_git.join("config"))
        } else if dot_git.is_file() {
            fs::read_to_string(&dot_git).ok().and_then(|txt| {
                let gitdir_raw = txt.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
                let gitdir = if Path::new(gitdir_raw).is_absolute() {
                    PathBuf::from(gitdir_raw)
                } else {
                    repo_path.join(gitdir_raw)
                };
                let common = fs::read_to_string(gitdir.join("commondir")).ok();
                let base = match common {
                    Some(rel) => {
                        let p = Path::new(rel.trim());
                        if p.is_absolute() {
                            p.to_path_buf()
                        } else {
                            gitdir.join(p)
                        }
                    }
                    None => gitdir,
                };
                Some(base.join("config"))
            })
        } else {
            None
        };

        if let Some(cfg_path) = config_path {
            if let Ok(raw) = fs::read_to_string(cfg_path) {
                for line in raw.lines() {
                    let trimmed = line.trim();
                    if let Some(inner) = trimmed
                        .strip_prefix("[remote \"")
                        .and_then(|s| s.strip_suffix("\"]"))
                    {
                        remotes.insert(inner.to_string());
                    }
                }
            }
        }

        remotes.into_iter().collect()
    }

    fn list_worktrees(&self, repo_path: &Path) -> Vec<WorktreeEntry> {
        let Ok(raw) = self.run_git(repo_path, &["worktree", "list", "--porcelain"]) else {
            return Vec::new();
        };

        let mut entries = Vec::new();
        let mut current_path: Option<String> = None;
        let mut current_head: Option<String> = None;
        let mut current_branch: Option<String> = None;
        let mut is_prunable = false;

        let flush = |entries: &mut Vec<WorktreeEntry>,
                     path: &mut Option<String>,
                     head: &mut Option<String>,
                     branch: &mut Option<String>,
                     prunable: &mut bool| {
            if let Some(p) = path.take() {
                let is_main = entries.is_empty();
                entries.push(WorktreeEntry {
                    path: p,
                    branch: branch.take(),
                    head: head.take(),
                    is_main,
                    is_prunable: *prunable,
                });
            }
            *prunable = false;
        };

        for line in raw.lines() {
            if line.trim().is_empty() {
                flush(
                    &mut entries,
                    &mut current_path,
                    &mut current_head,
                    &mut current_branch,
                    &mut is_prunable,
                );
                continue;
            }
            if let Some(val) = line.strip_prefix("worktree ") {
                current_path = Some(val.to_string());
            } else if let Some(val) = line.strip_prefix("HEAD ") {
                current_head = Some(val.chars().take(8).collect());
            } else if let Some(val) = line.strip_prefix("branch ") {
                let cleaned = val.strip_prefix("refs/heads/").unwrap_or(val).to_string();
                current_branch = Some(cleaned);
            } else if line.starts_with("prunable") {
                is_prunable = true;
            }
        }

        flush(
            &mut entries,
            &mut current_path,
            &mut current_head,
            &mut current_branch,
            &mut is_prunable,
        );

        entries
    }
}

impl VcsBackend for GitCliBackend {
    fn inspect_repo(
        &self,
        repo_path: &Path,
        workspace_root: &Path,
        default_trunk: Option<&str>,
    ) -> Result<RepoStatus> {
        let vcs_kind = detect_vcs_kind(repo_path);
        if vcs_kind == VcsKind::Jj && !repo_path.join(".git").exists() {
            let st_out = crate::exec::jj(repo_path, &["diff", "--summary"], GitAccess::Read)
                .context("未安装 jj，无法读取纯 jj 仓库状态")?;
            let modified_count = st_out.lines().count() as u32;
            let canonical_path = fs::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
            let canonical_root =
                fs::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf());
            let relative_path = canonical_path
                .strip_prefix(&canonical_root)
                .map(|r| r.to_string_lossy().to_string())
                .unwrap_or_else(|_| canonical_path.to_string_lossy().to_string());
            let name = canonical_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| relative_path.clone());
            let path_str = canonical_path.to_string_lossy().to_string();
            let id = blake3::hash(path_str.as_bytes()).to_hex()[..16].to_string();
            return Ok(RepoStatus {
                id,
                name,
                path: path_str,
                relative_path,
                vcs_kind,
                is_worktree: false,
                current_branch: "jj@".to_string(),
                is_detached: false,
                trunk_branch: default_trunk.map(str::to_string),
                is_on_trunk: false,
                has_dirty_on_trunk: false,
                upstream: None,
                remotes: Vec::new(),
                upstream_ahead: 0,
                upstream_behind: 0,
                trunk_ahead: 0,
                trunk_behind: 0,
                staged_count: 0,
                modified_count,
                untracked_count: 0,
                conflicted_count: 0,
                last_commit_hash: None,
                last_commit_message: None,
                last_commit_author: None,
                last_commit_timestamp: None,
                worktrees: Vec::new(),
            });
        }

        let status_raw = self.run_git(repo_path, &["status", "--porcelain=v2", "--branch"])?;

        let mut current_branch = String::from("HEAD");
        let mut is_detached = false;
        let mut upstream: Option<String> = None;
        let mut upstream_ahead = 0_u32;
        let mut upstream_behind = 0_u32;
        let mut staged_count = 0_u32;
        let mut modified_count = 0_u32;
        let mut untracked_count = 0_u32;
        let mut conflicted_count = 0_u32;

        for line in status_raw.lines() {
            if let Some(head_name) = line.strip_prefix("# branch.head ") {
                if head_name == "(detached)" {
                    is_detached = true;
                    current_branch = String::from("detached");
                } else {
                    current_branch = head_name.to_string();
                }
            } else if let Some(up) = line.strip_prefix("# branch.upstream ") {
                upstream = Some(up.to_string());
            } else if let Some(ab) = line.strip_prefix("# branch.ab ") {
                for token in ab.split_whitespace() {
                    if let Some(num) = token.strip_prefix('+') {
                        upstream_ahead = num.parse().unwrap_or(0);
                    } else if let Some(num) = token.strip_prefix('-') {
                        upstream_behind = num.parse().unwrap_or(0);
                    }
                }
            } else if line.starts_with("1 ") || line.starts_with("2 ") {
                let mut parts = line.split_whitespace();
                let _tag = parts.next();
                if let Some(xy) = parts.next() {
                    let mut chars = xy.chars();
                    let x = chars.next().unwrap_or('.');
                    let y = chars.next().unwrap_or('.');
                    if x != '.' {
                        staged_count += 1;
                    }
                    if y != '.' {
                        modified_count += 1;
                    }
                }
            } else if line.starts_with("u ") {
                conflicted_count += 1;
            } else if line.starts_with("? ") {
                untracked_count += 1;
            }
        }

        let repo_yaml = load_kanso_yaml(repo_path).ok().flatten();
        let preferred_trunk = repo_yaml
            .as_ref()
            .and_then(|cfg| cfg.trunk_branch.as_deref())
            .or(default_trunk);

        let refs = self.list_refs(repo_path);
        let remotes = self.parse_remotes(repo_path, &refs);
        let trunk_info = self.resolve_trunk(repo_path, preferred_trunk, &refs);
        let trunk_branch = trunk_info.as_ref().map(|(name, _)| name.clone());
        let is_on_trunk = !is_detached
            && trunk_branch
                .as_deref()
                .is_some_and(|trunk| trunk == current_branch);

        let (trunk_ahead, trunk_behind) = if !is_detached && !is_on_trunk {
            if let Some((_, trunk_ref)) = &trunk_info {
                let range = format!("{trunk_ref}...HEAD");
                if let Ok(counts) = self.run_git(repo_path, &["rev-list", "--left-right", "--count", &range]) {
                    let mut nums = counts.split_whitespace();
                    let behind = nums.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    let ahead = nums.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    (ahead, behind)
                } else {
                    (0, 0)
                }
            } else {
                (0, 0)
            }
        } else {
            (0, 0)
        };

        let (last_commit_hash, last_commit_message, last_commit_author, last_commit_timestamp) =
            match self.run_git(repo_path, &["log", "-1", "--format=%h%x1f%s%x1f%an%x1f%ct"]) {
                Ok(raw) => {
                    let parts: Vec<&str> = raw.trim_end().split('\x1f').collect();
                    if parts.len() == 4 {
                        (
                            Some(parts[0].to_string()),
                            Some(parts[1].to_string()),
                            Some(parts[2].to_string()),
                            parts[3].parse::<i64>().ok(),
                        )
                    } else {
                        (None, None, None, None)
                    }
                }
                Err(_) => (None, None, None, None),
            };

        let is_worktree = repo_path.join(".git").is_file();
        let canonical_path = fs::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
        let path_str = canonical_path.to_string_lossy().to_string();
        let worktrees = if is_worktree || repo_path.join(".git").join("worktrees").exists() {
            self.list_worktrees(repo_path)
        } else {
            vec![WorktreeEntry {
                path: path_str.clone(),
                branch: if is_detached {
                    None
                } else {
                    Some(current_branch.clone())
                },
                head: last_commit_hash.clone(),
                is_main: true,
                is_prunable: false,
            }]
        };
        let canonical_root =
            fs::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf());
        let relative_path = canonical_path
            .strip_prefix(&canonical_root)
            .map(|rel| {
                let s = rel.to_string_lossy().to_string();
                if s.is_empty() {
                    String::from(".")
                } else {
                    s
                }
            })
            .unwrap_or_else(|_| canonical_path.to_string_lossy().to_string());

        let name = canonical_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| relative_path.clone());

        let id = blake3::hash(path_str.as_bytes()).to_hex()[..16].to_string();
        let has_dirty_on_trunk =
            is_on_trunk && (staged_count + modified_count + untracked_count + conflicted_count > 0);

        Ok(RepoStatus {
            id,
            name,
            path: path_str,
            relative_path,
            vcs_kind,
            is_worktree,
            current_branch,
            is_detached,
            trunk_branch,
            is_on_trunk,
            has_dirty_on_trunk,
            upstream,
            remotes,
            upstream_ahead,
            upstream_behind,
            trunk_ahead,
            trunk_behind,
            staged_count,
            modified_count,
            untracked_count,
            conflicted_count,
            last_commit_hash,
            last_commit_message,
            last_commit_author,
            last_commit_timestamp,
            worktrees,
        })
    }
}

pub fn discover_repos(
    root: &Path,
    ignore_dirs: &[String],
    max_depth: usize,
) -> Vec<PathBuf> {
    let mut ignored: HashSet<String> = DEFAULT_IGNORED_DIRS
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    for dir in ignore_dirs {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            ignored.insert(trimmed.to_string());
        }
    }

    let mut repos = Vec::new();
    walk_discover(root, &ignored, 0, max_depth, &mut repos);
    repos.sort();
    repos
}

fn walk_discover(
    dir: &Path,
    ignored: &HashSet<String>,
    depth: usize,
    max_depth: usize,
    out: &mut Vec<PathBuf>,
) {
    if depth > max_depth {
        return;
    }

    let dot_git = dir.join(".git");
    let dot_jj = dir.join(".jj");
    if dot_git.exists() || dot_jj.exists() {
        out.push(dir.to_path_buf());
        return;
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') || ignored.contains(name_str.as_ref()) {
            continue;
        }
        walk_discover(&path, ignored, depth + 1, max_depth, out);
    }
}

pub fn resolve_workspace_settings(workspace: &WorkspaceConfig) -> (Vec<String>, Option<String>) {
    let root = Path::new(&workspace.root_path);
    let root_yaml = load_kanso_yaml(root).ok().flatten();
    let mut ignore_dirs = workspace.ignore_dirs.clone();
    let mut effective_trunk = workspace.default_trunk.clone();
    if let Some(yaml) = &root_yaml {
        ignore_dirs.extend(yaml.ignore_dirs.iter().cloned());
        if effective_trunk.is_none() {
            effective_trunk = yaml.trunk_branch.clone();
        }
    }
    (ignore_dirs, effective_trunk)
}

fn scan_pool() -> &'static rayon::ThreadPool {
    static SCAN_POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    SCAN_POOL.get_or_init(|| {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2);
        let num_threads = (cpus / 2).min(4).max(1);
        rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .build()
            .expect("build kanso scan thread pool")
    })
}

fn compute_relative_path(repo_path: &Path, root: &Path) -> String {
    let canonical_path = fs::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
    let canonical_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    canonical_path
        .strip_prefix(&canonical_root)
        .map(|rel| {
            let s = rel.to_string_lossy().to_string();
            if s.is_empty() {
                String::from(".")
            } else {
                s
            }
        })
        .unwrap_or_else(|_| canonical_path.to_string_lossy().to_string())
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

pub fn scan_workspace(workspace: &WorkspaceConfig) -> Result<WorkspaceSnapshot> {
    let start = Instant::now();
    let root = PathBuf::from(&workspace.root_path);
    if !root.exists() || !root.is_dir() {
        return Err(anyhow!(
            "Workspace root directory does not exist: {}",
            workspace.root_path
        ));
    }

    let (ignore_dirs, effective_trunk) = resolve_workspace_settings(workspace);

    let repo_paths = discover_repos(&root, &ignore_dirs, DEFAULT_MAX_DEPTH);
    let backend = GitCliBackend;

    let outcomes: Vec<std::result::Result<RepoStatus, RepoFailure>> = scan_pool().install(|| {
        repo_paths
            .par_iter()
            .map(|path| {
                let res = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    backend.inspect_repo(path, &root, effective_trunk.as_deref())
                }));
                match res {
                    Ok(Ok(repo)) => Ok(repo),
                    Ok(Err(err)) => Err(RepoFailure {
                        repo_path: path.to_string_lossy().to_string(),
                        relative_path: compute_relative_path(path, &root),
                        error: err.to_string(),
                    }),
                    Err(payload) => Err(RepoFailure {
                        repo_path: path.to_string_lossy().to_string(),
                        relative_path: compute_relative_path(path, &root),
                        error: format!("扫描异常 panic: {}", panic_payload_message(payload)),
                    }),
                }
            })
            .collect()
    });

    let mut repos = Vec::new();
    let mut failures = Vec::new();
    for outcome in outcomes {
        match outcome {
            Ok(repo) => repos.push(repo),
            Err(failure) => failures.push(failure),
        }
    }

    repos.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    failures.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    let tasks = aggregate_tasks(&repos);
    let alerts = detect_alerts(&repos);

    Ok(WorkspaceSnapshot {
        workspace_id: workspace.id.clone(),
        workspace_name: workspace.name.clone(),
        root_path: workspace.root_path.clone(),
        scanned_at: Utc::now().timestamp(),
        scan_duration_ms: start.elapsed().as_millis() as u64,
        repos,
        failures,
        tasks,
        alerts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::GuardrailKind;
    use std::fs;
    use std::process::Command;
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

    #[test]
    fn scans_multi_repo_workspace_tasks_and_guardrails() {
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path();

        let repo_a = root.join("service-auth");
        let repo_b = root.join("service-order");
        let repo_c = root.join("web-admin");

        init_repo(&repo_a, "main");
        init_repo(&repo_b, "main");
        init_repo(&repo_c, "main");

        git(&repo_a, &["checkout", "-b", "feat/ai-checkout"]);
        fs::write(repo_a.join("auth.txt"), "v1\n").expect("write auth");
        git(&repo_a, &["add", "."]);
        git(&repo_a, &["commit", "-m", "add auth hook"]);
        fs::write(repo_a.join("auth.txt"), "v2\n").expect("modify auth");

        git(&repo_b, &["checkout", "-b", "feat/ai-checkout"]);

        fs::write(repo_c.join("dirty.txt"), "oops on trunk\n").expect("write dirty");

        let workspace = WorkspaceConfig {
            id: "ws-1".to_string(),
            name: "test-ws".to_string(),
            root_path: root.to_string_lossy().to_string(),
            default_trunk: Some("main".to_string()),
            ignore_dirs: Vec::new(),
        };

        let snapshot = scan_workspace(&workspace).expect("scan workspace");
        assert_eq!(snapshot.repos.len(), 3);
        assert_eq!(snapshot.tasks.len(), 1);
        assert_eq!(snapshot.tasks[0].branch, "feat/ai-checkout");
        assert_eq!(snapshot.tasks[0].total_repos, 2);
        assert_eq!(snapshot.tasks[0].dirty_repos, 1);

        assert_eq!(snapshot.alerts.len(), 1);
        assert_eq!(snapshot.alerts[0].repo_name, "web-admin");
        assert_eq!(snapshot.alerts[0].kind, GuardrailKind::DirtyTrunk);
    }

    #[test]
    fn scans_worktrees_kanso_yaml_and_detached_head() {
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path();

        fs::write(
            root.join(".kanso.yaml"),
            "trunk_branch: develop\nignore_dirs:\n  - archive\n",
        )
        .unwrap();

        let main_repo = root.join("core-service");
        init_repo(&main_repo, "develop");

        // Linked worktree placed under workspace root
        let wt_repo = root.join("core-service-wt");
        git(
            &main_repo,
            &[
                "worktree",
                "add",
                "-b",
                "feat/demo-task",
                wt_repo.to_str().unwrap(),
            ],
        );

        // Detached HEAD repo
        let detached_repo = root.join("detached-service");
        init_repo(&detached_repo, "develop");
        git(&detached_repo, &["checkout", "--detach", "HEAD"]);

        // Ignored repo inside archive/
        let ignored_repo = root.join("archive").join("legacy-repo");
        init_repo(&ignored_repo, "main");

        let workspace = WorkspaceConfig {
            id: "ws-yaml".to_string(),
            name: "yaml-ws".to_string(),
            root_path: root.to_string_lossy().to_string(),
            default_trunk: None,
            ignore_dirs: Vec::new(),
        };

        let snapshot = scan_workspace(&workspace).expect("scan");
        assert_eq!(snapshot.repos.len(), 3);
        assert!(!snapshot.repos.iter().any(|r| r.name == "legacy-repo"));

        let wt = snapshot
            .repos
            .iter()
            .find(|r| r.name == "core-service-wt")
            .expect("worktree repo");
        assert!(wt.is_worktree);
        assert_eq!(wt.trunk_branch.as_deref(), Some("develop"));
        assert_eq!(wt.worktrees.len(), 2);

        assert!(snapshot.alerts.iter().any(|a| {
            a.repo_name == "detached-service" && a.kind == GuardrailKind::DetachedHead
        }));
    }

    #[test]
    fn records_repo_failure_without_dropping_healthy_repos() {
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path();

        let ok_a = root.join("service-a");
        let ok_b = root.join("service-b");
        let broken = root.join("service-broken");

        init_repo(&ok_a, "main");
        init_repo(&ok_b, "main");
        init_repo(&broken, "main");
        fs::write(broken.join(".git/HEAD"), "not-a-valid-git-head\n").expect("corrupt HEAD");

        let workspace = WorkspaceConfig {
            id: "ws-fail".to_string(),
            name: "fail-ws".to_string(),
            root_path: root.to_string_lossy().to_string(),
            default_trunk: Some("main".to_string()),
            ignore_dirs: Vec::new(),
        };

        let snapshot = scan_workspace(&workspace).expect("scan succeeds overall");
        assert_eq!(snapshot.repos.len(), 2);
        assert_eq!(snapshot.failures.len(), 1);
        assert_eq!(snapshot.failures[0].relative_path, "service-broken");
        assert!(!snapshot.failures[0].error.is_empty());
    }
}
