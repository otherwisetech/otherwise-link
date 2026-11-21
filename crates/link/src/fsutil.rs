//! Atomic file replacement: readers in other processes see the old file or the new one, never
//! half of either.
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Writes `bytes` to a temp file next to `path` (same folder, so the rename can't cross file
/// systems), flushes it to disk and renames it over `path`. Rename replaces atomically on Unix and
/// on Windows (`MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`). Temp names start with `.` and end
/// with `.tmp`, so readers skip them.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let res = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        rename_replacing(&tmp, path)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

/// Windows refuses to replace a file another process has open for a moment (antivirus, a reader):
/// retry briefly instead of failing the write.
fn rename_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut last = None;
    for i in 0..20 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if cfg!(windows) && e.kind() == std::io::ErrorKind::PermissionDenied => {
                last = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(5 * (i + 1)));
            }
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("rename failed")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_and_leaves_no_temp() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub").join("a.json");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        let names: Vec<_> = std::fs::read_dir(p.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1, "{names:?}");
    }
}
