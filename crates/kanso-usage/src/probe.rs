use crate::model::{DesktopAppInfo, Family, PathKind, ToolVersion, UpdaterKind};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, BufRead, BufReader};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{self, Path, PathBuf};
use std::process::{ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};
use wait_timeout::ChildExt;

const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

/// 按 Codex、Claude Code、Gemini CLI 的顺序探测安装状态、版本和升级命令。
pub fn probe_tools() -> Vec<ToolVersion> {
    let search_path = env::var_os("PATH");
    [
        ("codex", Family::OpenAi),
        ("claude", Family::Anthropic),
        ("gemini", Family::Google),
    ]
    .into_iter()
    .map(|(tool, family)| {
        let executable = search_path
            .as_deref()
            .and_then(|search_path| find_executable(search_path, tool));
        let path_kind = executable
            .as_ref()
            .map(|(_, real_path)| classify(real_path));
        let upgrade_cmd = path_kind
            .as_ref()
            .and_then(|kind| upgrade_command(&family, kind))
            .map(str::to_owned);
        ToolVersion {
            tool: tool.to_owned(),
            family,
            installed: executable.is_some(),
            version: executable.as_ref().and_then(|(path, _)| read_version(path)),
            path_kind,
            upgrade_cmd,
        }
    })
    .collect()
}

fn find_executable(search_path: &OsStr, tool: &str) -> Option<(PathBuf, PathBuf)> {
    #[cfg(windows)]
    let names = ["exe", "com", "bat", "cmd"].map(|extension| format!("{tool}.{extension}"));
    #[cfg(not(windows))]
    let names = [tool];

    env::split_paths(search_path).find_map(|dir| {
        names.iter().find_map(|name| {
            let executable = path::absolute(dir.join(name)).ok()?;
            let real_path = fs::canonicalize(&executable).ok()?;
            is_executable(&real_path).then_some((executable, real_path))
        })
    })
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o111 == 0 {
        return false;
    }
    metadata.is_file()
}

fn classify(real_path: &Path) -> PathKind {
    let path = real_path.to_string_lossy();
    if path.contains("node_modules") {
        PathKind::Npm
    } else if path.contains("/Cellar/") || path.contains("/opt/homebrew/") {
        PathKind::Brew
    } else {
        PathKind::Binary
    }
}

fn upgrade_command(family: &Family, kind: &PathKind) -> Option<&'static str> {
    match (family, kind) {
        (_, PathKind::Binary) => None,
        (Family::OpenAi, PathKind::Npm) => Some("npm i -g @openai/codex@latest"),
        (Family::OpenAi, PathKind::Brew) => Some("brew upgrade codex"),
        (Family::Anthropic, _) => Some("claude update"),
        (Family::Google, PathKind::Npm) => Some("npm i -g @google/gemini-cli@latest"),
        (Family::Google, PathKind::Brew) => Some("brew upgrade gemini-cli"),
    }
}

fn read_version(executable: &Path) -> Option<String> {
    let mut child = Command::new(executable)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    let output = child.stdout.take().and_then(version_stdout);
    match child.wait_timeout(VERSION_TIMEOUT.saturating_sub(started.elapsed())) {
        Ok(Some(status)) if status.success() => output?
            .recv_timeout(VERSION_TIMEOUT.saturating_sub(started.elapsed()))
            .ok(),
        Ok(Some(_)) => None,
        Ok(None) | Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

fn version_stdout(stdout: ChildStdout) -> Option<Receiver<String>> {
    let (sender, receiver) = mpsc::channel();
    // 并行读取，避免子进程因写满管道而无法退出。
    thread::Builder::new()
        .spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = Vec::new();
            while let Ok(1..) = reader.read_until(b'\n', &mut line) {
                let text = String::from_utf8_lossy(&line);
                if !text.trim().is_empty() {
                    let _ = sender.send(text.trim().to_owned());
                    break;
                }
                line.clear();
            }
            let _ = io::copy(&mut reader, &mut io::sink());
        })
        .ok()?;
    Some(receiver)
}

/// 读取桌面应用的安装信息和指定偏好域内的更新设置。
pub fn probe_desktop_app(app_dir: &Path, defaults_domain: &str) -> DesktopAppInfo {
    probe_desktop_app_with(app_dir, defaults_domain, read_default)
}

fn probe_desktop_app_with(
    app_dir: &Path,
    defaults_domain: &str,
    mut read: impl FnMut(&OsStr, &str) -> Option<String>,
) -> DesktopAppInfo {
    let contents = app_dir.join("Contents");
    if !contents.join("Info.plist").is_file() {
        return DesktopAppInfo {
            installed: false,
            name: String::new(),
            version: None,
            updater: UpdaterKind::Unknown,
            auto_check: None,
            auto_install: None,
            last_check: None,
        };
    }

    let info = contents.join("Info");
    let updater = if contents.join("Frameworks/Sparkle.framework").exists() {
        UpdaterKind::Sparkle
    } else {
        UpdaterKind::Unknown
    };
    let domain = OsStr::new(defaults_domain);
    DesktopAppInfo {
        installed: true,
        name: read(info.as_os_str(), "CFBundleName").unwrap_or_default(),
        version: read(info.as_os_str(), "CFBundleShortVersionString"),
        updater,
        auto_check: read(domain, "SUEnableAutomaticChecks")
            .as_deref()
            .and_then(parse_bool),
        auto_install: read(domain, "SUAutomaticallyUpdate")
            .as_deref()
            .and_then(parse_bool),
        last_check: read(domain, "SULastCheckTime"),
    }
}

fn read_default(domain: &OsStr, key: &str) -> Option<String> {
    let output = Command::new("defaults")
        .arg("read")
        .arg(domain)
        .arg(key)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = str::from_utf8(&output.stdout).ok()?.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Some(true),
        "0" | "false" | "no" => Some(false),
        _ => None,
    }
}
