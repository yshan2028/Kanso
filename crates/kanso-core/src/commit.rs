use crate::model::{
    BatchOpResult, CommitPreflightIssue, CommitRequest, CommitResult, FileChangeEntry,
    FileChangeStatus, LandResult, PushResult, RepoChangeset, VcsKind,
};
use crate::scanner::GitCliBackend;
use crate::scanner::VcsBackend;
use anyhow::{Context, Result, anyhow};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_NORMAL_FILE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_DIFF_PREVIEW_BYTES: usize = 120 * 1024;

pub fn detect_jj_version() -> Option<String> {
    let out = Command::new("jj").arg("--version").output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

pub fn detect_vcs_kind(repo_path: &Path) -> VcsKind {
    let has_git = repo_path.join(".git").exists();
    let has_jj = repo_path.join(".jj").exists();
    match (has_git, has_jj) {
        (true, true) => VcsKind::JjColocated,
        (false, true) => VcsKind::Jj,
        _ => VcsKind::Git,
    }
}

fn repo_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

fn is_sensitive_filename(rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();
    let file_name = Path::new(&lower)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    file_name == ".env"
        || file_name.starts_with(".env.")
        || file_name.ends_with(".pem")
        || file_name.ends_with(".key")
        || file_name.ends_with(".p12")
        || file_name == "id_rsa"
        || file_name == "id_ed25519"
}

fn contains_secret_pattern(line: &str) -> Option<&'static str> {
    let patterns = [
        ("BEGIN PRIVATE KEY", "私钥内容"),
        ("BEGIN RSA PRIVATE KEY", "RSA 私钥内容"),
        ("BEGIN OPENSSH PRIVATE KEY", "OpenSSH 私钥内容"),
        ("ghp_", "GitHub Personal Access Token"),
        ("AKIA", "AWS Access Key"),
    ];
    for (pat, label) in patterns {
        if line.contains(pat) {
            return Some(label);
        }
    }
    None
}

pub fn run_preflight_checks(
    _repo_path: &Path,
    files: &[FileChangeEntry],
    diff_text: &str,
    message: Option<&str>,
) -> Vec<CommitPreflightIssue> {
    let mut issues = Vec::new();

    for file in files {
        if file.size_bytes > MAX_NORMAL_FILE_BYTES {
            let mb = file.size_bytes as f64 / (1024.0 * 1024.0);
            issues.push(CommitPreflightIssue {
                severity: "error".to_string(),
                code: "LARGE_FILE".to_string(),
                message: format!("文件体积过大 ({mb:.1} MB > 10 MB)，建议清理或忽略后再提交"),
                path: Some(file.path.clone()),
            });
        }
        if file.status != FileChangeStatus::Deleted && is_sensitive_filename(&file.path) {
            issues.push(CommitPreflightIssue {
                severity: "error".to_string(),
                code: "SECRET_RISK".to_string(),
                message: format!("疑似敏感凭证文件，禁止直接提交: {}", file.path),
                path: Some(file.path.clone()),
            });
        }
    }

    for line in diff_text.lines() {
        if let Some(added) = line.strip_prefix('+') {
            if line.starts_with("+++") {
                continue;
            }
            if let Some(label) = contains_secret_pattern(added) {
                issues.push(CommitPreflightIssue {
                    severity: "error".to_string(),
                    code: "SECRET_RISK".to_string(),
                    message: format!("新增变更中检测到疑似{label}，已拦截"),
                    path: None,
                });
                break;
            }
        }
    }

    if let Some(msg) = message {
        let trimmed = msg.trim();
        if trimmed.is_empty() {
            issues.push(CommitPreflightIssue {
                severity: "error".to_string(),
                code: "EMPTY_MESSAGE".to_string(),
                message: "提交说明不能为空".to_string(),
                path: None,
            });
        } else {
            if trimmed.to_lowercase().contains("co-authored-by:") {
                issues.push(CommitPreflightIssue {
                    severity: "error".to_string(),
                    code: "AI_COAUTHOR".to_string(),
                    message: "提交说明禁止包含 Co-Authored-By 署名行".to_string(),
                    path: None,
                });
            }
            if let Some(first_line) = trimmed.lines().next() {
                if first_line.chars().count() > 72 {
                    issues.push(CommitPreflightIssue {
                        severity: "warning".to_string(),
                        code: "LONG_SUBJECT".to_string(),
                        message: format!(
                            "提交标题长度 {} 字符，建议控制在 72 字符以内",
                            first_line.chars().count()
                        ),
                        path: None,
                    });
                }
            }
        }
    }

    issues
}

fn parse_porcelain_status(repo_path: &Path, raw: &str) -> Vec<FileChangeEntry> {
    let mut files = Vec::new();
    for line in raw.lines() {
        if line.len() < 4 {
            continue;
        }
        let xy = &line[..2];
        let mut path_part = line[3..].trim();
        if let Some((_, new_path)) = path_part.split_once(" -> ") {
            path_part = new_path;
        }
        let path_part = path_part.trim_matches('"').to_string();
        let full_path = repo_path.join(&path_part);
        let size_bytes = fs::metadata(&full_path).map(|m| m.len()).unwrap_or(0);

        let mut chars = xy.chars();
        let x = chars.next().unwrap_or(' ');
        let y = chars.next().unwrap_or(' ');

        let (status, staged) = if xy == "??" {
            (FileChangeStatus::Untracked, false)
        } else if xy.contains('U') || xy == "AA" || xy == "DD" {
            (FileChangeStatus::Conflicted, false)
        } else if x == 'R' || y == 'R' {
            (FileChangeStatus::Renamed, x != ' ')
        } else if x == 'A' || y == 'A' {
            (FileChangeStatus::Added, x != ' ')
        } else if x == 'D' || y == 'D' {
            (FileChangeStatus::Deleted, x != ' ')
        } else {
            (FileChangeStatus::Modified, x != ' ' && y == ' ')
        };

        files.push(FileChangeEntry {
            path: path_part,
            status,
            staged,
            size_bytes,
        });
    }
    files
}

fn append_untracked_previews(repo_path: &Path, files: &[FileChangeEntry], diff: &mut String) {
    for file in files {
        if diff.len() >= MAX_DIFF_PREVIEW_BYTES {
            diff.push_str("\n... (Diff 内容过长，已截断预览) ...\n");
            break;
        }
        if file.status != FileChangeStatus::Untracked {
            continue;
        }
        let full = repo_path.join(&file.path);
        if file.size_bytes > 64 * 1024 {
            diff.push_str(&format!(
                "\n--- /dev/null\n+++ b/{} (新文件 {} B，跳过全文预览)\n",
                file.path, file.size_bytes
            ));
            continue;
        }
        if let Ok(bytes) = fs::read(&full) {
            if bytes.contains(&0) {
                diff.push_str(&format!(
                    "\n--- /dev/null\n+++ b/{} (二进制新文件 {} B)\n",
                    file.path, file.size_bytes
                ));
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            diff.push_str(&format!("\ndiff --git a/{0} b/{0}\nnew file\n--- /dev/null\n+++ b/{0}\n", file.path));
            for line in text.lines().take(120) {
                diff.push('+');
                diff.push_str(line);
                diff.push('\n');
            }
        }
    }
}

pub fn inspect_repo_changes(repo_path: &Path) -> Result<RepoChangeset> {
    let canonical = fs::canonicalize(repo_path)
        .with_context(|| format!("仓库目录不存在: {}", repo_path.display()))?;
    let vcs_kind = detect_vcs_kind(&canonical);
    let backend = GitCliBackend;
    let status = backend.inspect_repo(&canonical, &canonical, None)?;

    let files = if vcs_kind == VcsKind::Jj && !canonical.join(".git").exists() {
        let out = Command::new("jj")
            .arg("-R")
            .arg(&canonical)
            .args(["diff", "--summary"])
            .output()
            .context("执行 jj diff --summary 失败")?;
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let kind = parts.next()?;
                let p = parts.next()?.to_string();
                let st = match kind {
                    "A" => FileChangeStatus::Added,
                    "D" => FileChangeStatus::Deleted,
                    _ => FileChangeStatus::Modified,
                };
                let size_bytes = fs::metadata(canonical.join(&p)).map(|m| m.len()).unwrap_or(0);
                Some(FileChangeEntry {
                    path: p,
                    status: st,
                    staged: false,
                    size_bytes,
                })
            })
            .collect()
    } else {
        let raw_status = Command::new("git")
            .arg("-C")
            .arg(&canonical)
            .args(["status", "--porcelain=v1", "-uall"])
            .output()
            .context("执行 git status 失败")?;
        if !raw_status.status.success() {
            return Err(anyhow!(
                "读取仓库变更失败: {}",
                String::from_utf8_lossy(&raw_status.stderr).trim()
            ));
        }
        parse_porcelain_status(&canonical, &String::from_utf8_lossy(&raw_status.stdout))
    };

    let mut diff_text = if vcs_kind == VcsKind::Jj && !canonical.join(".git").exists() {
        let out = Command::new("jj")
            .arg("-R")
            .arg(&canonical)
            .args(["diff", "--git"])
            .output()
            .context("执行 jj diff 失败")?;
        String::from_utf8_lossy(&out.stdout).to_string()
    } else {
        let head_exists = Command::new("git")
            .arg("-C")
            .arg(&canonical)
            .args(["rev-parse", "--verify", "--quiet", "HEAD"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        let diff_args = if head_exists {
            vec!["diff", "HEAD"]
        } else {
            vec!["diff", "--cached"]
        };
        let out = Command::new("git")
            .arg("-C")
            .arg(&canonical)
            .args(&diff_args)
            .output()
            .context("执行 git diff 失败")?;
        let mut text = String::from_utf8_lossy(&out.stdout).to_string();
        if text.len() > MAX_DIFF_PREVIEW_BYTES {
            text.truncate(MAX_DIFF_PREVIEW_BYTES);
            text.push_str("\n... (Diff 内容过长，已截断预览) ...\n");
        } else {
            append_untracked_previews(&canonical, &files, &mut text);
        }
        text
    };

    if diff_text.len() > MAX_DIFF_PREVIEW_BYTES {
        diff_text.truncate(MAX_DIFF_PREVIEW_BYTES);
        diff_text.push_str("\n... (Diff 内容过长，已截断预览) ...\n");
    }

    let preflight_issues = run_preflight_checks(&canonical, &files, &diff_text, None);

    Ok(RepoChangeset {
        repo_path: canonical.to_string_lossy().to_string(),
        repo_name: repo_name(&canonical),
        vcs_kind,
        current_branch: status.current_branch,
        trunk_branch: status.trunk_branch,
        remotes: status.remotes,
        files,
        diff_text,
        preflight_issues,
    })
}

pub fn commit_repo(req: &CommitRequest) -> Result<CommitResult> {
    let repo_path = Path::new(&req.repo_path);
    let changeset = inspect_repo_changes(repo_path)?;

    let target_files: Vec<FileChangeEntry> = if req.selected_files.is_empty() {
        changeset.files.clone()
    } else {
        let wanted: BTreeSet<&str> = req.selected_files.iter().map(String::as_str).collect();
        changeset
            .files
            .iter()
            .filter(|f| wanted.contains(f.path.as_str()))
            .cloned()
            .collect()
    };

    if target_files.is_empty() {
        return Err(anyhow!("工作区没有待提交的文件改动"));
    }

    let issues = run_preflight_checks(
        repo_path,
        &target_files,
        &changeset.diff_text,
        Some(&req.message),
    );
    for issue in &issues {
        if issue.severity == "error" {
            if issue.code == "LARGE_FILE" && req.allow_large_files {
                continue;
            }
            return Err(anyhow!("提交安检拦截 [{}]: {}", issue.code, issue.message));
        }
    }

    let msg = req.message.trim();
    if matches!(changeset.vcs_kind, VcsKind::Jj | VcsKind::JjColocated)
        && detect_jj_version().is_some()
    {
        let desc = Command::new("jj")
            .arg("-R")
            .arg(repo_path)
            .args(["describe", "-m", msg])
            .output()
            .context("执行 jj describe 失败")?;
        if !desc.status.success() {
            return Err(anyhow!(
                "jj describe 失败: {}",
                String::from_utf8_lossy(&desc.stderr).trim()
            ));
        }

        let new_cmd = Command::new("jj")
            .arg("-R")
            .arg(repo_path)
            .arg("new")
            .output()
            .context("执行 jj new 失败")?;
        if !new_cmd.status.success() {
            return Err(anyhow!(
                "jj new 失败: {}",
                String::from_utf8_lossy(&new_cmd.stderr).trim()
            ));
        }

        let rev = Command::new("jj")
            .arg("-R")
            .arg(repo_path)
            .args(["log", "-r", "@-", "--no-graph", "-T", "change_id.short()"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "jj-change".to_string());

        return Ok(CommitResult {
            repo_path: changeset.repo_path,
            repo_name: changeset.repo_name,
            vcs_kind: changeset.vcs_kind,
            commit_hash: rev.clone(),
            summary: format!("已通过 jj 完成变更提交 ({rev})"),
        });
    }

    if req.selected_files.is_empty() {
        let add_all = Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .args(["add", "-A"])
            .output()
            .context("执行 git add -A 失败")?;
        if !add_all.status.success() {
            return Err(anyhow!(
                "git add 失败: {}",
                String::from_utf8_lossy(&add_all.stderr).trim()
            ));
        }
    } else {
        let mut add_cmd = Command::new("git");
        add_cmd.arg("-C").arg(repo_path).args(["add", "--"]);
        for f in &req.selected_files {
            add_cmd.arg(f);
        }
        let add_out = add_cmd.output().context("执行 git add 失败")?;
        if !add_out.status.success() {
            return Err(anyhow!(
                "git add 失败: {}",
                String::from_utf8_lossy(&add_out.stderr).trim()
            ));
        }
    }

    let mut commit_cmd = Command::new("git");
    commit_cmd
        .arg("-C")
        .arg(repo_path)
        .args(["commit", "-m", msg]);
    if let Some(name) = req.author_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        commit_cmd
            .env("GIT_AUTHOR_NAME", name)
            .env("GIT_COMMITTER_NAME", name);
    }
    if let Some(email) = req.author_email.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        commit_cmd
            .env("GIT_AUTHOR_EMAIL", email)
            .env("GIT_COMMITTER_EMAIL", email);
    }

    let commit_out = commit_cmd.output().context("执行 git commit 失败")?;
    if !commit_out.status.success() {
        let stderr = String::from_utf8_lossy(&commit_out.stderr);
        let stdout = String::from_utf8_lossy(&commit_out.stdout);
        return Err(anyhow!(
            "git commit 失败: {} {}",
            stderr.trim(),
            stdout.trim()
        ));
    }

    let hash_out = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "HEAD".to_string());

    Ok(CommitResult {
        repo_path: changeset.repo_path,
        repo_name: changeset.repo_name,
        vcs_kind: changeset.vcs_kind,
        commit_hash: hash_out.clone(),
        summary: format!("已提交 {} 个文件 ({hash_out})", target_files.len()),
    })
}

pub fn push_repo(repo_path: &Path, remote: &str, branch: Option<&str>) -> Result<PushResult> {
    let remote = remote.trim();
    if remote.is_empty() {
        return Err(anyhow!("远端仓库名称不能为空"));
    }

    let vcs_kind = detect_vcs_kind(repo_path);
    let current_branch = match branch.map(str::trim).filter(|s| !s.is_empty()) {
        Some(b) => b.to_string(),
        None => {
            let out = Command::new("git")
                .arg("-C")
                .arg(repo_path)
                .args(["rev-parse", "--abbrev-ref", "HEAD"])
                .output()
                .context("读取当前分支失败")?;
            let b = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if b.is_empty() || b == "HEAD" {
                return Err(anyhow!("当前处于 Detached HEAD，无法直接推送"));
            }
            b
        }
    };

    if matches!(vcs_kind, VcsKind::Jj | VcsKind::JjColocated) && detect_jj_version().is_some() {
        let out = Command::new("jj")
            .arg("-R")
            .arg(repo_path)
            .args(["git", "push", "--remote", remote])
            .output()
            .context("执行 jj git push 失败")?;
        if !out.status.success() {
            return Err(anyhow!(
                "jj 推送失败: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        return Ok(PushResult {
            repo_path: repo_path.to_string_lossy().to_string(),
            remote: remote.to_string(),
            branch: current_branch,
            summary: format!("已通过 jj 推送至 {remote}"),
        });
    }

    let out = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["push", "-u", remote, &current_branch])
        .output()
        .context("执行 git push 失败")?;
    if !out.status.success() {
        return Err(anyhow!(
            "git push 失败: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    Ok(PushResult {
        repo_path: repo_path.to_string_lossy().to_string(),
        remote: remote.to_string(),
        branch: current_branch.clone(),
        summary: format!("已推送 {current_branch} 至 {remote}"),
    })
}

pub fn land_branch_to_trunk(
    repo_path: &Path,
    trunk_branch: &str,
    delete_source: bool,
) -> Result<LandResult> {
    let trunk = trunk_branch.trim();
    if trunk.is_empty() {
        return Err(anyhow!("目标主干分支名称不能为空"));
    }

    let status_out = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["status", "--porcelain"])
        .output()
        .context("检查工作区状态失败")?;
    if !String::from_utf8_lossy(&status_out.stdout).trim().is_empty() {
        return Err(anyhow!("工作区存在未提交改动，请先提交或清理后再合入主干"));
    }

    let head_out = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .context("读取当前分支失败")?;
    let source_branch = String::from_utf8_lossy(&head_out.stdout).trim().to_string();
    if source_branch == "HEAD" {
        return Err(anyhow!("当前处于 Detached HEAD，无法合入主干"));
    }
    if source_branch == trunk {
        return Err(anyhow!("当前已位于主干分支 {trunk}"));
    }

    let co_trunk = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["checkout", trunk])
        .output()
        .context("切换到主干分支失败")?;
    if !co_trunk.status.success() {
        return Err(anyhow!(
            "切换至主干 {trunk} 失败: {}",
            String::from_utf8_lossy(&co_trunk.stderr).trim()
        ));
    }

    let merge_msg = format!("merge: 合入分支 {source_branch} 至 {trunk}");
    let merge_out = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["merge", "--no-ff", &source_branch, "-m", &merge_msg])
        .output()
        .context("执行 git merge 失败")?;
    if !merge_out.status.success() {
        let _ = Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .args(["merge", "--abort"])
            .output();
        let _ = Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .args(["checkout", &source_branch])
            .output();
        return Err(anyhow!(
            "合入主干存在冲突，已自动中止合并并切回 {source_branch}: {}",
            String::from_utf8_lossy(&merge_out.stdout).trim()
        ));
    }

    let mut deleted = false;
    if delete_source {
        let del_out = Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .args(["branch", "-d", &source_branch])
            .output();
        if let Ok(out) = del_out {
            deleted = out.status.success();
        }
    }

    Ok(LandResult {
        repo_path: repo_path.to_string_lossy().to_string(),
        source_branch: source_branch.clone(),
        trunk_branch: trunk.to_string(),
        deleted_source: deleted,
        summary: if deleted {
            format!("已将 {source_branch} 合入 {trunk} 并清理源分支")
        } else {
            format!("已将 {source_branch} 合入 {trunk}")
        },
    })
}

pub fn enable_jj_colocated(repo_path: &Path) -> Result<String> {
    if detect_jj_version().is_none() {
        return Err(anyhow!(
            "本机尚未安装 Jujutsu (jj)，可执行 brew install jj 后重试"
        ));
    }
    let out = Command::new("jj")
        .arg("git")
        .arg("init")
        .arg("--colocate")
        .arg(repo_path)
        .output()
        .context("执行 jj git init --colocate 失败")?;
    if !out.status.success() {
        return Err(anyhow!(
            "开启 jj 共存失败: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok("已开启 jj + Git 共存模式，可无暂存区提交并支持 jj undo 全局撤销".to_string())
}

pub fn jj_undo(repo_path: &Path) -> Result<String> {
    if detect_jj_version().is_none() {
        return Err(anyhow!("本机尚未安装 Jujutsu (jj)"));
    }
    let out = Command::new("jj")
        .arg("-R")
        .arg(repo_path)
        .args(["op", "undo"])
        .output()
        .context("执行 jj op undo 失败")?;
    if !out.status.success() {
        return Err(anyhow!(
            "jj undo 失败: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok("已通过 jj op undo 撤销上一步仓库操作".to_string())
}

pub fn batch_commit_and_push(
    requests: &[CommitRequest],
    remote: Option<&str>,
) -> Vec<BatchOpResult> {
    requests
        .iter()
        .map(|req| {
            let path = PathBuf::from(&req.repo_path);
            let name = repo_name(&path);
            match commit_repo(req) {
                Ok(res) => {
                    if let Some(rm) = remote.map(str::trim).filter(|s| !s.is_empty()) {
                        match push_repo(&path, rm, None) {
                            Ok(push_res) => BatchOpResult {
                                repo_path: req.repo_path.clone(),
                                repo_name: name,
                                success: true,
                                message: format!("{} · {}", res.summary, push_res.summary),
                            },
                            Err(err) => BatchOpResult {
                                repo_path: req.repo_path.clone(),
                                repo_name: name,
                                success: false,
                                message: format!("已提交({})但推送失败: {err}", res.commit_hash),
                            },
                        }
                    } else {
                        BatchOpResult {
                            repo_path: req.repo_path.clone(),
                            repo_name: name,
                            success: true,
                            message: res.summary,
                        }
                    }
                }
                Err(err) => BatchOpResult {
                    repo_path: req.repo_path.clone(),
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
    use tempfile::TempDir;

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

    fn init_repo(dir: &Path, branch: &str) {
        fs::create_dir_all(dir).unwrap();
        git_ok(dir, &["init", "-b", branch]);
        git_ok(dir, &["config", "user.name", "Test"]);
        git_ok(dir, &["config", "user.email", "t@kanso.dev"]);
        fs::write(dir.join("README.md"), "# base\n").unwrap();
        git_ok(dir, &["add", "."]);
        git_ok(dir, &["commit", "-m", "init"]);
    }

    #[test]
    fn inspect_changes_and_commit_standard_git_repo() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("svc");
        init_repo(&repo, "main");

        fs::write(repo.join("README.md"), "# updated\n").unwrap();
        fs::write(repo.join("new_file.txt"), "hello kanso\n").unwrap();

        let cs = inspect_repo_changes(&repo).expect("inspect");
        assert_eq!(cs.vcs_kind, VcsKind::Git);
        assert_eq!(cs.files.len(), 2);
        assert!(cs.diff_text.contains("hello kanso"));
        assert!(cs.preflight_issues.is_empty());

        let result = commit_repo(&CommitRequest {
            repo_path: repo.to_string_lossy().to_string(),
            message: "feat(svc): 更新说明并新增测试文件".to_string(),
            selected_files: vec!["new_file.txt".to_string()],
            author_name: Some("chayuan".to_string()),
            author_email: Some("chayuan@example.com".to_string()),
            allow_large_files: false,
        })
        .expect("commit selected file");
        assert!(!result.commit_hash.is_empty());

        let after = inspect_repo_changes(&repo).expect("after");
        assert_eq!(after.files.len(), 1);
        assert_eq!(after.files[0].path, "README.md");
    }

    #[test]
    fn preflight_blocks_secret_and_ai_coauthor() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("sec");
        init_repo(&repo, "main");

        fs::write(repo.join(".env"), "SECRET=123\n").unwrap();
        let err_secret = commit_repo(&CommitRequest {
            repo_path: repo.to_string_lossy().to_string(),
            message: "chore: add env".to_string(),
            selected_files: Vec::new(),
            author_name: None,
            author_email: None,
            allow_large_files: false,
        })
        .expect_err("should block .env");
        assert!(err_secret.to_string().contains("SECRET_RISK"));

        fs::remove_file(repo.join(".env")).unwrap();
        fs::write(repo.join("README.md"), "ok\n").unwrap();
        let err_ai = commit_repo(&CommitRequest {
            repo_path: repo.to_string_lossy().to_string(),
            message: "feat: test\n\nCo-Authored-By: Bot <bot@example.com>".to_string(),
            selected_files: Vec::new(),
            author_name: None,
            author_email: None,
            allow_large_files: false,
        })
        .expect_err("should block co-author");
        assert!(err_ai.to_string().contains("AI_COAUTHOR"));
    }

    #[test]
    fn push_to_local_bare_remote_and_land_to_trunk_and_abort_on_conflict() {
        let temp = TempDir::new().unwrap();
        let origin = temp.path().join("forgejo.git");
        fs::create_dir_all(&origin).unwrap();
        git_ok(&origin, &["init", "--bare", "-b", "main"]);

        let repo = temp.path().join("app");
        init_repo(&repo, "main");
        git_ok(&repo, &["remote", "add", "forgejo", origin.to_str().unwrap()]);
        git_ok(&repo, &["push", "-u", "forgejo", "main"]);

        git_ok(&repo, &["checkout", "-b", "feat/kanso-flow"]);
        fs::write(repo.join("feature.rs"), "pub fn ok() {}\n").unwrap();
        commit_repo(&CommitRequest {
            repo_path: repo.to_string_lossy().to_string(),
            message: "feat: 新增功能模块".to_string(),
            selected_files: Vec::new(),
            author_name: None,
            author_email: None,
            allow_large_files: false,
        })
        .unwrap();

        let pushed = push_repo(&repo, "forgejo", None).expect("push to forgejo");
        assert_eq!(pushed.remote, "forgejo");
        assert_eq!(pushed.branch, "feat/kanso-flow");

        let landed = land_branch_to_trunk(&repo, "main", true).expect("land to main");
        assert!(landed.deleted_source);
        assert!(repo.join("feature.rs").exists());

        // Test merge conflict auto-abort keeps trunk clean
        git_ok(&repo, &["checkout", "-b", "feat/conflict"]);
        fs::write(repo.join("README.md"), "# from branch\n").unwrap();
        git_ok(&repo, &["commit", "-am", "branch edit"]);
        git_ok(&repo, &["checkout", "main"]);
        fs::write(repo.join("README.md"), "# from main\n").unwrap();
        git_ok(&repo, &["commit", "-am", "main edit"]);
        git_ok(&repo, &["checkout", "feat/conflict"]);

        let conflict_err = land_branch_to_trunk(&repo, "main", false).expect_err("should abort");
        assert!(conflict_err.to_string().contains("冲突"));
        let status_after = inspect_repo_changes(&repo).unwrap();
        assert_eq!(status_after.current_branch, "feat/conflict");
        assert!(status_after.files.is_empty());
    }
}
