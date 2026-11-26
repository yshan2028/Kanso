use anyhow::{Context, Result, anyhow};
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
use thiserror::Error;
use wait_timeout::ChildExt;

pub const READ_TIMEOUT: Duration = Duration::from_secs(10);
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
pub const NETWORK_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitAccess {
    Read,
    Write,
    Network,
}

impl GitAccess {
    pub fn default_timeout(self) -> Duration {
        match self {
            Self::Read => READ_TIMEOUT,
            Self::Write => WRITE_TIMEOUT,
            Self::Network => NETWORK_TIMEOUT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutput {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Error)]
pub enum ExecError {
    #[error("启动外部进程 `{program}` 失败: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("外部进程 `{program} {args}` 执行超时 ({timeout:?})，已强制终止")]
    Timeout {
        program: String,
        args: String,
        timeout: Duration,
    },
}

pub(crate) fn run_raw_with_timeout(
    program: &str,
    args: &[OsString],
    envs: &[(&str, &str)],
    access: GitAccess,
    timeout: Duration,
) -> std::result::Result<ExecOutput, ExecError> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0");

    if access == GitAccess::Read {
        cmd.env("GIT_OPTIONAL_LOCKS", "0");
    }

    for (k, v) in envs {
        cmd.env(k, v);
    }

    let mut child = cmd.spawn().map_err(|source| ExecError::Spawn {
        program: program.to_string(),
        source,
    })?;

    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();

    let stdout_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(ref mut pipe) = stdout_pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });

    let stderr_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(ref mut pipe) = stderr_pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });

    let status = match child.wait_timeout(timeout) {
        Ok(Some(status)) => status,
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_handle.join();
            let _ = stderr_handle.join();
            let args_str = args
                .iter()
                .map(|a| a.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ");
            return Err(ExecError::Timeout {
                program: program.to_string(),
                args: args_str,
                timeout,
            });
        }
        Err(source) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_handle.join();
            let _ = stderr_handle.join();
            return Err(ExecError::Spawn {
                program: program.to_string(),
                source,
            });
        }
    };

    let stdout_bytes = stdout_handle.join().unwrap_or_default();
    let stderr_bytes = stderr_handle.join().unwrap_or_default();

    Ok(ExecOutput {
        success: status.success(),
        code: status.code(),
        stdout: String::from_utf8_lossy(&stdout_bytes).to_string(),
        stderr: String::from_utf8_lossy(&stderr_bytes).to_string(),
    })
}

pub fn run_raw(
    program: &str,
    args: &[OsString],
    envs: &[(&str, &str)],
    access: GitAccess,
) -> std::result::Result<ExecOutput, ExecError> {
    run_raw_with_timeout(program, args, envs, access, access.default_timeout())
}

fn build_git_args(repo: &Path, args: &[&str]) -> Vec<OsString> {
    let mut full = Vec::with_capacity(args.len() + 2);
    full.push(OsString::from("-C"));
    full.push(repo.as_os_str().to_os_string());
    full.extend(args.iter().map(|s| OsStr::new(s).to_os_string()));
    full
}

fn build_jj_args(repo: &Path, args: &[&str]) -> Vec<OsString> {
    let mut full = Vec::with_capacity(args.len() + 2);
    full.push(OsString::from("-R"));
    full.push(repo.as_os_str().to_os_string());
    full.extend(args.iter().map(|s| OsStr::new(s).to_os_string()));
    full
}

pub fn git_output_env(
    repo: &Path,
    args: &[&str],
    access: GitAccess,
    envs: &[(&str, &str)],
) -> Result<ExecOutput> {
    let full_args = build_git_args(repo, args);
    run_raw("git", &full_args, envs, access)
        .with_context(|| format!("执行 git {} 失败 ({})", args.join(" "), repo.display()))
}

pub fn git_output(repo: &Path, args: &[&str], access: GitAccess) -> Result<ExecOutput> {
    git_output_env(repo, args, access, &[])
}

pub fn git(repo: &Path, args: &[&str], access: GitAccess) -> Result<String> {
    let out = git_output(repo, args, access)?;
    if !out.success {
        let detail = if out.stderr.trim().is_empty() {
            out.stdout.trim()
        } else {
            out.stderr.trim()
        };
        return Err(anyhow!(
            "git {} failed in {}: {}",
            args.join(" "),
            repo.display(),
            detail
        ));
    }
    Ok(out.stdout)
}

pub fn jj_output(repo: &Path, args: &[&str], access: GitAccess) -> Result<ExecOutput> {
    let full_args = build_jj_args(repo, args);
    run_raw("jj", &full_args, &[], access)
        .with_context(|| format!("执行 jj {} 失败 ({})", args.join(" "), repo.display()))
}

pub fn jj(repo: &Path, args: &[&str], access: GitAccess) -> Result<String> {
    let out = jj_output(repo, args, access)?;
    if !out.success {
        let detail = if out.stderr.trim().is_empty() {
            out.stdout.trim()
        } else {
            out.stderr.trim()
        };
        return Err(anyhow!(
            "jj {} failed in {}: {}",
            args.join(" "),
            repo.display(),
            detail
        ));
    }
    Ok(out.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Instant;
    use tempfile::TempDir;

    #[test]
    fn exec_times_out_and_kills() {
        let started = Instant::now();
        let args = vec![OsString::from("5")];
        let err = run_raw_with_timeout(
            "sleep",
            &args,
            &[],
            GitAccess::Read,
            Duration::from_millis(200),
        )
        .expect_err("expected timeout");
        let elapsed = started.elapsed();

        assert!(
            matches!(err, ExecError::Timeout { .. }),
            "unexpected error: {err:?}"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "process kill took too long: {elapsed:?}"
        );
    }

    #[test]
    fn read_access_does_not_touch_index() {
        let temp = TempDir::new().expect("temp dir");
        let repo = temp.path();

        let init = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["init", "-b", "main"])
            .output()
            .unwrap();
        assert!(init.status.success());
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["config", "user.name", "Test"])
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["config", "user.email", "t@kanso.dev"])
            .output()
            .unwrap();
        fs::write(repo.join("tracked.txt"), "hello\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["add", "."])
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["commit", "-m", "init"])
            .output()
            .unwrap();

        let index_path = repo.join(".git/index");
        let before = fs::metadata(&index_path).unwrap().modified().unwrap();

        std::thread::sleep(Duration::from_millis(1100));
        // Touch tracked file with identical contents to simulate racily-clean stat drift.
        fs::write(repo.join("tracked.txt"), "hello\n").unwrap();

        let out = git(repo, &["status", "--porcelain"], GitAccess::Read).unwrap();
        assert!(out.trim().is_empty());

        let after = fs::metadata(&index_path).unwrap().modified().unwrap();
        assert_eq!(before, after, "GIT_OPTIONAL_LOCKS=0 must not rewrite .git/index");
    }
}
