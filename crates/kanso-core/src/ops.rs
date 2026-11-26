use crate::model::BatchOpResult;
use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

fn validate_branch_name(repo_path: &Path, branch_name: &str) -> Result<()> {
    let trimmed = branch_name.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Branch name cannot be empty"));
    }
    let check_ref = format!("refs/heads/{trimmed}");
    let format_ok = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["check-ref-format", &check_ref])
        .status()
        .context("Failed to execute git check-ref-format")?;
    if !format_ok.success() {
        return Err(anyhow!("Invalid Git branch name: {trimmed}"));
    }

    let exists = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["show-ref", "--verify", "--quiet", &check_ref])
        .status()
        .context("Failed to execute git show-ref")?;
    if exists.success() {
        return Err(anyhow!("Branch '{trimmed}' already exists"));
    }

    Ok(())
}

pub fn rescue_dirty_trunk(repo_path: &Path, new_branch: &str) -> Result<String> {
    let branch = new_branch.trim();
    validate_branch_name(repo_path, branch)?;

    let out = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["checkout", "-b", branch])
        .output()
        .with_context(|| format!("Failed to checkout new branch in {}", repo_path.display()))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow!("Failed to rescue changes: {}", stderr.trim()));
    }

    Ok(format!("Moved uncommitted changes to branch '{branch}'"))
}

pub fn batch_create_branch(
    repo_paths: &[PathBuf],
    branch_name: &str,
    base_branch: Option<&str>,
) -> Vec<BatchOpResult> {
    let branch = branch_name.trim();
    repo_paths
        .iter()
        .map(|path| {
            let name = repo_name(path);
            let path_str = path.to_string_lossy().to_string();

            if let Err(err) = validate_branch_name(path, branch) {
                return BatchOpResult {
                    repo_path: path_str,
                    repo_name: name,
                    success: false,
                    message: err.to_string(),
                };
            }

            let mut args = vec!["checkout", "-b", branch];
            if let Some(base) = base_branch.map(str::trim).filter(|s| !s.is_empty()) {
                args.push(base);
            }

            match Command::new("git").arg("-C").arg(path).args(&args).output() {
                Ok(out) if out.status.success() => BatchOpResult {
                    repo_path: path_str,
                    repo_name: name,
                    success: true,
                    message: format!("Switched to new branch '{branch}'"),
                },
                Ok(out) => BatchOpResult {
                    repo_path: path_str,
                    repo_name: name,
                    success: false,
                    message: String::from_utf8_lossy(&out.stderr).trim().to_string(),
                },
                Err(err) => BatchOpResult {
                    repo_path: path_str,
                    repo_name: name,
                    success: false,
                    message: err.to_string(),
                },
            }
        })
        .collect()
}

pub fn batch_pull_trunk(repos: &[(PathBuf, String)]) -> Vec<BatchOpResult> {
    repos
        .iter()
        .map(|(path, trunk)| {
            let name = repo_name(path);
            let path_str = path.to_string_lossy().to_string();

            let status = Command::new("git")
                .arg("-C")
                .arg(path)
                .args(["status", "--porcelain"])
                .output();
            match status {
                Ok(out) if !String::from_utf8_lossy(&out.stdout).trim().is_empty() => {
                    return BatchOpResult {
                        repo_path: path_str,
                        repo_name: name,
                        success: false,
                        message: "Skipped: working tree has uncommitted changes".to_string(),
                    };
                }
                Err(err) => {
                    return BatchOpResult {
                        repo_path: path_str,
                        repo_name: name,
                        success: false,
                        message: err.to_string(),
                    };
                }
                _ => {}
            }

            let head = Command::new("git")
                .arg("-C")
                .arg(path)
                .args(["rev-parse", "--abbrev-ref", "HEAD"])
                .output();
            if let Ok(out) = head {
                let current = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if current != *trunk {
                    return BatchOpResult {
                        repo_path: path_str,
                        repo_name: name,
                        success: false,
                        message: format!("Skipped: currently on '{current}', not '{trunk}'"),
                    };
                }
            }

            match Command::new("git")
                .arg("-C")
                .arg(path)
                .args(["pull", "--ff-only"])
                .output()
            {
                Ok(out) if out.status.success() => BatchOpResult {
                    repo_path: path_str,
                    repo_name: name,
                    success: true,
                    message: String::from_utf8_lossy(&out.stdout)
                        .lines()
                        .last()
                        .unwrap_or("Up to date")
                        .to_string(),
                },
                Ok(out) => BatchOpResult {
                    repo_path: path_str,
                    repo_name: name,
                    success: false,
                    message: String::from_utf8_lossy(&out.stderr).trim().to_string(),
                },
                Err(err) => BatchOpResult {
                    repo_path: path_str,
                    repo_name: name,
                    success: false,
                    message: err.to_string(),
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn rescues_dirty_trunk_without_losing_changes() {
        let temp = TempDir::new().expect("temp dir");
        let repo = temp.path().join("app");
        fs::create_dir_all(&repo).expect("mkdir");
        Command::new("git").arg("-C").arg(&repo).args(["init", "-b", "main"]).output().unwrap();
        Command::new("git").arg("-C").arg(&repo).args(["config", "user.name", "Test"]).output().unwrap();
        Command::new("git").arg("-C").arg(&repo).args(["config", "user.email", "t@kanso.dev"]).output().unwrap();
        fs::write(repo.join("a.txt"), "base\n").unwrap();
        Command::new("git").arg("-C").arg(&repo).args(["add", "."]).output().unwrap();
        Command::new("git").arg("-C").arg(&repo).args(["commit", "-m", "init"]).output().unwrap();

        fs::write(repo.join("a.txt"), "dirty\n").unwrap();
        let msg = rescue_dirty_trunk(&repo, "fix/ai-rescue").expect("rescue");
        assert!(msg.contains("fix/ai-rescue"));

        let branch = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&branch.stdout).trim(), "fix/ai-rescue");
        assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "dirty\n");
    }

    fn git_ok(dir: &Path, args: &[&str]) {
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

    fn init_simple_repo(dir: &Path, branch: &str) {
        fs::create_dir_all(dir).expect("mkdir");
        git_ok(dir, &["init", "-b", branch]);
        git_ok(dir, &["config", "user.name", "Test"]);
        git_ok(dir, &["config", "user.email", "t@kanso.dev"]);
        fs::write(dir.join("base.txt"), "v1\n").unwrap();
        git_ok(dir, &["add", "."]);
        git_ok(dir, &["commit", "-m", "init"]);
    }

    #[test]
    fn batch_create_branch_handles_partial_failures() {
        let temp = TempDir::new().expect("temp dir");
        let repo_a = temp.path().join("repo-a");
        let repo_b = temp.path().join("repo-b");
        init_simple_repo(&repo_a, "main");
        init_simple_repo(&repo_b, "main");

        // Pre-create target branch in repo_b so repo_b fails while repo_a succeeds
        git_ok(&repo_b, &["branch", "feat/multi"]);

        let results = batch_create_branch(&[repo_a.clone(), repo_b.clone()], "feat/multi", None);
        assert_eq!(results.len(), 2);
        assert!(results[0].success);
        assert!(!results[1].success);
        assert!(results[1].message.contains("already exists"));

        let invalid = batch_create_branch(&[repo_a], "invalid branch name", None);
        assert!(!invalid[0].success);
    }

    #[test]
    fn batch_pull_trunk_updates_clean_trunk_and_skips_dirty_or_feature() {
        let temp = TempDir::new().expect("temp dir");
        let origin = temp.path().join("origin.git");
        fs::create_dir_all(&origin).unwrap();
        git_ok(&origin, &["init", "--bare", "-b", "main"]);

        // Seed repo pushes initial + second commit to origin
        let seed = temp.path().join("seed");
        init_simple_repo(&seed, "main");
        git_ok(&seed, &["remote", "add", "origin", origin.to_str().unwrap()]);
        git_ok(&seed, &["push", "-u", "origin", "main"]);

        // Clone clean trunk repo, dirty trunk repo, and feature branch repo before second commit
        let clean_repo = temp.path().join("clean-repo");
        let dirty_repo = temp.path().join("dirty-repo");
        let feature_repo = temp.path().join("feature-repo");

        for target in [&clean_repo, &dirty_repo, &feature_repo] {
            let out = Command::new("git")
                .args(["clone", origin.to_str().unwrap(), target.to_str().unwrap()])
                .output()
                .unwrap();
            assert!(out.status.success());
            git_ok(target, &["config", "user.name", "Test"]);
            git_ok(target, &["config", "user.email", "t@kanso.dev"]);
        }

        // Advance origin/main via seed
        fs::write(seed.join("base.txt"), "v2\n").unwrap();
        git_ok(&seed, &["add", "."]);
        git_ok(&seed, &["commit", "-m", "v2"]);
        git_ok(&seed, &["push", "origin", "main"]);

        // Make dirty_repo dirty on main; switch feature_repo to feature branch
        fs::write(dirty_repo.join("uncommitted.txt"), "wip\n").unwrap();
        git_ok(&feature_repo, &["checkout", "-b", "feat/x"]);

        let results = batch_pull_trunk(&[
            (clean_repo.clone(), "main".to_string()),
            (dirty_repo.clone(), "main".to_string()),
            (feature_repo.clone(), "main".to_string()),
        ]);

        assert_eq!(results.len(), 3);
        assert!(results[0].success);
        assert_eq!(fs::read_to_string(clean_repo.join("base.txt")).unwrap(), "v2\n");
        assert!(!results[1].success);
        assert!(results[1].message.contains("uncommitted"));
        assert!(!results[2].success);
        assert!(results[2].message.contains("feat/x"));
    }
}
