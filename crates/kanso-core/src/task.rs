use crate::model::{GuardrailAlert, GuardrailKind, RepoStatus, TaskGroup, TaskStage};
use std::collections::BTreeMap;

pub fn aggregate_tasks(repos: &[RepoStatus]) -> Vec<TaskGroup> {
    let mut grouped: BTreeMap<String, Vec<&RepoStatus>> = BTreeMap::new();

    for repo in repos {
        if repo.is_detached || repo.is_on_trunk {
            continue;
        }
        grouped
            .entry(repo.current_branch.clone())
            .or_default()
            .push(repo);
    }

    let mut tasks = Vec::with_capacity(grouped.len());
    for (branch, members) in grouped {
        let total_repos = members.len();
        let dirty_repos = members.iter().filter(|r| r.is_dirty()).count();
        let unpushed_repos = members
            .iter()
            .filter(|r| r.upstream.is_none() || r.upstream_ahead > 0)
            .count();
        let behind_trunk_repos = members.iter().filter(|r| r.trunk_behind > 0).count();
        let has_commits_ahead = members.iter().any(|r| r.trunk_ahead > 0);

        let (stage, progress_percent) = if dirty_repos > 0 {
            (TaskStage::Dirty, 35)
        } else if behind_trunk_repos > 0 {
            (TaskStage::InProgress, 55)
        } else if unpushed_repos > 0 && has_commits_ahead {
            (TaskStage::ReadyForCi, 75)
        } else if has_commits_ahead {
            (TaskStage::ReadyToLand, 90)
        } else {
            (TaskStage::InProgress, 20)
        };

        let id = blake3::hash(branch.as_bytes()).to_hex()[..12].to_string();
        tasks.push(TaskGroup {
            id,
            branch,
            stage,
            progress_percent,
            repo_ids: members.iter().map(|r| r.id.clone()).collect(),
            repo_names: members.iter().map(|r| r.name.clone()).collect(),
            total_repos,
            dirty_repos,
            unpushed_repos,
            behind_trunk_repos,
        });
    }

    tasks.sort_by(|a, b| {
        b.dirty_repos
            .cmp(&a.dirty_repos)
            .then_with(|| b.total_repos.cmp(&a.total_repos))
            .then_with(|| a.branch.cmp(&b.branch))
    });

    tasks
}

pub fn detect_alerts(repos: &[RepoStatus]) -> Vec<GuardrailAlert> {
    let mut alerts = Vec::new();

    for repo in repos {
        if repo.conflicted_count > 0 {
            alerts.push(GuardrailAlert {
                repo_id: repo.id.clone(),
                repo_name: repo.name.clone(),
                repo_path: repo.path.clone(),
                trunk_branch: repo.trunk_branch.clone(),
                dirty_files: repo.conflicted_count,
                kind: GuardrailKind::MergeConflict,
            });
        } else if repo.is_detached {
            alerts.push(GuardrailAlert {
                repo_id: repo.id.clone(),
                repo_name: repo.name.clone(),
                repo_path: repo.path.clone(),
                trunk_branch: repo.trunk_branch.clone(),
                dirty_files: repo.total_dirty(),
                kind: GuardrailKind::DetachedHead,
            });
        } else if repo.has_dirty_on_trunk {
            alerts.push(GuardrailAlert {
                repo_id: repo.id.clone(),
                repo_name: repo.name.clone(),
                repo_path: repo.path.clone(),
                trunk_branch: repo.trunk_branch.clone(),
                dirty_files: repo.total_dirty(),
                kind: GuardrailKind::DirtyTrunk,
            });
        }
    }

    alerts
}
