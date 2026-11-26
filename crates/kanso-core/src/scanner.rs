use crate::config::load_kanso_yaml;
use crate::model::{RepoStatus, WorkspaceConfig, WorkspaceSnapshot, WorktreeEntry};
use crate::task::{aggregate_tasks, detect_alerts};
use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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

const DEFAULT_MAX_DEPTH: usize = 6;
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
        let output = Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .args(args)
            .output()
            .with_context(|| format!("Failed to spawn git in {}", repo_path.display()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!(
                "git {} failed in {}: {}",
                args.join(" "),
                repo_path.display(),
                stderr.trim()
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn list_refs(&self, repo_path: &Path) -> HashSet<String> {
        let Ok(output) = Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("show-ref")
            .output()
        else {
            return HashSet::new();
        };
        if !output.status.success() {
            return HashSet::new();
        }
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.split_whitespace().nth(1).map(str::to_string))
            .collect()
    }

    fn resolve_trunk(&self, repo_path: &Path, preferred: Option<&str>) -> Option<(String, String)> {
        let refs = self.list_refs(repo_path);

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

        let trunk_info = self.resolve_trunk(repo_path, preferred_trunk);
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
            is_worktree,
            current_branch,
            is_detached,
            trunk_branch,
            is_on_trunk,
            has_dirty_on_trunk,
            upstream,
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

pub fn discover_git_repos(
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
    if dot_git.exists() {
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

pub fn scan_workspace(workspace: &WorkspaceConfig) -> Result<WorkspaceSnapshot> {
    let start = Instant::now();
    let root = PathBuf::from(&workspace.root_path);
    if !root.exists() || !root.is_dir() {
        return Err(anyhow!(
            "Workspace root directory does not exist: {}",
            workspace.root_path
        ));
    }

    let root_yaml = load_kanso_yaml(&root).ok().flatten();
    let mut ignore_dirs = workspace.ignore_dirs.clone();
    let mut effective_trunk = workspace.default_trunk.clone();
    if let Some(yaml) = &root_yaml {
        ignore_dirs.extend(yaml.ignore_dirs.iter().cloned());
        if effective_trunk.is_none() {
            effective_trunk = yaml.trunk_branch.clone();
        }
    }

    let repo_paths = discover_git_repos(&root, &ignore_dirs, DEFAULT_MAX_DEPTH);
    let backend = GitCliBackend;

    let mut repos: Vec<RepoStatus> = repo_paths
        .par_iter()
        .filter_map(|path| {
            backend
                .inspect_repo(path, &root, effective_trunk.as_deref())
                .ok()
        })
        .collect();

    repos.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    let tasks = aggregate_tasks(&repos);
    let alerts = detect_alerts(&repos);

    Ok(WorkspaceSnapshot {
        workspace_id: workspace.id.clone(),
        workspace_name: workspace.name.clone(),
        root_path: workspace.root_path.clone(),
        scanned_at: Utc::now().timestamp(),
        scan_duration_ms: start.elapsed().as_millis() as u64,
        repos,
        tasks,
        alerts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::GuardrailKind;
    use std::fs;
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
            "trunk_branch: dev-Java\nignore_dirs:\n  - archive\n",
        )
        .unwrap();

        let main_repo = root.join("core-service");
        init_repo(&main_repo, "dev-Java");

        // Linked worktree placed under workspace root
        let wt_repo = root.join("core-service-wt");
        git(
            &main_repo,
            &[
                "worktree",
                "add",
                "-b",
                "java/jv-200-test",
                wt_repo.to_str().unwrap(),
            ],
        );

        // Detached HEAD repo
        let detached_repo = root.join("detached-service");
        init_repo(&detached_repo, "dev-Java");
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
        assert_eq!(wt.trunk_branch.as_deref(), Some("dev-Java"));
        assert_eq!(wt.worktrees.len(), 2);

        assert!(snapshot.alerts.iter().any(|a| {
            a.repo_name == "detached-service" && a.kind == GuardrailKind::DetachedHead
        }));
    }
}
