pub mod mft;
#[cfg(windows)]
mod native;
mod walk;
#[cfg(windows)]
pub mod windows;

use crate::model::Scan;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

#[derive(Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub folders: AtomicU64,
    pub bytes: AtomicU64,
    pub cancel: AtomicBool,
    pub metadata_bytes: AtomicU64,
    pub metadata_total: AtomicU64,
    pub mft_records: AtomicU64,
    /// 0: folder scan/idle, 1: bulk index read, 2: tree construction, 3: snapshot load.
    pub phase: AtomicU64,
}

impl Progress {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Legacy folder scanner retained for comparison and filesystem regressions.
/// The desktop and command-line application use `scan_mft_only` exclusively.
pub fn scan(root: PathBuf, progress: Arc<Progress>, try_mft: bool) -> Result<Scan, String> {
    let start = Instant::now();
    let root = std::fs::canonicalize(&root)
        .map_err(|e| format!("Cannot access {}: {e}", root.display()))?;
    if !root.is_dir() {
        return Err("Choose a drive or folder to scan.".into());
    }
    let mut fallback_reason = None;
    #[cfg(windows)]
    if try_mft && windows::is_volume_root(&root) {
        match mft::scan_volume(&root, &progress) {
            Ok(mut result) => {
                result.stats.elapsed_seconds = start.elapsed().as_secs_f64();
                return Ok(result);
            }
            Err(reason) => {
                if progress.cancelled() {
                    return Err("Scan cancelled.".into());
                }
                fallback_reason = Some(reason);
                progress.files.store(0, Ordering::Relaxed);
                progress.folders.store(0, Ordering::Relaxed);
                progress.bytes.store(0, Ordering::Relaxed);
                progress.phase.store(0, Ordering::Relaxed);
            }
        }
    }
    #[cfg(not(windows))]
    let _ = try_mft;
    #[cfg(windows)]
    let mut result = match native::scan_folder(root.clone(), progress.clone()) {
        Ok(result) => result,
        Err(reason) => {
            progress.files.store(0, Ordering::Relaxed);
            progress.folders.store(0, Ordering::Relaxed);
            progress.bytes.store(0, Ordering::Relaxed);
            let mut result = walk::scan_folder(root, progress)?;
            result.stats.notices.insert(
                0,
                format!(
                    "Batched directory reads unavailable: {reason}. Used compatible folder scanner."
                ),
            );
            result
        }
    };
    #[cfg(not(windows))]
    let mut result = walk::scan_folder(root, progress)?;
    result.stats.elapsed_seconds = start.elapsed().as_secs_f64();
    if let Some(reason) = fallback_reason {
        result.stats.notices.insert(
            0,
            format!("MFT unavailable: {reason}. Used folder scanner."),
        );
    }
    Ok(result)
}

/// Whole-volume MFT scan. Errors are returned without a folder fallback.
pub fn scan_mft_only(root: PathBuf, progress: Arc<Progress>) -> Result<Scan, String> {
    #[cfg(windows)]
    {
        let root = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
        if !windows::is_volume_root(&root) {
            return Err("MFT scanning requires a whole NTFS drive.".into());
        }
        let before = windows::volume_space(&root);
        let mut scan = mft::scan_volume(&root, &progress)?;
        if let (Some((start_capacity, free_at_start)), Some((capacity, free))) =
            (before, windows::volume_space(&root))
            && capacity == start_capacity
        {
            scan.stats.volume_space = Some(crate::model::VolumeSpace {
                capacity,
                free,
                free_at_start,
                captured_unix: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            });
        }
        Ok(scan)
    }
    #[cfg(not(windows))]
    {
        let _ = (root, progress);
        Err("Live NTFS scanning requires Windows.".into())
    }
}
