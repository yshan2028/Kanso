use anyhow::{Context, Result, anyhow};
use clap::{Parser, Subcommand};
use kanso_ci::{CiRunner, LocalCiRunner, PipelineDefinition, PipelineStep, discover_pipelines};
use kanso_core::{
    CommitRequest, WorkspaceConfig, commit_repo, inspect_repo_changes, land_branch_to_trunk,
    push_repo, scan_workspace,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "kanso", version, about = "Kanso multi-repo control tower CLI & MCP server")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan a parent directory containing multiple Git repositories
    Scan {
        dir: PathBuf,
        #[arg(long)]
        trunk: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Aggregate active feature-branch tasks across repositories
    Tasks {
        dir: PathBuf,
        #[arg(long)]
        trunk: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Run auto-detected or .kanso.yaml local CI on a repository
    Ci {
        repo_dir: PathBuf,
        #[arg(long)]
        pipeline: Option<String>,
        #[arg(long)]
        command: Option<String>,
    },
    /// Inspect uncommitted file changes, preflight checks, and diff of a repository
    Diff {
        repo_dir: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Commit changes (with preflight checks) via Git or Jujutsu and optionally push
    Commit {
        repo_dir: PathBuf,
        #[arg(short = 'm', long)]
        message: String,
        #[arg(long)]
        push: Option<String>,
        #[arg(long)]
        allow_large_files: bool,
    },
    /// Safely merge current feature branch into trunk
    Land {
        repo_dir: PathBuf,
        #[arg(long)]
        trunk: String,
        #[arg(long)]
        delete_source: bool,
    },
    /// Start stdio JSON-RPC 2.0 MCP Server for AI coding agents
    Mcp,
}

fn build_workspace_config(dir: &Path, trunk: Option<String>) -> Result<WorkspaceConfig> {
    let canonical = std::fs::canonicalize(dir)
        .with_context(|| format!("Directory not found: {}", dir.display()))?;
    let name = canonical
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "workspace".to_string());
    Ok(WorkspaceConfig {
        id: "cli-workspace".to_string(),
        name,
        root_path: canonical.to_string_lossy().to_string(),
        default_trunk: trunk,
        ignore_dirs: Vec::new(),
    })
}

fn print_scan_table(dir: &Path, trunk: Option<String>, json_out: bool) -> Result<()> {
    let cfg = build_workspace_config(dir, trunk)?;
    let snapshot = scan_workspace(&cfg)?;
    if json_out {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        return Ok(());
    }

    println!(
        "Kanso Scan: {} ({} repos, {} tasks, {} alerts, {} ms)\n",
        snapshot.workspace_name,
        snapshot.repos.len(),
        snapshot.tasks.len(),
        snapshot.alerts.len(),
        snapshot.scan_duration_ms
    );

    println!(
        "{:<26} {:<24} {:<12} {:<10} {:<10} {}",
        "REPO", "BRANCH", "TRUNK", "TRUNK +/-", "DIRTY", "LAST COMMIT"
    );
    println!("{}", "-".repeat(104));
    for repo in &snapshot.repos {
        let trunk = repo.trunk_branch.as_deref().unwrap_or("-");
        let trunk_delta = if repo.is_on_trunk {
            "on trunk".to_string()
        } else {
            format!("+{} / -{}", repo.trunk_ahead, repo.trunk_behind)
        };
        let dirty = repo.total_dirty();
        let dirty_str = if dirty == 0 {
            "clean".to_string()
        } else if repo.has_dirty_on_trunk {
            format!("{dirty} (TRUNK!)")
        } else {
            format!("{dirty}")
        };
        let commit_msg = repo
            .last_commit_message
            .as_deref()
            .unwrap_or("-")
            .chars()
            .take(36)
            .collect::<String>();

        println!(
            "{:<26} {:<24} {:<12} {:<10} {:<10} {}",
            repo.name.chars().take(25).collect::<String>(),
            repo.current_branch.chars().take(23).collect::<String>(),
            trunk.chars().take(11).collect::<String>(),
            trunk_delta,
            dirty_str,
            commit_msg
        );
    }

    if !snapshot.alerts.is_empty() {
        println!("\nGuardrail Alerts:");
        for alert in &snapshot.alerts {
            println!(
                "  - [{}] {} ({} files)",
                serde_json::to_string(&alert.kind)?,
                alert.repo_name,
                alert.dirty_files
            );
        }
    }

    Ok(())
}

fn print_tasks(dir: &Path, trunk: Option<String>, json_out: bool) -> Result<()> {
    let cfg = build_workspace_config(dir, trunk)?;
    let snapshot = scan_workspace(&cfg)?;
    if json_out {
        println!("{}", serde_json::to_string_pretty(&snapshot.tasks)?);
        return Ok(());
    }

    if snapshot.tasks.is_empty() {
        println!("No active feature-branch tasks detected.");
        return Ok(());
    }

    for task in &snapshot.tasks {
        println!(
            "Task: {} | stage={:?} ({}%) | repos={} | dirty={} | unpushed={}",
            task.branch,
            task.stage,
            task.progress_percent,
            task.total_repos,
            task.dirty_repos,
            task.unpushed_repos
        );
        println!("  Repos: {}", task.repo_names.join(", "));
    }
    Ok(())
}

async fn run_cli_ci(
    repo_dir: &Path,
    pipeline_name: Option<String>,
    command: Option<String>,
) -> Result<()> {
    let canonical = std::fs::canonicalize(repo_dir)
        .with_context(|| format!("Repository not found: {}", repo_dir.display()))?;

    let pipeline = if let Some(cmd) = command {
        PipelineDefinition {
            name: "Custom Command".to_string(),
            source: "cli".to_string(),
            steps: vec![PipelineStep {
                name: "Run".to_string(),
                command: cmd,
                working_dir: None,
            }],
        }
    } else {
        let pipelines = discover_pipelines(&canonical);
        if pipelines.is_empty() {
            return Err(anyhow!(
                "No .kanso.yaml or recognized project stack found in {}",
                canonical.display()
            ));
        }
        if let Some(target) = pipeline_name {
            pipelines
                .into_iter()
                .find(|p| p.name.eq_ignore_ascii_case(&target))
                .ok_or_else(|| anyhow!("Pipeline '{target}' not found"))?
        } else {
            pipelines.into_iter().next().unwrap()
        }
    };

    let (tx, mut rx) = mpsc::unbounded_channel::<kanso_ci::CiLogEvent>();
    let (_cancel_tx, cancel_rx) = oneshot::channel();
    let run_id = Uuid::new_v4().to_string();

    let print_handle = tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            if ev.stream == "stderr" {
                eprintln!("{}", ev.line);
            } else {
                println!("{}", ev.line);
            }
        }
    });

    let runner = LocalCiRunner;
    let outcome = runner
        .run_pipeline(&run_id, &canonical, &pipeline, tx, cancel_rx)
        .await?;
    let _ = print_handle.await;

    if outcome.status != "passed" {
        std::process::exit(outcome.exit_code.unwrap_or(1));
    }
    Ok(())
}

#[derive(Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

async fn handle_mcp_call(name: &str, args: &Value) -> Result<Value> {
    match name {
        "kanso_scan_workspace" => {
            let root = args
                .get("rootPath")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("Missing required argument: rootPath"))?;
            let trunk = args
                .get("defaultTrunk")
                .and_then(Value::as_str)
                .map(str::to_string);
            let cfg = build_workspace_config(Path::new(root), trunk)?;
            let snapshot = scan_workspace(&cfg)?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&snapshot)?
                }]
            }))
        }
        "kanso_list_tasks" => {
            let root = args
                .get("rootPath")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("Missing required argument: rootPath"))?;
            let trunk = args
                .get("defaultTrunk")
                .and_then(Value::as_str)
                .map(str::to_string);
            let cfg = build_workspace_config(Path::new(root), trunk)?;
            let snapshot = scan_workspace(&cfg)?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&snapshot.tasks)?
                }]
            }))
        }
        "kanso_run_local_ci" => {
            let repo_path = args
                .get("repoPath")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("Missing required argument: repoPath"))?;
            let canonical = std::fs::canonicalize(repo_path)?;
            let pipeline = if let Some(cmd) = args.get("command").and_then(Value::as_str) {
                PipelineDefinition {
                    name: "MCP Custom Command".to_string(),
                    source: "mcp".to_string(),
                    steps: vec![PipelineStep {
                        name: "Command".to_string(),
                        command: cmd.to_string(),
                        working_dir: None,
                    }],
                }
            } else {
                let pipelines = discover_pipelines(&canonical);
                let target = args.get("pipelineName").and_then(Value::as_str);
                match target {
                    Some(name) => pipelines
                        .into_iter()
                        .find(|p| p.name == name)
                        .ok_or_else(|| anyhow!("Pipeline '{name}' not found"))?,
                    None => pipelines
                        .into_iter()
                        .next()
                        .ok_or_else(|| anyhow!("No pipeline found for {}", canonical.display()))?,
                }
            };

            let (tx, _rx) = mpsc::unbounded_channel();
            let (_cancel_tx, cancel_rx) = oneshot::channel();
            let outcome = LocalCiRunner
                .run_pipeline(&Uuid::new_v4().to_string(), &canonical, &pipeline, tx, cancel_rx)
                .await?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&outcome)?
                }],
                "isError": outcome.status != "passed"
            }))
        }
        "kanso_repo_changes" => {
            let repo_path = args
                .get("repoPath")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("Missing required argument: repoPath"))?;
            let cs = inspect_repo_changes(Path::new(repo_path))?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&cs)?
                }]
            }))
        }
        "kanso_commit_repo" => {
            let repo_path = args
                .get("repoPath")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("Missing required argument: repoPath"))?;
            let message = args
                .get("message")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("Missing required argument: message"))?
                .to_string();
            let author_name = args
                .get("authorName")
                .and_then(Value::as_str)
                .map(str::to_string);
            let author_email = args
                .get("authorEmail")
                .and_then(Value::as_str)
                .map(str::to_string);
            let push_remote = args
                .get("pushRemote")
                .and_then(Value::as_str)
                .map(str::to_string);

            let commit_res = commit_repo(&CommitRequest {
                repo_path: repo_path.to_string(),
                message,
                selected_files: Vec::new(),
                author_name,
                author_email,
                allow_large_files: false,
            })?;

            let push_res = if let Some(remote) = push_remote {
                Some(push_repo(Path::new(repo_path), &remote, None)?)
            } else {
                None
            };

            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&json!({
                        "commit": commit_res,
                        "push": push_res
                    }))?
                }]
            }))
        }
        _ => Err(anyhow!("Unknown tool: {name}")),
    }
}

async fn run_mcp_server() -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(err) => {
                let resp = JsonRpcResponse {
                    jsonrpc: "2.0",
                    id: Value::Null,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32700,
                        message: format!("Parse error: {err}"),
                    }),
                };
                writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
                stdout.flush()?;
                continue;
            }
        };

        let Some(id) = req.id else {
            continue;
        };

        let response = match req.method.as_str() {
            "initialize" => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": "kanso",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                })),
                error: None,
            },
            "ping" => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(json!({})),
                error: None,
            },
            "tools/list" => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(json!({
                    "tools": [
                        {
                            "name": "kanso_scan_workspace",
                            "description": "Scan a multi-repository workspace directory and return branches, trunk offsets, dirty files, tasks, and guardrail alerts.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "rootPath": { "type": "string", "description": "Absolute path to workspace directory" },
                                    "defaultTrunk": { "type": "string", "description": "Optional preferred trunk branch name" }
                                },
                                "required": ["rootPath"]
                            }
                        },
                        {
                            "name": "kanso_list_tasks",
                            "description": "List cross-repository feature branch tasks and their automatically derived progress stage.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "rootPath": { "type": "string", "description": "Absolute path to workspace directory" },
                                    "defaultTrunk": { "type": "string", "description": "Optional preferred trunk branch name" }
                                },
                                "required": ["rootPath"]
                            }
                        },
                        {
                            "name": "kanso_run_local_ci",
                            "description": "Run local CI pipeline (.kanso.yaml, auto-detected stack, or custom command) inside a repository.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "repoPath": { "type": "string", "description": "Absolute path to Git repository" },
                                    "pipelineName": { "type": "string", "description": "Optional pipeline name from .kanso.yaml or auto-detection" },
                                    "command": { "type": "string", "description": "Optional explicit shell command to execute" }
                                },
                                "required": ["repoPath"]
                            }
                        },
                        {
                            "name": "kanso_repo_changes",
                            "description": "Inspect modified/untracked files, preflight guardrail checks, remotes, and diff of a Git or Jujutsu repository.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "repoPath": { "type": "string", "description": "Absolute path to repository" }
                                },
                                "required": ["repoPath"]
                            }
                        },
                        {
                            "name": "kanso_commit_repo",
                            "description": "Run preflight checks, commit changes in a Git or Jujutsu repository, and optionally push to a remote.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "repoPath": { "type": "string", "description": "Absolute path to repository" },
                                    "message": { "type": "string", "description": "Commit message" },
                                    "pushRemote": { "type": "string", "description": "Optional remote name to push after commit (e.g. origin or forgejo)" },
                                    "authorName": { "type": "string", "description": "Optional commit author name" },
                                    "authorEmail": { "type": "string", "description": "Optional commit author email" }
                                },
                                "required": ["repoPath", "message"]
                            }
                        }
                    ]
                })),
                error: None,
            },
            "tools/call" => {
                let tool_name = req
                    .params
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let args = req.params.get("arguments").cloned().unwrap_or_else(|| json!({}));
                match handle_mcp_call(tool_name, &args).await {
                    Ok(result) => JsonRpcResponse {
                        jsonrpc: "2.0",
                        id,
                        result: Some(result),
                        error: None,
                    },
                    Err(err) => JsonRpcResponse {
                        jsonrpc: "2.0",
                        id,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32602,
                            message: err.to_string(),
                        }),
                    },
                }
            }
            _ => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32601,
                    message: format!("Method not found: {}", req.method),
                }),
            },
        };

        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }

    Ok(())
}

fn print_diff(repo_dir: &Path, json_out: bool) -> Result<()> {
    let cs = inspect_repo_changes(repo_dir)?;
    if json_out {
        println!("{}", serde_json::to_string_pretty(&cs)?);
        return Ok(());
    }
    println!(
        "仓库: {} | 引擎: {:?} | 分支: {} | 远端: {:?}",
        cs.repo_name, cs.vcs_kind, cs.current_branch, cs.remotes
    );
    println!("变更文件 ({}):", cs.files.len());
    for f in &cs.files {
        println!("  - [{:?}] {} ({} B)", f.status, f.path, f.size_bytes);
    }
    if !cs.preflight_issues.is_empty() {
        println!("提交前安检提示:");
        for issue in &cs.preflight_issues {
            println!("  - [{}] {}: {}", issue.severity, issue.code, issue.message);
        }
    }
    if !cs.diff_text.trim().is_empty() {
        println!("\n{}", cs.diff_text);
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Scan { dir, trunk, json } => print_scan_table(&dir, trunk, json),
        Commands::Tasks { dir, trunk, json } => print_tasks(&dir, trunk, json),
        Commands::Ci {
            repo_dir,
            pipeline,
            command,
        } => run_cli_ci(&repo_dir, pipeline, command).await,
        Commands::Diff { repo_dir, json } => print_diff(&repo_dir, json),
        Commands::Commit {
            repo_dir,
            message,
            push,
            allow_large_files,
        } => {
            let res = commit_repo(&CommitRequest {
                repo_path: repo_dir.to_string_lossy().to_string(),
                message,
                selected_files: Vec::new(),
                author_name: None,
                author_email: None,
                allow_large_files,
            })?;
            println!("{}", res.summary);
            if let Some(remote) = push {
                let push_res = push_repo(&repo_dir, &remote, None)?;
                println!("{}", push_res.summary);
            }
            Ok(())
        }
        Commands::Land {
            repo_dir,
            trunk,
            delete_source,
        } => {
            let res = land_branch_to_trunk(&repo_dir, &trunk, delete_source)?;
            println!("{}", res.summary);
            Ok(())
        }
        Commands::Mcp => run_mcp_server().await,
    }
}
