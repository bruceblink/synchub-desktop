use crate::models::TrashEntry;
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub fn list_trash_entries(root: &Path, limit: usize) -> Result<Vec<TrashEntry>> {
    if limit == 0 {
        bail!("trash entry limit must be positive");
    }
    let trash_root = root.join(".synchub").join("trash");
    if !trash_root.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for batch in fs::read_dir(&trash_root).context("read local trash")? {
        let batch = batch.context("read local trash batch")?;
        if !batch.file_type()?.is_dir() {
            continue;
        }
        let batch_name = batch.file_name().to_string_lossy().to_string();
        if !valid_batch(&batch_name) {
            continue;
        }
        for item in fs::read_dir(batch.path()).context("read local trash entries")? {
            let item = item.context("read local trash entry")?;
            let metadata = item.metadata().context("read local trash metadata")?;
            let is_dir = metadata.is_dir();
            let mut path = item.file_name().to_string_lossy().to_string();
            if is_dir {
                path.push('/');
            }
            entries.push(TrashEntry {
                batch: batch_name.clone(),
                path,
                size: if is_dir { 0 } else { metadata.len() as i64 },
                is_dir,
            });
        }
    }
    entries.sort_by(|left, right| {
        right
            .batch
            .cmp(&left.batch)
            .then_with(|| left.path.cmp(&right.path))
    });
    entries.truncate(limit);
    Ok(entries)
}

pub fn restore_trash_entry(root: &Path, entry: &TrashEntry) -> Result<PathBuf> {
    if !valid_batch(entry.batch.trim()) {
        bail!("valid trash batch is required");
    }
    let relative = clean_entry_path(&entry.path)?;
    let trash_root = root.join(".synchub").join("trash");
    let source = trash_root.join(entry.batch.trim()).join(&relative);
    if !source.exists() {
        bail!("trash entry not found");
    }

    let target = root.join(&relative);
    if target.exists() {
        bail!("restore target already exists: {}", target.display());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).context("create restore target directory")?;
    }
    fs::rename(&source, &target).context("restore local trash entry")?;
    Ok(target)
}

fn valid_batch(batch: &str) -> bool {
    !batch.is_empty() && batch != "." && batch != ".." && !batch.contains(['/', '\\'])
}

fn clean_entry_path(value: &str) -> Result<PathBuf> {
    let normalized = value.trim().trim_matches(['/', '\\']).replace('\\', "/");
    if normalized.is_empty() {
        bail!("trash entry is required");
    }
    let path = Path::new(&normalized);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("trash entry path is invalid");
    }
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "synchub-desktop-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn lists_and_restores_local_trash_without_cli() {
        let root = temp_root("native-trash");
        let _ = fs::remove_dir_all(&root);
        let old = root.join(".synchub/trash/20260701T010000Z/old.txt");
        let nested = root.join(".synchub/trash/20260702T010000Z/docs/nested/a.txt");
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        fs::create_dir_all(nested.parent().unwrap()).unwrap();
        fs::write(&old, b"old").unwrap();
        fs::write(&nested, b"nested").unwrap();

        let entries = list_trash_entries(&root, 200).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "docs/");
        assert_eq!(entries[1].size, 3);

        let restored = restore_trash_entry(&root, &entries[0]).unwrap();
        assert_eq!(restored, root.join("docs"));
        assert_eq!(fs::read(restored.join("nested/a.txt")).unwrap(), b"nested");
        assert_eq!(list_trash_entries(&root, 200).unwrap().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restore_rejects_traversal_and_existing_targets() {
        let root = temp_root("native-trash-validation");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".synchub/trash/batch")).unwrap();
        fs::write(root.join(".synchub/trash/batch/file.txt"), b"trash").unwrap();
        fs::write(root.join("file.txt"), b"current").unwrap();

        let traversal = TrashEntry {
            batch: "batch".into(),
            path: "../outside.txt".into(),
            ..TrashEntry::default()
        };
        assert!(restore_trash_entry(&root, &traversal).is_err());
        let existing = TrashEntry {
            batch: "batch".into(),
            path: "file.txt".into(),
            ..TrashEntry::default()
        };
        assert!(restore_trash_entry(&root, &existing).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
