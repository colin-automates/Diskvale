//! Experimental NTFS index reader. All volume handles and operations are read only.
mod format;
mod source;
#[cfg(test)]
mod tests;
mod tree;

use super::Progress;
use crate::model::Scan;
use std::{io::Read, path::Path, sync::atomic::Ordering, time::Instant};

const CHUNK_BYTES: usize = 8 * 1024 * 1024;

fn read_index(
    mut reader: impl Read,
    root: &Path,
    geometry: source::Geometry,
    progress: &Progress,
    engine: &str,
) -> Result<Scan, String> {
    geometry.validate()?;
    let started = Instant::now();
    progress
        .metadata_total
        .store(geometry.valid, Ordering::Relaxed);
    progress.phase.store(1, Ordering::Relaxed);
    let slots = usize::try_from(geometry.valid / geometry.record_size as u64)
        .map_err(|_| "NTFS index is too large")?;
    // Four bytes per slot; file paths and per-file hash tables are unnecessary.
    let mut positions = vec![u32::MAX; slots];
    let mut records = Vec::new();
    let mut extensions = Vec::new();
    let mut buffer = vec![0; CHUNK_BYTES / geometry.record_size * geometry.record_size];
    let mut consumed = 0u64;
    let mut index = 0usize;
    while consumed < geometry.valid && !progress.cancelled() {
        let len = (geometry.valid - consumed).min(buffer.len() as u64) as usize;
        reader
            .read_exact(&mut buffer[..len])
            .map_err(|e| format!("NTFS index read failed: {e}"))?;
        for raw in buffer[..len].chunks_exact_mut(geometry.record_size) {
            if progress.cancelled() {
                break;
            }
            if raw.get(..4) == Some(b"FILE") {
                if format::u16_at(raw, 22).is_some_and(|flags| flags & 1 != 0) {
                    format::fixup(raw, geometry.sector, false)
                        .map_err(|e| format!("NTFS record {index}: {e}"))?;
                    if let Some(record) =
                        format::parse(raw, index as u64, geometry.cluster, geometry.total_clusters)
                            .map_err(|e| format!("NTFS record {index}: {e}"))?
                    {
                        if record.base != 0 {
                            extensions.push(record);
                        } else {
                            positions[index] = u32::try_from(records.len())
                                .map_err(|_| "Too many NTFS records")?;
                            if record.directory {
                                progress.folders.fetch_add(1, Ordering::Relaxed);
                            } else {
                                progress.files.fetch_add(1, Ordering::Relaxed);
                            }
                            records.push(record);
                        }
                    }
                }
            } else if raw.iter().any(|&b| b != 0) {
                return Err(format!("Unrecognized NTFS record at index {index}"));
            }
            index += 1;
        }
        consumed += len as u64;
        progress.metadata_bytes.store(consumed, Ordering::Relaxed);
        progress.mft_records.store(index as u64, Ordering::Relaxed);
    }
    let read_seconds = started.elapsed().as_secs_f64();
    progress.phase.store(2, Ordering::Relaxed);
    let extension_owners: std::collections::HashMap<_, _> =
        extensions.iter().map(|r| (r.reference, r.base)).collect();
    for record in &records {
        for entry in &record.required {
            let reference = entry.reference;
            if !progress.cancelled()
                && reference != record.reference
                && extension_owners.get(&reference) != Some(&record.reference)
            {
                return Err("Missing or unrelated NTFS attribute-list record".into());
            }
        }
    }
    for extension in extensions {
        let position = positions
            .get(format::slot(extension.base))
            .copied()
            .filter(|&p| p != u32::MAX);
        if let Some(base) = position.and_then(|p| records.get_mut(p as usize)) {
            if base.reference != extension.base {
                if progress.cancelled() {
                    continue;
                }
                return Err(
                    "NTFS extension refers to a reused record; rescan the live volume".into(),
                );
            }
            base.merge(extension)?;
        } else if !progress.cancelled() {
            return Err("Missing NTFS base record for an extension".into());
        }
    }
    let build_started = Instant::now();
    let mut scan = tree::build(root, records, progress, engine)?;
    scan.stats.metadata_bytes = consumed;
    scan.stats.mft_records = index as u64;
    scan.stats.mft_read_seconds = read_seconds;
    scan.stats.tree_build_seconds = build_started.elapsed().as_secs_f64();
    scan.stats.elapsed_seconds = started.elapsed().as_secs_f64();
    progress.files.store(scan.stats.files, Ordering::Relaxed);
    progress
        .folders
        .store(scan.stats.folders, Ordering::Relaxed);
    progress
        .bytes
        .store(scan.nodes[0].allocated, Ordering::Relaxed);
    progress.phase.store(0, Ordering::Relaxed);
    Ok(scan)
}

/// Replay a flat saved MFT through the same bulk parser, using an ordinary file.
pub fn scan_file(root: &Path, file: &Path, progress: &Progress) -> Result<Scan, String> {
    use std::io::{Seek, SeekFrom};
    let mut file = std::fs::File::open(file).map_err(|e| format!("Cannot open saved MFT: {e}"))?;
    let geometry = source::saved_geometry(&mut file)?;
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut scan = read_index(file, root, geometry, progress, "Saved NTFS MFT (read only)")?;
    scan.stats.runtime = crate::model::RuntimeState::new(if scan.stats.cancelled {
        crate::model::Provenance::Incomplete
    } else {
        crate::model::Provenance::Replay
    });
    scan.stats.notices.insert(
        0,
        "Replayed a saved NTFS index; this is not a live scan of the displayed root.".into(),
    );
    Ok(scan)
}

#[cfg(windows)]
pub fn scan_volume(root: &Path, progress: &Progress) -> Result<Scan, String> {
    let started = Instant::now();
    let root = super::windows::display_path(root.to_path_buf());
    if progress.cancelled() {
        return tree::build(&root, vec![], progress, "Experimental NTFS MFT");
    }
    let mut volume = super::windows::volume_file(&root).map_err(volume_access_error)?;
    let root_handle = super::windows::identity_handle(&root, true)
        .map_err(|e| format!("Cannot open NTFS root identity: {e}"))?;
    let volume_serial = super::windows::handle_identity(&root_handle)
        .map_err(|e| format!("Cannot verify NTFS volume identity: {e}"))?
        .0;
    let (geometry, mapping) = source::volume_mapping(&mut volume)?;
    let reader = source::MappedReader::new(&mut volume, mapping, geometry.cluster, geometry.valid)?;
    let mut scan = read_index(reader, &root, geometry, progress, "Experimental NTFS MFT")?;
    scan.stats.volume_serial = Some(volume_serial);
    scan.stats.runtime = crate::model::RuntimeState::new(if scan.stats.cancelled {
        crate::model::Provenance::Incomplete
    } else {
        crate::model::Provenance::Live
    });
    scan.stats.elapsed_seconds = started.elapsed().as_secs_f64();
    Ok(scan)
}

#[cfg(windows)]
fn volume_access_error(error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        format!(
            "Windows denied access to the NTFS index ({error}). Live MFT scans require administrator access. Close Diskvale and start it with Run as administrator to scan."
        )
    } else {
        format!("Read-only NTFS volume access failed ({error})")
    }
}
