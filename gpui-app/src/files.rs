//! Safe file IO for documents: versions to notice outside edits, and
//! atomic saves that never leave a half-written file behind.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What a file looked like on disk when we last read or wrote it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskVersion {
    modified: Option<SystemTime>,
    len: u64,
}

impl DiskVersion {
    /// The current version of `path`, or `None` if it does not exist.
    pub fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        Some(Self {
            modified: meta.modified().ok(),
            len: meta.len(),
        })
    }
}

/// Read a text document and the version it was read at.
pub fn read_text(path: &Path) -> io::Result<(String, Option<DiskVersion>)> {
    let version = DiskVersion::of(path);
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8(bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "文件不是 UTF-8 文本，已拒绝打开以免损坏内容",
        )
    })?;
    Ok((text, version))
}

/// Write `contents` to `path` atomically: write a temporary file in the
/// same directory, flush it to disk, then rename it over the target. A
/// failure at any step leaves the original file untouched and removes the
/// temporary file. Returns the new version of `path`.
pub fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<Option<DiskVersion>> {
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.permissions().readonly() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "文件是只读的",
            ));
        }
    }
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temp = temp_path(dir, path);
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        // On Windows this is MoveFileExW(MOVEFILE_REPLACE_EXISTING).
        std::fs::rename(&temp, path)
    })();
    if let Err(err) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(err);
    }
    Ok(DiskVersion::of(path))
}

fn temp_path(dir: &Path, target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document".into());
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    dir.join(format!(".{name}.{}-{nanos}.zj-tmp", std::process::id()))
}

/// A user-facing reason for an IO error.
pub fn describe(err: &io::Error) -> String {
    match err.kind() {
        io::ErrorKind::PermissionDenied => format!("没有写入权限（{err}）"),
        io::ErrorKind::NotFound => "文件夹不存在或已被移动".into(),
        io::ErrorKind::StorageFull => "磁盘空间不足".into(),
        _ => err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "zj-files-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".zj-tmp"))
            .collect()
    }

    #[test]
    fn writes_new_and_replaces_existing_files() {
        let dir = temp_dir("write");
        let path = dir.join("a.typ");
        let v1 = atomic_write(&path, "第一版".as_bytes()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "第一版");
        let v2 = atomic_write(&path, "第二版，更长一些".as_bytes()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "第二版，更长一些");
        assert_ne!(v1, v2);
        assert_eq!(DiskVersion::of(&path), v2);
        assert!(leftovers(&dir).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn readonly_files_are_left_untouched() {
        let dir = temp_dir("ro");
        let path = dir.join("locked.typ");
        std::fs::write(&path, "original").unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms.clone()).unwrap();

        let err = atomic_write(&path, b"changed").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        assert!(leftovers(&dir).is_empty());

        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(&path, perms).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_writes_clean_up_the_temporary_file() {
        let dir = temp_dir("fail");
        // Renaming onto an existing directory fails after the temp file was
        // written; the temp file must not be left behind.
        let target = dir.join("is-a-dir");
        std::fs::create_dir_all(&target).unwrap();
        assert!(atomic_write(&target, b"x").is_err());
        assert!(leftovers(&dir).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn versions_notice_outside_edits() {
        let dir = temp_dir("ver");
        let path = dir.join("b.typ");
        let saved = atomic_write(&path, b"short").unwrap();
        std::fs::write(&path, "edited by another program").unwrap();
        assert_ne!(DiskVersion::of(&path), saved);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(DiskVersion::of(&path), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn non_utf8_files_are_refused() {
        let dir = temp_dir("utf8");
        let path = dir.join("bin.typ");
        std::fs::write(&path, [0xff, 0xfe, 0x00, 0x80]).unwrap();
        assert_eq!(
            read_text(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
