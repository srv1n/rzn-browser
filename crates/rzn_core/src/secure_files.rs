use crate::runtime_paths::default_app_base_dir;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub fn ensure_private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn write_secret_file(path: &Path, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        ensure_private_dir(parent)?;
    }

    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }

    let mut file = options.open(path)?;
    file.write_all(bytes.as_ref())?;
    file.flush()?;
    set_secret_file_permissions(path)?;
    Ok(())
}

pub fn append_secret_file_capped(
    path: &Path,
    bytes: impl AsRef<[u8]>,
    max_bytes: u64,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        ensure_private_dir(parent)?;
    }

    if max_bytes > 0 {
        let mut content = fs::read(path).unwrap_or_default();
        content.extend_from_slice(bytes.as_ref());
        let max_bytes = usize::try_from(max_bytes).unwrap_or(usize::MAX);
        if content.len() > max_bytes {
            let start = content.len().saturating_sub(max_bytes);
            content = content[start..].to_vec();
        }
        return write_secret_file(path, content);
    }

    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }

    let mut file = options.open(path)?;
    file.write_all(bytes.as_ref())?;
    file.flush()?;
    set_secret_file_permissions(path)?;
    Ok(())
}

pub fn set_secret_file_permissions(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

pub fn secure_dir(name: &str) -> io::Result<PathBuf> {
    let dir = default_app_base_dir().join("secure").join(name);
    ensure_private_dir(&dir)?;
    Ok(dir)
}

pub fn cleanup_secure_artifacts(
    dir: &Path,
    max_age: Duration,
    max_files: usize,
) -> io::Result<usize> {
    if !dir.exists() {
        return Ok(0);
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let metadata = match entry.metadata() {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => continue,
        };
        let modified = metadata.modified().ok();
        entries.push((entry.path(), modified));
    }

    let mut removed = 0usize;
    for (path, modified) in &entries {
        let expired = match modified.and_then(|time| time.elapsed().ok()) {
            Some(age) => age >= max_age,
            None => true,
        };
        if expired && fs::remove_file(path).is_ok() {
            removed += 1;
        }
    }

    if max_files > 0 {
        let mut remaining = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let metadata = match entry.metadata() {
                Ok(metadata) if metadata.is_file() => metadata,
                _ => continue,
            };
            remaining.push((entry.path(), metadata.modified().ok()));
        }
        remaining.sort_by(|left, right| right.1.cmp(&left.1));
        for (path, _) in remaining.into_iter().skip(max_files) {
            if fs::remove_file(path).is_ok() {
                removed += 1;
            }
        }
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock drift")
            .as_nanos();
        std::env::temp_dir().join(format!("rzn-secure-files-{name}-{unique}"))
    }

    #[test]
    fn write_secret_file_sets_private_modes() {
        let root = temp_root("write");
        let path = root.join("nested").join("secret.txt");
        write_secret_file(&path, b"secret\n").expect("write secret");

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "secret\n");
        #[cfg(unix)]
        {
            let dir_mode = std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            let file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(dir_mode, 0o700);
            assert_eq!(file_mode, 0o600);
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_secure_artifacts_removes_expired_files() {
        let root = temp_root("cleanup");
        ensure_private_dir(&root).expect("create private dir");
        write_secret_file(&root.join("artifact.json"), b"{}").expect("write artifact");

        let removed =
            cleanup_secure_artifacts(&root, Duration::ZERO, 50).expect("cleanup artifacts");

        assert_eq!(removed, 1);
        assert!(!root.join("artifact.json").exists());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn append_secret_file_capped_truncates_existing_file() {
        let root = temp_root("append");
        let path = root.join("debug.log");
        write_secret_file(&path, b"0123456789").expect("seed log");

        append_secret_file_capped(&path, b"new\n", 4).expect("append capped");

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn append_secret_file_capped_truncates_large_new_entry() {
        let root = temp_root("append-large");
        let path = root.join("debug.log");

        append_secret_file_capped(&path, b"0123456789", 4).expect("append capped");

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "6789");

        let _ = std::fs::remove_dir_all(root);
    }
}
