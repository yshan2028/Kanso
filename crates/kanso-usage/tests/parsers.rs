use std::fs::{self, File, FileTimes};
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use chrono::DateTime;
use kanso_usage::{
    Family, QuotaSnapshot, QuotaWindow, ReqStatus, SourceKind, parse_claude_logs, parse_codex_logs,
};

const CLAUDE_LOG: &str = include_str!("fixtures/claude.jsonl");
const CODEX_LOG: &str = include_str!("fixtures/codex.jsonl");

fn timestamp(value: &str) -> i64 {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .timestamp_millis()
}

fn cutoff_ms() -> i64 {
    timestamp("2026-11-26T00:00:00Z")
}

fn write_log(root: &Path, name: &str, contents: &str, modified_ms: i64) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, contents).unwrap();
    let modified = UNIX_EPOCH + Duration::from_millis(modified_ms.try_into().unwrap());
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
}

#[test]
fn claude_deduplicates_and_parses_success_and_error_usage() {
    let root = tempfile::tempdir().unwrap();
    let modified_ms = cutoff_ms() + 60_000;
    write_log(
        root.path(),
        "project/session.jsonl",
        CLAUDE_LOG,
        modified_ms,
    );
    write_log(root.path(), "old.jsonl", CLAUDE_LOG, cutoff_ms() - 1);

    let requests = parse_claude_logs(root.path(), cutoff_ms()).unwrap();
    assert_eq!(
        requests
            .iter()
            .map(|request| request.id.as_str())
            .collect::<Vec<_>>(),
        [
            "message-id",
            "api-error",
            "synthetic-error",
            "missing-timestamp"
        ]
    );
    assert!(requests.iter().all(|request| {
        request.family == Family::Anthropic
            && request.source_kind == SourceKind::ClaudeCode
            && request.session_id == "claude-session"
            && request.project == "claude-project"
    }));
    let success = &requests[0];
    assert_eq!(success.status, ReqStatus::Success);
    assert_eq!(success.model.as_deref(), Some("claude-sonnet-test"));
    assert_eq!(success.ts_ms, cutoff_ms());
    assert_eq!(success.input_uncached, 11);
    assert_eq!(success.cache_write, 12);
    assert_eq!(success.cache_read, 13);
    assert_eq!(success.output, 14);
    assert_eq!(success.reasoning, 3);
    for error in &requests[1..=2] {
        assert_eq!(error.status, ReqStatus::Error);
        assert_eq!(error.model, None);
    }
    assert_eq!(requests[3].ts_ms, modified_ms);
}

#[test]
fn codex_deduplicates_requests_and_reads_quota_before_deduplication() {
    let root = tempfile::tempdir().unwrap();
    let modified_ms = cutoff_ms() + 60_000;
    write_log(root.path(), "2026/11/session.jsonl", CODEX_LOG, modified_ms);
    write_log(root.path(), "old.jsonl", CODEX_LOG, cutoff_ms() - 1);

    let (requests, quota) = parse_codex_logs(root.path(), cutoff_ms()).unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| {
        request.family == Family::OpenAi
            && request.source_kind == SourceKind::CodexDesktop
            && request.session_id == "codex-session"
            && request.project == "codex-project"
            && request.status == ReqStatus::Success
    }));
    let first = &requests[0];
    assert_eq!(first.ts_ms, cutoff_ms());
    assert_eq!(first.model.as_deref(), Some("gpt-test-first"));
    assert_eq!(first.input_uncached, 22);
    assert_eq!(first.cache_write, 6);
    assert_eq!(first.cache_read, 8);
    assert_eq!(first.output, 9);
    assert_eq!(first.reasoning, 3);
    assert_eq!(requests[1].model.as_deref(), Some("gpt-test-second"));
    assert_eq!(requests[1].input_uncached, 0);
    assert_eq!(requests[1].output, 10);
    assert_eq!(
        quota,
        Some(QuotaSnapshot {
            family: Family::OpenAi,
            limit_id: "test-limit".to_owned(),
            plan_type: Some("pro".to_owned()),
            primary: None,
            secondary: Some(QuotaWindow {
                used_percent: 21.5,
                window_minutes: 10080,
                resets_at: Some(1_796_256_000),
            }),
            captured_ts_ms: cutoff_ms() + 3_000,
        })
    );
}

#[test]
fn codex_selects_latest_quota_across_files_without_request_usage() {
    let root = tempfile::tempdir().unwrap();
    write_log(root.path(), "session.jsonl", CODEX_LOG, cutoff_ms());
    write_log(
        root.path(),
        "quota-only.jsonl",
        r#"{"type":"event_msg","timestamp":"2026-11-26T00:00:10Z","payload":{"type":"token_count","info":null,"rate_limits":{"primary":{"used_percent":0,"window_minutes":300,"resets_at":null},"secondary":null}}}"#,
        cutoff_ms(),
    );

    let (requests, quota) = parse_codex_logs(root.path(), cutoff_ms()).unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        quota,
        Some(QuotaSnapshot {
            family: Family::OpenAi,
            limit_id: "codex".to_owned(),
            plan_type: None,
            primary: Some(QuotaWindow {
                used_percent: 0.0,
                window_minutes: 300,
                resets_at: None,
            }),
            secondary: None,
            captured_ts_ms: cutoff_ms() + 10_000,
        })
    );
}

#[test]
fn codex_skips_null_and_older_quota_events() {
    let root = tempfile::tempdir().unwrap();
    let contents = r#"{"type":"event_msg","timestamp":"2026-11-25T23:59:59.999Z","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":40}}}}
{"type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":null,"secondary":null}}}"#;
    write_log(root.path(), "session.jsonl", contents, cutoff_ms());
    assert_eq!(
        parse_codex_logs(root.path(), cutoff_ms()).unwrap(),
        (Vec::new(), None)
    );
}

#[test]
fn codex_reads_exec_and_cli_sessions_with_missing_event_timestamps() {
    let root = tempfile::tempdir().unwrap();
    let usage = r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"output_tokens":1},"total_token_usage":{"total_tokens":1,"output_tokens":1}}}}"#;
    let exec_meta = r#"{"type":"session_meta","payload":{"originator":"codex_exec","id":"exec"}}"#;
    write_log(
        root.path(),
        "exec.jsonl",
        &format!("{exec_meta}\n{usage}"),
        cutoff_ms(),
    );
    write_log(root.path(), "cli.jsonl", usage, cutoff_ms());

    let (requests, quota) = parse_codex_logs(root.path(), cutoff_ms()).unwrap();
    assert_eq!(requests.len(), 2);
    assert!(quota.is_none());
    assert!(requests.iter().any(|request| {
        request.source_kind == SourceKind::CodexExec && request.session_id == "exec"
    }));
    assert!(requests.iter().any(|request| {
        request.source_kind == SourceKind::CodexCli && request.session_id == "cli.jsonl"
    }));
    assert!(requests.iter().all(|request| {
        request.ts_ms == cutoff_ms()
            && request.model.is_none()
            && request.project == "-"
            && request.output == 1
    }));
}

#[test]
fn parsers_ignore_probe_paths_and_stop_at_probe_working_directories() {
    let claude_root = tempfile::tempdir().unwrap();
    let codex_root = tempfile::tempdir().unwrap();
    for (root, fixture, probe_meta) in [
        (
            claude_root.path(),
            CLAUDE_LOG,
            r#"{"type":"user","cwd":"/work/scratch/mcp_probe/project"}"#,
        ),
        (
            codex_root.path(),
            CODEX_LOG,
            r#"{"type":"session_meta","payload":{"cwd":"/work/scratch/mcp_probe/project"}}"#,
        ),
    ] {
        write_log(root, "mcp_probe.jsonl", fixture, cutoff_ms());
        write_log(root, "mcp_probe/sub/session.jsonl", fixture, cutoff_ms());
        write_log(
            root,
            "probe-cwd.jsonl",
            &format!("{probe_meta}\n{fixture}"),
            cutoff_ms(),
        );
        write_log(root, "not-a-log.json", fixture, cutoff_ms());
    }
    assert!(
        parse_claude_logs(claude_root.path(), cutoff_ms())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        parse_codex_logs(codex_root.path(), cutoff_ms()).unwrap(),
        (Vec::new(), None)
    );
}

#[test]
fn parsers_return_empty_results_for_missing_or_empty_roots() {
    let root = tempfile::tempdir().unwrap();
    for path in [root.path().to_path_buf(), root.path().join("missing")] {
        assert!(parse_claude_logs(&path, cutoff_ms()).unwrap().is_empty());
        assert_eq!(
            parse_codex_logs(&path, cutoff_ms()).unwrap(),
            (Vec::new(), None)
        );
    }
}
