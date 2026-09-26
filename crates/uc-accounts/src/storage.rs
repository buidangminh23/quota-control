use std::fs::{File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path};

use uuid::Uuid;

use crate::{AccountError, Result};

pub fn lock(root: &Path) -> Result<File> {
    lock_file(root, &root.join("registry.lock"))
}

/// Block until this process holds the exclusive lock on `path`, a lock file inside `root`.
pub fn lock_file(root: &Path, path: &Path) -> Result<File> {
    ensure_directory(root)?;
    ensure_directory(&root.join("credentials"))?;
    reject_links(path)?;
    let file = options()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .map_err(|_| AccountError::Storage)?;
    verify_file(&file)?;
    file.lock().map_err(|_| AccountError::Storage)?;
    Ok(file)
}

pub fn read(path: &Path, max_bytes: u64) -> Result<Option<Vec<u8>>> {
    reject_links(path)?;
    let file = match options().read(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(AccountError::Storage),
    };
    verify_file(&file)?;
    let mut bytes = Vec::new();
    file.take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| AccountError::Storage)?;
    if bytes.len() as u64 > max_bytes {
        return Err(AccountError::Storage);
    }
    Ok(Some(bytes))
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    reject_links(path)?;
    let parent = path.parent().ok_or(AccountError::UnsafePath)?;
    let temp = parent.join(format!(".{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = options()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|_| AccountError::Storage)?;
        verify_file(&file)?;
        file.write_all(bytes).map_err(|_| AccountError::Storage)?;
        file.sync_all().map_err(|_| AccountError::Storage)?;
        drop(file);
        reject_links(path)?;
        std::fs::rename(&temp, path).map_err(|_| AccountError::Storage)?;
        #[cfg(unix)]
        let _ = File::open(parent).and_then(|directory| directory.sync_all());
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

pub fn remove(path: &Path) -> Result<()> {
    reject_links(path)?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(AccountError::Storage),
    }
}

fn ensure_directory(path: &Path) -> Result<()> {
    reject_links(path)?;
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|_| AccountError::Storage)?;
    reject_links(path)?;
    let metadata = std::fs::symlink_metadata(path).map_err(|_| AccountError::Storage)?;
    if !metadata.is_dir() {
        return Err(AccountError::UnsafePath);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(AccountError::UnsafePath);
        }
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| AccountError::Storage)?;
    }
    Ok(())
}

fn verify_file(file: &File) -> Result<()> {
    let metadata = file.metadata().map_err(|_| AccountError::Storage)?;
    if !metadata.is_file() || is_link(&metadata) {
        return Err(AccountError::UnsafePath);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.nlink() != 1 {
            return Err(AccountError::UnsafePath);
        }
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|_| AccountError::Storage)?;
    }
    Ok(())
}

fn options() -> OpenOptions {
    #[allow(unused_mut)]
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    options
}

fn reject_links(path: &Path) -> Result<()> {
    let mut prefix = std::path::PathBuf::new();
    for component in path.components() {
        if component == Component::ParentDir {
            return Err(AccountError::UnsafePath);
        }
        prefix.push(component);
        match std::fs::symlink_metadata(&prefix) {
            Ok(metadata) if is_link(&metadata) => return Err(AccountError::UnsafePath),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(AccountError::Storage),
        }
    }
    Ok(())
}

fn is_link(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
