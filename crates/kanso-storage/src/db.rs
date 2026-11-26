use crate::compression::{compress_zstd, decompress_zstd};
use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRecord {
    pub id: String,
    pub name: String,
    pub root_path: String,
    pub default_trunk: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CiRunRecord {
    pub id: String,
    pub workspace_id: String,
    pub repo_path: String,
    pub pipeline_name: String,
    pub command: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub compressed_log_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct NewCiRun {
    pub id: String,
    pub workspace_id: String,
    pub repo_path: String,
    pub pipeline_name: String,
    pub command: String,
    pub started_at: i64,
}

pub struct StorageDb {
    conn: Mutex<Connection>,
}

impl StorageDb {
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create db directory {}", parent.display()))?;
        }
        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open SQLite database {}", db_path.display()))?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow!("DB lock poisoned: {e}"))?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS workspaces (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                root_path TEXT NOT NULL UNIQUE,
                default_trunk TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS ci_runs (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL,
                repo_path TEXT NOT NULL,
                pipeline_name TEXT NOT NULL,
                command TEXT NOT NULL,
                status TEXT NOT NULL,
                exit_code INTEGER,
                started_at INTEGER NOT NULL,
                finished_at INTEGER,
                duration_ms INTEGER,
                log_zstd BLOB NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_ci_runs_repo_started
                ON ci_runs (repo_path, started_at DESC);
            CREATE TABLE IF NOT EXISTS workspace_snapshots (
                workspace_id TEXT PRIMARY KEY,
                saved_at INTEGER NOT NULL,
                snapshot_zstd BLOB NOT NULL
            );
            ",
        )
        .context("Failed to initialize Kanso SQLite schema")?;
        Ok(())
    }

    pub fn save_snapshot(&self, workspace_id: &str, snapshot_json: &str) -> Result<()> {
        let compressed = compress_zstd(snapshot_json.as_bytes(), 3)?;
        let saved_at = Utc::now().timestamp();
        let conn = self.conn.lock().map_err(|e| anyhow!("DB lock poisoned: {e}"))?;
        conn.execute(
            "INSERT OR REPLACE INTO workspace_snapshots (workspace_id, saved_at, snapshot_zstd)
             VALUES (?1, ?2, ?3)",
            params![workspace_id, saved_at, compressed],
        )?;
        Ok(())
    }

    pub fn load_snapshot(&self, workspace_id: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("DB lock poisoned: {e}"))?;
        let blob: Option<Vec<u8>> = conn
            .query_row(
                "SELECT snapshot_zstd FROM workspace_snapshots WHERE workspace_id = ?1",
                params![workspace_id],
                |row| row.get(0),
            )
            .optional()?;
        match blob {
            Some(bytes) => {
                let raw = decompress_zstd(&bytes)?;
                Ok(Some(
                    String::from_utf8(raw).context("Snapshot JSON is not valid UTF-8")?,
                ))
            }
            None => Ok(None),
        }
    }

    pub fn list_workspaces(&self) -> Result<Vec<WorkspaceRecord>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("DB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, name, root_path, default_trunk, created_at FROM workspaces ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(WorkspaceRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                root_path: row.get(2)?,
                default_trunk: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to query workspaces")
    }

    pub fn add_workspace(
        &self,
        name: &str,
        root_path: &str,
        default_trunk: Option<&str>,
    ) -> Result<WorkspaceRecord> {
        let canonical = fs::canonicalize(root_path)
            .unwrap_or_else(|_| Path::new(root_path).to_path_buf())
            .to_string_lossy()
            .to_string();

        let conn = self.conn.lock().map_err(|e| anyhow!("DB lock poisoned: {e}"))?;
        let existing: Option<WorkspaceRecord> = conn
            .query_row(
                "SELECT id, name, root_path, default_trunk, created_at FROM workspaces WHERE root_path = ?1",
                params![canonical],
                |row| {
                    Ok(WorkspaceRecord {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        root_path: row.get(2)?,
                        default_trunk: row.get(3)?,
                        created_at: row.get(4)?,
                    })
                },
            )
            .optional()?;

        if let Some(ws) = existing {
            return Ok(ws);
        }

        let record = WorkspaceRecord {
            id: Uuid::new_v4().to_string(),
            name: name.trim().to_string(),
            root_path: canonical,
            default_trunk: default_trunk
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            created_at: Utc::now().timestamp(),
        };

        conn.execute(
            "INSERT INTO workspaces (id, name, root_path, default_trunk, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                record.id,
                record.name,
                record.root_path,
                record.default_trunk,
                record.created_at
            ],
        )?;

        Ok(record)
    }

    pub fn remove_workspace(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow!("DB lock poisoned: {e}"))?;
        conn.execute("DELETE FROM workspaces WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn update_workspace_trunk(
        &self,
        id: &str,
        default_trunk: Option<&str>,
    ) -> Result<WorkspaceRecord> {
        let trunk = default_trunk
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let conn = self.conn.lock().map_err(|e| anyhow!("DB lock poisoned: {e}"))?;
        conn.execute(
            "UPDATE workspaces SET default_trunk = ?1 WHERE id = ?2",
            params![trunk, id],
        )?;
        conn.query_row(
            "SELECT id, name, root_path, default_trunk, created_at FROM workspaces WHERE id = ?1",
            params![id],
            |row| {
                Ok(WorkspaceRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    root_path: row.get(2)?,
                    default_trunk: row.get(3)?,
                    created_at: row.get(4)?,
                })
            },
        )
        .context("Workspace not found after trunk update")
    }

    pub fn insert_ci_run(&self, run: NewCiRun) -> Result<CiRunRecord> {
        let empty_compressed = compress_zstd(b"", 3)?;
        let conn = self.conn.lock().map_err(|e| anyhow!("DB lock poisoned: {e}"))?;
        conn.execute(
            "INSERT INTO ci_runs (id, workspace_id, repo_path, pipeline_name, command, status, exit_code, started_at, finished_at, duration_ms, log_zstd)
             VALUES (?1, ?2, ?3, ?4, ?5, 'running', NULL, ?6, NULL, NULL, ?7)",
            params![
                run.id,
                run.workspace_id,
                run.repo_path,
                run.pipeline_name,
                run.command,
                run.started_at,
                empty_compressed
            ],
        )?;

        Ok(CiRunRecord {
            id: run.id,
            workspace_id: run.workspace_id,
            repo_path: run.repo_path,
            pipeline_name: run.pipeline_name,
            command: run.command,
            status: "running".to_string(),
            exit_code: None,
            started_at: run.started_at,
            finished_at: None,
            duration_ms: None,
            compressed_log_bytes: empty_compressed.len(),
        })
    }

    pub fn finish_ci_run(
        &self,
        id: &str,
        status: &str,
        exit_code: Option<i32>,
        finished_at: i64,
        duration_ms: i64,
        full_log: &str,
    ) -> Result<()> {
        let compressed = compress_zstd(full_log.as_bytes(), 3)?;
        let conn = self.conn.lock().map_err(|e| anyhow!("DB lock poisoned: {e}"))?;
        conn.execute(
            "UPDATE ci_runs SET status = ?1, exit_code = ?2, finished_at = ?3, duration_ms = ?4, log_zstd = ?5 WHERE id = ?6",
            params![status, exit_code, finished_at, duration_ms, compressed, id],
        )?;
        Ok(())
    }

    pub fn list_ci_runs(&self, repo_path: Option<&str>, limit: usize) -> Result<Vec<CiRunRecord>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("DB lock poisoned: {e}"))?;
        let limit_i64 = limit as i64;
        let mut records = Vec::new();

        if let Some(repo) = repo_path {
            let mut stmt = conn.prepare(
                "SELECT id, workspace_id, repo_path, pipeline_name, command, status, exit_code, started_at, finished_at, duration_ms, length(log_zstd)
                 FROM ci_runs WHERE repo_path = ?1 ORDER BY started_at DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![repo, limit_i64], map_ci_run_row)?;
            for row in rows {
                records.push(row?);
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, workspace_id, repo_path, pipeline_name, command, status, exit_code, started_at, finished_at, duration_ms, length(log_zstd)
                 FROM ci_runs ORDER BY started_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit_i64], map_ci_run_row)?;
            for row in rows {
                records.push(row?);
            }
        }

        Ok(records)
    }

    pub fn get_ci_run_log(&self, id: &str) -> Result<String> {
        let conn = self.conn.lock().map_err(|e| anyhow!("DB lock poisoned: {e}"))?;
        let blob: Vec<u8> = conn
            .query_row(
                "SELECT log_zstd FROM ci_runs WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .context("CI run log not found")?;
        let raw = decompress_zstd(&blob)?;
        String::from_utf8(raw).context("CI log is not valid UTF-8")
    }

    pub fn latest_ci_status_by_repo(&self) -> Result<HashMap<String, String>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("DB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT repo_path, status FROM ci_runs ORDER BY started_at DESC, rowid DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut latest = HashMap::new();
        for row in rows {
            let (repo_path, status) = row?;
            latest.entry(repo_path).or_insert(status);
        }
        Ok(latest)
    }
}

fn map_ci_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CiRunRecord> {
    let bytes: i64 = row.get(10)?;
    Ok(CiRunRecord {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        repo_path: row.get(2)?,
        pipeline_name: row.get(3)?,
        command: row.get(4)?,
        status: row.get(5)?,
        exit_code: row.get(6)?,
        started_at: row.get(7)?,
        finished_at: row.get(8)?,
        duration_ms: row.get(9)?,
        compressed_log_bytes: bytes.max(0) as usize,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn stores_workspaces_and_zstd_compressed_ci_logs() {
        let dir = TempDir::new().expect("temp dir");
        let db = StorageDb::open(&dir.path().join("kanso.db")).expect("open db");

        let ws_dir = dir.path().join("project");
        fs::create_dir_all(&ws_dir).unwrap();
        let ws = db
            .add_workspace("nexus", ws_dir.to_str().unwrap(), Some("dev-Java"))
            .expect("add workspace");
        assert_eq!(ws.default_trunk.as_deref(), Some("dev-Java"));
        assert_eq!(db.list_workspaces().unwrap().len(), 1);

        let updated = db
            .update_workspace_trunk(&ws.id, Some("main"))
            .expect("update trunk");
        assert_eq!(updated.default_trunk.as_deref(), Some("main"));

        let run = db
            .insert_ci_run(NewCiRun {
                id: "run-1".to_string(),
                workspace_id: ws.id.clone(),
                repo_path: ws_dir.to_string_lossy().to_string(),
                pipeline_name: "Check".to_string(),
                command: "cargo check".to_string(),
                started_at: 1000,
            })
            .expect("insert run");
        assert_eq!(run.status, "running");

        let log_body = "line 1 ok\nline 2 ok\n".repeat(100);
        db.finish_ci_run("run-1", "passed", Some(0), 1005, 5000, &log_body)
            .expect("finish run");

        let runs = db.list_ci_runs(None, 10).expect("list runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "passed");
        assert!(runs[0].compressed_log_bytes < log_body.len());

        let restored = db.get_ci_run_log("run-1").expect("get log");
        assert_eq!(restored, log_body);

        db.remove_workspace(&ws.id).expect("remove workspace");
        assert!(db.list_workspaces().unwrap().is_empty());
    }

    #[test]
    fn workspace_add_is_idempotent_and_ci_runs_order_by_latest() {
        let dir = TempDir::new().expect("temp dir");
        let db = StorageDb::open(&dir.path().join("kanso.db")).expect("open db");

        let ws_dir = dir.path().join("repo");
        fs::create_dir_all(&ws_dir).unwrap();
        let first = db
            .add_workspace("repo", ws_dir.to_str().unwrap(), None)
            .expect("first add");
        let second = db
            .add_workspace("repo-dup", ws_dir.to_str().unwrap(), Some("main"))
            .expect("second add");
        assert_eq!(first.id, second.id);
        assert_eq!(db.list_workspaces().unwrap().len(), 1);

        db.insert_ci_run(NewCiRun {
            id: "r1".to_string(),
            workspace_id: first.id.clone(),
            repo_path: ws_dir.to_string_lossy().to_string(),
            pipeline_name: "Old".to_string(),
            command: "echo 1".to_string(),
            started_at: 100,
        })
        .unwrap();
        db.insert_ci_run(NewCiRun {
            id: "r2".to_string(),
            workspace_id: first.id.clone(),
            repo_path: ws_dir.to_string_lossy().to_string(),
            pipeline_name: "New".to_string(),
            command: "echo 2".to_string(),
            started_at: 200,
        })
        .unwrap();

        let runs = db
            .list_ci_runs(Some(ws_dir.to_str().unwrap()), 1)
            .expect("limit 1");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, "r2");

        db.finish_ci_run("r1", "failed", Some(1), 110, 10, "err")
            .unwrap();
        db.finish_ci_run("r2", "passed", Some(0), 210, 10, "ok")
            .unwrap();
        let latest = db.latest_ci_status_by_repo().expect("latest map");
        assert_eq!(
            latest.get(ws_dir.to_str().unwrap()).map(String::as_str),
            Some("passed")
        );
    }

    #[test]
    fn saves_overwrites_and_loads_zstd_snapshots() {
        let dir = TempDir::new().expect("temp dir");
        let db = StorageDb::open(&dir.path().join("kanso.db")).expect("open db");

        assert!(db.load_snapshot("non-existent").unwrap().is_none());

        let v1 = r#"{"workspaceId":"ws-1","repos":[{"name":"alpha"}]}"#;
        db.save_snapshot("ws-1", v1).expect("save v1");
        assert_eq!(db.load_snapshot("ws-1").unwrap().as_deref(), Some(v1));

        let v2 = r#"{"workspaceId":"ws-1","repos":[{"name":"beta"}]}"#;
        db.save_snapshot("ws-1", v2).expect("overwrite v2");
        assert_eq!(db.load_snapshot("ws-1").unwrap().as_deref(), Some(v2));
    }
}
