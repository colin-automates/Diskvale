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
/// The desktop and CLI choose their volume engine through `scan_drive`.
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
        scan.stats.filesystem = Some("NTFS".into());
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveScanMethod {
    Ntfs,
    Directory,
}

impl DriveScanMethod {
    pub fn for_filesystem(filesystem: &str) -> Option<Self> {
        if filesystem.eq_ignore_ascii_case("NTFS") {
            Some(Self::Ntfs)
        } else if ["exFAT", "FAT32", "FAT"]
            .iter()
            .any(|fs| filesystem.eq_ignore_ascii_case(fs))
        {
            Some(Self::Directory)
        } else {
            None
        }
    }
}

fn dispatch_drive_scan<T>(
    method: DriveScanMethod,
    ntfs: impl FnOnce() -> Result<T, String>,
    directory: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    match method {
        DriveScanMethod::Ntfs => ntfs(),
        DriveScanMethod::Directory => directory(),
    }
}

#[cfg(windows)]
fn scan_directory_volume(
    root: PathBuf,
    progress: Arc<Progress>,
    filesystem: &str,
) -> Result<Scan, String> {
    let start = Instant::now();
    let mut scan = walk::scan_fat_volume(root, progress)?;
    scan.stats.engine = format!("{filesystem} directory scan");
    scan.stats.filesystem = Some(filesystem.to_owned());
    scan.stats.schema_version = 2;
    scan.stats.elapsed_seconds = start.elapsed().as_secs_f64();
    scan.stats.runtime =
        crate::model::RuntimeState::new(if scan.stats.cancelled || scan.stats.errors != 0 {
            crate::model::Provenance::Incomplete
        } else {
            crate::model::Provenance::Directory
        });
    scan.stats.notices.push(format!("{filesystem} scanned through file and folder metadata, without reading file contents. Directory allocation and filesystem metadata are excluded. This is a live view, not an atomic snapshot. Deletion is disabled for this scan."));
    if scan.stats.cancelled || scan.stats.errors != 0 {
        scan.stats.notices.insert(0, "This directory scan is incomplete; its totals cover only the entries read successfully.".into());
    }
    Ok(scan)
}

/// Whole local drives only. Choose by the detected filesystem; an NTFS error
/// never falls back to a directory scan or produces complete-looking results.
pub fn scan_drive(root: PathBuf, progress: Arc<Progress>) -> Result<Scan, String> {
    #[cfg(windows)]
    {
        let root = std::fs::canonicalize(root).map_err(|e| format!("Cannot access drive: {e}"))?;
        let before = windows::drive(&root).map_err(|e| e.to_string())?;
        let method = DriveScanMethod::for_filesystem(&before.filesystem).ok_or_else(|| {
            format!(
                "Unsupported filesystem {}. Choose NTFS, exFAT or FAT32.",
                before.filesystem
            )
        })?;
        let mut scan = dispatch_drive_scan(
            method,
            || scan_mft_only(root.clone(), progress.clone()),
            || scan_directory_volume(root.clone(), progress.clone(), &before.filesystem),
        )?;
        let after = windows::drive(&root).map_err(|e| {
            format!("Drive became unavailable during the scan: {e}. Reconnect it and rescan.")
        })?;
        if before.serial != after.serial
            || before.filesystem != after.filesystem
            || before.total != after.total
        {
            return Err("The drive changed during scanning. Rescan the connected drive.".into());
        }
        scan.stats.volume_space = Some(crate::model::VolumeSpace {
            capacity: after.total,
            free: after.free,
            free_at_start: before.free,
            captured_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        });
        Ok(scan)
    }
    #[cfg(not(windows))]
    {
        let _ = (root, progress);
        Err("Live drive scanning requires Windows.".into())
    }
}

#[cfg(test)]
mod drive_tests {
    use super::*;

    #[test]
    fn ntfs_failure_never_runs_the_directory_scanner() {
        let error = dispatch_drive_scan::<()>(
            DriveScanMethod::Ntfs,
            || Err("Invalid NTFS metadata".into()),
            || panic!("NTFS failures must not fall back to a folder scan"),
        )
        .unwrap_err();
        assert_eq!(error, "Invalid NTFS metadata");
    }

    #[test]
    fn exfat_and_fat_routes_do_not_attempt_to_read_an_mft() {
        for filesystem in ["exFAT", "EXFAT", "FAT32", "FAT"] {
            let result = dispatch_drive_scan(
                DriveScanMethod::for_filesystem(filesystem).unwrap(),
                || panic!("A FAT volume has no NTFS MFT"),
                || Ok(42),
            )
            .unwrap();
            assert_eq!(result, 42);
        }
        for filesystem in ["", "ReFS", "CDFS"] {
            assert_eq!(DriveScanMethod::for_filesystem(filesystem), None);
        }
    }

    #[cfg(windows)]
    #[test]
    fn directory_volume_totals_and_cancellation_never_authorize_deletion() {
        use crate::model::Provenance;
        let fixture = tempfile::tempdir().unwrap();
        std::fs::create_dir(fixture.path().join("nested")).unwrap();
        std::fs::write(fixture.path().join("a.bin"), vec![1; 8192]).unwrap();
        std::fs::write(fixture.path().join("nested/b.bin"), vec![2; 16384]).unwrap();
        let scan = scan_directory_volume(fixture.path().into(), Arc::default(), "exFAT").unwrap();
        assert_eq!(scan.stats.files, 2);
        assert_eq!(scan.stats.folders, 1);
        assert_eq!(scan.nodes[0].logical, 24576);
        assert_eq!(scan.nodes[0].allocated, 24576);
        assert_eq!(scan.stats.errors, 0);
        assert_eq!(scan.stats.unknown_allocations, 0);
        assert_eq!(scan.stats.runtime.provenance(), Provenance::Directory);
        assert!(!scan.deletion_eligible());
        let progress = Arc::new(Progress::default());
        progress.cancel.store(true, Ordering::Relaxed);
        let partial = scan_directory_volume(fixture.path().into(), progress, "FAT32").unwrap();
        assert!(partial.stats.cancelled);
        assert_eq!(partial.stats.runtime.provenance(), Provenance::Incomplete);
        assert!(!partial.deletion_eligible());
    }
}
