use crate::pipeline::PipelineDefinition;
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::process::Stdio;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CiLogEvent {
    pub run_id: String,
    pub stream: String,
    pub line: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CiRunOutcome {
    pub status: String,
    pub exit_code: Option<i32>,
    pub duration_ms: i64,
    pub full_log: String,
}

pub trait CiRunner: Send + Sync {
    fn run_pipeline<'a>(
        &'a self,
        run_id: &'a str,
        repo_path: &'a Path,
        pipeline: &'a PipelineDefinition,
        log_tx: mpsc::UnboundedSender<CiLogEvent>,
        cancel_rx: oneshot::Receiver<()>,
    ) -> Pin<Box<dyn Future<Output = Result<CiRunOutcome>> + Send + 'a>>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LocalCiRunner;

fn emit_line(
    run_id: &str,
    stream: &str,
    line: String,
    log_tx: &mpsc::UnboundedSender<CiLogEvent>,
    full_log: &mut String,
) {
    full_log.push_str(&line);
    full_log.push('\n');
    let _ = log_tx.send(CiLogEvent {
        run_id: run_id.to_string(),
        stream: stream.to_string(),
        line,
        timestamp: Utc::now().timestamp(),
    });
}

fn spawn_shell_command(command: &str, workdir: &Path) -> Command {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.args(["-lc", command]);
        c
    };

    cmd.current_dir(workdir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

impl CiRunner for LocalCiRunner {
    fn run_pipeline<'a>(
        &'a self,
        run_id: &'a str,
        repo_path: &'a Path,
        pipeline: &'a PipelineDefinition,
        log_tx: mpsc::UnboundedSender<CiLogEvent>,
        mut cancel_rx: oneshot::Receiver<()>,
    ) -> Pin<Box<dyn Future<Output = Result<CiRunOutcome>> + Send + 'a>> {
        Box::pin(async move {
            let start = Instant::now();
            let mut full_log = String::new();
            let mut last_exit_code = Some(0);

            for (idx, step) in pipeline.steps.iter().enumerate() {
                let workdir = match &step.working_dir {
                    Some(rel) if !rel.trim().is_empty() => repo_path.join(rel),
                    _ => repo_path.to_path_buf(),
                };

                emit_line(
                    run_id,
                    "system",
                    format!("==> [{}/{}] {} ({})", idx + 1, pipeline.steps.len(), step.name, step.command),
                    &log_tx,
                    &mut full_log,
                );

                let mut child = spawn_shell_command(&step.command, &workdir)
                    .spawn()
                    .with_context(|| format!("Failed to execute '{}'", step.command))?;

                let stdout = child.stdout.take();
                let stderr = child.stderr.take();
                let (line_tx, mut line_rx) = mpsc::unbounded_channel::<(&'static str, String)>();

                if let Some(out) = stdout {
                    let tx = line_tx.clone();
                    tokio::spawn(async move {
                        let mut lines = BufReader::new(out).lines();
                        while let Ok(Some(line)) = lines.next_line().await {
                            let _ = tx.send(("stdout", line));
                        }
                    });
                }

                if let Some(err) = stderr {
                    let tx = line_tx.clone();
                    tokio::spawn(async move {
                        let mut lines = BufReader::new(err).lines();
                        while let Ok(Some(line)) = lines.next_line().await {
                            let _ = tx.send(("stderr", line));
                        }
                    });
                }
                drop(line_tx);

                let mut cancelled = false;
                let mut process_status = None;

                loop {
                    tokio::select! {
                        maybe_line = line_rx.recv() => {
                            match maybe_line {
                                Some((stream, line)) => emit_line(run_id, stream, line, &log_tx, &mut full_log),
                                None => {
                                    if process_status.is_some() {
                                        break;
                                    }
                                    process_status = Some(child.wait().await?);
                                    break;
                                }
                            }
                        }
                        _ = &mut cancel_rx, if !cancelled => {
                            cancelled = true;
                            let _ = child.kill().await;
                            emit_line(run_id, "system", "!! Pipeline cancelled by user".to_string(), &log_tx, &mut full_log);
                            process_status = Some(child.wait().await?);
                            break;
                        }
                    }
                }

                while let Ok((stream, line)) = line_rx.try_recv() {
                    emit_line(run_id, stream, line, &log_tx, &mut full_log);
                }

                if cancelled {
                    return Ok(CiRunOutcome {
                        status: "cancelled".to_string(),
                        exit_code: process_status.and_then(|s| s.code()),
                        duration_ms: start.elapsed().as_millis() as i64,
                        full_log,
                    });
                }

                let status = match process_status {
                    Some(st) => st,
                    None => child.wait().await?,
                };
                last_exit_code = status.code();

                if !status.success() {
                    emit_line(
                        run_id,
                        "system",
                        format!("!! Step '{}' failed with exit code {:?}", step.name, last_exit_code),
                        &log_tx,
                        &mut full_log,
                    );
                    return Ok(CiRunOutcome {
                        status: "failed".to_string(),
                        exit_code: last_exit_code,
                        duration_ms: start.elapsed().as_millis() as i64,
                        full_log,
                    });
                }
            }

            emit_line(
                run_id,
                "system",
                "==> Pipeline completed successfully".to_string(),
                &log_tx,
                &mut full_log,
            );

            Ok(CiRunOutcome {
                status: "passed".to_string(),
                exit_code: last_exit_code,
                duration_ms: start.elapsed().as_millis() as i64,
                full_log,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover_pipelines;
    use crate::pipeline::PipelineStep;
    use std::fs;
    use tempfile::TempDir;

    #[tokio::test]
    async fn local_runner_streams_output_and_handles_failure() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n").unwrap();
        let detected = discover_pipelines(dir.path());
        assert_eq!(detected.len(), 1);
        assert_eq!(detected[0].name, "Rust Check & Test");

        let runner = LocalCiRunner;
        let pipeline = PipelineDefinition {
            name: "Echo Test".to_string(),
            source: "test".to_string(),
            steps: vec![
                PipelineStep {
                    name: "Out".to_string(),
                    command: "echo hello-kanso".to_string(),
                    working_dir: None,
                },
                PipelineStep {
                    name: "Fail".to_string(),
                    command: "echo err-kanso >&2 && exit 2".to_string(),
                    working_dir: None,
                },
            ],
        };

        let (tx, mut rx) = mpsc::unbounded_channel();
        let (_cancel_tx, cancel_rx) = oneshot::channel();
        let outcome = runner
            .run_pipeline("run-test", dir.path(), &pipeline, tx, cancel_rx)
            .await
            .expect("run pipeline");

        assert_eq!(outcome.status, "failed");
        assert_eq!(outcome.exit_code, Some(2));
        assert!(outcome.full_log.contains("hello-kanso"));
        assert!(outcome.full_log.contains("err-kanso"));

        let mut received = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            received.push(ev);
        }
        assert!(received.iter().any(|e| e.line.contains("hello-kanso")));
    }

    #[tokio::test]
    async fn local_runner_supports_cancellation() {
        let dir = TempDir::new().unwrap();
        let runner = LocalCiRunner;
        let pipeline = PipelineDefinition {
            name: "Long Sleep".to_string(),
            source: "test".to_string(),
            steps: vec![PipelineStep {
                name: "Wait".to_string(),
                command: "sleep 20".to_string(),
                working_dir: None,
            }],
        };

        let (tx, _rx) = mpsc::unbounded_channel();
        let (cancel_tx, cancel_rx) = oneshot::channel();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            let _ = cancel_tx.send(());
        });

        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(4),
            runner.run_pipeline("run-cancel", dir.path(), &pipeline, tx, cancel_rx),
        )
        .await
        .expect("cancel finished promptly")
        .expect("run pipeline");

        assert_eq!(outcome.status, "cancelled");
        assert!(outcome.full_log.contains("cancelled"));
    }

    #[test]
    fn discovers_kanso_yaml_and_polyglot_stacks() {
        let yaml_dir = TempDir::new().unwrap();
        fs::write(
            yaml_dir.path().join(".kanso.yaml"),
            "pipelines:\n  - name: Custom\n    steps:\n      - name: Echo\n        command: echo ok\n",
        )
        .unwrap();
        fs::write(yaml_dir.path().join("pom.xml"), "<project/>").unwrap();
        let yaml_pipes = discover_pipelines(yaml_dir.path());
        assert_eq!(yaml_pipes.len(), 1);
        assert_eq!(yaml_pipes[0].source, ".kanso.yaml");

        let maven_dir = TempDir::new().unwrap();
        fs::write(maven_dir.path().join("pom.xml"), "<project/>").unwrap();
        assert_eq!(discover_pipelines(maven_dir.path())[0].source, "auto:pom.xml");

        let node_dir = TempDir::new().unwrap();
        fs::write(node_dir.path().join("pnpm-lock.yaml"), "lockfileVersion: '9.0'\n").unwrap();
        fs::write(
            node_dir.path().join("package.json"),
            r#"{"scripts":{"typecheck":"tsc --noEmit","build":"vite build"}}"#,
        )
        .unwrap();
        let node_pipes = discover_pipelines(node_dir.path());
        assert_eq!(node_pipes[0].steps[0].command, "pnpm typecheck");
        assert_eq!(node_pipes[0].steps[1].command, "pnpm build");

        let go_dir = TempDir::new().unwrap();
        fs::write(go_dir.path().join("go.mod"), "module example.com/demo\n").unwrap();
        assert_eq!(discover_pipelines(go_dir.path())[0].source, "auto:go.mod");
    }
}
