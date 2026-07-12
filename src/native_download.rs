use anyhow::{Context, Result, bail};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub fn local_path_for_remote(root: &Path, remote_root: &str, remote_path: &str) -> Result<PathBuf> {
    let remote_root = clean_remote_path(remote_root)?;
    let remote_path = clean_remote_path(remote_path)?;
    let relative = if remote_root.as_os_str().is_empty() {
        remote_path
    } else {
        remote_path
            .strip_prefix(&remote_root)
            .context("remote file is outside workspace remote path")?
            .to_path_buf()
    };
    if relative.as_os_str().is_empty() {
        bail!("remote file maps to the workspace root");
    }
    if relative.starts_with(".synchub") {
        bail!("remote file maps to protected workspace metadata");
    }
    Ok(root.join(relative))
}

pub fn write_downloaded_file(path: &Path, content: &[u8]) -> Result<u64> {
    let parent = path
        .parent()
        .context("download target has no parent directory")?;
    fs::create_dir_all(parent).context("create download target directory")?;
    if path.is_dir() {
        bail!("download target is a directory: {}", path.display());
    }
    let mut temporary = tempfile::Builder::new()
        .prefix(".synchub-download-")
        .tempfile_in(parent)
        .context("create temporary download file")?;
    temporary
        .write_all(content)
        .context("write downloaded file")?;
    temporary.flush().context("flush downloaded file")?;
    if path.exists() {
        fs::remove_file(path).context("replace existing download target")?;
    }
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .context("install downloaded file")?;
    Ok(content.len() as u64)
}

fn clean_remote_path(value: &str) -> Result<PathBuf> {
    let normalized = value.trim().trim_matches('/').replace('\\', "/");
    let mut path = PathBuf::new();
    for component in Path::new(&normalized).components() {
        match component {
            Component::Normal(part) => path.push(part),
            _ => bail!("remote path is invalid"),
        }
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_only_remote_files_inside_workspace() {
        assert_eq!(
            local_path_for_remote(Path::new("C:/work"), "/project", "/project/docs/a.txt").unwrap(),
            Path::new("C:/work").join("docs/a.txt")
        );
        assert!(local_path_for_remote(Path::new("C:/work"), "/project", "/other/a.txt").is_err());
        assert!(
            local_path_for_remote(Path::new("C:/work"), "/project", "/project/.synchub/a").is_err()
        );
        assert!(local_path_for_remote(Path::new("C:/work"), "/project", "/project/../a").is_err());
    }

    #[test]
    fn writes_and_replaces_download_target() {
        let root = std::env::temp_dir().join(format!(
            "synchub-desktop-download-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        let target = root.join("docs/a.txt");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, b"old").unwrap();
        assert_eq!(write_downloaded_file(&target, b"new").unwrap(), 3);
        assert_eq!(fs::read(&target).unwrap(), b"new");
        fs::remove_dir_all(root).unwrap();
    }
}
