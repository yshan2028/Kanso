use kanso_core::load_kanso_yaml;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineStep {
    pub name: String,
    pub command: String,
    pub working_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineDefinition {
    pub name: String,
    pub source: String,
    pub steps: Vec<PipelineStep>,
}

pub fn discover_pipelines(repo_path: &Path) -> Vec<PipelineDefinition> {
    if let Ok(Some(yaml)) = load_kanso_yaml(repo_path) {
        if !yaml.pipelines.is_empty() {
            return yaml
                .pipelines
                .into_iter()
                .map(|p| PipelineDefinition {
                    name: p.name,
                    source: ".kanso.yaml".to_string(),
                    steps: p
                        .steps
                        .into_iter()
                        .map(|s| PipelineStep {
                            name: s.name,
                            command: s.command,
                            working_dir: s.working_dir,
                        })
                        .collect(),
                })
                .collect();
        }
    }

    let mut discovered = Vec::new();

    if repo_path.join("Cargo.toml").exists() {
        discovered.push(PipelineDefinition {
            name: "Rust Check & Test".to_string(),
            source: "auto:Cargo.toml".to_string(),
            steps: vec![
                PipelineStep {
                    name: "Cargo Check".to_string(),
                    command: "cargo check".to_string(),
                    working_dir: None,
                },
                PipelineStep {
                    name: "Cargo Test".to_string(),
                    command: "cargo test".to_string(),
                    working_dir: None,
                },
            ],
        });
    }

    if repo_path.join("pom.xml").exists() {
        discovered.push(PipelineDefinition {
            name: "Maven Compile & Test".to_string(),
            source: "auto:pom.xml".to_string(),
            steps: vec![
                PipelineStep {
                    name: "Maven Test Compile".to_string(),
                    command: "mvn -q test-compile".to_string(),
                    working_dir: None,
                },
                PipelineStep {
                    name: "Maven Test".to_string(),
                    command: "mvn -q test".to_string(),
                    working_dir: None,
                },
            ],
        });
    }

    if repo_path.join("build.gradle").exists() || repo_path.join("build.gradle.kts").exists() {
        let gradle_cmd = if repo_path.join("gradlew").exists() {
            "./gradlew test"
        } else {
            "gradle test"
        };
        discovered.push(PipelineDefinition {
            name: "Gradle Test".to_string(),
            source: "auto:gradle".to_string(),
            steps: vec![PipelineStep {
                name: "Gradle Test".to_string(),
                command: gradle_cmd.to_string(),
                working_dir: None,
            }],
        });
    }

    let pkg_path = repo_path.join("package.json");
    if pkg_path.exists() {
        let pm = if repo_path.join("pnpm-lock.yaml").exists() {
            "pnpm"
        } else if repo_path.join("yarn.lock").exists() {
            "yarn"
        } else {
            "npm run"
        };

        let mut steps = Vec::new();
        if let Ok(raw) = fs::read_to_string(&pkg_path) {
            if let Ok(json) = serde_json::from_str::<Value>(&raw) {
                if let Some(scripts) = json.get("scripts").and_then(Value::as_object) {
                    for key in ["typecheck", "lint", "build", "test"] {
                        if scripts.contains_key(key) {
                            let cmd = if pm == "npm run" {
                                format!("npm run {key}")
                            } else {
                                format!("{pm} {key}")
                            };
                            steps.push(PipelineStep {
                                name: format!("Script: {key}"),
                                command: cmd,
                                working_dir: None,
                            });
                        }
                    }
                }
            }
        }

        if steps.is_empty() {
            steps.push(PipelineStep {
                name: "Node Test".to_string(),
                command: "npm test".to_string(),
                working_dir: None,
            });
        }

        discovered.push(PipelineDefinition {
            name: "Node Build & Verify".to_string(),
            source: "auto:package.json".to_string(),
            steps,
        });
    }

    if repo_path.join("go.mod").exists() {
        discovered.push(PipelineDefinition {
            name: "Go Test".to_string(),
            source: "auto:go.mod".to_string(),
            steps: vec![PipelineStep {
                name: "Go Test".to_string(),
                command: "go test ./...".to_string(),
                working_dir: None,
            }],
        });
    }

    discovered
}
