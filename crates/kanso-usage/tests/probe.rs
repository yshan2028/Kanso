use kanso_usage::{
    Coverage, DailyBucket, Family, ProviderSource, SourceKind, ToolVersion, UsageSnapshot,
    derive_providers,
};

#[cfg(unix)]
mod local {
    use kanso_usage::{Family, PathKind, UpdaterKind, probe_desktop_app, probe_tools};
    use std::env;
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::Path;
    use std::process::Command;
    use std::time::{Duration, Instant};
    use tempfile::tempdir;

    const FIXTURE_ROOT: &str = "KANSO_PROBE_FIXTURE_ROOT";

    // 只替换子测试进程的 PATH，确保并行测试不共享环境修改。
    fn with_fake_commands(test: &str, setup: impl FnOnce(&Path), check: impl FnOnce(&Path)) {
        if let Some(root) = env::var_os(FIXTURE_ROOT) {
            check(Path::new(&root));
            return;
        }

        let root = tempdir().unwrap();
        let bin = root.path().join("bin");
        let ignored = root.path().join("ignored");
        fs::create_dir(&bin).unwrap();
        fs::create_dir(&ignored).unwrap();
        setup(root.path());
        let output = Command::new(env::current_exe().unwrap())
            .args(["--exact", test])
            .env(FIXTURE_ROOT, root.path())
            .env("PATH", env::join_paths([ignored, bin]).unwrap())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write_executable(path: &Path, body: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn install_version(root: &Path, tool: &str, location: &str) {
        let executable = root.join(location).join(tool);
        write_executable(
            &executable,
            &format!(
                "[ \"$#\" -eq 1 ] && [ \"$1\" = --version ] || exit 2\n\
                 printf '\\n  \\n  {tool} 1.2.3  \\nignored\\n'\n"
            ),
        );
        symlink(executable, root.join("bin").join(tool)).unwrap();
    }

    #[test]
    fn classifies_canonical_paths_and_reads_versions() {
        with_fake_commands(
            "local::classifies_canonical_paths_and_reads_versions",
            |root| {
                fs::write(root.join("ignored/codex"), "non-executable").unwrap();
                install_version(root, "codex", "opt/homebrew/lib/node_modules/codex");
                install_version(root, "claude", "Cellar/claude/1.2.3/bin");
                install_version(root, "gemini", "standalone");
            },
            |_| {
                let tools = probe_tools();
                assert_eq!(tools.len(), 3);
                for (tool, (name, family, kind, upgrade)) in tools.iter().zip([
                    (
                        "codex",
                        Family::OpenAi,
                        PathKind::Npm,
                        Some("npm i -g @openai/codex@latest"),
                    ),
                    (
                        "claude",
                        Family::Anthropic,
                        PathKind::Brew,
                        Some("claude update"),
                    ),
                    ("gemini", Family::Google, PathKind::Binary, None),
                ]) {
                    assert_eq!(tool.tool, name);
                    assert_eq!(tool.family, family);
                    assert!(tool.installed);
                    assert_eq!(tool.path_kind, Some(kind));
                    assert_eq!(tool.version, Some(format!("{name} 1.2.3")));
                    assert_eq!(tool.upgrade_cmd.as_deref(), upgrade);
                }
            },
        );
    }

    #[test]
    fn builds_upgrade_commands_for_each_package_manager() {
        with_fake_commands(
            "local::builds_upgrade_commands_for_each_package_manager",
            |root| {
                install_version(root, "codex", "opt/homebrew/bin");
                install_version(root, "claude", "lib/node_modules/claude");
                install_version(root, "gemini", "Cellar/gemini-cli/1.2.3/bin");
            },
            |root| {
                let tools = probe_tools();
                assert_eq!(tools[0].upgrade_cmd.as_deref(), Some("brew upgrade codex"));
                assert_eq!(tools[1].upgrade_cmd.as_deref(), Some("claude update"));
                assert_eq!(
                    tools[2].upgrade_cmd.as_deref(),
                    Some("brew upgrade gemini-cli")
                );
                fs::remove_file(root.join("bin/gemini")).unwrap();
                install_version(root, "gemini", "lib/node_modules/gemini");
                assert_eq!(
                    probe_tools()[2].upgrade_cmd.as_deref(),
                    Some("npm i -g @google/gemini-cli@latest")
                );
            },
        );
    }

    #[test]
    fn kills_a_stalled_version_process_after_five_seconds() {
        with_fake_commands(
            "local::kills_a_stalled_version_process_after_five_seconds",
            |root| {
                write_executable(
                    &root.join("bin/codex"),
                    r#"printf '%s\n' "$$" > "$KANSO_PROBE_FIXTURE_ROOT/cli.pid"
printf 'early version\n'
exec /bin/sleep 30
"#,
                );
                write_executable(&root.join("bin/claude"), "printf 'ignored\n'\nexit 1\n");
            },
            |root| {
                let start = Instant::now();
                let tools = probe_tools();
                let elapsed = start.elapsed();
                assert!(elapsed >= Duration::from_secs(5), "{elapsed:?}");
                assert!(elapsed < Duration::from_secs(10), "{elapsed:?}");
                let pid = fs::read_to_string(root.join("cli.pid")).unwrap();
                let pid: u32 = pid.trim().parse().unwrap();
                let status = Command::new("/bin/kill")
                    .args(["-0", &pid.to_string()])
                    .output()
                    .unwrap();
                assert!(!status.status.success(), "CLI 子进程 {pid} 仍在运行");
                assert!(tools[0].installed);
                assert_eq!(tools[0].version, None);
                assert!(tools[1].installed);
                assert_eq!(tools[1].version, None);
                assert!(!tools[2].installed);
                assert_eq!(tools[2].version, None);
                assert_eq!(tools[2].path_kind, None);
                assert_eq!(tools[2].upgrade_cmd, None);
            },
        );
    }

    #[test]
    fn reads_only_requested_bundle_and_update_keys() {
        with_fake_commands(
            "local::reads_only_requested_bundle_and_update_keys",
            |root| {
                let contents = root.join("Sample App.app/Contents");
                fs::create_dir_all(contents.join("Frameworks/Sparkle.framework")).unwrap();
                fs::write(contents.join("Info.plist"), "").unwrap();
                write_executable(
                    &root.join("bin/defaults"),
                    r#"printf '%s\t%s\t%s\n' "$1" "$2" "$3" >> "$KANSO_PROBE_FIXTURE_ROOT/defaults.log"
[ "$#" -eq 3 ] && [ "$1" = read ] || exit 2
case "$2" in
    "$KANSO_PROBE_FIXTURE_ROOT/Sample App.app/Contents/Info")
        case "$3" in
            CFBundleName) printf 'Sample App\n' ;;
            CFBundleShortVersionString) printf '1.2.3\n' ;;
            *) exit 2 ;;
        esac ;;
    sample.updates)
        case "$3" in
            SUEnableAutomaticChecks) printf ' 1\n' ;;
            SUAutomaticallyUpdate) printf 'false\n' ;;
            SULastCheckTime) printf '2026-11-27 09:00:00 +0000\n' ;;
            *) exit 2 ;;
        esac ;;
    sample.missing)
        case "$3" in
            SUEnableAutomaticChecks) printf 'unknown\n' ;;
            *) exit 1 ;;
        esac ;;
    *) exit 2 ;;
esac
"#,
                );
            },
            |root| {
                let app = root.join("Sample App.app");
                let log = root.join("defaults.log");
                let info = probe_desktop_app(&app, "sample.updates");
                assert!(info.installed);
                assert_eq!(info.name, "Sample App");
                assert_eq!(info.version.as_deref(), Some("1.2.3"));
                assert_eq!(info.updater, UpdaterKind::Sparkle);
                assert_eq!(info.auto_check, Some(true));
                assert_eq!(info.auto_install, Some(false));
                assert_eq!(
                    info.last_check.as_deref(),
                    Some("2026-11-27 09:00:00 +0000")
                );

                let bundle = app.join("Contents/Info");
                let expected = format!(
                    "read\t{bundle}\tCFBundleName\n\
                     read\t{bundle}\tCFBundleShortVersionString\n\
                     read\tsample.updates\tSUEnableAutomaticChecks\n\
                     read\tsample.updates\tSUAutomaticallyUpdate\n\
                     read\tsample.updates\tSULastCheckTime\n",
                    bundle = bundle.display()
                );
                assert_eq!(fs::read_to_string(&log).unwrap(), expected);

                fs::remove_dir(app.join("Contents/Frameworks/Sparkle.framework")).unwrap();
                let info = probe_desktop_app(&app, "sample.missing");
                assert!(info.installed);
                assert_eq!(info.updater, UpdaterKind::Unknown);
                assert_eq!(info.auto_check, None);
                assert_eq!(info.auto_install, None);
                assert_eq!(info.last_check, None);

                fs::remove_file(&log).unwrap();
                fs::remove_file(app.join("Contents/Info.plist")).unwrap();
                let info = probe_desktop_app(&app, "sample.updates");
                assert!(!info.installed);
                assert_eq!(info.updater, UpdaterKind::Unknown);
                assert_eq!(info.version, None);
                assert!(!log.exists());
            },
        );
    }
}

fn bucket(family: Family, model: Option<&str>, calls: u64, errors: u64) -> DailyBucket {
    DailyBucket {
        source_kind: match family {
            Family::Anthropic => SourceKind::ClaudeCode,
            Family::OpenAi | Family::Google => SourceKind::CodexCli,
        },
        family,
        model: model.map(str::to_owned),
        calls,
        errors,
        input_uncached: 2,
        cache_write: 3,
        cache_read: 5,
        output: 7,
        reasoning: 11,
        est_usd: 0.0,
    }
}

fn tool(name: &str, family: Family, installed: bool, version: Option<&str>) -> ToolVersion {
    ToolVersion {
        tool: name.to_owned(),
        family,
        installed,
        version: version.map(str::to_owned),
        path_kind: None,
        upgrade_cmd: None,
    }
}

#[test]
fn merges_session_usage_with_installed_tools() {
    let snapshot = UsageSnapshot {
        requests: Vec::new(),
        total_requests: 10,
        total_errors: 3,
        daily: [
            (
                "2026-11-27".to_owned(),
                vec![
                    bucket(Family::OpenAi, Some("model-a"), 2, 0),
                    bucket(Family::OpenAi, Some("model-z"), 3, 1),
                ],
            ),
            (
                "2026-11-25".to_owned(),
                vec![
                    bucket(Family::OpenAi, Some("model-z"), 2, 1),
                    bucket(Family::OpenAi, None, 2, 1),
                    bucket(Family::Anthropic, Some("claude-model"), 1, 0),
                ],
            ),
        ]
        .into(),
        hourly_today: Vec::new(),
        coverage: Coverage {
            lookback_days: 7,
            first_day: "2026-11-25".to_owned(),
            last_day: "2026-11-27".to_owned(),
            active_days: 2,
        },
        quota: Vec::new(),
    };
    let providers = derive_providers(
        &snapshot,
        &[
            tool("codex", Family::OpenAi, true, Some("1.2.3")),
            tool("claude", Family::Anthropic, false, None),
            tool("gemini", Family::Google, true, Some("4.5.6")),
        ],
    );
    assert_eq!(providers.len(), 3);

    let codex = providers
        .iter()
        .find(|entry| entry.tool == "codex")
        .unwrap();
    assert_eq!(codex.id, "local-openai");
    assert_eq!(codex.name, "Codex");
    assert_eq!(codex.family, Family::OpenAi);
    assert_eq!(codex.source, ProviderSource::SessionLog);
    assert_eq!(codex.version.as_deref(), Some("1.2.3"));
    assert_eq!(codex.models, ["model-a", "model-z"]);
    assert_eq!((codex.calls, codex.errors, codex.tokens), (9, 3, 68));
    assert_eq!(codex.last_day.as_deref(), Some("2026-11-27"));

    let claude = providers
        .iter()
        .find(|entry| entry.tool == "claude")
        .unwrap();
    assert_eq!(claude.id, "local-anthropic");
    assert_eq!(claude.name, "Claude Code");
    assert_eq!(claude.family, Family::Anthropic);
    assert_eq!(claude.source, ProviderSource::SessionLog);
    assert_eq!(claude.version, None);
    assert_eq!(claude.last_day.as_deref(), Some("2026-11-25"));

    let gemini = providers
        .iter()
        .find(|entry| entry.tool == "gemini")
        .unwrap();
    assert_eq!(gemini.id, "local-google");
    assert_eq!(gemini.name, "Gemini CLI");
    assert_eq!(gemini.family, Family::Google);
    assert_eq!(gemini.source, ProviderSource::CliProbe);
    assert_eq!(gemini.version.as_deref(), Some("4.5.6"));
    assert!(gemini.models.is_empty());
    assert_eq!((gemini.calls, gemini.errors, gemini.tokens), (0, 0, 0));
    assert_eq!(gemini.last_day, None);

    let snapshot_without_logs = UsageSnapshot {
        daily: Default::default(),
        ..snapshot
    };
    assert!(
        derive_providers(
            &snapshot_without_logs,
            &[tool("claude", Family::Anthropic, false, None)]
        )
        .is_empty()
    );
}
