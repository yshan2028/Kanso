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
}
