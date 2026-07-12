use crate::models::{Manifest, ManifestEntry, WorkspaceSnapshot};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::Path;
use std::time::SystemTime;

const IGNORE_FILE: &str = ".synchubignore";

#[derive(Clone, Debug, Eq, PartialEq)]
struct IgnoreRule {
    pattern: String,
    directory: bool,
}

pub fn scan_and_save_manifest(workspace: &WorkspaceSnapshot) -> Result<Manifest> {
    let manifest = scan_current_manifest(workspace)?;
    write_manifest(
        &workspace.root_path().join(".synchub").join("manifest.json"),
        &manifest,
    )?;
    Ok(manifest)
}

pub fn scan_current_manifest(workspace: &WorkspaceSnapshot) -> Result<Manifest> {
    let root = workspace.root_path();
    if root.as_os_str().is_empty() || !root.is_dir() {
        bail!("workspace root is not a directory: {}", root.display());
    }
    let remote_path = normalize_remote_path(&workspace.remote_path());
    let rules = load_ignore_rules(&root)?;
    let previous = workspace
        .manifest
        .as_ref()
        .map(previous_versions)
        .unwrap_or_default();
    let mut items = Vec::new();
    scan_directory(&root, &root, &remote_path, &rules, &previous, &mut items)?;
    items.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let manifest = Manifest {
        version: 1,
        root: root.display().to_string(),
        remote_path,
        generated_at: Some(crate::app::time::rfc3339_from_system_time(SystemTime::now())),
        items,
    };
    Ok(manifest)
}

fn scan_directory(
    root: &Path,
    directory: &Path,
    remote_path: &str,
    rules: &[IgnoreRule],
    previous: &HashMap<String, (i64, String, Option<i64>)>,
    items: &mut Vec<ManifestEntry>,
) -> Result<()> {
    for entry in fs::read_dir(directory).with_context(|| format!("read {}", directory.display()))? {
        let entry = entry.with_context(|| format!("read entry in {}", directory.display()))?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        let file_type = entry
            .file_type()
            .with_context(|| format!("read type {}", path.display()))?;
        if file_type.is_dir() {
            if entry.file_name().to_str() == Some(".synchub")
                || matches_rules(rules, &relative, true)
            {
                continue;
            }
            scan_directory(root, &path, remote_path, rules, previous, items)?;
            continue;
        }
        if !file_type.is_file()
            || (relative != IGNORE_FILE && matches_rules(rules, &relative, false))
        {
            continue;
        }
        let metadata = entry
            .metadata()
            .with_context(|| format!("read metadata {}", path.display()))?;
        let sha256 = file_sha256(&path)?;
        let size = metadata.len() as i64;
        let remote_version = previous
            .get(&relative)
            .and_then(|(old_size, old_hash, version)| {
                (*old_size == size && old_hash == &sha256)
                    .then_some(*version)
                    .flatten()
            });
        items.push(ManifestEntry {
            path: join_remote_path(remote_path, &relative),
            relative_path: relative,
            size,
            sha256,
            mtime: metadata
                .modified()
                .ok()
                .map(crate::app::time::rfc3339_from_system_time),
            remote_version,
        });
    }
    Ok(())
}

fn previous_versions(manifest: &Manifest) -> HashMap<String, (i64, String, Option<i64>)> {
    manifest
        .items
        .iter()
        .map(|item| {
            (
                item.relative_path.replace('\\', "/"),
                (item.size, item.sha256.clone(), item.remote_version),
            )
        })
        .collect()
}

fn load_ignore_rules(root: &Path) -> Result<Vec<IgnoreRule>> {
    let path = root.join(IGNORE_FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    Ok(raw
        .lines()
        .map(|line| line.trim_start_matches('\u{feff}').trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let normalized = line.replace('\\', "/");
            let pattern = normalized.trim_start_matches('/').trim_end_matches('/');
            (!pattern.is_empty()).then(|| IgnoreRule {
                pattern: pattern.to_string(),
                directory: normalized.ends_with('/'),
            })
        })
        .collect())
}

fn matches_rules(rules: &[IgnoreRule], relative: &str, directory: bool) -> bool {
    rules.iter().any(|rule| {
        if rule.directory && !directory {
            return false;
        }
        if rule.pattern.contains('/') {
            wildcard_match(&rule.pattern, relative)
        } else {
            relative
                .split('/')
                .any(|part| wildcard_match(&rule.pattern, part))
        }
    })
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    let (pattern, value) = (pattern.as_bytes(), value.as_bytes());
    let (mut p, mut v, mut star, mut retry) = (0, 0, None, 0);
    while v < value.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == value[v]) {
            p += 1;
            v += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            p += 1;
            retry = v;
        } else if let Some(index) = star {
            p = index + 1;
            retry += 1;
            v = retry;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

fn file_sha256(path: &Path) -> Result<String> {
    let mut reader =
        BufReader::new(File::open(path).with_context(|| format!("open {}", path.display()))?);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn normalize_remote_path(path: &str) -> String {
    let mut parts = Vec::new();
    for part in path
        .trim()
        .replace('\\', "/")
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
    {
        if part == ".." {
            parts.pop();
        } else {
            parts.push(part.to_string());
        }
    }
    format!("/{}", parts.join("/"))
}

fn join_remote_path(root: &str, relative: &str) -> String {
    if root == "/" {
        format!("/{relative}")
    } else {
        format!("{}/{relative}", root.trim_end_matches('/'))
    }
}

pub fn write_manifest(path: &Path, manifest: &Manifest) -> Result<()> {
    let parent = path.parent().context("manifest path has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let temporary = parent.join("manifest.json.tmp");
    let backup = parent.join("manifest.json.bak");
    let raw = serde_json::to_vec_pretty(manifest)?;
    fs::write(&temporary, [raw.as_slice(), b"\n"].concat())
        .with_context(|| format!("write {}", temporary.display()))?;
    let had_previous = path.exists();
    if had_previous {
        let _ = fs::remove_file(&backup);
        fs::rename(path, &backup).with_context(|| format!("backup {}", path.display()))?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if had_previous {
            let _ = fs::rename(&backup, path);
        }
        return Err(error).with_context(|| format!("replace {}", path.display()));
    }
    if had_previous {
        let _ = fs::remove_file(backup);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::WorkspaceRegistryEntry;

    #[test]
    fn wildcard_rules_match_names_and_paths() {
        assert!(wildcard_match("*.tmp", "draft.tmp"));
        assert!(wildcard_match("build/*", "build/output.bin"));
        assert!(!wildcard_match("build/*", "src/output.bin"));
    }

    #[test]
    fn scans_workspace_and_preserves_remote_versions() {
        let root =
            std::env::temp_dir().join(format!("synchub-native-manifest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("docs")).expect("create docs");
        fs::create_dir_all(root.join("build")).expect("create build");
        fs::create_dir_all(root.join(".synchub")).expect("create metadata");
        fs::write(root.join("docs").join("readme.txt"), "hello").expect("write tracked");
        fs::write(root.join("build").join("output.bin"), "ignored").expect("write ignored");
        fs::write(root.join("draft.tmp"), "ignored").expect("write temp");
        fs::write(root.join(".synchubignore"), "build/\n*.tmp\n").expect("write ignores");
        fs::write(root.join(".synchub").join("private"), "ignored").expect("write metadata");
        let hash = file_sha256(&root.join("docs").join("readme.txt")).expect("hash tracked");
        let workspace = WorkspaceSnapshot {
            entry: WorkspaceRegistryEntry {
                root: root.display().to_string(),
                remote_path: "/workspace".to_string(),
                ..WorkspaceRegistryEntry::default()
            },
            manifest: Some(Manifest {
                items: vec![ManifestEntry {
                    relative_path: "docs/readme.txt".to_string(),
                    size: 5,
                    sha256: hash,
                    remote_version: Some(7),
                    ..ManifestEntry::default()
                }],
                ..Manifest::default()
            }),
            ..WorkspaceSnapshot::default()
        };

        let manifest = scan_and_save_manifest(&workspace).expect("scan manifest");
        assert_eq!(
            manifest
                .items
                .iter()
                .map(|item| item.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec![".synchubignore", "docs/readme.txt"]
        );
        let tracked = manifest
            .items
            .iter()
            .find(|item| item.relative_path == "docs/readme.txt")
            .expect("tracked item");
        assert_eq!(tracked.path, "/workspace/docs/readme.txt");
        assert_eq!(tracked.remote_version, Some(7));
        let saved: Manifest = serde_json::from_str(
            &fs::read_to_string(root.join(".synchub").join("manifest.json"))
                .expect("read saved manifest"),
        )
        .expect("decode saved manifest");
        assert_eq!(saved, manifest);

        let rescanned = scan_and_save_manifest(&WorkspaceSnapshot {
            manifest: Some(manifest),
            ..workspace
        })
        .expect("replace existing manifest");
        assert_eq!(rescanned.items.len(), 2);
        assert!(!root.join(".synchub").join("manifest.json.bak").exists());

        fs::remove_dir_all(root).expect("remove temp workspace");
    }
}
