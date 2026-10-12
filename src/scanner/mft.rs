//! Experimental NTFS index reader. All volume handles and operations are read only.
mod format;
mod source;
#[cfg(test)]
mod tests;
mod tree;

use super::Progress;
use crate::model::{MftTimings, Scan};
use std::{io::Read, path::Path, sync::atomic::Ordering, time::Instant};

const CHUNK_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug)]
enum IndexError {
    ParentReference(String),
    Invalid(String),
}
impl From<String> for IndexError {
    fn from(message: String) -> Self {
        Self::Invalid(message)
    }
}
impl From<&str> for IndexError {
    fn from(message: &str) -> Self {
        Self::Invalid(message.into())
    }
}
impl std::fmt::Display for IndexError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ParentReference(message) | Self::Invalid(message) => message.fmt(formatter),
        }
    }
}

fn read_index(
    reader: impl Read,
    root: &Path,
    geometry: source::Geometry,
    progress: &Progress,
    engine: &str,
) -> Result<Scan, String> {
    read_index_resolved(source::FlatIndex(reader), root, geometry, progress, engine)
        .map_err(|error| error.to_string())
}

fn read_index_resolved(
    reader: impl source::IndexSource,
    root: &Path,
    geometry: source::Geometry,
    progress: &Progress,
    engine: &str,
) -> Result<Scan, IndexError> {
    let mut timings = MftTimings {
        index_attempts: 1,
        ..Default::default()
    };
    read_index_timed(reader, root, geometry, progress, engine, &mut timings)
}

fn read_index_timed(
    mut reader: impl source::IndexSource,
    root: &Path,
    geometry: source::Geometry,
    progress: &Progress,
    engine: &str,
    timings: &mut MftTimings,
) -> Result<Scan, IndexError> {
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
    let mut records = Vec::with_capacity(slots.min(8192));
    let mut extensions = Vec::new();
    let mut buffer = vec![0; CHUNK_BYTES / geometry.record_size * geometry.record_size];
    let mut consumed = 0u64;
    let mut index = 0usize;
    while consumed < geometry.valid && !progress.cancelled() {
        let len = (geometry.valid - consumed).min(buffer.len() as u64) as usize;
        let io_started = Instant::now();
        let read = reader.read_exact(&mut buffer[..len]);
        timings.index_io_seconds += io_started.elapsed().as_secs_f64();
        read.map_err(|e| format!("NTFS index read failed: {e}"))?;
        let parse_started = Instant::now();
        let lists_before = timings.external_list_seconds;
        let mut files = 0;
        let mut folders = 0;
        let parsed_chunk = (|| -> Result<(), IndexError> {
            for raw in buffer[..len].chunks_exact_mut(geometry.record_size) {
                if progress.cancelled() {
                    break;
                }
                if raw.get(..4) == Some(b"FILE") {
                    if format::u16_at(raw, 22).is_some_and(|flags| flags & 1 != 0) {
                        format::fixup(raw, geometry.sector, false)
                            .map_err(|e| format!("NTFS record {index}: {e}"))?;
                        reader
                            .validate_record(index, raw)
                            .map_err(|e| format!("NTFS record {index}: {e}"))?;
                        let parsed = format::parse_with_lists(
                            raw,
                            index as u64,
                            geometry.cluster,
                            geometry.total_clusters,
                            |attr| {
                                let started = Instant::now();
                                let resolved = reader.resolve_list(attr, geometry, progress);
                                timings.external_list_seconds += started.elapsed().as_secs_f64();
                                timings.external_lists += 1;
                                resolved
                            },
                        );
                        let parsed = match parsed {
                            Err(_) if progress.cancelled() => break,
                            result => result.map_err(|e| format!("NTFS record {index}: {e}"))?,
                        };
                        if let Some(record) = parsed {
                            reader
                                .validate_parsed(&record)
                                .map_err(|e| format!("NTFS record {index}: {e}"))?;
                            if record.base != 0 {
                                extensions.push(record);
                            } else {
                                positions[index] = u32::try_from(records.len())
                                    .map_err(|_| "Too many NTFS records")?;
                                if record.directory {
                                    folders += 1;
                                } else {
                                    files += 1;
                                }
                                records.push(record);
                            }
                        }
                    } else {
                        reader.validate_record(index, raw)?;
                    }
                } else {
                    reader.validate_record(index, raw)?;
                    if raw.iter().any(|&b| b != 0) {
                        return Err(format!("Unrecognized NTFS record at index {index}").into());
                    }
                }
                index += 1;
            }
            Ok(())
        })();
        timings.parse_seconds += (parse_started.elapsed().as_secs_f64()
            - (timings.external_list_seconds - lists_before))
            .max(0.0);
        progress.files.fetch_add(files, Ordering::Relaxed);
        progress.folders.fetch_add(folders, Ordering::Relaxed);
        parsed_chunk?;
        consumed += len as u64;
        progress.metadata_bytes.store(consumed, Ordering::Relaxed);
        progress.mft_records.store(index as u64, Ordering::Relaxed);
    }
    if !progress.cancelled() {
        reader.finish_validation()?;
    }
    let read_seconds = started.elapsed().as_secs_f64();
    progress.phase.store(2, Ordering::Relaxed);
    let merge_started = Instant::now();
    let merged = (|| -> Result<(), IndexError> {
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
        Ok(())
    })();
    timings.extension_merge_seconds += merge_started.elapsed().as_secs_f64();
    merged?;
    let build_started = Instant::now();
    let result = tree::build(root, records, progress, engine);
    let tree_seconds = build_started.elapsed().as_secs_f64();
    timings.tree_seconds += tree_seconds;
    let mut scan = result?;
    scan.stats.metadata_bytes = consumed;
    scan.stats.mft_records = index as u64;
    scan.stats.mft_read_seconds = read_seconds;
    scan.stats.tree_build_seconds = tree_seconds;
    scan.stats.mft_timings = Some(timings.clone());
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

// A changing live namespace can span multiple sequential chunks. Retry the
// entire validated index once; never repair references or attach orphan nodes.
// Saved-MFT replay deliberately does not enter this live-only path.
#[cfg(any(windows, test))]
fn scan_live_with_retry(
    root: &Path,
    progress: &Progress,
    mut attempt: impl FnMut(&mut MftTimings) -> Result<Scan, IndexError>,
) -> Result<Scan, String> {
    let mut timings = MftTimings::default();
    for number in 0..2 {
        if progress.cancelled() {
            return tree::build(root, vec![], progress, "Experimental NTFS MFT")
                .map_err(|error| error.to_string());
        }
        timings.index_attempts += 1;
        match attempt(&mut timings) {
            Ok(mut scan) => {
                scan.stats.mft_timings = Some(timings);
                if number != 0 && !scan.stats.cancelled {
                    scan.stats.notices.insert(0, "Repeated the live NTFS index read once after inconsistent parent references. The second read passed reference validation; this is not an atomic filesystem snapshot.".into());
                }
                return Ok(scan);
            }
            Err(_) if progress.cancelled() => {
                return tree::build(root, vec![], progress, "Experimental NTFS MFT")
                    .map_err(|error| error.to_string());
            }
            Err(IndexError::ParentReference(_)) if number == 0 => {
                // Reset only progress for the discarded attempt, never cancellation.
                for counter in [
                    &progress.files,
                    &progress.folders,
                    &progress.bytes,
                    &progress.metadata_bytes,
                    &progress.metadata_total,
                    &progress.mft_records,
                ] {
                    counter.store(0, Ordering::Relaxed);
                }
                progress.phase.store(1, Ordering::Relaxed);
            }
            Err(error) => {
                return Err(if number == 0 {
                    error.to_string()
                } else {
                    format!("{error}. The fresh-index retry also failed")
                });
            }
        }
    }
    unreachable!("Both bounded attempts return or the first advances to the second")
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
        return tree::build(&root, vec![], progress, "Experimental NTFS MFT")
            .map_err(|error| error.to_string());
    }
    let mut volume = super::windows::volume_file(&root).map_err(volume_access_error)?;
    let root_handle = super::windows::identity_handle(&root, true)
        .map_err(|e| format!("Cannot open NTFS root identity: {e}"))?;
    let volume_serial = super::windows::handle_identity(&root_handle)
        .map_err(|e| format!("Cannot verify NTFS volume identity: {e}"))?
        .0;
    let mut scan = scan_live_with_retry(&root, progress, |timings| {
        let started = Instant::now();
        let result = source::volume_mapping(&mut volume, progress);
        timings.bootstrap_seconds += started.elapsed().as_secs_f64();
        let (geometry, mapping) = result?;
        let reader = source::MappedReader::with_bootstrap(&mut volume, mapping, geometry)?;
        read_index_timed(
            reader,
            &root,
            geometry,
            progress,
            "Experimental NTFS MFT",
            timings,
        )
    })?;
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
