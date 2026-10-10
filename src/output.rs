//! Same-directory staged output. The rename is the commit point; cancellation
//! after that point cannot undo it. Syncing file contents is not a promise of
//! crash/power-loss durability for directory entries on every Windows filesystem.
use crate::{model::Scan, scanner::Progress};
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Stage {
    path: PathBuf,
}
impl Drop for Stage {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn cancelled(progress: &Progress) -> Result<(), String> {
    if progress.cancelled() {
        Err("Output cancelled before replacement.".into())
    } else {
        Ok(())
    }
}

fn replace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use crate::scanner::windows::{extended_path, wide};
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let from = wide(&extended_path(from));
        let to = wide(&extended_path(to));
        // Same-volume rename only. Never remove the old file or enable copy/delete fallback.
        if unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(from, to)
    }
}

fn staged(
    destination: &Path,
    progress: &Progress,
    write: impl FnOnce(&mut File) -> Result<(), String>,
    finish: impl FnOnce(&mut File) -> io::Result<()>,
    commit: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<(), String> {
    cancelled(progress)?;
    let destination = std::path::absolute(destination).map_err(|e| e.to_string())?;
    let parent = destination
        .parent()
        .ok_or("Output needs a parent directory")?;
    let (stage, mut file) = (0..128)
        .find_map(|_| {
            let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path = parent.join(format!(
                ".diskvale-{}-{stamp}-{sequence}.tmp",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => Some(Ok((Stage { path }, file))),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(format!("Creating staged output: {e}"))),
            }
        })
        .ok_or("Unable to create a unique staged output")??;
    write(&mut file)?;
    cancelled(progress)?;
    finish(&mut file).map_err(|e| format!("Flushing/syncing staged output: {e}"))?;
    drop(file);
    cancelled(progress)?;
    commit(&stage.path, &destination)
        .map_err(|e| format!("Replacing {}: {e}", destination.display()))?;
    Ok(())
}

struct CancellableWriter<'a, W> {
    inner: W,
    progress: &'a Progress,
}
impl<W: Write> Write for CancellableWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.progress.cancelled() {
            return Err(io::Error::other("Output cancelled"));
        }
        let n = self.inner.write(bytes)?;
        self.progress
            .metadata_bytes
            .fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

pub fn snapshot(scan: &Scan, path: &Path, progress: &Progress) -> Result<(), String> {
    #[derive(serde::Serialize)]
    struct Snapshot<'a> {
        root: &'a Path,
        nodes: &'a [crate::model::Node],
        stats: crate::model::ScanStats,
    }
    let mut stats = scan.stats.clone();
    stats.schema_version = 2;
    let snapshot = Snapshot {
        root: &scan.root,
        nodes: &scan.nodes,
        stats,
    };
    staged(
        path,
        progress,
        |file| {
            let mut writer = std::io::BufWriter::new(CancellableWriter {
                inner: file,
                progress,
            });
            serde_json::to_writer(&mut writer, &snapshot)
                .map_err(|e| format!("Serializing snapshot: {e}"))?;
            writer
                .flush()
                .map_err(|e| format!("Flushing snapshot: {e}"))
        },
        |f| {
            f.flush()?;
            f.sync_all()
        },
        replace,
    )
}

pub fn csv(scan: &Scan, path: &Path, progress: &Progress) -> Result<(), String> {
    staged(
        path,
        progress,
        |file| {
            let mut writer = csv::Writer::from_writer(CancellableWriter {
                inner: file,
                progress,
            });
            writer
                .write_record([
                    "path",
                    "directory",
                    "logical_bytes",
                    "allocated_bytes",
                    "shared_link",
                    "allocation_unknown",
                    "files",
                    "modified_unix_seconds",
                    "directory_stream_bucket",
                    "own_logical_bytes",
                    "own_allocated_bytes",
                    "named_logical_bytes",
                ])
                .map_err(|e| e.to_string())?;
            for (id, node) in scan.nodes.iter().enumerate() {
                cancelled(progress)?;
                writer
                    .write_record([
                        scan.path(id).display().to_string(),
                        node.directory.to_string(),
                        node.logical.to_string(),
                        node.allocated.to_string(),
                        node.shared.to_string(),
                        node.allocation_unknown.to_string(),
                        node.files.to_string(),
                        node.modified.to_string(),
                        node.synthetic.to_string(),
                        node.own_logical.to_string(),
                        node.own_allocated.to_string(),
                        node.named_logical.to_string(),
                    ])
                    .map_err(|e| format!("Writing CSV row {id}: {e}"))?;
                progress.files.store(id as u64 + 1, Ordering::Relaxed);
            }
            writer.flush().map_err(|e| format!("Flushing CSV: {e}"))
        },
        |f| {
            f.flush()?;
            f.sync_all()
        },
        replace,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_survives_write_flush_cancel_and_replacement_failure() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("東京 café.json");
        for failure in 0..4 {
            std::fs::write(&path, b"previous valid output").unwrap();
            let progress = Progress::default();
            let result = staged(
                &path,
                &progress,
                |file| {
                    file.write_all(b"incomplete new output").unwrap();
                    if failure == 0 {
                        return Err("injected mid-write failure".into());
                    }
                    if failure == 2 {
                        progress.cancel.store(true, Ordering::Relaxed);
                    }
                    Ok(())
                },
                |file| {
                    if failure == 1 {
                        Err(io::Error::other("injected flush failure"))
                    } else {
                        file.sync_all()
                    }
                },
                |from, to| {
                    if failure == 3 {
                        Err(io::Error::other("injected replace failure"))
                    } else {
                        replace(from, to)
                    }
                },
            );
            assert!(result.is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"previous valid output");
            assert_eq!(std::fs::read_dir(fixture.path()).unwrap().count(), 1);
        }
    }
    #[test]
    fn output_creates_and_overwrites_unicode_destination() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("東京 export.csv");
        for bytes in [b"first".as_slice(), b"second".as_slice()] {
            staged(
                &path,
                &Progress::default(),
                |file| file.write_all(bytes).map_err(|e| e.to_string()),
                |file| file.sync_all(),
                replace,
            )
            .unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            assert_eq!(std::fs::read_dir(fixture.path()).unwrap().count(), 1);
        }
    }
    #[cfg(windows)]
    #[test]
    fn windows_locked_destination_preserves_old_output_and_cleans_stage() {
        use std::os::windows::fs::OpenOptionsExt;
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("locked-export.csv");
        std::fs::write(&path, b"previous valid output").unwrap();
        let _guard = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let result = staged(
            &path,
            &Progress::default(),
            |file| file.write_all(b"new output").map_err(|e| e.to_string()),
            |file| file.sync_all(),
            replace,
        );
        assert!(result.unwrap_err().contains("Replacing"));
        drop(_guard);
        assert_eq!(std::fs::read(&path).unwrap(), b"previous valid output");
        assert_eq!(std::fs::read_dir(fixture.path()).unwrap().count(), 1);
    }
}
