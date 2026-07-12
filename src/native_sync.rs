use crate::models::{Manifest, ManifestEntry, WorkspaceSnapshot};
use crate::native_manifest::scan_current_manifest;
use anyhow::Result;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncPlanAction {
    Create,
    Update,
    Delete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncPlanEntry {
    pub action: SyncPlanAction,
    pub relative_path: String,
    pub remote_path: String,
    pub size: i64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SyncPlan {
    pub entries: Vec<SyncPlanEntry>,
}

impl SyncPlan {
    pub fn created(&self) -> usize {
        self.count(SyncPlanAction::Create)
    }

    pub fn updated(&self) -> usize {
        self.count(SyncPlanAction::Update)
    }

    pub fn deleted(&self) -> usize {
        self.count(SyncPlanAction::Delete)
    }

    pub fn summary(&self) -> String {
        format!(
            "{} change(s): {} created, {} updated, {} deleted",
            self.entries.len(),
            self.created(),
            self.updated(),
            self.deleted()
        )
    }

    pub fn display(&self) -> String {
        if self.entries.is_empty() {
            return "No local changes".to_string();
        }
        self.entries
            .iter()
            .map(|entry| {
                let action = match entry.action {
                    SyncPlanAction::Create => "create",
                    SyncPlanAction::Update => "update",
                    SyncPlanAction::Delete => "delete",
                };
                format!("{action} {}", entry.relative_path)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn count(&self, action: SyncPlanAction) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.action == action)
            .count()
    }
}

pub fn build_sync_plan(workspace: &WorkspaceSnapshot) -> Result<(Manifest, SyncPlan)> {
    let current = scan_current_manifest(workspace)?;
    let previous = workspace.manifest.clone().unwrap_or_default();
    Ok((current.clone(), compare_manifests(&previous, &current)))
}

fn compare_manifests(previous: &Manifest, current: &Manifest) -> SyncPlan {
    let previous = entries_by_path(previous);
    let current = entries_by_path(current);
    let mut entries = Vec::new();

    for (path, item) in &current {
        match previous.get(path) {
            None => entries.push(plan_entry(SyncPlanAction::Create, item)),
            Some(old) if old.size != item.size || old.sha256 != item.sha256 => {
                entries.push(plan_entry(SyncPlanAction::Update, item));
            }
            _ => {}
        }
    }
    for (path, item) in &previous {
        if !current.contains_key(path) {
            entries.push(plan_entry(SyncPlanAction::Delete, item));
        }
    }
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    SyncPlan { entries }
}

fn entries_by_path(manifest: &Manifest) -> BTreeMap<String, &ManifestEntry> {
    manifest
        .items
        .iter()
        .filter_map(|item| {
            let path = item.relative_path.trim().replace('\\', "/");
            (!path.is_empty()).then_some((path, item))
        })
        .collect()
}

fn plan_entry(action: SyncPlanAction, item: &ManifestEntry) -> SyncPlanEntry {
    SyncPlanEntry {
        action,
        relative_path: item.relative_path.replace('\\', "/"),
        remote_path: item.path.clone(),
        size: item.size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::WorkspaceRegistryEntry;
    use std::fs;

    #[test]
    fn plans_created_updated_and_deleted_files_without_changing_baseline() {
        let root = std::env::temp_dir().join(format!(
            "synchub-native-sync-plan-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".synchub")).unwrap();
        fs::write(root.join("created.txt"), b"created").unwrap();
        fs::write(root.join("updated.txt"), b"new").unwrap();
        let baseline_path = root.join(".synchub/manifest.json");
        fs::write(&baseline_path, b"baseline remains unchanged").unwrap();
        let workspace = WorkspaceSnapshot {
            entry: WorkspaceRegistryEntry {
                root: root.display().to_string(),
                remote_path: "/workspace".into(),
                ..Default::default()
            },
            manifest: Some(Manifest {
                items: vec![
                    ManifestEntry {
                        relative_path: "updated.txt".into(),
                        path: "/workspace/updated.txt".into(),
                        size: 3,
                        sha256: "old".into(),
                        ..Default::default()
                    },
                    ManifestEntry {
                        relative_path: "deleted.txt".into(),
                        path: "/workspace/deleted.txt".into(),
                        size: 7,
                        sha256: "old".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        };

        let (_, plan) = build_sync_plan(&workspace).unwrap();
        assert_eq!((plan.created(), plan.updated(), plan.deleted()), (1, 1, 1));
        assert_eq!(
            fs::read_to_string(baseline_path).unwrap(),
            "baseline remains unchanged"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
